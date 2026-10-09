//! Settings: its sections are listed in the sidebar, and the chosen one
//! fills the page. Connectors is the only section so far.

use std::time::SystemTime;

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Key, Layout, Margin,
    Modifiers, Response, RichText, Sense, Stroke, TextEdit, Ui,
};
use scripture_study_core::search;

use crate::google_drive::{GoogleDrive, Phase, Snapshot};
use crate::icons;
use crate::theme::{self, Palette};

/// Widest the settings content gets, however wide the window.
const MAX_WIDTH: f32 = 640.0;
const LABEL_WIDTH: f32 = 112.0;
const FOLDER_FIELD: &str = "settings-drive-folder";
const CLIENT_ID_FIELD: &str = "settings-client-id";
const CLIENT_SECRET_FIELD: &str = "settings-client-secret";
const ERROR: Color32 = Color32::from_rgb(0xD4, 0x4C, 0x47);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Connectors,
}

impl Section {
    const ALL: [Self; 1] = [Self::Connectors];

    fn title(self) -> &'static str {
        match self {
            Self::Connectors => "Connectors",
        }
    }
}

#[derive(Default)]
pub struct SettingsPage {
    open: bool,
    section: Section,
    /// Text typed into a field that the connector hasn't taken yet.
    folder: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
}

impl SettingsPage {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Window title while the page is open.
    pub fn title(&self) -> Option<String> {
        self.open
            .then(|| format!("Settings · {}", self.section.title()))
    }

    /// The sidebar: one row per section.
    pub fn show_index(&mut self, ui: &mut Ui, palette: &Palette) {
        let typing = ui.memory(|m| m.focused().is_some_and(is_field));
        if !typing
            && !egui::Popup::is_any_open(ui.ctx())
            && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.close();
            return;
        }
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
        let title = theme::semibold(ui, "Settings", 17.0, palette.text);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
        ui.painter().galley(
            rect.left_center() + vec2(18.0, -title.size().y / 2.0),
            title,
            palette.text,
        );
        ui.add_space(8.0);
        for section in Section::ALL {
            if section_row(ui, section, section == self.section, palette) {
                self.section = section;
            }
        }
    }

    /// The page for the chosen section.
    pub fn show_page(&mut self, ui: &mut Ui, palette: &Palette, drive: &GoogleDrive) {
        egui::ScrollArea::vertical()
            .id_salt("settings-page")
            .auto_shrink(false)
            .show(ui, |ui| {
                let width = ui.available_width().min(MAX_WIDTH);
                ui.set_max_width(width);
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                match self.section {
                    Section::Connectors => self.connectors(ui, palette, drive),
                }
                ui.add_space(40.0);
            });
    }

    fn connectors(&mut self, ui: &mut Ui, palette: &Palette, drive: &GoogleDrive) {
        heading(
            ui,
            "Connectors",
            "Sync your notes with other apps and services.",
            palette,
        );
        ui.add_space(16.0);
        let snapshot = drive.snapshot();
        card(ui, palette, |ui| {
            self.google_drive(ui, palette, drive, &snapshot)
        });
    }

    fn google_drive(&mut self, ui: &mut Ui, palette: &Palette, drive: &GoogleDrive, s: &Snapshot) {
        ui.horizontal(|ui| {
            let (icon, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
            ui.painter().rect_filled(icon, 8.0, palette.menu_selected);
            icons::drive(ui.painter(), icon.center());
            ui.add_space(4.0);
            // Leaves room for the switch, and wraps the description.
            let width = (ui.available_width() - 56.0).max(80.0);
            ui.allocate_ui_with_layout(vec2(width, 36.0), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let title = theme::semibold(ui, "Google Drive", 15.0, palette.text);
                let (rect, _) = ui.allocate_exact_size(title.size(), Sense::hover());
                ui.painter().galley(rect.min, title, palette.text);
                ui.add(
                    egui::Label::new(
                        RichText::new("Keep a Google Doc copy of every note in a Drive folder.")
                            .size(13.0)
                            .color(palette.faint),
                    )
                    .wrap(),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if switch(ui, s.enabled, "Google Drive", palette).clicked() {
                    drive.set_enabled(!s.enabled);
                }
            });
        });
        if !s.enabled {
            return;
        }
        ui.add_space(6.0);
        divider(ui, palette);
        ui.add_space(4.0);

        if !s.builtin_client {
            self.client_fields(ui, palette, drive, s);
        }
        self.account_row(ui, palette, drive, s);
        self.folder_row(ui, palette, drive, s);
        if s.connected {
            status_row(ui, palette, drive, s);
        }
        if let Some(error) = &s.error {
            ui.add_space(2.0);
            ui.label(RichText::new(error).size(13.0).color(ERROR));
        }
    }

    /// The user's own OAuth client, for builds that don't come with one.
    fn client_fields(&mut self, ui: &mut Ui, palette: &Palette, drive: &GoogleDrive, s: &Snapshot) {
        let id = setting_row(ui, "Client ID", palette, |ui| {
            field(
                ui,
                CLIENT_ID_FIELD,
                &mut self.client_id,
                &s.client_id,
                "xxxx.apps.googleusercontent.com",
                false,
                palette,
            )
        });
        let secret = setting_row(ui, "Client secret", palette, |ui| {
            field(
                ui,
                CLIENT_SECRET_FIELD,
                &mut self.client_secret,
                &s.client_secret,
                "GOCSPX-…",
                true,
                palette,
            )
        });
        if id || secret {
            let id = self
                .client_id
                .clone()
                .unwrap_or_else(|| s.client_id.clone());
            let secret = self
                .client_secret
                .clone()
                .unwrap_or_else(|| s.client_secret.clone());
            drive.set_client(id.trim().to_string(), secret.trim().to_string());
        }
        setting_row(ui, "", palette, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Create a Desktop app OAuth client in Google Cloud Console, \
                         with the Google Drive API turned on.",
                    )
                    .size(12.5)
                    .color(palette.faint),
                )
                .wrap(),
            );
        });
        ui.add_space(4.0);
    }

    fn account_row(&mut self, ui: &mut Ui, palette: &Palette, drive: &GoogleDrive, s: &Snapshot) {
        setting_row(ui, "Account", palette, |ui| {
            let signing_in = s.phase == Phase::SigningIn;
            let text = if signing_in {
                "Finish signing in with your browser…".to_string()
            } else if s.connected {
                s.account.clone().unwrap_or_else(|| "Connected".into())
            } else {
                "Not connected".to_string()
            };
            let color = if s.connected {
                palette.text
            } else {
                palette.faint
            };
            ui.label(RichText::new(text).size(14.0).color(color));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if signing_in {
                    if button(ui, "Cancel", false, true, palette).clicked() {
                        drive.cancel_connect();
                    }
                } else if s.connected {
                    if button(ui, "Disconnect", false, true, palette).clicked() {
                        drive.disconnect();
                    }
                } else if button(ui, "Connect", true, s.has_client(), palette).clicked() {
                    drive.connect();
                }
            });
        });
    }

    fn folder_row(&mut self, ui: &mut Ui, palette: &Palette, drive: &GoogleDrive, s: &Snapshot) {
        let committed = setting_row(ui, "Drive folder", palette, |ui| {
            // Room for the link to the right of the field.
            let link_width = if s.folder_url.is_some() { 122.0 } else { 0.0 };
            let committed = ui
                .allocate_ui_with_layout(
                    vec2(ui.available_width() - link_width, 32.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        field(
                            ui,
                            FOLDER_FIELD,
                            &mut self.folder,
                            &s.folder_name,
                            crate::google_drive::DEFAULT_FOLDER,
                            false,
                            palette,
                        )
                    },
                )
                .inner;
            if let Some(url) = &s.folder_url {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Open in Drive", false, true, palette).clicked() {
                        if let Err(e) = crate::google_drive::open_in_browser(url) {
                            ui.label(RichText::new(e.to_string()).color(ERROR));
                        }
                    }
                });
            }
            committed
        });
        if committed {
            if let Some(name) = &self.folder {
                if name.trim().is_empty() {
                    self.folder = None;
                } else {
                    drive.set_folder(name.clone());
                }
            }
        }
    }
}

fn status_row(ui: &mut Ui, palette: &Palette, drive: &GoogleDrive, s: &Snapshot) {
    setting_row(ui, "Status", palette, |ui| {
        let text = match (&s.phase, s.last_synced) {
            (Phase::Syncing { done, total }, _) if *total > 0 => {
                format!("Syncing {} of {total}…", done + 1)
            }
            (Phase::Syncing { .. }, _) => "Syncing…".to_string(),
            (_, Some(when)) => {
                let age = search::relative_time(when, SystemTime::now());
                let when = if age == "now" {
                    "just now".to_string()
                } else {
                    format!("{age} ago")
                };
                let notes = match s.synced_notes {
                    1 => "1 note".to_string(),
                    n => format!("{n} notes"),
                };
                format!("Synced {when} · {notes}")
            }
            _ if !s.enabled => "Paused".to_string(),
            _ => "Not synced yet".to_string(),
        };
        ui.label(RichText::new(text).size(14.0).color(palette.faint));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let idle = s.phase == Phase::Idle;
            if button(ui, "Sync now", false, idle, palette).clicked() {
                drive.sync_now();
            }
        });
    });
    // Keeps "5m ago" current while the page is open.
    ui.ctx().request_repaint_after_secs(30.0);
}

fn is_field(id: Id) -> bool {
    [FOLDER_FIELD, CLIENT_ID_FIELD, CLIENT_SECRET_FIELD]
        .iter()
        .any(|name| Id::new(name) == id)
}

fn section_row(ui: &mut Ui, section: Section, selected: bool, palette: &Palette) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let rect = rect.shrink2(vec2(8.0, 0.0));
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, section.title()));
    if selected || response.hovered() {
        let fill = if selected {
            palette.menu_selected
        } else {
            palette.hover
        };
        ui.painter().rect_filled(rect, 7.0, fill);
    }
    let y = rect.center().y;
    icons::plug(ui.painter(), pos2(rect.left() + 16.0, y), palette.text);
    ui.painter().text(
        pos2(rect.left() + 34.0, y),
        Align2::LEFT_CENTER,
        section.title(),
        FontId::proportional(14.0),
        palette.text,
    );
    response.clicked()
}

fn heading(ui: &mut Ui, title: &str, subtitle: &str, palette: &Palette) {
    let galley = theme::semibold(ui, title, 28.0, palette.text);
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y), Sense::hover());
    ui.painter().galley(rect.left_top(), galley, palette.text);
    ui.label(RichText::new(subtitle).size(14.0).color(palette.faint));
}

fn card(ui: &mut Ui, palette: &Palette, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn divider(ui: &mut Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, palette.border),
    );
}

/// A label on the left and its control on the right.
fn setting_row<R>(
    ui: &mut Ui,
    label: &str,
    palette: &Palette,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        ui.set_min_height(34.0);
        let (rect, response) = ui.allocate_exact_size(vec2(LABEL_WIDTH, 30.0), Sense::hover());
        if !label.is_empty() {
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
        }
        ui.painter().text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(14.0),
            palette.text,
        );
        add(ui)
    })
    .inner
}

/// A single-line field showing `draft` while it's being edited and `saved`
/// otherwise. Returns true when an edit is finished (Enter or focus lost).
fn field(
    ui: &mut Ui,
    id: &str,
    draft: &mut Option<String>,
    saved: &str,
    hint: &str,
    password: bool,
    palette: &Palette,
) -> bool {
    // Drop the draft once the connector has caught up with it.
    let id = Id::new(id);
    let focused = ui.memory(|m| m.has_focus(id));
    if !focused && draft.as_deref() == Some(saved) {
        *draft = None;
    }
    let mut text = draft.clone().unwrap_or_else(|| saved.to_string());
    let output = Frame::new()
        .fill(palette.menu_selected)
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            TextEdit::singleline(&mut text)
                .id(id)
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .desired_width(ui.available_width())
                .font(FontId::proportional(14.0))
                .text_color(palette.text)
                .password(password)
                .hint_text(RichText::new(hint).color(palette.faint))
                .show(ui)
        })
        .inner;
    let response = output.response;
    if response.changed() {
        *draft = Some(text);
    }
    response.lost_focus() && draft.is_some()
}

pub(crate) fn button(
    ui: &mut Ui,
    label: &str,
    primary: bool,
    enabled: bool,
    palette: &Palette,
) -> Response {
    let (fill, color) = if primary {
        (palette.accent, Color32::WHITE)
    } else {
        (palette.menu_selected, palette.text)
    };
    let button = egui::Button::new(RichText::new(label).size(13.5).color(color))
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(7))
        .min_size(vec2(0.0, 30.0));
    ui.add_enabled(enabled, button)
}

/// An on/off switch. Reads as a checkbox named `label` to assistive tech.
fn switch(ui: &mut Ui, on: bool, label: &str, palette: &Palette) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, label));
    let t = ui.ctx().animate_bool_responsive(response.id, on);
    let track = if on { palette.accent } else { palette.subtle };
    ui.painter().rect_filled(rect, 10.0, track);
    let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, t);
    ui.painter()
        .circle_filled(pos2(x, rect.center().y), 8.0, Color32::WHITE);
    response
}
