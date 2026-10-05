//! The scripture picker opened by the `/scripture` command: a search field and
//! a fuzzy list of passages. Choosing one cites it.

use eframe::egui::{
    self, vec2, Color32, CornerRadius, EventFilter, FontId, Frame, Id, Key, Margin, Modifiers,
    Order, Pos2, Rect, RichText, Sense, Shadow, Stroke, TextEdit, Ui,
};
use scripture_study_core::scriptures::{self, Hit};

use crate::theme::Palette;

const WIDTH: f32 = 440.0;
const ROW_HEIGHT: f32 = 46.0;
const MAX_LIST: f32 = 320.0;

pub struct ScripturePicker {
    pub block: usize,
    pub at: usize,
    query: String,
    selected: usize,
    anchor: Pos2,
    /// Focus the search field on the frame the picker opens.
    focus: bool,
    scroll: bool,
    hits: Vec<Hit>,
}

impl ScripturePicker {
    pub fn new(block: usize, at: usize, anchor: Pos2) -> Self {
        Self {
            block,
            at,
            query: String::new(),
            selected: 0,
            anchor,
            focus: true,
            scroll: false,
            hits: Vec::new(),
        }
    }
}

pub enum Outcome {
    Open,
    Cancel,
    Insert(Hit),
}

impl ScripturePicker {
    pub fn show(&mut self, ui: &Ui, note_id: &str, palette: &Palette) -> Outcome {
        let mut outcome = Outcome::Open;
        let ctx = ui.ctx();
        let up = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp));
        let down = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
        let enter = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let escape = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let just_opened = self.focus;
        let mut clicked = None;

        let area = egui::Area::new(Id::new(("scripture-menu", note_id)))
            .order(Order::Foreground)
            .fixed_pos(self.anchor)
            .constrain(true)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(palette.menu_bg)
                    .stroke(Stroke::new(1.0, palette.border))
                    .corner_radius(CornerRadius::same(8))
                    .shadow(Shadow {
                        offset: [0, 6],
                        blur: 20,
                        spread: 0,
                        color: Color32::from_black_alpha(40),
                    })
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH);
                        ui.label(
                            RichText::new("Scripture")
                                .strong()
                                .size(13.0)
                                .color(palette.text),
                        );
                        ui.add_space(4.0);
                        let search = TextEdit::singleline(&mut self.query)
                            .id(Id::new(("scripture-search", note_id)))
                            .hint_text("Filter by reference or words")
                            .desired_width(WIDTH)
                            .event_filter(EventFilter {
                                tab: false,
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                escape: true,
                            })
                            .show(ui);
                        if self.focus {
                            search.response.request_focus();
                            self.focus = false;
                        }
                        if search.response.changed() {
                            self.selected = 0;
                            self.scroll = true;
                            self.hits = scriptures::search(&self.query);
                        }
                        if !self.hits.is_empty() {
                            let n = self.hits.len();
                            self.selected = self.selected.min(n - 1);
                            if up {
                                self.selected = (self.selected + n - 1) % n;
                                self.scroll = true;
                            }
                            if down {
                                self.selected = (self.selected + 1) % n;
                                self.scroll = true;
                            }
                        }
                        ui.add_space(6.0);
                        clicked = self.list(ui, palette);
                    });
            });

        if let Some(index) = clicked {
            outcome = Outcome::Insert(self.hits[index].clone());
        } else if enter && !self.hits.is_empty() {
            outcome = Outcome::Insert(self.hits[self.selected].clone());
        }
        let rect = area.response.rect;
        let pressed_outside = ctx.input(|i| {
            i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))
        });
        if escape || (!just_opened && pressed_outside) {
            outcome = Outcome::Cancel;
        }
        outcome
    }

    fn list(&mut self, ui: &mut Ui, palette: &Palette) -> Option<usize> {
        if self.query.trim().is_empty() {
            ui.label(
                RichText::new("1 Nephi 1:11 or Ether 2:1-4")
                    .size(13.0)
                    .color(palette.faint),
            );
            return None;
        }
        if self.hits.is_empty() {
            ui.label(
                RichText::new("No matching scriptures")
                    .size(13.0)
                    .color(palette.faint),
            );
            return None;
        }
        let mut clicked = None;
        let rows: Vec<(String, String)> = self
            .hits
            .iter()
            .map(|hit| (hit.label.clone(), hit.preview.clone()))
            .collect();
        egui::ScrollArea::vertical()
            .max_height(MAX_LIST)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (n, (label, preview)) in rows.iter().enumerate() {
                    let (rect, response) = ui.allocate_exact_size(
                        vec2(ui.available_width(), ROW_HEIGHT),
                        Sense::click(),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
                    });
                    if response.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                        self.selected = n;
                    }
                    if n == self.selected {
                        ui.painter().rect_filled(rect, 5.0, palette.menu_selected);
                        if std::mem::take(&mut self.scroll) {
                            response.scroll_to_me(None);
                        }
                    }
                    ui.painter().text(
                        rect.left_top() + vec2(8.0, 6.0),
                        egui::Align2::LEFT_TOP,
                        label,
                        FontId::proportional(14.0),
                        palette.text,
                    );
                    let preview_rect = Rect::from_min_size(
                        rect.left_top() + vec2(8.0, 24.0),
                        vec2(rect.width() - 16.0, 16.0),
                    );
                    ui.painter().with_clip_rect(preview_rect).text(
                        preview_rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        preview,
                        FontId::proportional(12.0),
                        palette.faint,
                    );
                    if response.clicked() {
                        clicked = Some(n);
                    }
                }
            });
        clicked
    }
}

/// Keeps a space that was typed before `/` when a slash command is removed
/// from the end of a block. Returns the caret, in characters.
pub fn restore_space(text: &mut String, at: usize, space: &str) -> usize {
    if space.is_empty() {
        return at;
    }
    let byte = scripture_study_core::editor::char_to_byte(text, at);
    if text[..byte].ends_with(space) {
        return at;
    }
    text.insert_str(byte, space);
    at + space.chars().count()
}
