//! The document picker opened by the `/file-link` command: a search field and
//! a list of notes. Choosing one inserts a link to that note.
//!
//! Drawn with the same popup as the scripture picker.

use eframe::egui::{
    self, vec2, Align, EventFilter, FontId, Frame, Id, Key, Layout, Margin, Modifiers, Order, Pos2,
    Rect, RichText, Sense, TextEdit, Ui, UiBuilder,
};
use scripture_study_core::NoteMeta;

use crate::icons;
use crate::menu;
use crate::theme::Palette;

const WIDTH: f32 = 360.0;
const SEARCH_HEIGHT: f32 = 28.0;
const ROW_HEIGHT: f32 = 44.0;
const HINT_HEIGHT: f32 = 28.0;
const MAX_LIST: f32 = 320.0;
const FONT: f32 = 13.5;
const PREVIEW: f32 = 12.0;
const ICON_X: f32 = 16.0;
const TEXT_LEFT: f32 = 32.0;
const TEXT_RIGHT: f32 = 10.0;

pub struct FilePicker {
    pub block: usize,
    pub at: usize,
    query: String,
    selected: usize,
    anchor: Pos2,
    /// Focus the search field on the frame the picker opens.
    focus: bool,
    scroll: bool,
}

/// A note chosen from the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub title: String,
}

impl FilePicker {
    pub fn new(block: usize, at: usize, anchor: Pos2) -> Self {
        Self {
            block,
            at,
            query: String::new(),
            selected: 0,
            anchor,
            focus: true,
            scroll: false,
        }
    }
}

pub enum Outcome {
    Open,
    Cancel,
    Insert(Choice),
}

impl FilePicker {
    pub fn show(
        &mut self,
        ui: &Ui,
        note_id: &str,
        notes: &[NoteMeta],
        palette: &Palette,
    ) -> Outcome {
        let mut outcome = Outcome::Open;
        let ctx = ui.ctx();
        let up = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp));
        let down = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
        let enter = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let escape = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let just_opened = self.focus;
        let mut chosen = None;

        let style = menu::popup_style(ctx, palette);
        let area = egui::Area::new(Id::new(("file-link-menu", note_id)))
            .order(Order::Foreground)
            .fixed_pos(self.anchor)
            .constrain(true)
            .show(ctx, |ui| {
                ui.set_style(style.clone());
                Frame::popup(&style)
                    .show(ui, |ui| {
                        ui.set_width(WIDTH);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        if self.search_field(ui, note_id, palette) {
                            self.selected = 0;
                            self.scroll = true;
                        }
                        let matches = filter_notes(notes, &self.query);
                        if !matches.is_empty() {
                            let n = matches.len();
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
                        menu::separator(ui, palette);
                        let clicked = self.list(ui, palette, &matches);
                        if let Some(index) = clicked {
                            chosen = Some(choice(matches[index]));
                        } else if enter && !matches.is_empty() {
                            chosen = Some(choice(matches[self.selected]));
                        }
                    })
                    .inner;
            });

        if let Some(choice) = chosen {
            outcome = Outcome::Insert(choice);
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

    /// The search row: a menu item with a magnifying glass and no field chrome.
    /// Returns whether the query changed.
    fn search_field(&mut self, ui: &mut Ui, note_id: &str, palette: &Palette) -> bool {
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), SEARCH_HEIGHT), Sense::hover());
        icons::search(
            ui.painter(),
            egui::pos2(rect.left() + ICON_X, rect.center().y),
            palette.faint,
        );
        let field = Rect::from_min_max(
            egui::pos2(rect.left() + TEXT_LEFT, rect.top()),
            egui::pos2(rect.right() - TEXT_RIGHT, rect.bottom()),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(field)
                .layout(Layout::left_to_right(Align::Center)),
        );
        let search = TextEdit::singleline(&mut self.query)
            .id(Id::new(("file-link-search", note_id)))
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .desired_width(field.width())
            .font(FontId::proportional(FONT))
            .text_color(palette.text)
            .hint_text(
                RichText::new("Filter documents")
                    .size(FONT)
                    .color(palette.faint),
            )
            .event_filter(EventFilter {
                tab: false,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            })
            .show(&mut child);
        if self.focus {
            search.response.request_focus();
            self.focus = false;
        }
        search.response.changed()
    }

    fn list(&mut self, ui: &mut Ui, palette: &Palette, matches: &[&NoteMeta]) -> Option<usize> {
        if matches.is_empty() {
            let text = if self.query.trim().is_empty() {
                "No documents"
            } else {
                "No matching documents"
            };
            hint(ui, palette, text);
            return None;
        }
        let mut clicked = None;
        let rows: Vec<(String, String)> = matches
            .iter()
            .map(|note| (note.title.clone(), note.folder().to_string()))
            .collect();
        // The area keeps last frame's size, and a scroll area will shrink to
        // fit that instead of asking it to grow. Reserve the list's height so
        // a short empty picker can open up once there are results.
        let spacing = 1.0;
        let full = rows.len() as f32 * ROW_HEIGHT + (rows.len().saturating_sub(1)) as f32 * spacing;
        let height = full.min(MAX_LIST);
        if ui.available_height() + 1.0 < height {
            ui.ctx().request_repaint();
        }
        ui.set_min_height(height);
        egui::ScrollArea::vertical()
            .max_height(height)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = spacing;
                for (n, (label, folder)) in rows.iter().enumerate() {
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
                    let selected = n == self.selected;
                    if selected {
                        ui.painter().rect_filled(rect, 6.0, palette.menu_selected);
                        if std::mem::take(&mut self.scroll) {
                            response.scroll_to_me(None);
                        }
                    }
                    let icon_color = if selected {
                        palette.text
                    } else {
                        palette.faint
                    };
                    icons::page(
                        ui.painter(),
                        egui::pos2(rect.left() + ICON_X, rect.center().y),
                        icon_color,
                    );
                    let text_width = rect.right() - TEXT_RIGHT - (rect.left() + TEXT_LEFT);
                    let label_galley =
                        crate::sidebar::elided(ui, label, FONT, palette.text, text_width);
                    let x = rect.left() + TEXT_LEFT;
                    if folder.is_empty() {
                        ui.painter().galley(
                            egui::pos2(x, rect.center().y - label_galley.size().y / 2.0),
                            label_galley,
                            palette.text,
                        );
                    } else {
                        let preview_galley =
                            crate::sidebar::elided(ui, folder, PREVIEW, palette.faint, text_width);
                        let block = label_galley.size().y + 1.0 + preview_galley.size().y;
                        let top = rect.center().y - block / 2.0;
                        ui.painter()
                            .galley(egui::pos2(x, top), label_galley, palette.text);
                        ui.painter().galley(
                            egui::pos2(x, top + block - preview_galley.size().y),
                            preview_galley,
                            palette.faint,
                        );
                    }
                    if response.clicked() {
                        clicked = Some(n);
                    }
                }
            });
        clicked
    }
}

fn choice(note: &NoteMeta) -> Choice {
    Choice {
        id: note.id.clone(),
        title: note.title.clone(),
    }
}

/// A faint line where results will be, in the same place as a menu label.
fn hint(ui: &mut Ui, palette: &Palette, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HINT_HEIGHT), Sense::hover());
    let width = rect.right() - TEXT_RIGHT - (rect.left() + TEXT_LEFT);
    let galley = crate::sidebar::elided(ui, text, FONT, palette.faint, width);
    ui.painter().galley(
        egui::pos2(
            rect.left() + TEXT_LEFT,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        palette.faint,
    );
}

/// Notes matching `query`, best match first. An empty query keeps `notes`'s
/// order (most recently modified first).
fn filter_notes<'a>(notes: &'a [NoteMeta], query: &str) -> Vec<&'a NoteMeta> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return notes.iter().collect();
    }
    let mut scored: Vec<(i32, usize, &NoteMeta)> = notes
        .iter()
        .enumerate()
        .filter_map(|(i, note)| {
            let title = note.title.to_lowercase();
            let folder = note.folder().to_lowercase();
            let score = match_text(&query, &title)
                .max(match_text(&query, &folder).map(|s| s.saturating_sub(10)));
            score.map(|s| (s, i, note))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, note)| note).collect()
}

/// Scores how well `query` matches `text` (both lowercase).
fn match_text(query: &str, text: &str) -> Option<i32> {
    if text.is_empty() {
        return None;
    }
    if text == query {
        return Some(100);
    }
    if text.starts_with(query) {
        return Some(80);
    }
    if text.split_whitespace().any(|w| w.starts_with(query)) {
        return Some(60);
    }
    if text.contains(query) {
        return Some(40);
    }
    let mut chars = text.chars();
    let query = query.replace(' ', "");
    query.chars().all(|q| chars.any(|c| c == q)).then_some(20)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn note(id: &str, title: &str) -> NoteMeta {
        NoteMeta {
            id: id.into(),
            title: title.into(),
            modified: SystemTime::UNIX_EPOCH,
            created: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn empty_query_keeps_order_and_typing_ranks_titles() {
        let notes = vec![
            note("journal", "Journal"),
            note("My Notes/grace", "Grace"),
            note("plans/study", "Study plan"),
        ];
        let all = filter_notes(&notes, "");
        assert_eq!(all[0].id, "journal");
        assert_eq!(filter_notes(&notes, "gra")[0].title, "Grace");
        assert_eq!(filter_notes(&notes, "plans")[0].title, "Study plan");
        assert!(filter_notes(&notes, "zzz").is_empty());
    }
}
