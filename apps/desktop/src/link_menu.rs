//! Right-clicking a link in a note: a menu to copy, edit, remove, or turn it
//! into a citation, and the small form for editing its text and address.

use std::ops::Range;

use eframe::egui::{
    self, vec2, Color32, CornerRadius, FontId, Frame, Id, Key, Margin, Pos2, RichText, Sense,
    Stroke, TextEdit, Ui,
};
use scripture_study_core::{inline, links};

use crate::icons;
use crate::menu::{self, Item};
use crate::theme::{self, Palette};

/// A link in a block's text.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkTarget {
    pub block: usize,
    /// Byte range of the whole link, markup included.
    pub full: Range<usize>,
    /// Byte range of the text it shows.
    pub label: Range<usize>,
    pub url: String,
}

impl LinkTarget {
    /// The link at byte `at` of block `block`'s `text`, if any.
    pub fn at(block: usize, text: &str, at: usize) -> Option<Self> {
        inline::links(text)
            .into_iter()
            .find(|l| l.full.contains(&at))
            .map(|l| Self {
                block,
                full: l.full,
                label: l.range,
                url: l.url,
            })
    }

    /// A bare web address, which is a link because of what it says.
    pub fn is_bare(&self) -> bool {
        self.full == self.label
    }

    /// Whether `text` still has this link where it was.
    pub fn still_in(&self, text: &str) -> bool {
        Self::at(self.block, text, self.full.start).as_ref() == Some(self)
    }

    /// `text` with the link replaced by `with`, and the character just after.
    pub fn replace(&self, text: &str, with: &str) -> (String, usize) {
        let mut out = text.to_string();
        out.replace_range(self.full.clone(), with);
        let end = text[..self.full.start].chars().count() + with.chars().count();
        (out, end)
    }
}

pub enum Choice {
    Copy,
    Edit,
    Remove,
    ConvertToCitation,
}

pub struct LinkMenu {
    pub target: LinkTarget,
    pos: Pos2,
    just_opened: bool,
}

impl LinkMenu {
    pub fn new(target: LinkTarget, pos: Pos2) -> Self {
        Self {
            target,
            pos,
            just_opened: true,
        }
    }

    /// Draws the menu. Returns the chosen item, and whether it stays open.
    pub fn show(&mut self, ctx: &egui::Context, palette: &Palette) -> (Option<Choice>, bool) {
        let bare = self.target.is_bare();
        let (choice, open) = menu::menu_at(
            ctx,
            Id::new("link-menu"),
            self.pos,
            palette,
            std::mem::take(&mut self.just_opened),
            |ui| {
                if Item::new("Copy link address")
                    .icon(icons::copy)
                    .show(ui, palette)
                {
                    return Some(Choice::Copy);
                }
                if Item::new("Edit link").icon(icons::pencil).show(ui, palette) {
                    return Some(Choice::Edit);
                }
                if Item::new("Convert to citation")
                    .icon(icons::quote)
                    .show(ui, palette)
                {
                    return Some(Choice::ConvertToCitation);
                }
                menu::separator(ui, palette);
                let remove = Item::new("Remove link")
                    .icon(icons::unlink)
                    .enabled(!bare)
                    .disabled_hint("A web address typed out in full is always a link");
                if remove.show(ui, palette) {
                    return Some(Choice::Remove);
                }
                None
            },
        );
        let open = open && choice.is_none();
        (choice, open)
    }
}

pub enum FormOutcome {
    Open,
    Cancel,
    /// Save the link as this Markdown (or plain text, if the address was
    /// cleared).
    Save(String),
}

pub struct LinkForm {
    pub target: LinkTarget,
    pub text: String,
    pub url: String,
    focus: bool,
}

impl LinkForm {
    pub fn new(target: LinkTarget, text: &str) -> Self {
        let label = inline::plain_text(&text[target.label.clone()]);
        let label = if target.is_bare() {
            String::new()
        } else {
            label
        };
        Self {
            url: target.url.clone(),
            text: label,
            target,
            focus: true,
        }
    }

    /// What the link becomes: `[text](url)`, or just the text if the address
    /// was cleared. `None` while the address isn't a web address.
    fn result(&self) -> Option<String> {
        if self.url.trim().is_empty() {
            return Some(self.text.trim().to_string());
        }
        let url = links::as_url(&self.url)?;
        Some(links::markdown(&url, Some(self.text.as_str())))
    }

    pub fn show(&mut self, ctx: &egui::Context, palette: &Palette) -> FormOutcome {
        let mut outcome = FormOutcome::Open;
        let modal = egui::Modal::new(Id::new("link-form"))
            .backdrop_color(Color32::from_black_alpha(110))
            .frame(
                Frame::new()
                    .fill(palette.menu_bg)
                    .stroke(Stroke::new(1.0, palette.border))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(22))
                    .shadow(egui::Shadow {
                        offset: [0, 10],
                        blur: 32,
                        spread: 0,
                        color: Color32::from_black_alpha(80),
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
                let title = theme::semibold(ui, "Edit link", 18.0, palette.text);
                let (rect, _) = ui.allocate_exact_size(title.size(), Sense::hover());
                ui.painter().galley(rect.min, title, palette.text);
                ui.add_space(2.0);

                let focus = std::mem::take(&mut self.focus);
                let hint = links::as_url(&self.url)
                    .map(|url| links::label(&links::clean(&url)))
                    .unwrap_or_default();
                field(ui, palette, "Text", &mut self.text, &hint, focus);
                field(ui, palette, "Link", &mut self.url, "https://…", false);
                let note = if self.url.trim().is_empty() {
                    "With no link, the text stays as plain text."
                } else if links::as_url(&self.url).is_none() {
                    "Enter a web address, starting with https://"
                } else {
                    ""
                };
                if !note.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(60.0);
                        ui.label(RichText::new(note).size(12.0).color(palette.faint));
                    });
                }
                ui.add_space(4.0);
                buttons(ui, palette, self.result())
            });
        let close = modal.should_close();
        if let Some(chosen) = modal.inner {
            outcome = chosen;
        }
        if close && matches!(outcome, FormOutcome::Open) {
            outcome = FormOutcome::Cancel;
        }
        if matches!(outcome, FormOutcome::Open) {
            if let Some(result) = self.result() {
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
                    outcome = FormOutcome::Save(result);
                }
            }
        }
        outcome
    }
}

fn field(ui: &mut Ui, palette: &Palette, label: &str, value: &mut String, hint: &str, focus: bool) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(52.0, 30.0), Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.5),
            palette.faint,
        );
        let response = ui.add(
            TextEdit::singleline(value)
                .id(Id::new(("link-field", label)))
                .hint_text(RichText::new(hint).color(palette.subtle))
                .font(FontId::proportional(14.0))
                .desired_width(ui.available_width())
                .margin(Margin::symmetric(8, 6)),
        );
        if focus {
            response.request_focus();
        }
    });
}

fn buttons(ui: &mut Ui, palette: &Palette, result: Option<String>) -> Option<FormOutcome> {
    let mut outcome = None;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let save = egui::Button::new(RichText::new("Save").size(14.0).color(Color32::WHITE))
            .fill(palette.accent)
            .corner_radius(CornerRadius::same(8))
            .min_size(vec2(72.0, 32.0));
        if ui.add_enabled(result.is_some(), save).clicked() {
            outcome = result.map(FormOutcome::Save);
        }
        let cancel = egui::Button::new(RichText::new("Cancel").size(14.0).color(palette.text))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(CornerRadius::same(8))
            .min_size(vec2(0.0, 32.0));
        if ui.add(cancel).clicked() {
            outcome = Some(FormOutcome::Cancel);
        }
    });
    outcome
}
