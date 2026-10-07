//! Fonts, colors, and how Markdown text is laid out as rich text.

use std::sync::Arc;

use eframe::egui::{
    self,
    epaint::text::ByteIndex,
    epaint::text::VariationCoords,
    text::{LayoutJob, TextFormat},
    Color32, FontData, FontFamily, FontId, Stroke,
};
use scripture_study_core::{inline, BlockKind};

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter.ttf");
const INTER_ITALIC: &[u8] = include_bytes!("../assets/fonts/Inter-Italic.ttf");

/// Space kept on each side of the page column. Paragraph numbers hang to
/// the left of the text, so this is wide enough that they stay clear of the
/// sidebar and the window edge.
pub const PAGE_MARGIN: f32 = 72.0;
pub const GUTTER: f32 = 28.0;

/// Width of the page column. It grows with the window so wrapped text
/// reflows, and keeps a margin from the edges.
pub fn column_width(available: f32) -> f32 {
    (available - PAGE_MARGIN * 2.0).max(0.0)
}

/// Moves `current` toward `target`. Returns whether another frame is needed.
///
/// The first width, and a jump large enough to be the sidebar rather than a
/// resize, land immediately. A resize eases, and never trails the window by
/// more than [`MAX_LAG`], so line breaks don't flip on every pixel and the
/// column still stays inside the margins.
const MAX_LAG: f32 = 32.0;

pub fn ease_width(current: &mut f32, target: f32, dt: f32) -> bool {
    const SNAP: f32 = 160.0;
    const TAU: f32 = 0.08;
    const DONE: f32 = 0.4;

    // A big dt is a stalled frame or a test step, not a drag. Catch up at once
    // so the column doesn't keep requesting frames after the window has settled.
    if *current <= 0.0 || (target - *current).abs() >= SNAP || dt >= 0.2 {
        *current = target;
        return false;
    }
    let delta = target - *current;
    if delta.abs() <= DONE {
        *current = target;
        return false;
    }
    let t = 1.0 - (-dt.max(0.0) / TAU).exp();
    *current += delta * t;
    let lag = target - *current;
    if lag.abs() > MAX_LAG {
        *current = target - lag.signum() * MAX_LAG;
    }
    (target - *current).abs() > DONE
}

pub fn italic_family() -> FontFamily {
    FontFamily::Name("italic".into())
}

/// Inter for body text (bold comes from its variable weight axis), with egui's
/// defaults kept as fallbacks for emoji and other scripts.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("Inter".into(), Arc::new(FontData::from_static(INTER)));
    fonts.font_data.insert(
        "Inter-Italic".into(),
        Arc::new(FontData::from_static(INTER_ITALIC)),
    );

    let fallbacks = fonts.families[&FontFamily::Proportional].clone();
    let family = |first: &str| {
        std::iter::once(first.to_string())
            .chain(fallbacks.clone())
            .collect()
    };
    fonts
        .families
        .insert(FontFamily::Proportional, family("Inter"));
    fonts
        .families
        .insert(italic_family(), family("Inter-Italic"));
    ctx.set_fonts(fonts);
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color32,
    pub text: Color32,
    pub faint: Color32,
    pub subtle: Color32,
    pub code_bg: Color32,
    pub accent: Color32,
    pub menu_bg: Color32,
    pub menu_selected: Color32,
    pub border: Color32,
    pub sidebar: Color32,
    /// The icon rail, a shade darker than the sidebar.
    pub rail: Color32,
    pub hover: Color32,
}

impl Palette {
    pub fn for_ui(ui: &egui::Ui) -> Self {
        if ui.visuals().dark_mode {
            Self {
                background: Color32::from_rgb(0x19, 0x19, 0x19),
                text: Color32::from_rgb(0xE6, 0xE6, 0xE4),
                faint: Color32::from_rgb(0x7F, 0x7F, 0x7C),
                subtle: Color32::from_rgb(0x55, 0x55, 0x53),
                code_bg: Color32::from_rgb(0x25, 0x25, 0x25),
                accent: Color32::from_rgb(0x52, 0x9C, 0xCA),
                menu_bg: Color32::from_rgb(0x25, 0x25, 0x25),
                menu_selected: Color32::from_rgb(0x33, 0x33, 0x33),
                border: Color32::from_rgb(0x2E, 0x2E, 0x2E),
                sidebar: Color32::from_rgb(0x20, 0x20, 0x20),
                rail: Color32::from_rgb(0x1C, 0x1C, 0x1C),
                hover: Color32::from_rgb(0x2A, 0x2A, 0x2A),
            }
        } else {
            Self {
                background: Color32::WHITE,
                text: Color32::from_rgb(0x37, 0x35, 0x2F),
                faint: Color32::from_rgb(0x9B, 0x9A, 0x97),
                subtle: Color32::from_rgb(0xD3, 0xD1, 0xCB),
                code_bg: Color32::from_rgb(0xF7, 0xF6, 0xF3),
                accent: Color32::from_rgb(0x23, 0x83, 0xE2),
                menu_bg: Color32::WHITE,
                menu_selected: Color32::from_rgb(0xE9, 0xE9, 0xE7),
                border: Color32::from_rgb(0xE9, 0xE9, 0xE7),
                sidebar: Color32::from_rgb(0xF7, 0xF7, 0xF5),
                rail: Color32::from_rgb(0xF0, 0xF0, 0xEE),
                hover: Color32::from_rgb(0xEF, 0xEF, 0xED),
            }
        }
    }
}

/// Soft tag colors as (background, text), one pair per slot, tuned
/// separately for light and dark mode.
/// An RGB (background, text) pair.
type ColorPair = ((u8, u8, u8), (u8, u8, u8));

const TAG_COLORS_LIGHT: [ColorPair; 8] = [
    ((0xE1, 0xEC, 0xFA), (0x1F, 0x5F, 0xA8)), // blue
    ((0xDF, 0xF1, 0xE6), (0x23, 0x7A, 0x4B)), // green
    ((0xEC, 0xE5, 0xFA), (0x65, 0x3E, 0xB0)), // purple
    ((0xFA, 0xEA, 0xDB), (0xA8, 0x55, 0x1A)), // orange
    ((0xFA, 0xE3, 0xEE), (0xA8, 0x2E, 0x6A)), // pink
    ((0xF8, 0xF0, 0xD2), (0x8A, 0x6A, 0x10)), // yellow
    ((0xFB, 0xE4, 0xE4), (0xB0, 0x32, 0x32)), // red
    ((0xDD, 0xF1, 0xF1), (0x1E, 0x74, 0x78)), // teal
];
const TAG_COLORS_DARK: [ColorPair; 8] = [
    ((0x1F, 0x33, 0x4F), (0x9C, 0xC7, 0xF5)),
    ((0x1D, 0x3A, 0x2C), (0x8E, 0xD6, 0xA9)),
    ((0x33, 0x28, 0x4F), (0xC6, 0xAE, 0xF5)),
    ((0x4A, 0x2F, 0x1C), (0xF3, 0xB7, 0x86)),
    ((0x49, 0x22, 0x37), (0xF2, 0xA6, 0xC9)),
    ((0x45, 0x3A, 0x18), (0xEB, 0xD1, 0x7C)),
    ((0x4C, 0x22, 0x22), (0xF5, 0xA3, 0xA3)),
    ((0x1A, 0x3C, 0x3F), (0x86, 0xD5, 0xD8)),
];

/// The (background, text) colors for a tag or a person's avatar.
pub fn tag_colors(ui: &egui::Ui, name: &str) -> (Color32, Color32) {
    let table = if ui.visuals().dark_mode {
        &TAG_COLORS_DARK
    } else {
        &TAG_COLORS_LIGHT
    };
    let ((br, bg, bb), (fr, fg, fb)) =
        table[scripture_study_core::properties::color_slot(name, table.len())];
    (Color32::from_rgb(br, bg, bb), Color32::from_rgb(fr, fg, fb))
}

/// Font size, weight, and line height for a block's body text.
fn block_metrics(kind: &BlockKind) -> (f32, f32, f32) {
    match kind {
        BlockKind::Heading(1) => (30.0, 700.0, 35.0),
        BlockKind::Heading(2) => (24.0, 650.0, 27.0),
        BlockKind::Heading(_) => (19.0, 600.0, 22.0),
        BlockKind::Code { .. } => (14.0, 400.0, 17.0),
        _ => (16.0, 400.0, 20.0),
    }
}

/// Height of the text itself on one line of a block: the line minus the
/// extra spacing below it. The caret is drawn this tall.
pub fn text_height(ui: &egui::Ui, kind: &BlockKind) -> f32 {
    let (size, _, _) = block_metrics(kind);
    let font = match kind {
        BlockKind::Code { .. } => FontId::monospace(size),
        _ => FontId::proportional(size),
    };
    ui.painter()
        .layout_no_wrap("Ag".into(), font, Color32::WHITE)
        .size()
        .y
}

/// Height of the first line of a block, for aligning gutter decorations.
pub fn line_height(kind: &BlockKind) -> f32 {
    block_metrics(kind).2
}

/// A single line of UI text in Inter SemiBold.
pub fn semibold(ui: &egui::Ui, text: &str, size: f32, color: Color32) -> Arc<egui::Galley> {
    let format = TextFormat {
        font_id: FontId::proportional(size),
        coords: weight(600.0),
        color,
        ..Default::default()
    };
    ui.painter()
        .layout_job(LayoutJob::single_section(text.to_string(), format))
}

fn weight(w: f32) -> VariationCoords {
    VariationCoords::new([(b"wght", w)])
}

/// Underline ink. Pastel swatches are darkened on a light page so the line
/// stays visible; on a dark page the swatch color is already light enough.
fn underline_ink(color: u32, dark: bool) -> Color32 {
    let (r, g, b) = inline::color_rgb(color);
    if dark {
        Color32::from_rgb(r, g, b)
    } else {
        Color32::from_rgb(
            ((u16::from(r) * 58) / 100) as u8,
            ((u16::from(g) * 58) / 100) as u8,
            ((u16::from(b) * 58) / 100) as u8,
        )
    }
}

/// Lays out a block's raw Markdown as rich text. Inline delimiters are shown
/// faintly while `show_markers` (the block is being edited) and hidden
/// otherwise. Link markup is always hidden, so a link reads as just its label.
pub fn layout(
    ui: &egui::Ui,
    text: &str,
    kind: &BlockKind,
    muted: bool,
    show_markers: bool,
    wrap_width: f32,
) -> Arc<egui::Galley> {
    let palette = Palette::for_ui(ui);
    let (size, base_weight, line_height) = block_metrics(kind);
    let color = if muted { palette.faint } else { palette.text };
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;

    if let BlockKind::Code { .. } = kind {
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: FontId::monospace(size),
                line_height: Some(line_height),
                color,
                ..Default::default()
            },
        );
        return ui.painter().layout_job(job);
    }

    // Link markup (`[`, `](url)`) stays hidden even while editing.
    let link_markup = inline::hidden_markup(text);
    for span in inline::parse(text) {
        let style = span.style;
        let always_hidden = span.marker
            && link_markup
                .iter()
                .any(|r| r.start <= span.range.start && span.range.end <= r.end);
        let mut format = TextFormat {
            font_id: FontId::new(
                size,
                if style.italic {
                    italic_family()
                } else {
                    FontFamily::Proportional
                },
            ),
            coords: weight(if style.bold { 700.0 } else { base_weight }),
            line_height: Some(line_height),
            color,
            ..Default::default()
        };
        if style.code {
            format.font_id = FontId::monospace(size * 0.88);
            format.background = palette.code_bg;
            format.color = if muted {
                palette.faint
            } else {
                Color32::from_rgb(0xEB, 0x57, 0x57)
            };
        }
        if style.link && !style.code {
            format.color = if muted { palette.faint } else { palette.accent };
            format.underline = Stroke::new(1.0, format.color.gamma_multiply(0.5));
        }
        let dark = ui.visuals().dark_mode;
        if let Some(color) = style.highlight {
            let (r, g, b) = inline::color_rgb(color);
            let alpha = if dark { 80 } else { 170 };
            format.background = Color32::from_rgba_unmultiplied(r, g, b, alpha);
        }
        if style.footnote {
            // A citation number: small, raised, and in the link color.
            format.font_id.size = size * 0.62;
            format.valign = egui::Align::TOP;
            format.coords = weight(600.0);
            format.color = if muted { palette.faint } else { palette.accent };
            format.underline = Stroke::NONE;
        }
        if let Some(color) = style.underline {
            format.underline = Stroke::new(1.6, underline_ink(color, dark));
        }
        if style.strike || muted {
            format.strikethrough = Stroke::new(1.0, color);
        }
        if span.marker {
            if show_markers && !always_hidden {
                format.color = palette.subtle;
                format.background = Color32::TRANSPARENT;
                format.strikethrough = Stroke::NONE;
                format.underline = Stroke::NONE;
            } else {
                // Keep the characters (so the text is unchanged) but make them vanish.
                format.font_id.size = 0.05;
                format.color = Color32::TRANSPARENT;
                format.background = Color32::TRANSPARENT;
                format.strikethrough = Stroke::NONE;
                format.underline = Stroke::NONE;
            }
        }
        job.append(&text[span.range], 0.0, format);
    }
    if job.sections.is_empty() {
        // An empty block still needs the block's font, or the caret (sized
        // from the row) falls back to egui's small default font.
        job.append(
            "",
            0.0,
            TextFormat {
                font_id: FontId::proportional(size),
                coords: weight(base_weight),
                line_height: Some(line_height),
                color,
                ..Default::default()
            },
        );
    }
    if *kind == BlockKind::Paragraph && text.split('\n').any(|line| line.starts_with('\t')) {
        return layout_indented_lines(ui, job);
    }
    ui.painter().layout_job(job)
}

/// How far each indent level (a Tab) moves text right.
pub const INDENT_WIDTH: f32 = 28.0;

/// Lays out a paragraph whose lines start with tabs: each line is laid out
/// on its own, narrower by its indent and shifted right, so its wrapped rows
/// hang at the indent too. The tabs themselves are hidden.
fn layout_indented_lines(ui: &egui::Ui, job: LayoutJob) -> Arc<egui::Galley> {
    let text = job.text.clone();
    let mut parts = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        let end = start + line.len();
        let tabs = line.chars().take_while(|&c| c == '\t').count();
        let shift = tabs as f32 * INDENT_WIDTH;
        let mut part = LayoutJob {
            text: line.to_string(),
            wrap: egui::text::TextWrapping {
                max_width: (job.wrap.max_width - shift).max(INDENT_WIDTH),
                ..job.wrap.clone()
            },
            halign: job.halign,
            justify: job.justify,
            round_output_to_gui: job.round_output_to_gui,
            ..Default::default()
        };
        for section in &job.sections {
            let (section_start, section_end) =
                (section.byte_range.start.0, section.byte_range.end.0);
            let from = section_start.max(start);
            let to = section_end.min(end);
            if from >= to && !(line.is_empty() && (section_start..=section_end).contains(&start)) {
                continue;
            }
            // Split off the leading tabs, which are hidden.
            let tab_end = (start + tabs).clamp(from, to);
            for (range, hidden) in [(from..tab_end, true), (tab_end..to, false)] {
                if range.is_empty() && !(line.is_empty() && part.sections.is_empty()) {
                    continue;
                }
                let mut format = section.format.clone();
                if hidden {
                    format.font_id.size = 0.05;
                    format.color = Color32::TRANSPARENT;
                    format.background = Color32::TRANSPARENT;
                    format.underline = Stroke::NONE;
                    format.strikethrough = Stroke::NONE;
                }
                part.sections.push(egui::text::LayoutSection {
                    leading_space: 0.0,
                    byte_range: ByteIndex(range.start - start)..ByteIndex(range.end - start),
                    format,
                });
            }
        }
        if part.sections.is_empty() {
            // An empty line still needs the paragraph's font for its height.
            if let Some(section) = job.sections.first() {
                part.sections.push(egui::text::LayoutSection {
                    leading_space: 0.0,
                    byte_range: ByteIndex(0)..ByteIndex(0),
                    format: section.format.clone(),
                });
            }
        }
        let laid_out = ui.painter().layout_job(part);
        let mut galley = (*laid_out).clone();
        for row in &mut galley.rows {
            row.pos.x += shift;
        }
        galley.rect = galley.rect.translate(egui::vec2(shift, 0.0));
        galley.mesh_bounds = galley.mesh_bounds.translate(egui::vec2(shift, 0.0));
        parts.push(Arc::new(galley));
        start = end + 1; // Past the '\n'.
    }
    let pixels_per_point = ui.ctx().pixels_per_point();
    Arc::new(egui::Galley::concat(
        Arc::new(job),
        &parts,
        pixels_per_point,
    ))
}

#[cfg(test)]
mod tests {
    use super::ease_width;

    #[test]
    fn ease_width_snaps_the_first_frame_and_big_jumps() {
        let mut width = 0.0;
        assert!(!ease_width(&mut width, 640.0, 1.0 / 60.0));
        assert_eq!(width, 640.0);

        // The sidebar opens and closes in one step.
        assert!(!ease_width(&mut width, 400.0, 1.0 / 60.0));
        assert_eq!(width, 400.0);
    }

    #[test]
    fn ease_width_follows_a_resize_without_jumping_to_it() {
        let mut width = 640.0;
        assert!(ease_width(&mut width, 670.0, 1.0 / 60.0));
        assert!(width > 640.0 && width < 670.0, "{width}");

        let mut stepped = 640.0;
        assert!(!ease_width(&mut stepped, 670.0, 0.25));
        assert_eq!(stepped, 670.0);
    }
}
