//! Conference talks: downloaded conferences live in the sidebar; search, the
//! download picker, a conference's sessions, and a talk's text fill the page.

use std::collections::BTreeSet;
use std::time::SystemTime;

use eframe::egui::{
    self, pos2,
    text::{LayoutJob, TextFormat},
    vec2, Align, Align2, Color32, FontId, Id, Key, Modifiers, Pos2, Rect, Response, Sense, Ui,
};
use scripture_study_core::talks::{self, Conference, Talk, TalkHit};

use crate::find_bar::{self, FindBar};
use crate::icons;
use crate::menu;
use crate::reader::{
    back_row, escape_pressed, heading, note, panel_heading, results_section, section, text_field,
};
use crate::settings_page::button;
use crate::talk_library::{Status, TalkLibrary};
use crate::theme::{self, Palette};

const TALK_SEARCH: &str = "talk-search";
const INITIAL_RESULTS: usize = 30;
const RESULTS_PAGE: usize = 30;
const TEXT_SIZE: f32 = 16.5;
const LEADING: f32 = 26.0;
const DANGER: Color32 = Color32::from_rgb(0xD4, 0x4C, 0x47);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Place {
    /// Search every downloaded talk.
    #[default]
    Search,
    /// Pick conferences to download.
    Download,
    Conference(Conference),
    Talk {
        conference: Conference,
        session: usize,
        talk: usize,
    },
}

/// The conference talks page. Stays on the last place when closed.
#[derive(Default)]
pub struct TalksPage {
    open: bool,
    place: Place,
    query: String,
    /// The query [`Self::hits`] was built for.
    query_for: String,
    hits: Vec<TalkHit>,
    visible_hits: usize,
    selected: usize,
    /// Conferences checked in the download picker.
    picked: BTreeSet<Conference>,
    /// Paragraph to scroll to on the next draw of a talk.
    reveal: Option<usize>,
    /// Paragraph a search result pointed at.
    highlight: Option<usize>,
    /// Find within the open talk (Cmd+F).
    pub talk_find: FindBar,
    /// Talk the bar last searched, so opening another reveals its match.
    searched_talk: String,
    /// Conference whose sidebar menu is open, and where.
    menu: Option<(Conference, Pos2)>,
    error: Option<String>,
}

impl TalksPage {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn ensure_open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn open_find(&mut self) {
        self.talk_find.open();
    }

    pub fn show_find_bar(&mut self, ctx: &egui::Context, top_right: Pos2, palette: &Palette) {
        self.talk_find
            .show(ctx, top_right, palette, find_bar::TALK_HINT);
    }

    /// Downloads arrived or were removed, so search results point elsewhere.
    pub fn library_changed(&mut self) {
        self.query_for = String::from("\u{0}stale");
    }

    /// Window title while the page is open.
    pub fn title(&self, library: &TalkLibrary) -> Option<String> {
        if !self.open {
            return None;
        }
        Some(match self.place {
            Place::Search => "Conference Talks".to_string(),
            Place::Download => "Download talks".to_string(),
            Place::Conference(conference) => conference.label(),
            Place::Talk { .. } => self
                .talk(library)
                .map_or_else(|| "Conference Talks".to_string(), |talk| talk.title.clone()),
        })
    }

    fn talk<'a>(&self, library: &'a TalkLibrary) -> Option<&'a Talk> {
        let Place::Talk {
            conference,
            session,
            talk,
        } = self.place
        else {
            return None;
        };
        library
            .get(conference)?
            .sessions
            .get(session)?
            .talks
            .get(talk)
    }

    fn go(&mut self, place: Place) {
        self.place = place;
        self.reveal = None;
        self.highlight = None;
        self.menu = None;
    }

    fn go_back(&mut self) {
        match self.place {
            Place::Search => self.open = false,
            Place::Download | Place::Conference(_) => self.go(Place::Search),
            Place::Talk { conference, .. } => self.go(Place::Conference(conference)),
        }
    }

    /// The sidebar: where to go, download progress, and what's downloaded.
    pub fn show_index(&mut self, ui: &mut Ui, palette: &Palette, library: &mut TalkLibrary) {
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
        panel_heading(ui, palette, "Conference Talks");
        ui.add_space(8.0);
        let mut nav = None;
        // Inset like the notes list, so rows don't touch the panel's edges.
        let area = ui.available_rect_before_wrap().shrink2(vec2(10.0, 0.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            if icon_row(
                ui,
                "Search talks",
                self.place == Place::Search,
                palette,
                icons::search,
            ) {
                nav = Some(Place::Search);
            }
            if icon_row(
                ui,
                "Download talks",
                self.place == Place::Download,
                palette,
                icons::download,
            ) {
                nav = Some(Place::Download);
            }
            let status = library.status();
            if status.is_busy() {
                ui.add_space(10.0);
                progress(ui, &status, palette);
            }
            if let Some(error) = &self.error {
                ui.add_space(6.0);
                wrapped(ui, error, 12.5, DANGER);
            }
            ui.add_space(14.0);
            section(ui, "Downloaded", palette);
            ui.add_space(4.0);
            let current = match self.place {
                Place::Conference(c) | Place::Talk { conference: c, .. } => Some(c),
                _ => None,
            };
            egui::ScrollArea::vertical()
                .id_salt("talk-index")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if library.conferences().is_empty() {
                        note(ui, "Nothing downloaded yet", palette);
                    }
                    for downloaded in library.conferences() {
                        let conference = downloaded.conference;
                        let response = conference_row(
                            ui,
                            &conference.label(),
                            &downloaded.talk_count().to_string(),
                            current == Some(conference),
                            palette,
                        );
                        if response.clicked() {
                            nav = Some(Place::Conference(conference));
                        }
                        if response.secondary_clicked() {
                            let at = ui
                                .input(|i| i.pointer.interact_pos())
                                .unwrap_or(response.rect.right_bottom());
                            self.menu = Some((conference, at));
                        }
                    }
                    ui.add_space(12.0);
                });
        });
        if let Some(place) = nav {
            self.go(place);
        }
        if let Some((conference, at)) = self.menu.take() {
            let (remove, open) = menu::menu_at(
                ui.ctx(),
                Id::new("talk-conference-menu"),
                at,
                palette,
                false,
                |ui| {
                    menu::Item::new("Remove download")
                        .icon(icons::trash)
                        .danger()
                        .show(ui, palette)
                },
            );
            if remove {
                self.error = library
                    .remove(conference)
                    .err()
                    .map(|e| format!("Couldn't remove {}: {e}", conference.label()));
                self.library_changed();
                if matches!(self.place, Place::Conference(c) | Place::Talk { conference: c, .. } if c == conference)
                {
                    self.go(Place::Search);
                }
            } else if open {
                self.menu = Some((conference, at));
            }
        }
    }

    /// The page beside the sidebar. `index_open` is false while the sidebar
    /// is collapsed.
    pub fn show_page(&mut self, ui: &mut Ui, palette: &Palette, library: &mut TalkLibrary) {
        if escape_pressed(ui) && !self.talk_find.query_focused(ui.ctx()) {
            let focused = ui.memory(|m| m.has_focus(Id::new(TALK_SEARCH)));
            if focused && !self.query.is_empty() {
                self.query.clear();
            } else if focused {
                ui.memory_mut(|m| m.stop_text_input());
            } else {
                self.go_back();
            }
            ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        }
        // A conference that was removed can't be shown.
        let gone = match self.place {
            Place::Conference(c) => !library.has(c),
            Place::Talk { .. } => self.talk(library).is_none(),
            _ => false,
        };
        if gone {
            self.go(Place::Search);
        }
        self.sync_find(library);
        let nav = match self.place {
            Place::Search => self.show_search(ui, palette, library),
            Place::Download => self.show_download(ui, palette, library),
            Place::Conference(conference) => show_conference(ui, palette, library, conference),
            Place::Talk { .. } => self.show_talk(ui, palette, library),
        };
        if let Some((place, paragraph)) = nav {
            self.go(place);
            self.reveal = paragraph;
            self.highlight = paragraph;
        }
    }

    fn sync_find(&mut self, library: &TalkLibrary) {
        let talk = self.talk(library);
        let key = talk.map_or(String::new(), |talk| talk.uri.clone());
        if self.talk_find.open && self.searched_talk != key {
            self.searched_talk = key;
            self.talk_find.current = 0;
            self.talk_find.reveal = true;
        }
        if !self.talk_find.open {
            self.talk_find.update_plain(&[]);
            return;
        }
        let texts: Vec<&str> = talk
            .map(|talk| talk.paragraphs.iter().map(|p| p.text.as_str()).collect())
            .unwrap_or_default();
        self.talk_find.update_plain(&texts);
    }

    fn refresh_search(&mut self, library: &TalkLibrary) {
        if self.query_for == self.query {
            return;
        }
        self.query_for = self.query.clone();
        self.selected = 0;
        self.hits = library.search(&self.query, usize::MAX);
        self.visible_hits = self.hits.len().min(INITIAL_RESULTS);
    }

    fn show_search(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        library: &TalkLibrary,
    ) -> Option<(Place, Option<usize>)> {
        let count = library.conferences().len();
        let subtitle = match count {
            0 => "Download conferences to search their talks".to_string(),
            1 => format!("Search {} talks from 1 conference", library.talk_count()),
            n => format!("Search {} talks from {n} conferences", library.talk_count()),
        };
        heading(ui, palette, "Conference Talks", Some(&subtitle));
        ui.add_space(18.0);
        if count == 0 {
            note(ui, "No talks downloaded yet", palette);
            ui.add_space(8.0);
            let mut chosen = None;
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                if button(ui, "Choose conferences", true, true, palette).clicked() {
                    chosen = Some((Place::Download, None));
                }
            });
            return chosen;
        }
        self.refresh_search(library);
        let mut chosen = self.search_field(ui, palette, library);
        ui.add_space(22.0);
        if self.query.trim().is_empty() {
            note(ui, "Try “hope in Christ” or a speaker’s name", palette);
            return chosen;
        }
        results_section(ui, &format!("Results ({})", self.hits.len()), palette);
        ui.add_space(6.0);
        if self.hits.is_empty() {
            note(ui, "No matching talks", palette);
            return chosen;
        }
        let loaded = self.visible_hits.min(self.hits.len());
        let output = egui::ScrollArea::vertical()
            .id_salt("talk-search-results")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (n, hit) in self.hits[..loaded].iter().enumerate() {
                    if hit_row(ui, library, hit, n == self.selected, palette) {
                        chosen = open_hit(library, hit);
                    }
                }
            });
        let at_bottom =
            output.state.offset.y + output.inner_rect.height() >= output.content_size.y - 48.0;
        if at_bottom && self.visible_hits < self.hits.len() {
            self.visible_hits = (self.visible_hits + RESULTS_PAGE).min(self.hits.len());
            ui.ctx().request_repaint();
        }
        chosen
    }

    fn search_field(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        library: &TalkLibrary,
    ) -> Option<(Place, Option<usize>)> {
        let id = Id::new(TALK_SEARCH);
        let mut chosen = None;
        if ui.memory(|m| m.has_focus(id)) {
            let n = self.hits.len().min(self.visible_hits);
            let key = |ui: &mut Ui, key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
            if n > 0 && key(ui, Key::ArrowDown) {
                self.selected = (self.selected + 1).min(n - 1);
            }
            if key(ui, Key::ArrowUp) {
                self.selected = self.selected.saturating_sub(1);
            }
            if n > 0 && key(ui, Key::Enter) {
                chosen = open_hit(library, &self.hits[self.selected]);
            }
        }
        text_field(ui, TALK_SEARCH, &mut self.query, "Search talks", palette);
        chosen
    }

    fn show_download(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        library: &mut TalkLibrary,
    ) -> Option<(Place, Option<usize>)> {
        heading(
            ui,
            palette,
            "Download talks",
            Some("Pick years or conferences, then download every talk in them"),
        );
        ui.add_space(16.0);
        let status = library.status();
        self.picked
            .retain(|c| !library.has(*c) && !status.is_pending(*c));
        let mut download = false;
        let mut cancel = false;
        let mut clear = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
            let label = match self.picked.len() {
                0 => "Download".to_string(),
                1 => "Download 1 conference".to_string(),
                n => format!("Download {n} conferences"),
            };
            download = button(ui, &label, true, !self.picked.is_empty(), palette).clicked();
            if !self.picked.is_empty() {
                clear = button(ui, "Clear", false, true, palette).clicked();
            }
            if status.is_busy() {
                cancel = button(ui, "Cancel downloads", false, true, palette).clicked();
            }
        });
        if download {
            library.download(std::mem::take(&mut self.picked));
        }
        if clear {
            self.picked.clear();
        }
        if cancel {
            library.cancel();
        }
        if status.is_busy() {
            ui.add_space(12.0);
            progress(ui, &status, palette);
        }
        if let Some((conference, error)) = &status.error {
            ui.add_space(10.0);
            wrapped(
                ui,
                &format!("Couldn’t download {}: {error}", conference.label()),
                13.0,
                DANGER,
            );
        }
        ui.add_space(14.0);

        let all = Conference::all_through(Conference::latest(SystemTime::now()));
        let mut years: Vec<u16> = all.iter().map(|c| c.year).collect();
        years.dedup();
        let mut opened = None;
        egui::ScrollArea::vertical()
            .id_salt("talk-download-years")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                for year in years {
                    let held: Vec<Conference> =
                        [Conference::april(year), Conference::october(year)]
                            .into_iter()
                            .filter(|c| all.contains(c))
                            .collect();
                    let choosable: Vec<Conference> = held
                        .iter()
                        .copied()
                        .filter(|c| !library.has(*c) && !status.is_pending(*c))
                        .collect();
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
                    let year_rect = Rect::from_min_size(rect.min, vec2(96.0, rect.height()));
                    let all_picked =
                        !choosable.is_empty() && choosable.iter().all(|c| self.picked.contains(c));
                    if year_chip(
                        ui,
                        year_rect,
                        year,
                        all_picked,
                        !choosable.is_empty(),
                        palette,
                    ) {
                        if all_picked {
                            for c in &choosable {
                                self.picked.remove(c);
                            }
                        } else {
                            self.picked.extend(choosable.iter().copied());
                        }
                    }
                    for conference in held {
                        let column = if conference.month == 4 { 0.0 } else { 1.0 };
                        let chip = Rect::from_min_size(
                            pos2(year_rect.right() + 8.0 + column * 156.0, rect.top() + 4.0),
                            vec2(148.0, rect.height() - 8.0),
                        );
                        let state = if library.has(conference) {
                            Chip::Downloaded
                        } else if let Some(p) =
                            status.current.filter(|p| p.conference == conference)
                        {
                            Chip::Downloading(p.done, p.total)
                        } else if status.queued.contains(&conference) {
                            Chip::Queued
                        } else if self.picked.contains(&conference) {
                            Chip::Picked
                        } else {
                            Chip::Open
                        };
                        if conference_chip(ui, chip, conference, state, palette) {
                            match state {
                                Chip::Downloaded => opened = Some(conference),
                                Chip::Picked => {
                                    self.picked.remove(&conference);
                                }
                                Chip::Open => {
                                    self.picked.insert(conference);
                                }
                                Chip::Downloading(..) | Chip::Queued => {}
                            }
                        }
                    }
                }
                ui.add_space(24.0);
            });
        opened.map(|c| (Place::Conference(c), None))
    }

    fn show_talk(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        library: &TalkLibrary,
    ) -> Option<(Place, Option<usize>)> {
        let Place::Talk { conference, .. } = self.place else {
            return None;
        };
        let talk = self.talk(library)?;
        let back = back_row(ui, palette, &conference.label());
        ui.add_space(10.0);
        let reveal = self.reveal.take();
        let highlight = self.highlight;
        let find = &mut self.talk_find;
        egui::ScrollArea::vertical()
            .id_salt(("talk", talk.uri.as_str()))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let width = ui.available_width();
                let title = theme::semibold_wrapped(ui, &talk.title, 28.0, palette.text, width);
                let (rect, _) = ui.allocate_exact_size(title.size(), Sense::hover());
                ui.painter().galley(rect.min, title, palette.text);
                ui.add_space(10.0);
                if !talk.speaker.is_empty() {
                    wrapped(ui, &talk.speaker, 14.5, palette.text);
                }
                if !talk.role.is_empty() {
                    ui.add_space(2.0);
                    wrapped(ui, &talk.role, 13.0, palette.faint);
                }
                if !talk.kicker.is_empty() {
                    ui.add_space(14.0);
                    let mut job = LayoutJob::default();
                    job.wrap.max_width = width;
                    job.append(
                        &talk.kicker,
                        0.0,
                        TextFormat {
                            font_id: FontId::new(15.5, theme::italic_family()),
                            line_height: Some(23.0),
                            color: palette.faint,
                            ..Default::default()
                        },
                    );
                    let galley = ui.painter().layout_job(job);
                    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                    ui.painter().galley(rect.min, galley, palette.faint);
                }
                ui.add_space(22.0);
                paragraphs(ui, palette, talk, find, reveal, highlight);
                ui.add_space(48.0);
            });
        back.then_some((Place::Conference(conference), None))
    }
}

/// A conference's sessions and their talks.
fn show_conference(
    ui: &mut Ui,
    palette: &Palette,
    library: &TalkLibrary,
    conference: Conference,
) -> Option<(Place, Option<usize>)> {
    let downloaded = library.get(conference)?;
    let mut chosen = None;
    if back_row(ui, palette, "Conference Talks") {
        chosen = Some((Place::Search, None));
    }
    ui.add_space(10.0);
    let subtitle = format!("{} talks", downloaded.talk_count());
    heading(ui, palette, &conference.label(), Some(&subtitle));
    ui.add_space(18.0);
    egui::ScrollArea::vertical()
        .id_salt(("talk-conference", conference.key()))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            for (s, session) in downloaded.sessions.iter().enumerate() {
                if s > 0 {
                    ui.add_space(16.0);
                }
                section(ui, &session.title, palette);
                ui.add_space(4.0);
                for (t, talk) in session.talks.iter().enumerate() {
                    if two_line_row(ui, &talk.title, &talk.speaker, None, false, palette) {
                        chosen = Some((
                            Place::Talk {
                                conference,
                                session: s,
                                talk: t,
                            },
                            None,
                        ));
                    }
                }
            }
            ui.add_space(36.0);
        });
    chosen
}

fn open_hit(library: &TalkLibrary, hit: &TalkHit) -> Option<(Place, Option<usize>)> {
    let conference = library.conferences().get(hit.conference)?.conference;
    let paragraph = match hit.place {
        talks::Place::Talk => None,
        talks::Place::Paragraph(p) => Some(p),
    };
    Some((
        Place::Talk {
            conference,
            session: hit.session,
            talk: hit.talk,
        },
        paragraph,
    ))
}

/// The talk's text, with find matches marked and the searched-for
/// paragraph tinted.
fn paragraphs(
    ui: &mut Ui,
    palette: &Palette,
    talk: &Talk,
    find: &mut FindBar,
    reveal: Option<usize>,
    highlight: Option<usize>,
) {
    let (all_color, current_color) = find_bar::match_colors(ui);
    let current = find.current_match();
    let mut revealed = false;
    let width = ui.available_width();
    for (index, paragraph) in talk.paragraphs.iter().enumerate() {
        let (size, leading, top) = if paragraph.heading {
            (19.0, 27.0, 14.0)
        } else {
            (TEXT_SIZE, LEADING, 0.0)
        };
        ui.add_space(top);
        let galley = if paragraph.heading {
            theme::semibold_wrapped(ui, &paragraph.text, size, palette.text, width)
        } else {
            let mut job = LayoutJob::default();
            job.wrap.max_width = width;
            job.append(
                &paragraph.text,
                0.0,
                TextFormat {
                    font_id: FontId::proportional(size),
                    line_height: Some(leading),
                    color: palette.text,
                    ..Default::default()
                },
            );
            ui.painter().layout_job(job)
        };
        let (rect, response) =
            ui.allocate_exact_size(vec2(width, galley.size().y + 14.0), Sense::hover());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &paragraph.text)
        });
        if highlight == Some(index) {
            ui.painter().rect_filled(
                rect.expand2(vec2(8.0, 0.0)),
                4.0,
                palette.menu_selected.gamma_multiply(0.45),
            );
        }
        // Half the row's spacing above the text, so a tint sits evenly.
        let origin = rect.min + vec2(0.0, 7.0);
        for m in find.matches.iter().filter(|m| m.block == index) {
            let color = if Some(*m) == current {
                current_color
            } else {
                all_color
            };
            for shape in find_bar::highlight_shapes(&galley, origin, m.start, m.end, false, color) {
                ui.painter().add(shape);
            }
        }
        if let Some(m) = current.filter(|m| find.reveal && m.block == index) {
            let target = find_bar::match_rect(&galley, origin, m.start);
            ui.scroll_to_rect(target.expand(48.0), Some(Align::Center));
            revealed = true;
        }
        ui.painter().galley(origin, galley, palette.text);
        if reveal == Some(index) {
            ui.scroll_to_rect(rect, Some(Align::Center));
        }
    }
    if revealed {
        find.reveal = false;
    }
}

fn progress(ui: &mut Ui, status: &Status, palette: &Palette) {
    let Some(current) = status.current else {
        let waiting = format!(
            "{} waiting to download",
            count(status.queued.len(), "conference")
        );
        wrapped(ui, &waiting, 12.5, palette.faint);
        return;
    };
    wrapped(
        ui,
        &format!("Downloading {}", current.conference.label()),
        13.0,
        palette.text,
    );
    ui.add_space(2.0);
    let mut detail = if current.total == 0 {
        "Reading the list of talks".to_string()
    } else {
        format!("{} of {} talks", current.done, current.total)
    };
    if !status.queued.is_empty() {
        detail.push_str(&format!(" · {} more", status.queued.len()));
    }
    wrapped(ui, &detail, 12.5, palette.faint);
    ui.add_space(6.0);
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width().min(320.0), 4.0), Sense::hover());
    ui.painter().rect_filled(rect, 2.0, palette.menu_selected);
    let fraction = if current.total == 0 {
        0.0
    } else {
        current.done as f32 / current.total as f32
    };
    let mut filled = rect;
    filled.set_width(rect.width() * fraction);
    ui.painter().rect_filled(filled, 2.0, palette.accent);
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Text wrapped to the available width.
fn wrapped(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: FontId::proportional(size),
            color,
            ..Default::default()
        },
    );
    let galley = ui.painter().layout_job(job);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    ui.painter().galley(rect.min, galley, color);
}

fn icon_row(
    ui: &mut Ui,
    label: &str,
    selected: bool,
    palette: &Palette,
    icon: fn(&egui::Painter, Pos2, Color32),
) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    paint_row(ui, rect, &response, selected, palette);
    icon(
        ui.painter(),
        rect.left_center() + vec2(16.0, 0.0),
        palette.text,
    );
    ui.painter().text(
        rect.left_center() + vec2(34.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        palette.text,
    );
    response.clicked()
}

/// A downloaded conference: its name and how many talks it has.
fn conference_row(
    ui: &mut Ui,
    label: &str,
    detail: &str,
    selected: bool,
    palette: &Palette,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    paint_row(ui, rect, &response, selected, palette);
    ui.painter().text(
        rect.left_center() + vec2(8.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        palette.text,
    );
    ui.painter().text(
        rect.right_center() - vec2(10.0, 0.0),
        Align2::RIGHT_CENTER,
        detail,
        FontId::proportional(12.0),
        palette.faint,
    );
    response
}

fn paint_row(ui: &Ui, rect: Rect, response: &Response, selected: bool, palette: &Palette) {
    if selected {
        ui.painter().rect_filled(rect, 7.0, palette.menu_selected);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 7.0, palette.hover);
    }
}

/// A talk in a list: title, then speaker (and conference), then an
/// optional snippet.
fn two_line_row(
    ui: &mut Ui,
    title: &str,
    detail: &str,
    snippet: Option<&str>,
    selected: bool,
    palette: &Palette,
) -> bool {
    let height = if snippet.is_some() { 64.0 } else { 48.0 };
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    paint_row(ui, rect, &response, selected, palette);
    let inner = rect.shrink2(vec2(8.0, 0.0));
    let width = inner.width() - 20.0;
    let lines = [
        Some(crate::sidebar::elided(ui, title, 14.5, palette.text, width)),
        Some(crate::sidebar::elided(
            ui,
            detail,
            12.5,
            palette.faint,
            width,
        )),
        snippet.map(|s| crate::sidebar::elided(ui, s, 12.5, palette.faint, width)),
    ];
    let total: f32 = lines
        .iter()
        .flatten()
        .map(|g| g.size().y + 2.0)
        .sum::<f32>()
        - 2.0;
    let mut y = inner.center().y - total / 2.0;
    for galley in lines.into_iter().flatten() {
        let height = galley.size().y;
        ui.painter()
            .galley(pos2(inner.left(), y), galley, palette.text);
        y += height + 2.0;
    }
    icons::chevron(
        ui.painter(),
        pos2(rect.right() - 14.0, rect.center().y),
        false,
        palette.faint,
    );
    response.clicked()
}

fn hit_row(
    ui: &mut Ui,
    library: &TalkLibrary,
    hit: &TalkHit,
    selected: bool,
    palette: &Palette,
) -> bool {
    let Some(downloaded) = library.conferences().get(hit.conference) else {
        return false;
    };
    let Some(talk) = downloaded
        .sessions
        .get(hit.session)
        .and_then(|s| s.talks.get(hit.talk))
    else {
        return false;
    };
    let detail = format!("{} · {}", talk.speaker, downloaded.conference.label());
    let snippet = (!hit.snippet.is_empty()).then_some(hit.snippet.as_str());
    two_line_row(ui, &talk.title, &detail, snippet, selected, palette)
}

/// The year at the start of a picker row. Clicking it picks (or unpicks)
/// both of the year's conferences.
fn year_chip(
    ui: &mut Ui,
    rect: Rect,
    year: u16,
    picked: bool,
    enabled: bool,
    palette: &Palette,
) -> bool {
    let label = year.to_string();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, Id::new(("talk-year", year)), sense);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, picked, &label)
    });
    if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect.shrink2(vec2(0.0, 4.0)), 7.0, palette.hover);
    }
    let color = if enabled { palette.text } else { palette.faint };
    let galley = theme::semibold(ui, &label, 15.0, color);
    let pos = rect.left_center() + vec2(10.0, -galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
    response.clicked()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Chip {
    Open,
    Picked,
    Queued,
    Downloading(usize, usize),
    Downloaded,
}

fn conference_chip(
    ui: &mut Ui,
    rect: Rect,
    conference: Conference,
    state: Chip,
    palette: &Palette,
) -> bool {
    let label = conference.label();
    let clickable = matches!(state, Chip::Open | Chip::Picked | Chip::Downloaded);
    let sense = if clickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, Id::new(("talk-chip", conference.key())), sense);
    let checked = matches!(state, Chip::Picked | Chip::Downloaded);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, clickable, checked, &label)
    });
    let (fill, color) = match state {
        Chip::Picked => (palette.accent, Color32::WHITE),
        Chip::Downloaded | Chip::Queued | Chip::Downloading(..) => {
            (palette.menu_selected.gamma_multiply(0.6), palette.faint)
        }
        Chip::Open if response.hovered() => (palette.hover, palette.text),
        Chip::Open => (Color32::TRANSPARENT, palette.text),
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    if state == Chip::Open {
        ui.painter().rect_stroke(
            rect,
            7.0,
            egui::Stroke::new(1.0, palette.border),
            egui::StrokeKind::Inside,
        );
    }
    let text = match state {
        Chip::Queued => format!("{} · queued", conference.season()),
        Chip::Downloading(done, total) if total > 0 => {
            format!("{} · {done}/{total}", conference.season())
        }
        Chip::Downloading(..) => format!("{} · starting", conference.season()),
        _ => conference.season().to_string(),
    };
    let mut x = rect.left() + 12.0;
    if checked {
        icons::check(ui.painter(), pos2(x + 4.0, rect.center().y), color);
        x += 16.0;
    }
    ui.painter().text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.5),
        color,
    );
    if state == Chip::Downloaded && response.hovered() {
        ui.painter().text(
            rect.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            "Open",
            FontId::proportional(12.0),
            palette.faint,
        );
    }
    response.clicked()
}
