//! The properties row at the top of a note: colored tag chips, people,
//! custom fields, when the note was created and last edited, and whether
//! it's synced, all on one compact line.

use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{
    self, pos2, vec2, Align, Color32, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, Rect,
    RichText, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder,
};
use scripture_study_core::properties::{self, Properties};

use crate::google_drive::NoteSync;
use crate::icons;
use crate::theme::{self, Palette};

/// Height of every item on the row: chips, buttons, and fields.
const ITEM_H: f32 = 24.0;
/// Space between groups (tags, people, custom fields).
const GROUP_GAP: f32 = 16.0;
const FONT: f32 = 13.0;
const FIELD_MARGIN_Y: f32 = 2.0;

pub struct NoteTimes {
    pub created: SystemTime,
    pub updated: SystemTime,
    /// Shown beside the dates while a connector is syncing notes.
    pub sync: Option<NoteSync>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    Tags,
    People,
}

impl Field {
    /// (accessible name, empty-state button, one item, input hint)
    fn words(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Field::Tags => ("Tags", "Add tags", "tag", "Tag name"),
            Field::People => ("Mentions", "Add people", "mention", "Name"),
        }
    }

    fn icon(self) -> fn(&egui::Painter, egui::Pos2, Color32) {
        match self {
            Field::Tags => icons::tag,
            Field::People => icons::person,
        }
    }
}

#[derive(Default)]
pub struct MetaEditor {
    tags: String,
    mentions: String,
    /// The group whose add-field is open.
    open: Option<Field>,
    /// Focus that field on the next frame (it doesn't exist until then).
    focus_open: bool,
    naming: bool,
    name: String,
    focus_name: bool,
    /// A property just added by name; its value field gets the caret next.
    focus_value: Option<String>,
}

impl MetaEditor {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        properties: &mut Properties,
        times: NoteTimes,
        palette: &Palette,
        note_id: &str,
    ) -> bool {
        let mut changed = false;
        let top = ui.cursor().min;
        let width = ui.available_width();

        // Dates sit at the right end of the first line.
        let (short, full) = date_texts(&times);
        let dates = ui
            .painter()
            .layout_no_wrap(short, FontId::proportional(12.5), palette.faint);
        let badge = times.sync.as_ref().map(|sync| {
            let (short, _, _) = sync_texts(sync);
            let galley = ui.painter().layout_no_wrap(
                short.to_string(),
                FontId::proportional(12.5),
                palette.faint,
            );
            // Room for the widest state, so a change of state doesn't
            // move the dates. Icon, text, and a gap before the dates.
            let widest = ui
                .painter()
                .layout_no_wrap(
                    sync_texts(&NoteSync::Pending).0.to_string(),
                    FontId::proportional(12.5),
                    palette.faint,
                )
                .size()
                .x;
            let width = 20.0 + widest.max(galley.size().x) + 14.0;
            (galley, width)
        });
        let badge_w = badge.as_ref().map_or(0.0, |(_, w)| *w);
        let dates_w = dates.size().x + 20.0 + badge_w;
        // In a narrow window the dates get their own line above the rest,
        // instead of squeezing (or overlapping) the tags.
        let stacked = width - dates_w - GROUP_GAP < 260.0;
        let (dates_rect, left) = if stacked {
            let dates_rect = Rect::from_min_size(top, vec2(dates_w, ITEM_H));
            ui.allocate_rect(dates_rect, Sense::hover());
            ui.add_space(4.0);
            let below = pos2(top.x, top.y + ITEM_H + 4.0 + ui.spacing().item_spacing.y);
            (dates_rect, Rect::from_min_size(below, vec2(width, 400.0)))
        } else {
            let dates_rect =
                Rect::from_min_size(pos2(top.x + width - dates_w, top.y), vec2(dates_w, ITEM_H));
            // Room to wrap onto more lines; only the used part is allocated.
            (
                dates_rect,
                Rect::from_min_size(top, vec2(width - dates_w - GROUP_GAP, 400.0)),
            )
        };
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(left)
                // Every item is ITEM_H tall, so top alignment lines them up.
                .layout(Layout::left_to_right(Align::Min).with_main_wrap(true)),
            |ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
                changed |=
                    self.token_group(ui, Field::Tags, &mut properties.tags, palette, note_id);
                ui.add_space(GROUP_GAP);
                changed |= self.token_group(
                    ui,
                    Field::People,
                    &mut properties.mentions,
                    palette,
                    note_id,
                );

                let mut removed = None;
                for (key, value) in &mut properties.extra {
                    ui.add_space(GROUP_GAP);
                    let before = value.clone();
                    let focus = self.focus_value.as_deref() == Some(key.as_str());
                    extra_field(ui, key, value, focus, palette, note_id, &mut removed);
                    if focus {
                        self.focus_value = None;
                    }
                    changed |= *value != before;
                }
                if let Some(key) = removed {
                    properties.remove_extra(&key);
                    changed = true;
                }

                ui.add_space(GROUP_GAP);
                changed |= self.name_field(ui, properties, palette, note_id);
            },
        );

        let (badge_rect, dates_rect) =
            dates_rect.split_left_right_at_x(dates_rect.left() + badge_w);
        if let (Some((galley, _)), Some(sync)) = (badge, &times.sync) {
            let (_, label, detail) = sync_texts(sync);
            let response = ui.interact(badge_rect, Id::new(("meta-sync", note_id)), Sense::hover());
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
            let (icon, text) = match sync {
                NoteSync::Synced | NoteSync::Pending => (palette.faint, palette.faint),
                NoteSync::Syncing => (palette.accent, palette.faint),
                NoteSync::Failed(_) => (ERROR, ERROR),
            };
            icons::cloud(
                ui.painter(),
                badge_rect.left_center() + vec2(8.0, 0.0),
                icon,
                *sync == NoteSync::Synced,
            );
            let pos = pos2(
                badge_rect.left() + 20.0,
                badge_rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(pos, galley, text);
            response.on_hover_text(detail);
        }

        let dates_response =
            ui.interact(dates_rect, Id::new(("meta-dates", note_id)), Sense::hover());
        dates_response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &full));
        icons::clock(
            ui.painter(),
            dates_rect.left_center() + vec2(7.0, 0.0),
            palette.faint,
        );
        let text_pos = pos2(
            dates_rect.left() + 18.0,
            dates_rect.center().y - dates.size().y / 2.0,
        );
        ui.painter().galley(text_pos, dates, palette.faint);
        dates_response.on_hover_text(full);

        ui.add_space(20.0);
        changed
    }

    /// Chips for the tags (or people), then either a "+" or the open field.
    /// With none yet, a single "Add tags" / "Add people" button.
    fn token_group(
        &mut self,
        ui: &mut Ui,
        field: Field,
        tokens: &mut Vec<String>,
        palette: &Palette,
        note_id: &str,
    ) -> bool {
        let (label, empty, item, hint) = field.words();
        let open = self.open == Some(field);
        if tokens.is_empty() && !open {
            if ghost_button(ui, Some(field.icon()), empty, palette).clicked() {
                self.open_field(field);
            }
            return false;
        }

        let (rect, _) = ui.allocate_exact_size(vec2(18.0, ITEM_H), Sense::hover());
        field.icon()(ui.painter(), rect.center(), palette.faint);

        let mut remove = None;
        for token in tokens.iter() {
            if token_chip(ui, field, token, palette) {
                remove = Some(token.clone());
            }
        }

        let mut pieces = Vec::new();
        let mut backspace = false;
        if open {
            let id = Id::new(("meta-draft", note_id, field));
            if std::mem::take(&mut self.focus_open) {
                ui.memory_mut(|m| m.request_focus(id));
            }
            let draft = match field {
                Field::Tags => &mut self.tags,
                Field::People => &mut self.mentions,
            };
            let was_empty = draft.is_empty();
            let output = boxed_field(ui, id, draft, hint, palette);
            output
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
            let focused = output.has_focus();
            let key = |k| ui.input(|i| i.key_pressed(k));
            if focused && key(Key::Escape) {
                draft.clear();
                self.open = None;
            } else {
                // Enter adds and keeps the field open for the next one.
                let enter = focused && key(Key::Enter);
                let blur = output.lost_focus() && !enter;
                backspace = was_empty && focused && key(Key::Backspace);
                pieces = drain_draft(draft, enter || blur);
                if blur {
                    self.open = None;
                }
            }
        } else if plus_button(ui, &format!("Add {item}"), palette).clicked() {
            self.open_field(field);
        }

        let before = tokens.clone();
        let mut props = Properties::default();
        match field {
            Field::Tags => {
                props.tags = std::mem::take(tokens);
                for piece in &pieces {
                    props.add_tags(piece);
                }
                if let Some(t) = &remove {
                    props.remove_tag(t);
                }
                if backspace {
                    props.remove_last_tag();
                }
                *tokens = props.tags;
            }
            Field::People => {
                props.mentions = std::mem::take(tokens);
                for piece in &pieces {
                    props.add_mentions(piece);
                }
                if let Some(t) = &remove {
                    props.remove_mention(t);
                }
                if backspace {
                    props.remove_last_mention();
                }
                *tokens = props.mentions;
            }
        }
        *tokens != before
    }

    fn open_field(&mut self, field: Field) {
        self.open = Some(field);
        self.focus_open = true;
    }

    /// "Add property", which turns into a field for the new property's name.
    fn name_field(
        &mut self,
        ui: &mut Ui,
        properties: &mut Properties,
        palette: &Palette,
        note_id: &str,
    ) -> bool {
        let id = Id::new(("meta-name", note_id));
        if !self.naming {
            if plus_button(ui, "Add property", palette).clicked() {
                self.naming = true;
                self.focus_name = true;
            }
            return false;
        }
        if std::mem::take(&mut self.focus_name) {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let output = boxed_field(ui, id, &mut self.name, "Property name", palette);
        let escape = output.has_focus() && ui.input(|i| i.key_pressed(Key::Escape));
        let enter = output.has_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let blur = output.lost_focus() && !self.name.trim().is_empty();
        let mut added = false;
        if escape {
            self.naming = false;
            self.name.clear();
        } else if enter || blur {
            let name = self.name.trim().to_string();
            added = properties.add_extra(&name);
            if added {
                // Straight on to typing its value.
                self.focus_value = Some(name);
            }
            self.naming = false;
            self.name.clear();
        } else if output.lost_focus() {
            self.naming = false;
        }
        added
    }
}

/// Commits every comma-separated piece. The tail commits only when `finish`.
fn drain_draft(draft: &mut String, finish: bool) -> Vec<String> {
    if !finish && !draft.contains(',') {
        return Vec::new();
    }
    let parts: Vec<String> = draft.split(',').map(str::to_string).collect();
    let take = if finish { parts.len() } else { parts.len() - 1 };
    if finish {
        draft.clear();
    } else if let Some(tail) = parts.last() {
        *draft = tail.trim_start().to_string();
    }
    parts.into_iter().take(take).collect()
}

/// A tag (soft color) or person (initial avatar). A remove "×" appears over
/// the chip's end on hover (without resizing it, so nothing shifts under the
/// pointer); returns true when it's clicked.
fn token_chip(ui: &mut Ui, field: Field, token: &str, palette: &Palette) -> bool {
    let id = Id::new(("meta-chip", field, token));
    let (bg, fg) = match field {
        Field::Tags => theme::tag_colors(ui, token),
        Field::People => (palette.code_bg, palette.text),
    };
    let text = ui
        .painter()
        .layout_no_wrap(token.to_string(), FontId::proportional(FONT), fg);
    let avatar = if field == Field::People { 20.0 } else { 0.0 };
    let pad = 8.0;
    let size = vec2(pad + avatar + text.size().x + pad, ITEM_H);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let item = field.words().2;
    response.widget_info(|| {
        let kind = if field == Field::Tags {
            "Tag"
        } else {
            "Mention"
        };
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{kind} {token}"))
    });
    let hovered = ui.rect_contains_pointer(rect);

    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, bg);
    let mut x = rect.left() + pad;
    if field == Field::People {
        let (abg, afg) = theme::tag_colors(ui, token);
        let center = pos2(x + 8.0, rect.center().y);
        painter.circle_filled(center, 8.0, afg);
        let initial = token
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            initial,
            FontId::proportional(10.5),
            abg,
        );
        x += avatar;
    }
    painter.galley(pos2(x, rect.center().y - text.size().y / 2.0), text, fg);

    if !hovered {
        return false;
    }
    let x_rect =
        Rect::from_center_size(pos2(rect.right() - 11.0, rect.center().y), vec2(16.0, 16.0));
    // Cover the end of the text so the × reads cleanly.
    let cover = Rect::from_min_max(pos2(x_rect.left() - 4.0, rect.top()), rect.max);
    ui.painter().rect_filled(
        cover,
        CornerRadius {
            nw: 0,
            sw: 0,
            ne: 6,
            se: 6,
        },
        bg,
    );
    let x = ui.interact(x_rect, id.with("remove"), Sense::click());
    x.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Remove {item} {token}"),
        )
    });
    if x.hovered() {
        ui.painter()
            .circle_filled(x_rect.center(), 7.0, fg.gamma_multiply(0.18));
    }
    icons::close(ui.painter(), x_rect.center(), fg);
    x.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A custom property: its name, an editable value, and a remove "×".
fn extra_field(
    ui: &mut Ui,
    key: &str,
    value: &mut String,
    focus: bool,
    palette: &Palette,
    note_id: &str,
    removed: &mut Option<String>,
) {
    // Measured up front so the row wraps before the pill, not through it.
    let key_w = text_width(ui, key);
    let value_w = field_width(ui, value, "Empty");
    let size = vec2(8.0 + key_w + 6.0 + value_w + 2.0 + 18.0 + 2.0, ITEM_H);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, palette.border),
        StrokeKind::Inside,
    );
    ui.painter().text(
        pos2(rect.left() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        key,
        FontId::proportional(FONT),
        palette.faint,
    );

    let id = Id::new(("meta-extra", note_id, key));
    if focus {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let value_rect = field_slot(
        ui,
        pos2(rect.left() + 8.0 + key_w + 6.0, rect.center().y),
        value_w,
    );
    place(
        ui,
        value_rect,
        text_edit(id, value, "Empty", value_w, palette),
    );

    let x_rect =
        Rect::from_center_size(pos2(rect.right() - 11.0, rect.center().y), vec2(16.0, 16.0));
    let x = ui.interact(x_rect, id.with("remove"), Sense::click());
    x.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Remove {key}"))
    });
    let color = if x.hovered() {
        palette.text
    } else {
        palette.subtle
    };
    icons::close(ui.painter(), x_rect.center(), color);
    if x.on_hover_text("Remove").clicked() {
        *removed = Some(key.to_string());
    }
}

/// A quiet button: optional icon and a label, with a background on hover.
fn ghost_button(
    ui: &mut Ui,
    icon: Option<fn(&egui::Painter, egui::Pos2, Color32)>,
    label: &str,
    palette: &Palette,
) -> egui::Response {
    let text =
        ui.painter()
            .layout_no_wrap(label.to_string(), FontId::proportional(FONT), palette.faint);
    let icon_w = if icon.is_some() { 20.0 } else { 0.0 };
    let size = vec2(8.0 + icon_w + text.size().x + 8.0, ITEM_H);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let color = if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, palette.hover);
        palette.text
    } else {
        palette.faint
    };
    if let Some(icon) = icon {
        icon(
            ui.painter(),
            rect.left_center() + vec2(8.0 + 8.0, 0.0),
            color,
        );
    }
    let text_pos = pos2(
        rect.left() + 8.0 + icon_w,
        rect.center().y - text.size().y / 2.0,
    );
    ui.painter().galley(text_pos, text, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small round "+" that adds another tag or person.
fn plus_button(ui: &mut Ui, label: &str, palette: &Palette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ITEM_H, ITEM_H), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let color = if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, palette.hover);
        palette.text
    } else {
        palette.subtle
    };
    icons::plus(ui.painter(), rect.center(), color);
    response.on_hover_text(label)
}

/// A bordered input, sized to its text, for adding tags, people, or a
/// property name.
fn boxed_field(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    hint: &str,
    palette: &Palette,
) -> egui::Response {
    let width = field_width(ui, text, hint);
    let (rect, _) = ui.allocate_exact_size(vec2(width + 14.0, ITEM_H), Sense::hover());
    let focused = ui.memory(|m| m.has_focus(id));
    let stroke = if focused {
        palette.accent
    } else {
        palette.border
    };
    ui.painter()
        .rect_stroke(rect, 6.0, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let inner = field_slot(ui, pos2(rect.left() + 7.0, rect.center().y), width);
    place(ui, inner, text_edit(id, text, hint, width, palette))
}

/// A frameless single-line field. Enter doesn't leave it; callers check for
/// Enter themselves so several entries can be typed in a row.
fn text_edit<'t>(
    id: Id,
    text: &'t mut String,
    hint: &str,
    width: f32,
    palette: &Palette,
) -> TextEdit<'t> {
    TextEdit::singleline(text)
        .id(id)
        .return_key(None)
        .frame(Frame::NONE)
        .desired_width(width)
        .text_color(palette.text)
        .hint_text(RichText::new(hint).size(FONT).color(palette.subtle))
        .font(FontId::proportional(FONT))
        .margin(Margin::symmetric(0, FIELD_MARGIN_Y as i8))
}

/// Where to put a text field of `width` so its text is vertically centered
/// on `left_center`: exactly one line tall plus the field's margins.
fn field_slot(ui: &Ui, left_center: egui::Pos2, width: f32) -> Rect {
    let line = ui
        .painter()
        .layout_no_wrap("Ag".into(), FontId::proportional(FONT), Color32::WHITE)
        .size()
        .y;
    Rect::from_min_size(
        pos2(left_center.x, left_center.y - line / 2.0 - FIELD_MARGIN_Y),
        vec2(width, line + 2.0 * FIELD_MARGIN_Y),
    )
}

/// Shows `widget` in `rect` without taking space in the surrounding row
/// (the pill it sits in was already allocated).
fn place(ui: &mut Ui, rect: Rect, widget: impl egui::Widget) -> egui::Response {
    ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    )
    .add(widget)
}

/// Width for a field showing `text` (or its hint while empty).
fn field_width(ui: &Ui, text: &str, hint: &str) -> f32 {
    let shown = if text.is_empty() { hint } else { text };
    (text_width(ui, shown) + 8.0).max(40.0)
}

fn text_width(ui: &Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(FONT), Color32::WHITE)
        .size()
        .x
}

const ERROR: Color32 = Color32::from_rgb(0xD4, 0x4C, 0x47);

/// (what the row shows, its accessible name, the hover explanation).
fn sync_texts(sync: &NoteSync) -> (&'static str, &'static str, String) {
    match sync {
        NoteSync::Synced => (
            "Synced",
            "Synced to Google Drive",
            "Google Drive has the latest version of this note.".into(),
        ),
        NoteSync::Syncing => (
            "Syncing",
            "Syncing to Google Drive",
            "Sending this note to Google Drive…".into(),
        ),
        NoteSync::Pending => (
            "Not synced",
            "Not synced to Google Drive",
            "Changes to this note go to Google Drive a few seconds after they're saved.".into(),
        ),
        NoteSync::Failed(error) => (
            "Not synced",
            "Not synced to Google Drive",
            format!("Couldn't sync to Google Drive: {error}"),
        ),
    }
}

/// ("Created Sep 28 · Edited 3:53 PM", the same with full dates).
fn date_texts(times: &NoteTimes) -> (String, String) {
    let (year, ..) = parts(SystemTime::now());
    let (cy, cm, cd, _, _) = parts(times.created);
    let (uy, um, ud, uh, umin) = parts(times.updated);
    let clock = properties::format_time(uh, umin);
    let today = {
        let (y, m, d, _, _) = parts(SystemTime::now());
        (uy, um, ud) == (y, m, d)
    };
    let edited = if today {
        clock.clone()
    } else {
        properties::format_short_date(uy, um, ud, year)
    };
    let short = format!(
        "Created {}  ·  Edited {edited}",
        properties::format_short_date(cy, cm, cd, year)
    );
    let full = format!(
        "Created {}\nUpdated {}, {clock}",
        properties::format_date(cy, cm, cd),
        properties::format_date(uy, um, ud),
    );
    (short, full)
}

fn parts(time: SystemTime) -> (i64, u32, u32, u32, u32) {
    #[cfg(unix)]
    if let Some(local) = local_parts(time) {
        return local;
    }
    let (year, month, day, hour, minute, _) = properties::created_utc_parts(time);
    (year, month, day, hour, minute)
}

#[cfg(unix)]
fn local_parts(time: SystemTime) -> Option<(i64, u32, u32, u32, u32)> {
    let secs = time.duration_since(UNIX_EPOCH).ok()?.as_secs() as libc::time_t;
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            return None;
        }
        Some((
            tm.tm_year as i64 + 1900,
            tm.tm_mon as u32 + 1,
            tm.tm_mday as u32,
            tm.tm_hour as u32,
            tm.tm_min as u32,
        ))
    }
}
