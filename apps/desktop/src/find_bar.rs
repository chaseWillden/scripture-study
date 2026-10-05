//! The find bar (Cmd+F): search the open note, exactly or fuzzily, with
//! every match highlighted and the current one scrolled into view.

use eframe::egui::{
    self, pos2, vec2, Color32, CornerRadius, FontId, Frame, Id, Key, Margin, Modifiers, Order,
    Pos2, Rect, RichText, Sense, Shadow, Stroke, TextEdit, Ui,
};
use scripture_study_core::find::{self, Match};
use scripture_study_core::Document;

use crate::icons;
use crate::theme::Palette;

const WIDTH: f32 = 380.0;
const BUTTON: f32 = 24.0;

#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    pub query: String,
    pub fuzzy: bool,
    pub matches: Vec<Match>,
    /// Index into `matches` of the highlighted one.
    pub current: usize,
    /// Scroll the current match into view on the next frame.
    pub reveal: bool,
    /// Focus (and select) the query field on the next frame.
    focus: bool,
    /// Give the field its focus back (after a button click) without
    /// selecting the query.
    refocus: bool,
}

/// Background for all matches, and for the current one.
pub fn match_colors(ui: &Ui) -> (Color32, Color32) {
    if ui.visuals().dark_mode {
        (
            Color32::from_rgba_unmultiplied(0xE8, 0xB3, 0x39, 0x46),
            Color32::from_rgba_unmultiplied(0xF0, 0x9A, 0x1A, 0xB0),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(0xFF, 0xD5, 0x4A, 0x70),
            Color32::from_rgba_unmultiplied(0xFF, 0x9F, 0x1C, 0xB0),
        )
    }
}

impl FindBar {
    /// Opens the bar, or re-focuses it (selecting the query) if it's open.
    pub fn open(&mut self) {
        self.open = true;
        self.focus = true;
        self.reveal = true;
    }

    pub fn current_match(&self) -> Option<Match> {
        self.matches.get(self.current).copied()
    }

    /// Re-runs the search against the note as it is now.
    pub fn update(&mut self, doc: &Document) {
        if !self.open {
            self.matches.clear();
            return;
        }
        self.matches = find::find(doc, &self.query, self.fuzzy);
        self.current = self.current.min(self.matches.len().saturating_sub(1));
    }

    fn step(&mut self, forward: bool) {
        let n = self.matches.len();
        if n > 0 {
            self.current = if forward {
                (self.current + 1) % n
            } else {
                (self.current + n - 1) % n
            };
            self.reveal = true;
        }
    }

    /// Draws the bar with its right edge at `top_right`. Returns the match to
    /// select in the note when the bar closes with one highlighted.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        top_right: Pos2,
        palette: &Palette,
    ) -> Option<Match> {
        if !self.open {
            return None;
        }
        let mut close = false;
        egui::Area::new(Id::new("find-bar"))
            .order(Order::Foreground)
            .fixed_pos(top_right - vec2(WIDTH, 0.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(palette.menu_bg)
                    .stroke(Stroke::new(1.0, palette.border))
                    .corner_radius(CornerRadius::same(8))
                    .shadow(Shadow {
                        offset: [0, 4],
                        blur: 16,
                        spread: 0,
                        color: Color32::from_black_alpha(36),
                    })
                    .inner_margin(Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH - 16.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            close = self.contents(ui, palette);
                        });
                    });
            });
        if !close {
            return None;
        }
        self.open = false;
        let selected = self.current_match();
        self.matches.clear();
        selected
    }

    /// The field, count, and buttons. Returns true to close.
    fn contents(&mut self, ui: &mut Ui, palette: &Palette) -> bool {
        let id = Id::new("find-query");
        let (icon, _) = ui.allocate_exact_size(vec2(18.0, BUTTON), Sense::hover());
        icons::search(ui.painter(), icon.center(), palette.faint);

        // Keys while typing in the field.
        let mut close = false;
        if ui.memory(|m| m.has_focus(id)) {
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::SHIFT, Key::Enter)
                    || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::G)
                {
                    self.step(false);
                }
                if i.consume_key(Modifiers::NONE, Key::Enter)
                    || i.consume_key(Modifiers::COMMAND, Key::G)
                {
                    self.step(true);
                }
                close = i.consume_key(Modifiers::NONE, Key::Escape);
            });
        }

        let output = TextEdit::singleline(&mut self.query)
            .id(id)
            .frame(Frame::NONE)
            .margin(Margin::symmetric(0, 3))
            .desired_width(170.0)
            .font(FontId::proportional(13.5))
            .text_color(palette.text)
            .hint_text(
                RichText::new("Find in note")
                    .size(13.5)
                    .color(palette.subtle),
            )
            .return_key(None)
            .event_filter(egui::EventFilter {
                escape: true,
                vertical_arrows: true,
                ..Default::default()
            })
            .show(ui);
        output.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Find in note")
        });
        if output.response.changed() {
            self.current = 0;
            self.reveal = true;
        }
        if std::mem::take(&mut self.refocus) {
            output.response.request_focus();
        }
        if std::mem::take(&mut self.focus) {
            output.response.request_focus();
            // Select what's there so typing replaces the last search.
            let mut state = output.state;
            let all = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(self.query.chars().count()),
            );
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), id);
        }

        let count = match (self.query.trim().is_empty(), self.matches.len()) {
            (true, _) => String::new(),
            (false, 0) => "No results".to_string(),
            (false, n) => format!("{} of {n}", self.current + 1),
        };
        let count = ui
            .painter()
            .layout_no_wrap(count, FontId::proportional(12.0), palette.faint);
        let (rect, response) = ui.allocate_exact_size(vec2(62.0, BUTTON), Sense::hover());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, count.text()));
        ui.painter().galley(
            pos2(
                rect.right() - count.size().x,
                rect.center().y - count.size().y / 2.0,
            ),
            count,
            palette.faint,
        );

        if self.fuzzy_toggle(ui, palette).clicked() {
            self.fuzzy = !self.fuzzy;
            self.current = 0;
            self.reveal = true;
            self.refocus = true;
        }
        if icon_button(ui, "Previous match", palette, |p, c, color| {
            arrow(p, c, color, true)
        })
        .clicked()
        {
            self.step(false);
            self.refocus = true;
        }
        if icon_button(ui, "Next match", palette, |p, c, color| {
            arrow(p, c, color, false)
        })
        .clicked()
        {
            self.step(true);
            self.refocus = true;
        }
        if icon_button(ui, "Close find", palette, icons::close).clicked() {
            close = true;
        }
        close
    }

    /// A pill that reads "Fuzzy" and fills with the accent color when on.
    fn fuzzy_toggle(&self, ui: &mut Ui, palette: &Palette) -> egui::Response {
        let text_color = if self.fuzzy {
            Color32::WHITE
        } else {
            palette.faint
        };
        let text =
            ui.painter()
                .layout_no_wrap("Fuzzy".into(), FontId::proportional(12.0), text_color);
        let (rect, response) =
            ui.allocate_exact_size(vec2(text.size().x + 16.0, BUTTON - 4.0), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, self.fuzzy, "Fuzzy")
        });
        let (fill, stroke) = if self.fuzzy {
            (palette.accent, Stroke::NONE)
        } else if response.hovered() {
            (palette.hover, Stroke::new(1.0, palette.border))
        } else {
            (Color32::TRANSPARENT, Stroke::new(1.0, palette.border))
        };
        ui.painter().rect(
            rect,
            rect.height() / 2.0,
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );
        let pos = rect.center() - text.size() / 2.0;
        ui.painter().galley(pos, text, text_color);
        response.on_hover_text("Fuzzy matching: also finds near-misses and typos")
    }
}

fn icon_button(
    ui: &mut Ui,
    label: &str,
    palette: &Palette,
    paint: impl Fn(&egui::Painter, Pos2, Color32),
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let color = if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, palette.hover);
        palette.text
    } else {
        palette.faint
    };
    paint(ui.painter(), rect.center(), color);
    response.on_hover_text(label)
}

/// A small chevron pointing up or down.
fn arrow(painter: &egui::Painter, center: Pos2, color: Color32, up: bool) {
    let d = if up { -1.0 } else { 1.0 };
    painter.line(
        vec![
            center + vec2(-4.0, -2.0 * d),
            center + vec2(0.0, 2.0 * d),
            center + vec2(4.0, -2.0 * d),
        ],
        Stroke::new(1.4, color),
    );
}

/// The on-screen rectangle of characters `start..end` on their first line, for
/// scrolling a match into view.
pub fn match_rect(galley: &egui::Galley, origin: Pos2, start: usize) -> Rect {
    galley
        .pos_from_cursor(egui::text::CCursor::new(start))
        .translate(origin.to_vec2())
}
