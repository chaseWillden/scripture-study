//! Highlight and underline: a bar that appears above a selection, and the
//! same colors in a right-click menu.

use eframe::egui::{
    self, vec2, Align2, Color32, CornerRadius, FontId, Frame, Id, Margin, Pos2, Rect, Sense,
    Shadow, Stroke, Ui,
};
use scripture_study_core::inline::{self, MarkKind, MARK_COLORS};

use crate::icons;
use crate::menu::{self, Item};
use crate::theme::Palette;

/// What the user picked. `color` of `None` removes that annotation.
/// `toggle` removes the color when the text already has it.
pub struct Choice {
    pub kind: MarkKind,
    pub color: Option<u32>,
    pub toggle: bool,
}

/// The menu opened by right-clicking text that isn't a link or a misspelling.
pub struct MarkMenu {
    pub selection: bool,
    pub block: usize,
    /// Character range. Equal ends mean the caret, which marks the word there.
    pub start: usize,
    pub end: usize,
    pos: Pos2,
    just_opened: bool,
}

impl MarkMenu {
    pub fn new(selection: bool, block: usize, start: usize, end: usize, pos: Pos2) -> Self {
        Self {
            selection,
            block,
            start,
            end,
            pos,
            just_opened: true,
        }
    }

    /// Draws the menu. Returns the choice and whether the menu stays open.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        palette: &Palette,
        highlight: Option<u32>,
        underline: Option<u32>,
    ) -> (Option<Choice>, bool) {
        let (choice, open) = menu::menu_at(
            ctx,
            Id::new("mark-menu"),
            self.pos,
            palette,
            std::mem::take(&mut self.just_opened),
            |ui| items(ui, palette, highlight, underline),
        );
        let open = open && choice.is_none();
        (choice, open)
    }
}

/// Highlight and Underline, each with a color row and a way to remove it.
/// A spelling or link menu can offer the same rows.
pub fn items(
    ui: &mut Ui,
    palette: &Palette,
    highlight: Option<u32>,
    underline: Option<u32>,
) -> Option<Choice> {
    if let Some(Some(choice)) = menu::submenu(ui, palette, "Highlight", icons::highlight, |ui| {
        kind_menu(ui, palette, "Highlight", MarkKind::Highlight, highlight)
    }) {
        return Some(choice);
    }
    if let Some(Some(choice)) = menu::submenu(ui, palette, "Underline", icons::underline, |ui| {
        kind_menu(ui, palette, "Underline", MarkKind::Underline, underline)
    }) {
        return Some(choice);
    }
    None
}

fn kind_menu(
    ui: &mut Ui,
    palette: &Palette,
    label: &str,
    kind: MarkKind,
    current: Option<u32>,
) -> Option<Choice> {
    if let Some(color) = swatches(ui, palette, label, current) {
        return Some(Choice {
            kind,
            color: Some(color),
            toggle: true,
        });
    }
    menu::separator(ui, palette);
    let remove = format!("Remove {label}");
    if Item::new(&remove)
        .icon(icons::close)
        .enabled(current.is_some())
        .show(ui, palette)
    {
        return Some(Choice {
            kind,
            color: None,
            toggle: false,
        });
    }
    None
}

/// What the selection bar did.
pub enum BarAction {
    /// Apply `kind` in `color` to the selection.
    Apply { kind: MarkKind, color: u32 },
    /// Take highlight and underline off the selection.
    Clear,
    /// Copy the selection. Highlight and underline tags are left out.
    Copy,
    /// Start moving the selected Markdown to a new page.
    MoveTo,
}

const SWATCH: f32 = 34.0;
const STRIP: f32 = 32.0;
const BUTTONS: f32 = 52.0;
const ACTIONS: f32 = 38.0;

/// The bar above a text selection: a strip of colors, then highlight,
/// underline, and clear. `kind` is the style a color applies. `color` is
/// the swatch that style uses.
pub fn show_bar(
    ctx: &egui::Context,
    palette: &Palette,
    selection: Rect,
    kind: MarkKind,
    color: u32,
) -> Option<BarAction> {
    let width = SWATCH * MARK_COLORS.len() as f32;
    let height = STRIP + BUTTONS + ACTIONS;
    let screen = ctx.content_rect();
    let x = (selection.center().x - width / 2.0).clamp(
        screen.min.x + 8.0,
        (screen.max.x - width - 8.0).max(screen.min.x + 8.0),
    );
    let above = selection.min.y - height - 10.0;
    let place_above = above >= screen.min.y + 4.0;
    let y = if place_above {
        above
    } else {
        selection.max.y + 10.0
    };
    let mut action = None;
    egui::Area::new(Id::new("mark-bar"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(x, y))
        .constrain(true)
        .show(ctx, |ui| {
            Frame::new()
                .fill(palette.menu_bg)
                .stroke(Stroke::new(1.0, palette.border))
                .corner_radius(CornerRadius::same(16))
                .inner_margin(Margin::ZERO)
                .shadow(Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: Color32::from_black_alpha(70),
                })
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    if let Some(picked) = color_strip(ui, color) {
                        action = Some(BarAction::Apply {
                            kind,
                            color: picked,
                        });
                    }
                    if let Some(picked) = style_row(ui, palette, kind, color, width) {
                        action = Some(picked);
                    }
                    if let Some(picked) = action_row(ui, palette, width) {
                        action = Some(picked);
                    }
                });
            let panel = ui.min_rect();
            let tip_x = selection
                .center()
                .x
                .clamp(panel.min.x + 16.0, panel.max.x - 16.0);
            let tail = if place_above {
                [
                    Pos2::new(tip_x - 7.0, panel.max.y - 1.0),
                    Pos2::new(tip_x + 7.0, panel.max.y - 1.0),
                    Pos2::new(tip_x, panel.max.y + 7.0),
                ]
            } else {
                [
                    Pos2::new(tip_x - 7.0, panel.min.y + 1.0),
                    Pos2::new(tip_x + 7.0, panel.min.y + 1.0),
                    Pos2::new(tip_x, panel.min.y - 7.0),
                ]
            };
            ui.painter().add(egui::Shape::convex_polygon(
                tail.to_vec(),
                palette.menu_bg,
                Stroke::NONE,
            ));
        });
    action
}

fn action_row(ui: &mut Ui, palette: &Palette, width: f32) -> Option<BarAction> {
    let mut action = None;
    let (rect, _) = ui.allocate_exact_size(vec2(width, ACTIONS), Sense::hover());
    let gap = 4.0;
    let button_width = (width - gap) / 2.0;
    for (index, (label, icon, picked)) in [
        (
            "Move To",
            icons::reveal as fn(&egui::Painter, Pos2, Color32),
            BarAction::MoveTo,
        ),
        (
            "Copy",
            icons::copy as fn(&egui::Painter, Pos2, Color32),
            BarAction::Copy,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let left = rect.left() + index as f32 * (button_width + gap);
        let button = Rect::from_min_size(Pos2::new(left, rect.top()), vec2(button_width, ACTIONS));
        let response = ui.allocate_rect(button, Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        if response.hovered() {
            ui.painter().rect_filled(button, 8.0, palette.menu_selected);
        }
        icon(
            ui.painter(),
            button.left_center() + vec2(14.0, 0.0),
            palette.faint,
        );
        ui.painter().text(
            button.left_center() + vec2(28.0, 0.0),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.0),
            palette.text,
        );
        if response.clicked() {
            action = Some(picked);
        }
    }
    action
}

fn color_strip(ui: &mut Ui, current: u32) -> Option<u32> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let last = MARK_COLORS.len() - 1;
        for (i, (name, color)) in MARK_COLORS.iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, STRIP), Sense::click());
            let label = format!("Color {name}");
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone())
            });
            let radius = CornerRadius {
                nw: if i == 0 { 16 } else { 0 },
                ne: if i == last { 16 } else { 0 },
                sw: 0,
                se: 0,
            };
            let (r, g, b) = inline::color_rgb(*color);
            ui.painter()
                .rect_filled(rect, radius, Color32::from_rgb(r, g, b));
            if *color == current {
                ui.painter().rect_filled(
                    Rect::from_min_max(rect.left_bottom() + vec2(0.0, -3.0), rect.right_bottom()),
                    0.0,
                    Color32::from_black_alpha(90),
                );
            }
            if response.clicked() {
                picked = Some(*color);
            }
        }
    });
    picked
}

fn style_row(
    ui: &mut Ui,
    palette: &Palette,
    kind: MarkKind,
    color: u32,
    width: f32,
) -> Option<BarAction> {
    let mut action = None;
    let (rect, _) = ui.allocate_exact_size(vec2(width, BUTTONS), Sense::hover());
    let columns = [
        (rect.left() + width / 6.0, MarkKind::Highlight),
        (rect.center().x, MarkKind::Underline),
    ];
    for (cx, style) in columns {
        let center = Pos2::new(cx, rect.center().y);
        let button = Rect::from_center_size(center, vec2(40.0, 40.0));
        let response = ui.allocate_rect(button, Sense::click());
        let label = match style {
            MarkKind::Highlight => "Highlight",
            MarkKind::Underline => "Underline",
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        let active = kind == style;
        paint_style(ui, button, palette, style, color, active);
        if response.clicked() {
            action = Some(BarAction::Apply { kind: style, color });
        }
    }
    let clear_center = Pos2::new(rect.right() - width / 6.0, rect.center().y);
    let clear = Rect::from_center_size(clear_center, vec2(40.0, 40.0));
    let response = ui.allocate_rect(clear, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Clear"));
    letter(ui, clear.center(), palette.faint);
    if response.clicked() {
        action = Some(BarAction::Clear);
    }
    action
}

fn paint_style(ui: &Ui, rect: Rect, palette: &Palette, kind: MarkKind, color: u32, active: bool) {
    let (r, g, b) = inline::color_rgb(color);
    let swatch = Color32::from_rgb(r, g, b);
    let painter = ui.painter();
    match kind {
        MarkKind::Highlight => {
            let chip = Rect::from_center_size(rect.center(), vec2(34.0, 34.0));
            painter.rect_filled(chip, CornerRadius::same(8), swatch);
            letter(ui, chip.center(), ink_on(color));
        }
        MarkKind::Underline => {
            if active {
                painter.rect_filled(rect, CornerRadius::same(8), palette.menu_selected);
            }
            letter(ui, rect.center() + vec2(0.0, -2.0), palette.text);
            painter.rect_filled(
                Rect::from_center_size(rect.center() + vec2(0.0, 12.0), vec2(18.0, 3.0)),
                CornerRadius::same(2),
                swatch,
            );
        }
    }
    if active && kind == MarkKind::Underline {
        painter.rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.5, swatch),
            egui::StrokeKind::Inside,
        );
    }
}

fn letter(ui: &Ui, center: Pos2, color: Color32) {
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        "A",
        FontId::proportional(22.0),
        color,
    );
}

fn ink_on(color: u32) -> Color32 {
    let (r, g, b) = inline::color_rgb(color);
    let luminance = 0.3 * f32::from(r) + 0.59 * f32::from(g) + 0.11 * f32::from(b);
    if luminance > 150.0 {
        Color32::from_rgb(28, 28, 26)
    } else {
        Color32::WHITE
    }
}

/// One row of color dots. The label is "{kind} {name}" so a test can click it.
fn swatches(ui: &mut Ui, palette: &Palette, kind: &str, current: Option<u32>) -> Option<u32> {
    let response = ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 32.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add_space(6.0);
            for (name, color) in MARK_COLORS {
                let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                let label = format!("{kind} {name}");
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone())
                });
                let (r, g, b) = inline::color_rgb(*color);
                let center = rect.center();
                ui.painter()
                    .circle_filled(center, 8.0, Color32::from_rgb(r, g, b));
                if current == Some(*color) {
                    ui.painter()
                        .circle_stroke(center, 10.5, Stroke::new(1.6, palette.text));
                }
                if response.clicked() {
                    ui.close();
                    return Some(*color);
                }
            }
            None
        },
    );
    response.inner
}
