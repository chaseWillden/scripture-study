//! Context menus drawn to match the sidebar: rounded, softly shadowed, with
//! an icon and a shortcut hint on each item. An item's shortcut also runs it
//! while its menu is open.

use eframe::egui::{
    self,
    containers::menu::{MenuState, SubMenu},
    vec2, Color32, CornerRadius, FontId, InnerResponse, KeyboardShortcut, Margin, Painter, Popup,
    Pos2, Rect, Response, Sense, Shadow, Stroke, Ui,
};

use crate::icons;
use crate::theme::Palette;

pub type Icon = fn(&Painter, Pos2, Color32);

const WIDTH: f32 = 224.0;
const SUBMENU_WIDTH: f32 = 200.0;
const ITEM_HEIGHT: f32 = 28.0;
const FONT: f32 = 13.5;
const DANGER: Color32 = Color32::from_rgb(0xEB, 0x57, 0x57);

/// Shows `add_contents` as a context menu when `response` is right-clicked.
pub fn context_menu<R>(
    response: &Response,
    palette: &Palette,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<InnerResponse<R>> {
    let palette = *palette;
    Popup::context_menu(response)
        .width(WIDTH)
        .style(move |style: &mut egui::Style| apply_style(style, &palette))
        .show(|ui| {
            ui.set_width(WIDTH);
            add_contents(ui)
        })
}

/// A menu at `pos`, for right-clicks on things that aren't whole widgets
/// (like a link inside some text). Returns what `add_contents` returned and
/// whether the menu stays open: a click elsewhere or Escape closes it, as
/// does choosing an item. `just_opened` skips closing on the opening click.
pub fn menu_at<R>(
    ctx: &egui::Context,
    id: egui::Id,
    pos: Pos2,
    palette: &Palette,
    just_opened: bool,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> (R, bool) {
    let style = popup_style(ctx, palette);
    let area = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .constrain(true)
        .show(ctx, |ui| {
            ui.set_style(style.clone());
            egui::Frame::popup(&style)
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    add_contents(ui)
                })
                .inner
        });
    let rect = area.response.rect;
    let pressed_outside = ctx.input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))
    });
    let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let open = just_opened || !(pressed_outside || escape);
    (area.inner, open)
}

/// The context menu's popup style, so other pickers can share its chrome.
pub fn popup_style(ctx: &egui::Context, palette: &Palette) -> egui::Style {
    let mut style = (*ctx.global_style()).clone();
    apply_style(&mut style, palette);
    style
}

pub(crate) fn apply_style(style: &mut egui::Style, palette: &Palette) {
    let dark = style.visuals.dark_mode;
    style.spacing.menu_margin = Margin::same(5);
    style.spacing.item_spacing = vec2(0.0, 1.0);
    style.visuals.menu_corner_radius = CornerRadius::same(10);
    style.visuals.window_fill = palette.menu_bg;
    style.visuals.window_stroke = Stroke::new(
        1.0,
        if dark {
            Color32::from_white_alpha(20)
        } else {
            Color32::from_black_alpha(22)
        },
    );
    style.visuals.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 22,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 90 } else { 34 }),
    };
}

/// One menu item.
pub struct Item<'a> {
    label: &'a str,
    icon: Option<Icon>,
    shortcut: Option<KeyboardShortcut>,
    enabled: bool,
    danger: bool,
    checked: bool,
    indent: usize,
    disabled_hint: Option<&'a str>,
}

impl<'a> Item<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            shortcut: None,
            enabled: true,
            danger: false,
            checked: false,
            indent: 0,
            disabled_hint: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn shortcut(mut self, shortcut: KeyboardShortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Destructive: drawn in red.
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    /// Marks the item as the current choice (and leaves it disabled).
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Nesting level, for items that list a tree.
    pub fn indent(mut self, depth: usize) -> Self {
        self.indent = depth;
        self
    }

    pub fn disabled_hint(mut self, hint: &'a str) -> Self {
        self.disabled_hint = Some(hint);
        self
    }

    /// Draws the item. Returns `true` (and closes the menu) when it's clicked
    /// or its shortcut is pressed.
    pub fn show(self, ui: &mut Ui, palette: &Palette) -> bool {
        let enabled = self.enabled && !self.checked;
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = allocate(ui, sense);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, self.label)
        });
        let hint = self
            .shortcut
            .map(|shortcut| ui.ctx().format_shortcut(&shortcut));
        paint(
            ui,
            &response,
            palette,
            Look {
                label: self.label,
                icon: self.icon,
                hint: hint.as_deref(),
                enabled,
                danger: self.danger,
                highlight: enabled && response.hovered(),
                checked: self.checked,
                indent: self.indent,
                submenu: false,
            },
        );
        if !enabled {
            if let Some(hint) = self.disabled_hint {
                response.on_hover_text(hint);
            }
            return false;
        }
        let pressed = self.shortcut.is_some_and(|shortcut| {
            ui.input_mut(|i| {
                i.modifiers.matches_exact(shortcut.modifiers) && i.consume_shortcut(&shortcut)
            })
        });
        let activated = response.clicked() || pressed;
        if activated {
            ui.close();
        }
        activated
    }
}

/// An item that opens `add_contents` as a submenu on hover.
pub fn submenu<R>(
    ui: &mut Ui,
    palette: &Palette,
    label: &str,
    icon: Icon,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let response = allocate(ui, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let submenu_id = SubMenu::id_from_widget_id(response.id);
    let open = MenuState::from_ui(ui, |state, _| state.open_item == Some(submenu_id));
    paint(
        ui,
        &response,
        palette,
        Look {
            label,
            icon: Some(icon),
            hint: None,
            enabled: true,
            danger: false,
            highlight: open || response.hovered(),
            checked: false,
            indent: 0,
            submenu: true,
        },
    );
    SubMenu::new()
        .show(ui, &response, |ui| {
            ui.set_width(SUBMENU_WIDTH);
            add_contents(ui)
        })
        .map(|inner| inner.inner)
}

/// A thin rule between groups of items.
pub fn separator(ui: &mut Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(
        rect.x_range().shrink(4.0),
        rect.center().y,
        Stroke::new(1.0, palette.border),
    );
}

fn allocate(ui: &mut Ui, sense: Sense) -> Response {
    ui.allocate_exact_size(vec2(ui.available_width(), ITEM_HEIGHT), sense)
        .1
}

struct Look<'a> {
    label: &'a str,
    icon: Option<Icon>,
    hint: Option<&'a str>,
    enabled: bool,
    danger: bool,
    highlight: bool,
    checked: bool,
    indent: usize,
    submenu: bool,
}

fn paint(ui: &Ui, response: &Response, palette: &Palette, look: Look) {
    let rect = response.rect;
    let painter = ui.painter();
    if look.highlight {
        let fill = if look.danger {
            DANGER.gamma_multiply(0.14)
        } else {
            palette.menu_selected
        };
        painter.rect_filled(rect, 6.0, fill);
    }
    let text = match (look.enabled || look.checked, look.danger) {
        (false, _) => palette.faint,
        (true, true) => DANGER,
        (true, false) => palette.text,
    };
    let icon_color = if look.danger && look.enabled {
        DANGER
    } else if look.enabled && look.highlight {
        palette.text
    } else {
        palette.faint
    };

    let y = rect.center().y;
    let start = rect.left() + look.indent as f32 * 14.0;
    if let Some(icon) = look.icon {
        icon(painter, egui::pos2(start + 16.0, y), icon_color);
    }
    let right = if look.submenu {
        icons::chevron(
            painter,
            egui::pos2(rect.right() - 12.0, y),
            false,
            palette.faint,
        );
        rect.right() - 22.0
    } else if look.checked {
        icons::check(painter, egui::pos2(rect.right() - 14.0, y), palette.text);
        rect.right() - 26.0
    } else if let Some(hint) = look.hint {
        let hint =
            painter.layout_no_wrap(hint.to_string(), FontId::proportional(12.0), palette.faint);
        let pos = egui::pos2(rect.right() - 10.0 - hint.size().x, y - hint.size().y / 2.0);
        let left = pos.x;
        painter.galley(pos, hint, palette.faint);
        left - 12.0
    } else {
        rect.right() - 10.0
    };

    let left = start + if look.icon.is_some() { 32.0 } else { 10.0 };
    let label = crate::sidebar::elided(ui, look.label, FONT, text, right - left);
    let label_rect = Rect::from_min_size(egui::pos2(left, y - label.size().y / 2.0), label.size());
    painter.galley(label_rect.min, label, text);
}
