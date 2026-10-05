//! The "Add citation" form: a modal that collects a source's details,
//! looks up a pasted link to fill them in, and previews the MLA citation.

use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, vec2, Color32, CornerRadius, FontId, Frame, Id, Key, Margin, RichText, Sense, Stroke,
    TextEdit, Ui,
};
use scripture_study_core::{
    citations::{self, Source, SourceKind},
    links, BlockKind,
};

use crate::link_menu::LinkTarget;
use crate::theme::{self, Palette};

const WIDTH: f32 = 480.0;
const LABEL_WIDTH: f32 = 112.0;
/// Wait this long after the link stops changing before looking it up.
const LOOKUP_DELAY: f64 = 0.4;

/// Downloads a web page's HTML (swapped out in tests).
pub type Fetch = fn(&str) -> Result<String, String>;

enum Lookup {
    Idle,
    Running(Arc<Mutex<Option<Result<Source, String>>>>),
    Filled,
    Failed(String),
}

pub enum Outcome {
    Open,
    Cancel,
    /// Insert this formatted citation.
    Insert(String),
}

pub struct CitationForm {
    /// Where the citation goes: block and character.
    pub block: usize,
    pub char: usize,
    /// A link being turned into this citation: it becomes plain text with
    /// the citation's number after it.
    pub converting: Option<LinkTarget>,
    pub source: Source,
    /// The type was picked by hand, so a lookup won't change it.
    kind_chosen: bool,
    /// The link last looked up, and when the link field last changed.
    looked_up: String,
    url_changed_at: f64,
    lookup: Lookup,
    focus: bool,
}

impl CitationForm {
    pub fn new(block: usize, char: usize) -> Self {
        Self {
            block,
            char,
            converting: None,
            source: Source {
                accessed: citations::today(),
                ..Default::default()
            },
            kind_chosen: false,
            looked_up: String::new(),
            url_changed_at: 0.0,
            lookup: Lookup::Idle,
            focus: true,
        }
    }

    /// A form for citing what a link points to, starting from its address
    /// and (as the title) its text. The page lookup fills in the rest.
    pub fn from_link(target: LinkTarget, title: String) -> Self {
        let mut form = Self::new(target.block, 0);
        form.source.url = target.url.clone();
        form.source.title = title;
        form.converting = Some(target);
        form.focus = false;
        form
    }

    pub fn show(&mut self, ctx: &egui::Context, palette: &Palette, fetch: Fetch) -> Outcome {
        self.poll_lookup();
        let mut outcome = Outcome::Open;
        let modal = egui::Modal::new(Id::new("citation-form"))
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
                ui.set_width(WIDTH);
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                let title = theme::semibold(ui, "Add citation", 18.0, palette.text);
                let (rect, _) = ui.allocate_exact_size(title.size(), Sense::hover());
                ui.painter().galley(rect.min, title, palette.text);
                ui.add_space(4.0);

                self.kind_picker(ui, palette);
                ui.add_space(4.0);
                self.fields(ui, palette);
                self.start_lookup(ui, fetch);
                ui.add_space(6.0);
                self.preview(ui, palette);
                ui.add_space(8.0);
                self.buttons(ui, palette)
            });
        let close = modal.should_close();
        if let Some(chosen) = modal.inner {
            outcome = chosen;
        }
        if close && matches!(outcome, Outcome::Open) {
            outcome = Outcome::Cancel;
        }
        // Enter anywhere in the form adds the citation.
        if matches!(outcome, Outcome::Open)
            && self.can_insert()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
        {
            outcome = Outcome::Insert(self.source.format_mla());
        }
        outcome
    }

    fn can_insert(&self) -> bool {
        !self.source.title.trim().is_empty() || links::as_url(&self.source.url).is_some()
    }

    fn kind_picker(&mut self, ui: &mut Ui, palette: &Palette) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for kind in SourceKind::ALL {
                let selected = self.source.kind == kind;
                let text = RichText::new(kind.label()).size(13.0).color(if selected {
                    palette.text
                } else {
                    palette.faint
                });
                let button = egui::Button::new(text)
                    .fill(if selected {
                        palette.menu_selected
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::new(1.0, palette.border))
                    .corner_radius(CornerRadius::same(14))
                    .min_size(vec2(0.0, 28.0));
                if ui.add(button).clicked() {
                    self.source.kind = kind;
                    self.kind_chosen = true;
                }
            }
        });
    }

    fn fields(&mut self, ui: &mut Ui, palette: &Palette) {
        let kind = self.source.kind;
        let url_status = match &self.lookup {
            Lookup::Idle => "Paste a link to fill in the rest".to_string(),
            Lookup::Running(_) => "Looking up the page…".to_string(),
            Lookup::Filled => "Filled in from the page — check the details".to_string(),
            Lookup::Failed(why) => format!("Couldn't read the page ({why})"),
        };
        let now = ui.input(|i| i.time);

        let url = field(
            ui,
            palette,
            "Link",
            &mut self.source.url,
            "https://…",
            std::mem::take(&mut self.focus),
        );
        if url.changed() {
            self.url_changed_at = now;
            if !matches!(self.lookup, Lookup::Running(_)) {
                self.lookup = Lookup::Idle;
            }
        }
        ui.horizontal(|ui| {
            ui.add_space(LABEL_WIDTH + 8.0);
            ui.label(RichText::new(url_status).size(12.0).color(palette.faint));
        });

        let s = &mut self.source;
        field(
            ui,
            palette,
            "Author(s)",
            &mut s.authors,
            "First Last; First Last",
            false,
        );
        let title_label = match kind {
            SourceKind::Website => "Page title",
            SourceKind::Book => "Title",
            SourceKind::Article => "Article title",
        };
        field(ui, palette, title_label, &mut s.title, "", false);
        match kind {
            SourceKind::Website => {
                field(ui, palette, "Website", &mut s.container, "", false);
                field(
                    ui,
                    palette,
                    "Publisher",
                    &mut s.publisher,
                    "If different from the website",
                    false,
                );
                field(
                    ui,
                    palette,
                    "Published",
                    &mut s.published,
                    "YYYY-MM-DD",
                    false,
                );
                field(
                    ui,
                    palette,
                    "Accessed",
                    &mut s.accessed,
                    "YYYY-MM-DD",
                    false,
                );
            }
            SourceKind::Book => {
                field(ui, palette, "Publisher", &mut s.publisher, "", false);
                field(ui, palette, "Year", &mut s.published, "YYYY", false);
                field(ui, palette, "Pages", &mut s.pages, "e.g. 12-15", false);
            }
            SourceKind::Article => {
                field(ui, palette, "Journal", &mut s.container, "", false);
                ui.horizontal(|ui| {
                    ui.add_space(LABEL_WIDTH + 8.0);
                    let small = |ui: &mut Ui, value: &mut String, hint: &str| {
                        ui.add(
                            TextEdit::singleline(value)
                                .hint_text(hint)
                                .desired_width(90.0)
                                .margin(Margin::symmetric(8, 6)),
                        )
                    };
                    small(ui, &mut s.volume, "Volume");
                    small(ui, &mut s.issue, "Issue");
                    small(ui, &mut s.pages, "Pages");
                });
                field(
                    ui,
                    palette,
                    "Published",
                    &mut s.published,
                    "YYYY-MM-DD",
                    false,
                );
            }
        }
    }

    /// Looks up the link once it has settled, on a background thread.
    fn start_lookup(&mut self, ui: &Ui, fetch: Fetch) {
        let Some(url) = links::as_url(&self.source.url) else {
            return;
        };
        if url == self.looked_up || matches!(self.lookup, Lookup::Running(_)) {
            return;
        }
        let waited = ui.input(|i| i.time) - self.url_changed_at;
        if waited < LOOKUP_DELAY {
            ui.ctx()
                .request_repaint_after_secs((LOOKUP_DELAY - waited) as f32);
            return;
        }
        self.looked_up = url.clone();
        let slot = Arc::new(Mutex::new(None));
        self.lookup = Lookup::Running(slot.clone());
        let ctx = ui.ctx().clone();
        std::thread::spawn(move || {
            let found = fetch(&url).map(|html| Source::from_html(&html, &url));
            if let Ok(mut slot) = slot.lock() {
                *slot = Some(found);
            }
            ctx.request_repaint();
        });
    }

    fn poll_lookup(&mut self) {
        let Lookup::Running(slot) = &self.lookup else {
            return;
        };
        let Some(found) = slot.lock().ok().and_then(|mut s| s.take()) else {
            return;
        };
        self.lookup = match found {
            Ok(found) => {
                if !self.kind_chosen {
                    self.source.kind = found.kind;
                }
                self.source.fill_from(&found);
                if found.title.is_empty() && found.authors.is_empty() {
                    // Often a sign-in or bot check instead of the page.
                    Lookup::Failed("it didn't share any details".into())
                } else {
                    Lookup::Filled
                }
            }
            Err(why) => Lookup::Failed(why),
        };
    }

    fn preview(&self, ui: &mut Ui, palette: &Palette) {
        ui.label(RichText::new("Preview").size(12.0).color(palette.faint));
        let text = self.source.format_mla();
        Frame::new()
            .fill(palette.code_bg)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if text.is_empty() {
                    ui.label(
                        RichText::new("Fill in the source to see its citation.")
                            .size(14.0)
                            .color(palette.faint),
                    );
                } else {
                    let galley = theme::layout(
                        ui,
                        &text,
                        &BlockKind::Paragraph,
                        false,
                        false,
                        ui.available_width(),
                    );
                    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                    ui.painter().galley(rect.min, galley, palette.text);
                }
            });
    }

    fn buttons(&self, ui: &mut Ui, palette: &Palette) -> Option<Outcome> {
        let mut outcome = None;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let add = egui::Button::new(
                RichText::new("Add citation")
                    .size(14.0)
                    .color(Color32::WHITE),
            )
            .fill(palette.accent)
            .corner_radius(CornerRadius::same(8))
            .min_size(vec2(0.0, 32.0));
            if ui.add_enabled(self.can_insert(), add).clicked() {
                outcome = Some(Outcome::Insert(self.source.format_mla()));
            }
            let cancel = egui::Button::new(RichText::new("Cancel").size(14.0).color(palette.text))
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0, palette.border))
                .corner_radius(CornerRadius::same(8))
                .min_size(vec2(0.0, 32.0));
            if ui.add(cancel).clicked() {
                outcome = Some(Outcome::Cancel);
            }
        });
        outcome
    }
}

/// A labeled single-line field.
fn field(
    ui: &mut Ui,
    palette: &Palette,
    label: &str,
    value: &mut String,
    hint: &str,
    focus: bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL_WIDTH, 30.0), Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.5),
            palette.faint,
        );
        let response = ui.add(
            TextEdit::singleline(value)
                .id(Id::new(("citation-field", label)))
                .hint_text(RichText::new(hint).color(palette.subtle))
                .font(FontId::proportional(14.0))
                .desired_width(ui.available_width())
                .margin(Margin::symmetric(8, 6)),
        );
        if focus {
            response.request_focus();
        }
        response
    })
    .inner
}

/// Downloads a page with the system's `curl`, which follows redirects and
/// handles compression and TLS for us.
pub fn fetch_with_curl(url: &str) -> Result<String, String> {
    const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
        AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";
    let url = links::as_url(url).ok_or("not a web address")?;
    let output = std::process::Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--compressed",
            "--max-time",
            "12",
            "--max-filesize",
            "8000000",
            "--user-agent",
            USER_AGENT,
            "--header",
            "Accept: text/html,application/xhtml+xml",
            "--",
            &url,
        ])
        .output()
        .map_err(|e| format!("couldn't run curl: {e}"))?;
    if !output.status.success() {
        let why = String::from_utf8_lossy(&output.stderr);
        let why = why.trim().trim_start_matches("curl: ");
        return Err(if why.is_empty() {
            "no response".into()
        } else {
            why.to_string()
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
