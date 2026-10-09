//! Scriptures: the book list lives in the sidebar, and the reader/search page opens beside it.

use eframe::egui::{
    self, pos2,
    text::{LayoutJob, TextFormat},
    vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Key, Margin, Modifiers, Pos2,
    Rect, Sense, TextEdit, Ui,
};
use scripture_study_core::scriptures::{self, Passage, VerseHit};

use crate::find_bar::{self, FindBar};
use crate::icons;
use crate::menu;
use crate::theme::{self, Palette};

const VERSE_SIZE: f32 = 16.5;
const VERSE_LEADING: f32 = 26.0;
const GUTTER: f32 = 36.0;
const BOOK_FILTER: &str = "scripture-book-filter";
const SCRIPTURE_FIND: &str = "scripture-find";
const INITIAL_SEARCH_RESULTS: usize = 20;
const SEARCH_RESULTS_PAGE: usize = 20;

#[derive(Default)]
enum Place {
    #[default]
    Library,
    Book(String),
    Chapter(Passage),
}

enum Nav {
    Back,
    Book(String),
    Chapter {
        title: String,
        number: u16,
        /// Verse range to scroll into view and highlight. `None` opens the
        /// chapter at the top without a highlighted range.
        verse: Option<(u16, u16)>,
    },
}

/// Book, chapter, and chapter text. Stays on the last place when closed.
#[derive(Default)]
pub struct Reader {
    open: bool,
    place: Place,
    /// Narrows the book list.
    filter: String,
    /// Searches the text of every verse.
    find: String,
    /// The query [`Self::hits`] was built for.
    find_for: String,
    hits: Vec<VerseHit>,
    /// Number of search hits currently mounted in the results viewport.
    visible_hits: usize,
    find_selected: usize,
    /// Verse to reveal on the next chapter draw.
    reveal: Option<u16>,
    /// Find within the open chapter (Cmd+F). Same bar as a note.
    pub chapter_find: FindBar,
    /// Chapter the bar last searched, so a turn of the page reveals a match.
    searched_chapter: String,
    /// Verse row indexes selected by dragging in the open chapter.
    verse_selection: Option<(usize, usize)>,
    /// Reference and anchor for the open verse context menu.
    verse_menu: Option<(String, Pos2)>,
}

impl Reader {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the scriptures page without changing the book already chosen.
    pub fn ensure_open(&mut self) {
        self.open = true;
    }

    /// Opens the chapter and selects the verses named by a scripture citation.
    pub fn open_reference(&mut self, reference: &str) {
        let Some(hit) = scriptures::find_verses(reference)
            .into_iter()
            .find(|hit| hit.label == reference)
        else {
            return;
        };
        self.open = true;
        self.apply(Nav::Chapter {
            title: hit.book,
            number: hit.chapter,
            verse: Some((hit.number, hit.end)),
        });
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Opens the chapter find bar, or selects its query if it's already open.
    pub fn open_find(&mut self) {
        self.chapter_find.open();
    }

    /// Draws the chapter find bar with its top-right corner at `top_right`.
    pub fn show_find_bar(&mut self, ctx: &egui::Context, top_right: Pos2, palette: &Palette) {
        self.chapter_find
            .show(ctx, top_right, palette, find_bar::CHAPTER_HINT);
    }

    /// Window title while the page is open.
    pub fn title(&self) -> Option<String> {
        if !self.open {
            return None;
        }
        Some(match &self.place {
            Place::Library => "Scriptures".to_string(),
            Place::Book(title) => title.clone(),
            Place::Chapter(passage) => format!("{} {}", passage.book, passage.number),
        })
    }

    #[cfg(test)]
    pub(crate) fn highlighted_verse_range(&self) -> Option<(u16, u16)> {
        let Place::Chapter(passage) = &self.place else {
            return None;
        };
        let (start, end) = self.verse_selection?;
        Some((passage.verses[start].number, passage.verses[end].number))
    }

    /// The sidebar: filter and the book list.
    pub fn show_index(&mut self, ui: &mut Ui, palette: &Palette) {
        if escape_pressed(ui) && !self.chapter_find.query_focused(ui.ctx()) {
            let filter_focus = ui.memory(|m| m.has_focus(Id::new(BOOK_FILTER)));
            let handled = if filter_focus && !self.filter.is_empty() {
                self.filter.clear();
                true
            } else if filter_focus {
                ui.memory_mut(|m| m.stop_text_input());
                true
            } else if matches!(self.place, Place::Library) {
                // Nothing is open in the reading column, so leave the page.
                self.go_back();
                true
            } else {
                false
            };
            if handled {
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
            }
        }
        self.refresh_find();
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
        panel_heading(ui, palette, "Scriptures");
        ui.add_space(8.0);
        self.book_filter(ui, palette);
        ui.add_space(14.0);
        let current = match &self.place {
            Place::Library => None,
            Place::Book(title) => Some(title.clone()),
            Place::Chapter(passage) => Some(passage.book.clone()),
        };
        let from_list = egui::ScrollArea::vertical()
            .id_salt("scripture-index")
            .auto_shrink([false, false])
            .show(ui, |ui| self.books(ui, palette, current.as_deref()))
            .inner;
        if let Some(nav) = from_list {
            self.apply(nav);
        }
    }

    /// The reading column: a book's chapters, then the chapter text.
    ///
    /// `index_open` is false while the sidebar panel is collapsed, so a filter
    /// field that has unmounted does not keep Escape to itself.
    pub fn show_reading(&mut self, ui: &mut Ui, palette: &Palette, index_open: bool) {
        if !index_open {
            let stuck = ui.memory(|m| {
                m.has_focus(Id::new(BOOK_FILTER)) || m.has_focus(Id::new(SCRIPTURE_FIND))
            });
            if stuck {
                ui.memory_mut(|m| m.stop_text_input());
            }
        }
        // Escape closes the find bar when its field is focused. Otherwise,
        // from a book or chapter, it steps back. The library handles its own
        // Escape in the sidebar.
        if !matches!(self.place, Place::Library)
            && escape_pressed(ui)
            && !self.chapter_find.query_focused(ui.ctx())
        {
            ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
            self.go_back();
        }
        if matches!(self.place, Place::Library) {
            self.show_library(ui, palette);
            return;
        }
        self.sync_find();
        let scroll_id = self.scroll_id();
        let nav = {
            let from_header = self.header(ui, palette);
            ui.add_space(18.0);
            let from_body = egui::ScrollArea::vertical()
                .id_salt(("scriptures", scroll_id))
                .auto_shrink([false, false])
                .show(ui, |ui| self.body(ui, palette))
                .inner;
            from_header.or(from_body)
        };
        if let Some(nav) = nav {
            self.apply(nav);
        }
    }

    /// Searches the open chapter. Anywhere else, the bar has nothing to find.
    fn sync_find(&mut self) {
        let key = match &self.place {
            Place::Chapter(passage) => format!("{}:{}", passage.book, passage.number),
            _ => String::new(),
        };
        if self.chapter_find.open && self.searched_chapter != key {
            self.searched_chapter = key;
            self.chapter_find.current = 0;
            self.chapter_find.reveal = true;
        }
        if !self.chapter_find.open {
            self.chapter_find.update_plain(&[]);
            return;
        }
        let owned: Vec<String> = match &self.place {
            Place::Chapter(passage) => passage
                .verses
                .iter()
                .map(|verse| verse.text.clone())
                .collect(),
            _ => Vec::new(),
        };
        let texts: Vec<&str> = owned.iter().map(String::as_str).collect();
        self.chapter_find.update_plain(&texts);
    }

    fn scroll_id(&self) -> String {
        match &self.place {
            Place::Library => "library".to_string(),
            Place::Book(title) => format!("book:{title}"),
            Place::Chapter(passage) => format!("{}:{}", passage.book, passage.number),
        }
    }

    fn go_back(&mut self) {
        self.verse_selection = None;
        self.verse_menu = None;
        self.place = match std::mem::replace(&mut self.place, Place::Library) {
            Place::Library => {
                self.open = false;
                Place::Library
            }
            Place::Book(_) => Place::Library,
            Place::Chapter(passage) => Place::Book(passage.book),
        };
    }

    fn apply(&mut self, nav: Nav) {
        match nav {
            Nav::Back => self.go_back(),
            Nav::Book(title) => {
                self.verse_selection = None;
                self.verse_menu = None;
                self.place = Place::Book(title);
            }
            Nav::Chapter {
                title,
                number,
                verse,
            } => {
                if let Some(passage) = scriptures::chapter(&title, number) {
                    self.verse_selection = verse.and_then(|(start, end)| {
                        let first = passage
                            .verses
                            .iter()
                            .position(|verse| verse.number == start)?;
                        let last = passage
                            .verses
                            .iter()
                            .position(|verse| verse.number == end)
                            .unwrap_or(first);
                        Some((first, last))
                    });
                    self.verse_menu = None;
                    self.reveal = verse.map(|(start, _)| start);
                    self.place = Place::Chapter(passage);
                }
            }
        }
    }

    fn refresh_find(&mut self) {
        if self.find_for == self.find {
            return;
        }
        self.find_for = self.find.clone();
        self.find_selected = 0;
        self.hits = if self.find.trim().is_empty() {
            Vec::new()
        } else {
            scriptures::find_verses(&self.find)
        };
        self.visible_hits = self.hits.len().min(INITIAL_SEARCH_RESULTS);
    }

    fn header(&mut self, ui: &mut Ui, palette: &Palette) -> Option<Nav> {
        match &self.place {
            Place::Library => None,
            Place::Book(title) => {
                let back = back_row(ui, palette, "Scriptures");
                ui.add_space(10.0);
                let subtitle =
                    scriptures::book(title).map(|book| count_label(&book.title, book.chapters));
                heading(ui, palette, title, subtitle.as_deref());
                back.then_some(Nav::Back)
            }
            Place::Chapter(passage) => {
                let back = back_row(ui, palette, &passage.book);
                ui.add_space(10.0);
                let title = format!("{} {}", passage.book, passage.number);
                let subtitle = (passage.volume != passage.book).then_some(passage.volume.as_str());
                heading(ui, palette, &title, subtitle);
                let nav = if passage.count > 1 {
                    ui.add_space(12.0);
                    chapter_nav(ui, palette, passage)
                } else {
                    None
                };
                if back {
                    Some(Nav::Back)
                } else {
                    nav
                }
            }
        }
    }

    fn book_filter(&mut self, ui: &mut Ui, palette: &Palette) {
        text_field(ui, BOOK_FILTER, &mut self.filter, "Filter books", palette);
    }

    fn show_library(&mut self, ui: &mut Ui, palette: &Palette) {
        if escape_pressed(ui) && !self.chapter_find.query_focused(ui.ctx()) {
            let focused = ui.memory(|m| m.has_focus(Id::new(SCRIPTURE_FIND)));
            if focused && !self.find.is_empty() {
                self.find.clear();
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
            } else if focused {
                ui.memory_mut(|m| m.stop_text_input());
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
            }
        }
        self.sync_find();
        heading(
            ui,
            palette,
            "Scriptures",
            Some("Search every scripture by reference or words"),
        );
        ui.add_space(18.0);
        let nav = self.find_field(ui, palette);
        ui.add_space(22.0);
        if !self.find.trim().is_empty() {
            results_section(ui, &format!("Results ({})", self.hits.len()), palette);
            ui.add_space(6.0);
            if self.hits.is_empty() {
                note(ui, "No matching scriptures", palette);
            } else {
                let available_height = ui.available_height();
                let mut chosen = None;
                let loaded = self.visible_hits.min(self.hits.len());
                let output = egui::ScrollArea::vertical()
                    .id_salt("scripture-search-results")
                    .auto_shrink([false, false])
                    .max_height(available_height)
                    .show(ui, |ui| {
                        for (index, hit) in self.hits[..loaded].iter().enumerate() {
                            if hit_row(ui, hit, index == self.find_selected, palette) {
                                chosen = Some(open_verse(hit));
                                break;
                            }
                        }
                    });
                let at_bottom = output.state.offset.y + output.inner_rect.height()
                    >= output.content_size.y - 48.0;
                if at_bottom && self.visible_hits < self.hits.len() {
                    self.visible_hits =
                        (self.visible_hits + SEARCH_RESULTS_PAGE).min(self.hits.len());
                    ui.ctx().request_repaint();
                }
                if let Some(nav) = chosen {
                    self.apply(nav);
                    return;
                }
            }
        } else {
            note(ui, "Try “in the beginning” or “Genesis 1:1”", palette);
        }
        if let Some(nav) = nav {
            self.apply(nav);
        }
    }

    fn find_field(&mut self, ui: &mut Ui, palette: &Palette) -> Option<Nav> {
        let id = Id::new(SCRIPTURE_FIND);
        let mut nav = None;
        // Act on last frame's focus, before the field sees the keys.
        if ui.memory(|m| m.has_focus(id)) {
            let n = self.hits.len();
            let key = |ui: &mut Ui, key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
            if n > 0 && key(ui, Key::ArrowDown) {
                self.find_selected = (self.find_selected + 1).min(n - 1);
            }
            if key(ui, Key::ArrowUp) {
                self.find_selected = self.find_selected.saturating_sub(1);
            }
            if n > 0 && key(ui, Key::Enter) {
                nav = Some(open_verse(&self.hits[self.find_selected]));
            }
        }
        text_field(
            ui,
            SCRIPTURE_FIND,
            &mut self.find,
            "Search scriptures",
            palette,
        );
        nav
    }

    fn body(&mut self, ui: &mut Ui, palette: &Palette) -> Option<Nav> {
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
        let reveal = self.reveal.take();
        let nav = match &self.place {
            Place::Library => None,
            Place::Book(title) => chapters(ui, palette, title),
            Place::Chapter(passage) => {
                chapter(
                    ui,
                    palette,
                    passage,
                    reveal,
                    &mut self.chapter_find,
                    &mut self.verse_selection,
                    &mut self.verse_menu,
                );
                None
            }
        };
        ui.add_space(36.0);
        nav
    }

    fn books(&self, ui: &mut Ui, palette: &Palette, current: Option<&str>) -> Option<Nav> {
        let mut chosen = None;
        let volumes = scriptures::volumes_matching(&self.filter);
        if volumes.is_empty() {
            if !self.filter.trim().is_empty() {
                note(ui, "No matching books", palette);
            }
            return chosen;
        }
        for (index, volume) in volumes.into_iter().enumerate() {
            if index > 0 {
                ui.add_space(16.0);
            }
            section(ui, &volume.title, palette);
            ui.add_space(4.0);
            for book in volume.books {
                let open = current == Some(book.title.as_str());
                if row(ui, &book.title, open, palette) {
                    chosen = Some(Nav::Book(book.title));
                }
            }
        }
        chosen
    }
}

fn open_verse(hit: &VerseHit) -> Nav {
    Nav::Chapter {
        title: hit.book.clone(),
        number: hit.chapter,
        verse: Some((hit.number, hit.end)),
    }
}

fn chapters(ui: &mut Ui, palette: &Palette, title: &str) -> Option<Nav> {
    let book = scriptures::book(title)?;
    let unit = unit(title);
    let size = vec2(48.0, 36.0);
    let gap = 8.0;
    let width = ui.available_width();
    let cols = ((width + gap) / (size.x + gap)).floor().max(1.0) as u16;
    let mut chosen = None;
    let mut number = 1u16;
    while number <= book.chapters {
        let (row_rect, _) = ui.allocate_exact_size(vec2(width, size.y), Sense::hover());
        for col in 0..cols {
            if number > book.chapters {
                break;
            }
            let rect =
                Rect::from_min_size(row_rect.min + vec2(col as f32 * (size.x + gap), 0.0), size);
            let label = format!("{unit} {number}");
            let response = ui.interact(
                rect,
                Id::new(("scripture-chapter", title, number)),
                Sense::click(),
            );
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            let fill = if response.hovered() {
                palette.menu_selected
            } else {
                palette.hover
            };
            ui.painter().rect_filled(rect, 8.0, fill);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                number.to_string(),
                FontId::proportional(14.0),
                palette.text,
            );
            if response.clicked() {
                chosen = Some(number);
            }
            number += 1;
        }
        ui.add_space(gap);
    }
    chosen.map(|number| Nav::Chapter {
        title: title.to_string(),
        number,
        verse: None,
    })
}

fn chapter(
    ui: &mut Ui,
    palette: &Palette,
    passage: &Passage,
    reveal: Option<u16>,
    find: &mut FindBar,
    verse_selection: &mut Option<(usize, usize)>,
    verse_menu: &mut Option<(String, Pos2)>,
) {
    let (all_color, current_color) = find_bar::match_colors(ui);
    let current = find.current_match();
    let reveal_match = find.reveal;
    let mut revealed = false;
    for (index, verse) in passage.verses.iter().enumerate() {
        let highlights: Vec<(usize, usize, Color32)> = find
            .matches
            .iter()
            .filter(|m| m.block == index)
            .map(|m| {
                let color = if Some(*m) == current {
                    current_color
                } else {
                    all_color
                };
                (m.start, m.end, color)
            })
            .collect();
        let scroll_char = current
            .filter(|m| reveal_match && m.block == index)
            .map(|m| m.start);
        if scroll_char.is_some() {
            revealed = true;
        }
        let selected = verse_selection
            .is_some_and(|(start, end)| index >= start.min(end) && index <= start.max(end));
        let (rect, response) = verse_row(
            ui,
            verse.number,
            &verse.text,
            palette,
            &highlights,
            scroll_char,
            selected,
        );
        if response.drag_started() {
            *verse_selection = Some((index, index));
        } else if response.hovered()
            && ui.input(|input| {
                input.pointer.primary_down() && input.pointer.is_decidedly_dragging()
            })
        {
            if let Some((anchor, _)) = *verse_selection {
                *verse_selection = Some((anchor, index));
            }
        }
        let secondary_clicked = response.hovered()
            && ui.input(|input| input.pointer.button_clicked(egui::PointerButton::Secondary));
        if secondary_clicked {
            let range = verse_selection
                .filter(|(start, end)| index >= (*start).min(*end) && index <= (*start).max(*end))
                .unwrap_or((index, index));
            *verse_selection = Some(range);
            let start = range.0.min(range.1);
            let end = range.0.max(range.1);
            let reference = verse_reference(
                &passage.book,
                passage.number,
                passage.verses[start].number,
                passage.verses[end].number,
            );
            let position = ui
                .input(|input| input.pointer.interact_pos())
                .unwrap_or(response.rect.right_bottom());
            *verse_menu = Some((reference, position));
        }
        if let Some((reference, position)) = verse_menu.take() {
            let (copy, open) = menu::menu_at(
                ui.ctx(),
                egui::Id::new("scripture-verse-menu"),
                position,
                palette,
                secondary_clicked,
                |ui| {
                    menu::Item::new("Copy reference")
                        .icon(icons::quote)
                        .show(ui, palette)
                },
            );
            if copy {
                ui.ctx().copy_text(reference.clone());
            } else if open {
                *verse_menu = Some((reference, position));
            }
        }
        if reveal == Some(verse.number) {
            ui.scroll_to_rect(rect, Some(Align::Center));
        }
    }
    if revealed {
        find.reveal = false;
    }
}

/// Previous and next sit under the title, so a long chapter can be turned
/// without scrolling back to the top.
fn chapter_nav(ui: &mut Ui, palette: &Palette, passage: &Passage) -> Option<Nav> {
    let mut nav = None;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let name = unit(&passage.book);
    if passage.number > 1 {
        let text = format!("{name} {}", passage.number - 1);
        if nav_button(ui, rect, false, "Previous chapter", &text, palette) {
            nav = Some(Nav::Chapter {
                title: passage.book.clone(),
                number: passage.number - 1,
                verse: None,
            });
        }
    }
    if passage.number < passage.count {
        let text = format!("{name} {}", passage.number + 1);
        if nav_button(ui, rect, true, "Next chapter", &text, palette) {
            nav = Some(Nav::Chapter {
                title: passage.book.clone(),
                number: passage.number + 1,
                verse: None,
            });
        }
    }
    nav
}

fn verse_row(
    ui: &mut Ui,
    number: u16,
    text: &str,
    palette: &Palette,
    highlights: &[(usize, usize, Color32)],
    scroll_char: Option<usize>,
    selected: bool,
) -> (Rect, egui::Response) {
    let width = ui.available_width();
    let text_width = (width - GUTTER).max(40.0);
    let mut job = LayoutJob::default();
    job.wrap.max_width = text_width;
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: FontId::proportional(VERSE_SIZE),
            line_height: Some(VERSE_LEADING),
            color: palette.text,
            ..Default::default()
        },
    );
    let galley = ui.painter().layout_job(job);
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, galley.size().y + 12.0), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    if selected {
        ui.painter()
            .rect_filled(rect, 4.0, palette.menu_selected.gamma_multiply(0.45));
    }
    ui.painter().text(
        pos2(rect.left() + GUTTER - 10.0, rect.top() + 2.0),
        Align2::RIGHT_TOP,
        number.to_string(),
        FontId::proportional(13.0),
        palette.faint,
    );
    let origin = pos2(rect.left() + GUTTER, rect.top());
    let shapes: Vec<_> = highlights
        .iter()
        .flat_map(|(start, end, color)| {
            find_bar::highlight_shapes(&galley, origin, *start, *end, false, *color)
        })
        .collect();
    let scroll = scroll_char.map(|start| find_bar::match_rect(&galley, origin, start));
    for shape in shapes {
        ui.painter().add(shape);
    }
    ui.painter().galley(origin, galley, palette.text);
    if let Some(rect) = scroll {
        ui.scroll_to_rect(rect.expand(48.0), Some(Align::Center));
    }
    if selected {
        // Keep a small edge marker above the row fill so the selected range
        // remains clear when find highlights are also present.
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
            1.5,
            palette.menu_selected,
        );
    }
    (rect, response)
}

fn verse_reference(book: &str, chapter: u16, start: u16, end: u16) -> String {
    if start == end {
        format!("{book} {chapter}:{start}")
    } else {
        format!("{book} {chapter}:{start}-{end}")
    }
}

pub(crate) fn text_field(ui: &mut Ui, id: &str, text: &mut String, hint: &str, palette: &Palette) {
    let frame = Frame::new()
        .fill(palette.menu_selected)
        .corner_radius(CornerRadius::same(8))
        .outer_margin(Margin::symmetric(12, 0))
        .inner_margin(Margin {
            left: 12,
            right: 12,
            top: 8,
            bottom: 8,
        });
    frame.show(ui, |ui| {
        TextEdit::singleline(text)
            .id(Id::new(id))
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .desired_width(f32::INFINITY)
            .font(FontId::proportional(14.0))
            .text_color(palette.text)
            .hint_text(egui::RichText::new(hint).color(palette.faint))
            .event_filter(egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
                tab: false,
            })
            .show(ui)
    });
}

fn hit_row(ui: &mut Ui, hit: &VerseHit, selected: bool, palette: &Palette) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &hit.label));
    if selected || response.hovered() {
        let fill = if selected {
            palette.menu_selected
        } else {
            palette.hover
        };
        ui.painter().rect_filled(rect, 7.0, fill);
    }
    let inner = rect.shrink2(vec2(8.0, 0.0));
    let title =
        ui.painter()
            .layout_no_wrap(hit.label.clone(), FontId::proportional(14.0), palette.text);
    let preview = crate::sidebar::elided(ui, &hit.preview, 12.5, palette.faint, inner.width());
    let title_height = title.size().y;
    let top = inner.center().y - (title_height + preview.size().y + 2.0) / 2.0;
    ui.painter()
        .galley(pos2(inner.left(), top), title, palette.text);
    ui.painter().galley(
        pos2(inner.left(), top + title_height + 2.0),
        preview,
        palette.faint,
    );
    response.clicked()
}

pub(crate) fn note(ui: &mut Ui, text: &str, palette: &Palette) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    ui.painter().text(
        pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(14.0),
        palette.faint,
    );
}

/// Same title treatment as the notes sidebar.
pub(crate) fn panel_heading(ui: &mut Ui, palette: &Palette, title: &str) {
    let galley = theme::semibold(ui, title, 17.0, palette.text);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let pos = rect.left_center() + vec2(18.0, -galley.size().y / 2.0);
    ui.painter().galley(pos, galley, palette.text);
}

pub(crate) fn escape_pressed(ui: &Ui) -> bool {
    !egui::Popup::is_any_open(ui.ctx()) && ui.input(|i| i.key_pressed(Key::Escape))
}

pub(crate) fn heading(ui: &mut Ui, palette: &Palette, title: &str, subtitle: Option<&str>) {
    let galley = theme::semibold(ui, title, 28.0, palette.text);
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y), Sense::hover());
    ui.painter().galley(rect.left_top(), galley, palette.text);
    if let Some(subtitle) = subtitle {
        ui.add_space(4.0);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
        ui.painter().text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            subtitle,
            FontId::proportional(13.0),
            palette.faint,
        );
    }
}

/// Full-width row. The accessible name is always "Back"; `label` is what's drawn.
pub(crate) fn back_row(ui: &mut Ui, palette: &Palette, label: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Back"));
    if response.hovered() {
        ui.painter().rect_filled(rect, 7.0, palette.hover);
    }
    let y = rect.center().y;
    icons::chevron_left(ui.painter(), pos2(rect.left() + 8.0, y), palette.faint);
    ui.painter().text(
        pos2(rect.left() + 20.0, y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        palette.faint,
    );
    response.clicked()
}

pub(crate) fn row(ui: &mut Ui, title: &str, selected: bool, palette: &Palette) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    if selected || response.hovered() {
        let fill = if selected {
            palette.menu_selected
        } else {
            palette.hover
        };
        ui.painter().rect_filled(rect, 7.0, fill);
    }
    let y = rect.center().y;
    ui.painter().text(
        pos2(rect.left() + 8.0, y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(15.0),
        palette.text,
    );
    icons::chevron(
        ui.painter(),
        pos2(rect.right() - 16.0, y),
        false,
        palette.faint,
    );
    response.clicked()
}

pub(crate) fn section(ui: &mut Ui, title: &str, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    ui.painter().text(
        pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.0),
        palette.faint,
    );
}

pub(crate) fn results_section(ui: &mut Ui, title: &str, palette: &Palette) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title));
    ui.painter().text(
        pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.0),
        palette.faint,
    );
}

fn nav_button(
    ui: &mut Ui,
    row: Rect,
    align_right: bool,
    access: &str,
    text: &str,
    palette: &Palette,
) -> bool {
    let width = 220.0_f32.min(row.width() / 2.0);
    let rect = if align_right {
        Rect::from_min_size(
            pos2(row.right() - width, row.top()),
            vec2(width, row.height()),
        )
    } else {
        Rect::from_min_size(row.min, vec2(width, row.height()))
    };
    let response = ui.interact(rect, Id::new(("scripture-nav", access)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, access));
    if response.hovered() {
        ui.painter().rect_filled(rect, 7.0, palette.hover);
    }
    let y = rect.center().y;
    if align_right {
        icons::chevron(
            ui.painter(),
            pos2(rect.right() - 10.0, y),
            false,
            palette.faint,
        );
        ui.painter().text(
            pos2(rect.right() - 22.0, y),
            Align2::RIGHT_CENTER,
            text,
            FontId::proportional(14.0),
            palette.text,
        );
    } else {
        icons::chevron_left(ui.painter(), pos2(rect.left() + 10.0, y), palette.faint);
        ui.painter().text(
            pos2(rect.left() + 22.0, y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(14.0),
            palette.text,
        );
    }
    response.clicked()
}

fn unit(book: &str) -> &'static str {
    if book == "Doctrine and Covenants" {
        "Section"
    } else {
        "Chapter"
    }
}

fn count_label(book: &str, n: u16) -> String {
    let word = match (book == "Doctrine and Covenants", n == 1) {
        (true, true) => "section",
        (true, false) => "sections",
        (false, true) => "chapter",
        (false, false) => "chapters",
    };
    format!("{n} {word}")
}
