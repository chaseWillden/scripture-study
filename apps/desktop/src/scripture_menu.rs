//! The scripture picker opened by the `/scripture` command: a search field and
//! a fuzzy list of passages. Choosing one cites it.
//!
//! Drawn with the context menu's popup: the same corner, hairline, and shadow,
//! a frameless search row, and results that highlight like menu items.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver, Sender},
    Arc,
};

use eframe::egui::{
    self, vec2, Align, EventFilter, FontId, Frame, Id, Key, Layout, Margin, Modifiers, Order, Pos2,
    Rect, RichText, Sense, TextEdit, Ui, UiBuilder,
};
use scripture_study_core::scriptures::{self, Hit};

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
const MAX_HITS: usize = 40;

struct SearchRequest {
    revision: u64,
    query: String,
    ctx: egui::Context,
}

struct SearchResult {
    revision: u64,
    hits: Vec<Hit>,
}

pub struct ScripturePicker {
    pub block: usize,
    pub at: usize,
    /// The existing scripture citation being edited, if any.
    pub editing: Option<String>,
    query: String,
    selected: usize,
    anchor: Pos2,
    /// Focus the search field on the frame the picker opens.
    focus: bool,
    scroll: bool,
    hits: Vec<Hit>,
    loading: bool,
    search_on_open: bool,
    search_revision: Arc<AtomicU64>,
    search_tx: Sender<SearchRequest>,
    search_rx: Receiver<SearchResult>,
    pending_enter: bool,
    list_height: f32,
}

impl ScripturePicker {
    pub fn new(block: usize, at: usize, anchor: Pos2) -> Self {
        scriptures::warm();
        let (search_tx, requests) = mpsc::channel::<SearchRequest>();
        let (results, search_rx) = mpsc::channel();
        let search_revision = Arc::new(AtomicU64::new(0));
        let latest = search_revision.clone();
        // One worker per picker. Coalesce queued queries and never publish an
        // obsolete result, even when the next query used the immediate path.
        std::thread::spawn(move || {
            while let Ok(mut request) = requests.recv() {
                while let Ok(newer) = requests.try_recv() {
                    request = newer;
                }
                if request.revision != latest.load(Ordering::Relaxed) {
                    continue;
                }
                let hits = scriptures::search_limited(&request.query, MAX_HITS);
                if request.revision == latest.load(Ordering::Relaxed) {
                    if results
                        .send(SearchResult {
                            revision: request.revision,
                            hits,
                        })
                        .is_err()
                    {
                        break;
                    }
                    request.ctx.request_repaint();
                }
            }
        });
        Self {
            block,
            at,
            editing: None,
            query: String::new(),
            selected: 0,
            anchor,
            focus: true,
            scroll: false,
            hits: Vec::new(),
            loading: false,
            search_on_open: false,
            search_revision,
            search_tx,
            search_rx,
            pending_enter: false,
            list_height: HINT_HEIGHT,
        }
    }

    pub fn for_edit(reference: String, anchor: Pos2) -> Self {
        let mut picker = Self::new(0, 0, anchor);
        picker.editing = Some(reference.clone());
        picker.query = reference;
        picker.search_on_open = true;
        picker
    }

    fn update_query(&mut self, ctx: &egui::Context) {
        let revision = self.search_revision.fetch_add(1, Ordering::Relaxed) + 1;
        self.pending_enter = false;
        self.selected = 0;
        self.scroll = true;
        if self.query.trim().is_empty() {
            self.hits.clear();
            self.loading = false;
        } else if let Some(hits) = scriptures::reference_search_if_ready(&self.query, MAX_HITS) {
            self.hits = hits;
            self.loading = false;
        } else {
            // Keep the current rows and popup height while the worker filters.
            self.loading = true;
            let _ = self.search_tx.send(SearchRequest {
                revision,
                query: self.query.clone(),
                ctx: ctx.clone(),
            });
        }
    }

    fn receive_results(&mut self) {
        while let Ok(result) = self.search_rx.try_recv() {
            if result.revision == self.search_revision.load(Ordering::Relaxed) {
                self.hits = result.hits;
                self.selected = 0;
                self.scroll = true;
                self.loading = false;
            }
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

        let style = menu::popup_style(ctx, palette);
        let area = egui::Area::new(Id::new(("scripture-menu", note_id)))
            .order(Order::Foreground)
            .fixed_pos(self.anchor)
            .constrain(true)
            .show(ctx, |ui| {
                ui.set_style(style.clone());
                Frame::popup(&style).show(ui, |ui| {
                    ui.set_width(WIDTH);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let changed = self.search_field(ui, note_id, palette);
                    if changed || std::mem::take(&mut self.search_on_open) {
                        self.update_query(ui.ctx());
                    }
                    self.receive_results();
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
                    menu::separator(ui, palette);
                    clicked = self.list(ui, palette);
                });
            });

        if enter && self.loading {
            self.pending_enter = true;
        }
        if let Some(index) = clicked.filter(|_| !self.loading) {
            outcome = Outcome::Insert(self.hits[index].clone());
        } else if !self.loading && (enter || self.pending_enter) && !self.hits.is_empty() {
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
            .id(Id::new(("scripture-search", note_id)))
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .desired_width(field.width())
            .font(FontId::proportional(FONT))
            .text_color(palette.text)
            .hint_text(
                RichText::new("Filter by reference or words")
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

    fn list(&mut self, ui: &mut Ui, palette: &Palette) -> Option<usize> {
        ui.set_min_height(self.list_height);
        if self.query.trim().is_empty() {
            hint(ui, palette, "1 Nephi 1:1,3 or Ether 2:1-4");
            return None;
        }
        if self.loading && self.hits.is_empty() {
            hint(ui, palette, "Searching scriptures…");
            return None;
        }
        if self.hits.is_empty() {
            hint(ui, palette, "No matching scriptures");
            return None;
        }
        let mut clicked = None;
        // The area keeps last frame's size, and a scroll area will shrink to
        // fit that instead of asking it to grow. Reserve the list's height so
        // a short empty picker can open up once there are results.
        let spacing = 1.0;
        let full = self.hits.len() as f32 * ROW_HEIGHT + (self.hits.len() - 1) as f32 * spacing;
        self.list_height = self.list_height.max(full.min(MAX_LIST));
        let height = self.list_height;
        if ui.available_height() + 1.0 < height {
            ui.ctx().request_repaint();
        }
        ui.set_min_height(height);
        egui::ScrollArea::vertical()
            .max_height(height)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = spacing;
                for (n, hit) in self.hits.iter().enumerate() {
                    let label = &hit.label;
                    let preview = &hit.preview;
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
                    icons::quote(
                        ui.painter(),
                        egui::pos2(rect.left() + ICON_X, rect.center().y),
                        icon_color,
                    );
                    let text_width = rect.right() - TEXT_RIGHT - (rect.left() + TEXT_LEFT);
                    let label_galley =
                        crate::sidebar::elided(ui, label, FONT, palette.text, text_width);
                    let preview_galley =
                        crate::sidebar::elided(ui, preview, PREVIEW, palette.faint, text_width);
                    let block = label_galley.size().y + 1.0 + preview_galley.size().y;
                    let top = rect.center().y - block / 2.0;
                    let x = rect.left() + TEXT_LEFT;
                    ui.painter()
                        .galley(egui::pos2(x, top), label_galley, palette.text);
                    ui.painter().galley(
                        egui::pos2(x, top + block - preview_galley.size().y),
                        preview_galley,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(label: &str) -> Hit {
        Hit {
            label: label.into(),
            text: "Verse text".into(),
            preview: "Verse preview".into(),
        }
    }

    #[test]
    fn pending_word_search_keeps_rows_and_ignores_outdated_results() {
        // Replace the worker boundary with controlled channels so completion
        // order is deterministic, including a late result for an old query.
        let mut picker = ScripturePicker::new(0, 0, Pos2::ZERO);
        let (requests, request_rx) = mpsc::channel();
        let (results, result_rx) = mpsc::channel();
        picker.search_tx = requests;
        picker.search_rx = result_rx;
        picker.hits = vec![hit("Ether 2:1")];
        picker.list_height = MAX_LIST;
        let ctx = egui::Context::default();
        picker.query = "and the".into();
        picker.update_query(&ctx);
        let old = request_rx.recv().unwrap();
        picker.query = "and the earth".into();
        picker.update_query(&ctx);
        let latest = request_rx.recv().unwrap();
        assert_eq!(picker.hits[0].label, "Ether 2:1");
        assert_eq!(picker.list_height, MAX_LIST);
        assert!(picker.loading);

        results
            .send(SearchResult {
                revision: old.revision,
                hits: vec![hit("Old result")],
            })
            .unwrap();
        picker.receive_results();
        assert_eq!(picker.hits[0].label, "Ether 2:1");
        assert!(picker.loading);

        results
            .send(SearchResult {
                revision: latest.revision,
                hits: vec![hit("Genesis 1:1")],
            })
            .unwrap();
        picker.receive_results();
        assert_eq!(picker.hits[0].label, "Genesis 1:1");
        assert!(!picker.loading);

        picker.query.clear();
        picker.update_query(&ctx);
        results
            .send(SearchResult {
                revision: latest.revision,
                hits: vec![hit("Late result")],
            })
            .unwrap();
        picker.receive_results();
        assert!(picker.hits.is_empty());
        assert!(!picker.loading);
    }
}
