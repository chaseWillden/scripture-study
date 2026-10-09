//! The collapsible sidebar: browse recent notes, organize them into
//! folders, search, and create notes.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::SystemTime;

use eframe::egui::{
    self,
    text::{LayoutJob, TextFormat, TextWrapping},
    vec2, Align2, CornerRadius, FontId, Frame, Galley, Id, Key, KeyboardShortcut, Margin,
    Modifiers, Rect, Response, Sense, TextEdit, Ui,
};
use scripture_study_core::{
    folders::{self, Folder},
    search::{self, IndexedNote, SearchHit},
    store::{self, is_valid_folder_name},
};

use crate::icons;
use crate::menu::{self, Item};
use crate::shortcuts::{
    DELETE_NOTE, NEW_FOLDER, NEW_NOTE, RENAME, REVEAL, SEARCH, SETTINGS, TOGGLE_SIDEBAR,
};
use crate::theme::{self, Palette};

pub const DEFAULT_WIDTH: f32 = 248.0;
pub const RAIL_WIDTH: f32 = 52.0;
const RAIL_BUTTON: f32 = 36.0;
const PAD_X: f32 = 12.0;
const ROW_HEIGHT: f32 = 30.0;
const RESULT_HEIGHT: f32 = 48.0;
const INDENT: f32 = 16.0;

pub enum SidebarAction {
    Open(String),
    New,
    /// New note inside a folder.
    NewIn(String),
    Delete(String),
    RenameNote {
        id: String,
        title: String,
    },
    Move {
        id: String,
        folder: String,
    },
    CreateFolder(String),
    RenameFolder {
        from: String,
        to: String,
    },
    DeleteFolder(String),
    /// Show a note's file in the system file manager.
    RevealNote(String),
    /// Show a folder in the system file manager.
    RevealFolder(String),
    /// Move a note out of the notes folder to a place picked on disk.
    MoveNoteOut(String),
    /// Move a folder out of the notes folder to a place picked on disk.
    MoveFolderOut(String),
    /// Show scriptures in the sidebar, in place of the notes list.
    OpenScriptures,
    /// Show conference talks in the sidebar, in place of the notes list.
    OpenTalks,
    /// Leave scriptures or settings and show the notes list.
    ShowNotes,
    /// Leave scriptures or settings and keep the sidebar view just chosen.
    ClosePage,
    /// Show settings in the sidebar and on the page.
    OpenSettings,
}

pub const MOVE_OUT_LABEL: &str = "Move to location…";

/// The platform's name for showing a file in its file manager.
pub const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else if cfg!(windows) {
    "Show in Explorer"
} else {
    "Show in Files"
};

/// What fills the window beside the rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Notes,
    Scriptures,
    Talks,
    Settings,
}

/// What the sidebar lists when it isn't searching.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// Every note, most recent first.
    Recent,
    /// The folder tree.
    Folders,
}

/// A note being dragged onto a folder.
struct DraggedNote(String);

/// A note's new title being typed.
struct NoteRename {
    id: String,
    title: String,
    focus: bool,
}

/// A folder name being typed: a new folder or a rename.
struct FolderEdit {
    /// Folder the new or renamed folder lives in.
    parent: String,
    /// The folder being renamed, or `None` for a new folder.
    renaming: Option<String>,
    name: String,
    focus: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TreeItem {
    Folder { path: String, has_children: bool },
    Note { id: String, folder: String },
}

impl TreeItem {
    fn folder_key(path: &str) -> String {
        format!("folder:{path}")
    }

    fn note_key(id: &str) -> String {
        format!("note:{id}")
    }

    fn key(&self) -> String {
        match self {
            Self::Folder { path, .. } => Self::folder_key(path),
            Self::Note { id, .. } => Self::note_key(id),
        }
    }

    fn is_descendant_of(&self, folder: &str) -> bool {
        match self {
            Self::Folder { path, .. } => path.starts_with(&format!("{folder}/")),
            Self::Note { folder: parent, .. } => {
                parent == folder || parent.starts_with(&format!("{folder}/"))
            }
        }
    }
}

#[derive(Default)]
struct Search {
    query: String,
    selected: usize,
    focus: bool,
}

pub struct Sidebar {
    pub open: bool,
    /// Whether the panel is taking space, including while it slides in or out.
    /// The rail's edge follows this, which is a frame behind `open`.
    panel_visible: bool,
    pub view: View,
    search: Option<Search>,
    /// Expanded folders in the folder view.
    expanded: HashSet<String>,
    editing: Option<FolderEdit>,
    renaming: Option<NoteRename>,
    /// The folder-tree item that owns keyboard focus, encoded as
    /// `folder:<path>` or `note:<id>`.
    tree_selection: Option<String>,
    tree_focus_request: Option<String>,
    tree_keyboard_active: bool,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self {
            open: true,
            panel_visible: true,
            view: View::Folders,
            search: None,
            expanded: HashSet::new(),
            editing: None,
            renaming: None,
            tree_selection: None,
            tree_focus_request: None,
            tree_keyboard_active: false,
        }
    }
}

/// What the sidebar needs to know about the notes.
pub struct Notes<'a> {
    pub index: &'a [IndexedNote],
    pub current: &'a str,
    /// Live title of the open note (the index only updates on save).
    pub current_title: &'a str,
    pub now: SystemTime,
    pub tree: &'a Folder,
}

impl Sidebar {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Whether the sidebar panel is on screen, including mid-slide.
    pub fn panel_visible(&self) -> bool {
        self.panel_visible
    }

    pub(crate) fn set_panel_visible(&mut self, visible: bool) {
        self.panel_visible = visible;
    }

    /// Opens the sidebar with the search field focused.
    pub fn start_search(&mut self) {
        self.open = true;
        self.search = Some(Search {
            focus: true,
            ..Default::default()
        });
    }

    pub fn is_searching(&self) -> bool {
        self.search.is_some()
    }

    /// The folder name being typed, if any.
    #[cfg(test)]
    pub fn editing_folder(&self) -> Option<&str> {
        self.editing.as_ref().map(|e| e.name.as_str())
    }

    /// The note title being typed, if any.
    #[cfg(test)]
    pub fn renaming_note(&self) -> Option<&str> {
        self.renaming.as_ref().map(|r| r.title.as_str())
    }

    #[cfg(test)]
    pub(crate) fn request_tree_focus(&mut self, key: String) {
        self.tree_focus_request = Some(key);
    }

    /// Whether a name is being typed in the sidebar.
    pub fn is_typing(&self) -> bool {
        self.renaming.is_some() || self.editing.is_some()
    }

    /// Starts renaming a note in place, showing the sidebar if needed.
    pub fn rename_note(&mut self, id: &str, title: &str) {
        self.open = true;
        self.search = None;
        self.tree_keyboard_active = false;
        if self.view == View::Folders {
            self.reveal(store::parent(id));
        }
        self.editing = None;
        self.renaming = Some(NoteRename {
            id: id.to_string(),
            title: title.to_string(),
            focus: true,
        });
    }

    /// Opens the folder view with a new top-level folder being named.
    /// Drops state about the notes shown, for switching to other notes.
    /// Whether the sidebar is open and its view stay.
    pub fn forget_notes(&mut self) {
        *self = Self {
            open: self.open,
            view: self.view,
            panel_visible: self.panel_visible,
            ..Self::default()
        };
    }

    pub fn new_folder(&mut self) {
        self.open = true;
        self.search = None;
        self.view = View::Folders;
        self.new_folder_in("");
    }

    /// Expands `folder` and everything above it, so the note inside is visible.
    pub fn reveal(&mut self, folder: &str) {
        self.expanded.extend(folders::ancestors(folder));
    }

    /// Keeps expanded folders expanded after a rename.
    pub fn folder_renamed(&mut self, from: &str, to: &str) {
        let renamed: Vec<String> = self
            .expanded
            .iter()
            .filter(|p| *p == from || p.starts_with(&format!("{from}/")))
            .cloned()
            .collect();
        for path in renamed {
            self.expanded.remove(&path);
            self.expanded.insert(format!("{to}{}", &path[from.len()..]));
        }
    }

    /// Shows `view`, or collapses the sidebar if it's already showing it.
    fn switch_to(&mut self, view: View) {
        if self.open && self.view == view && self.search.is_none() {
            self.open = false;
        } else {
            self.open = true;
            self.view = view;
            self.search = None;
        }
    }

    /// Opens the notes list. Used when leaving the scriptures page.
    pub fn show_recent(&mut self) {
        self.open = true;
        self.view = View::Recent;
        self.search = None;
    }

    pub fn show(&mut self, ui: &mut Ui, notes: &Notes, palette: &Palette) -> Option<SidebarAction> {
        let mut action = None;
        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);

        if self.search.is_none() && self.view == View::Folders {
            return self.folders_view(ui, notes, palette);
        }
        if self.search.is_some() {
            action = self.search_field(ui, notes, palette);
        } else {
            self.header(ui, "Notes", palette);
            ui.add_space(6.0);
            let (rect, row) = row(ui, ROW_HEIGHT);
            paint_row_bg(ui, rect, &row, false, palette);
            icons::compose(
                ui.painter(),
                rect.left_center() + vec2(14.0, 0.0),
                palette.text,
            );
            ui.painter().text(
                rect.left_center() + vec2(32.0, 0.0),
                Align2::LEFT_CENTER,
                "New note",
                FontId::proportional(14.0),
                palette.text,
            );
            let shortcut = ui.ctx().format_shortcut(&NEW_NOTE);
            let hint =
                ui.painter()
                    .layout_no_wrap(shortcut, FontId::proportional(12.0), palette.faint);
            ui.painter().galley(
                egui::pos2(
                    rect.right() - 8.0 - hint.size().x,
                    rect.center().y - hint.size().y / 2.0,
                ),
                hint,
                palette.faint,
            );
            row.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "New note")
            });
            if row.clicked() {
                action = Some(SidebarAction::New);
            }
        }

        ui.add_space(14.0);
        let query = self.search.as_ref().map_or("", |s| s.query.as_str());
        let hits = search::search(notes.index, query);
        let label = match (&self.search, hits.is_empty()) {
            (None, _) => "Recent",
            (Some(_), false) => "Results",
            (Some(_), true) => "No matching notes",
        };
        section_label(ui, label, palette);
        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                for (n, hit) in hits.iter().enumerate() {
                    if let Some(a) = self.note_row(ui, n, hit, notes, palette) {
                        action = Some(a);
                    }
                }
                ui.add_space(12.0);
            });
        action
    }

    fn header(&mut self, ui: &mut Ui, title: &str, palette: &Palette) -> Rect {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
        let title = theme::semibold(ui, title, 17.0, palette.text);
        let pos = rect.left_center() + vec2(PAD_X + 6.0, -title.size().y / 2.0);
        ui.painter().galley(pos, title, palette.text);
        rect
    }

    /// The icon rail that stays visible when the sidebar is collapsed.
    /// `page` is what the window shows beside it.
    pub fn rail(&mut self, ui: &mut Ui, palette: &Palette, page: Page) -> Option<SidebarAction> {
        let mut action = None;
        let mut next = egui::pos2(ui.max_rect().center().x, ui.cursor().top() + 2.0);
        let mut button = |ui: &mut Ui, label, shortcut, active, icon| {
            let rect = Rect::from_center_size(
                next + vec2(0.0, RAIL_BUTTON / 2.0),
                vec2(RAIL_BUTTON, RAIL_BUTTON),
            );
            next.y += RAIL_BUTTON + 6.0;
            icon_button(ui, rect, label, shortcut, active, palette, icon).clicked()
        };

        let scriptures = page == Page::Scriptures;
        let talks = page == Page::Talks;
        // Scriptures, talks, and settings replace the notes list, so the notes icons
        // are not the current ones and clicking them switches back instead of
        // collapsing.
        let elsewhere = page != Page::Notes;
        let showing = |view| self.open && self.view == view && self.search.is_none();
        let recent = showing(View::Recent) && !elsewhere;
        let organizing = showing(View::Folders) && !elsewhere;
        let label = if recent { "Hide notes" } else { "Show notes" };
        if button(ui, label, Some(TOGGLE_SIDEBAR), recent, icons::notes) {
            if elsewhere {
                action = Some(SidebarAction::ShowNotes);
            } else {
                self.switch_to(View::Recent);
            }
        }
        if button(ui, "Organize notes", None, organizing, icons::folder) {
            if elsewhere {
                self.open = true;
                self.view = View::Folders;
                self.search = None;
                action = Some(SidebarAction::ClosePage);
            } else {
                self.switch_to(View::Folders);
            }
        }
        // The book icon stays selected for the whole scriptures page. Clicking
        // it again collapses the panel, the same way the notes icon does.
        if button(ui, "Scriptures", None, scriptures, icons::book) {
            if scriptures && self.open {
                self.open = false;
            } else {
                self.open = true;
                self.search = None;
                action = Some(SidebarAction::OpenScriptures);
            }
        }
        if button(ui, "Conference talks", None, talks, icons::microphone) {
            if talks && self.open {
                self.open = false;
            } else {
                self.open = true;
                self.search = None;
                action = Some(SidebarAction::OpenTalks);
            }
        }
        let searching = self.open && self.search.is_some() && !elsewhere;
        if button(ui, "Search notes", Some(SEARCH), searching, icons::search) {
            self.start_search();
            if elsewhere {
                action = Some(SidebarAction::ClosePage);
            }
        }
        if button(ui, "New note", Some(NEW_NOTE), false, icons::compose) {
            action = Some(SidebarAction::New);
        }

        // Settings sits at the foot of the rail, apart from the notes tools.
        let settings = page == Page::Settings;
        let foot = ui.max_rect().bottom() - 12.0 - RAIL_BUTTON / 2.0;
        let rect = Rect::from_center_size(
            egui::pos2(ui.max_rect().center().x, foot.max(next.y + RAIL_BUTTON)),
            vec2(RAIL_BUTTON, RAIL_BUTTON),
        );
        let gear = icon_button(
            ui,
            rect,
            "Settings",
            Some(SETTINGS),
            settings,
            palette,
            icons::gear,
        );
        if gear.clicked() {
            if settings {
                action = Some(SidebarAction::ClosePage);
            } else {
                self.open = true;
                self.search = None;
                action = Some(SidebarAction::OpenSettings);
            }
        }
        ui.allocate_rect(ui.max_rect(), Sense::hover());
        action
    }

    fn search_field(
        &mut self,
        ui: &mut Ui,
        notes: &Notes,
        palette: &Palette,
    ) -> Option<SidebarAction> {
        let id = Id::new("sidebar-search");
        let search = self.search.as_mut()?;
        let mut action = None;
        let mut close = false;

        if ui.memory(|m| m.has_focus(id)) {
            let hits = search::search(notes.index, &search.query);
            let n = hits.len();
            let key = |ui: &mut Ui, key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
            if n > 0 && key(ui, Key::ArrowDown) {
                search.selected = (search.selected + 1).min(n - 1);
            }
            if key(ui, Key::ArrowUp) {
                search.selected = search.selected.saturating_sub(1);
            }
            if key(ui, Key::Enter) {
                if let Some(hit) = hits.get(search.selected) {
                    action = Some(SidebarAction::Open(hit.id.clone()));
                    close = true;
                }
            }
            if key(ui, Key::Escape) {
                close = true;
            }
        }

        let frame = Frame::new()
            .fill(palette.menu_selected)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin {
                left: 30,
                right: 10,
                top: 7,
                bottom: 7,
            })
            .outer_margin(Margin::symmetric(PAD_X as i8, 0));
        let inner = frame.show(ui, |ui| {
            TextEdit::singleline(&mut search.query)
                .id(id)
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .desired_width(f32::INFINITY)
                .font(FontId::proportional(14.0))
                .text_color(palette.text)
                .hint_text(egui::RichText::new("Search notes").color(palette.faint))
                // Keep ↑/↓/Esc here instead of letting them move or drop focus.
                .event_filter(egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                    tab: false,
                })
                .show(ui)
        });
        let rect = inner.response.rect;
        icons::search(
            ui.painter(),
            rect.left_center() + vec2(PAD_X + 16.0, 0.0),
            palette.faint,
        );
        let response = &inner.inner.response;
        if response.changed() {
            search.selected = 0;
        }
        // Clicking away from an empty search closes it.
        if response.lost_focus() && search.query.is_empty() {
            close = true;
        }
        if std::mem::take(&mut search.focus) {
            response.request_focus();
        }
        if close {
            self.search = None;
        }
        action
    }

    fn note_row(
        &mut self,
        ui: &mut Ui,
        n: usize,
        hit: &SearchHit,
        notes: &Notes,
        palette: &Palette,
    ) -> Option<SidebarAction> {
        let is_current = hit.id == notes.current;
        let title = if is_current {
            notes.current_title
        } else {
            hit.title.as_str()
        };
        // Under the title: the matching text while searching, otherwise the
        // folders the note is in.
        let folder = store::parent(&hit.id);
        let subtitle = hit.snippet.clone().or_else(|| {
            (!folder.is_empty()).then(|| folder.split('/').collect::<Vec<_>>().join(" › "))
        });
        let height = if subtitle.is_some() {
            RESULT_HEIGHT
        } else {
            ROW_HEIGHT
        };
        let (rect, row) = row(ui, height);
        row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));

        if self.is_renaming(&hit.id) {
            let field = rect.shrink2(vec2(10.0, 0.0));
            return self.note_title_field(ui, rect, field, palette);
        }

        let highlighted = match &self.search {
            Some(search) => search.selected == n,
            None => is_current,
        };
        paint_row_bg(ui, rect, &row, highlighted, palette);

        let inner = rect.shrink2(vec2(10.0, 0.0));
        let age = notes
            .index
            .iter()
            .find(|note| note.meta.id == hit.id)
            .map(|note| search::relative_time(note.meta.modified, notes.now))
            .unwrap_or_default();
        let age = ui
            .painter()
            .layout_no_wrap(age, FontId::proportional(12.0), palette.faint);
        let title_width = inner.width() - age.size().x - 10.0;
        let title_galley = elided(ui, title, 14.0, palette.text, title_width);

        if let Some(subtitle) = &subtitle {
            let size = if hit.snippet.is_some() { 12.5 } else { 12.0 };
            let snippet = elided(ui, subtitle, size, palette.faint, inner.width());
            let top = inner.center().y - (title_galley.size().y + snippet.size().y + 2.0) / 2.0;
            ui.painter().galley(
                egui::pos2(inner.left(), top),
                title_galley.clone(),
                palette.text,
            );
            let snippet_top = top + title_galley.size().y + 2.0;
            ui.painter().galley(
                egui::pos2(inner.left(), snippet_top),
                snippet,
                palette.faint,
            );
            let age_pos = egui::pos2(inner.right() - age.size().x, top + 1.0);
            ui.painter().galley(age_pos, age, palette.faint);
        } else {
            let y = inner.center().y;
            ui.painter().galley(
                egui::pos2(inner.left(), y - title_galley.size().y / 2.0),
                title_galley,
                palette.text,
            );
            let age_pos = egui::pos2(inner.right() - age.size().x, y - age.size().y / 2.0);
            ui.painter().galley(age_pos, age, palette.faint);
        }

        let mut action = None;
        if row.double_clicked() {
            self.rename_note(&hit.id, title);
        } else if row.clicked() {
            action = Some(SidebarAction::Open(hit.id.clone()));
            self.search = None;
        }
        menu::context_menu(&row, palette, |ui| {
            if let Some(a) = self.note_menu(ui, &hit.id, title, notes.tree, palette) {
                action = Some(a);
            }
        });
        action
    }

    fn folders_view(
        &mut self,
        ui: &mut Ui,
        notes: &Notes,
        palette: &Palette,
    ) -> Option<SidebarAction> {
        let mut action = None;
        if let Some(a) = self.handle_tree_keyboard(ui, notes.tree) {
            action = Some(a);
        }
        let header = self.header(ui, "Folders", palette);
        let button = Rect::from_center_size(
            header.right_center() - vec2(PAD_X + 12.0, 0.0),
            vec2(28.0, 28.0),
        );
        if icon_button(
            ui,
            button,
            "New folder",
            None,
            false,
            palette,
            icons::new_folder,
        )
        .clicked()
        {
            self.new_folder_in("");
        }
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                if let Some(a) = self.folder_contents(ui, notes.tree, 0, notes, palette) {
                    action = Some(a);
                }
                if notes.tree.is_empty() && self.editing.is_none() {
                    section_label(ui, "No notes yet", palette);
                }
                // Dropping a note on the empty space below moves it to the top level.
                let rest = ui.available_rect_before_wrap();
                let rest = rest.with_max_y(rest.max.y.max(rest.min.y + 80.0));
                let zone = ui.allocate_rect(rest, Sense::click());
                if zone.clicked() {
                    if let Some(folder) = notes.tree.folders.first() {
                        let key = TreeItem::folder_key(&folder.path);
                        self.tree_selection = Some(key.clone());
                        self.tree_focus_request = Some(key);
                        self.tree_keyboard_active = true;
                    }
                }
                if let Some(a) = drop_into(ui, &zone, rest, "", palette) {
                    action = Some(a);
                }
                menu::context_menu(&zone, palette, |ui| {
                    if Item::new("New folder")
                        .icon(icons::new_folder)
                        .shortcut(NEW_FOLDER)
                        .show(ui, palette)
                    {
                        self.new_folder_in("");
                    }
                    if Item::new("New document")
                        .icon(icons::compose)
                        .show(ui, palette)
                    {
                        action = Some(SidebarAction::New);
                    }
                });
            });
        action
    }

    fn handle_tree_keyboard(&mut self, ui: &Ui, tree: &Folder) -> Option<SidebarAction> {
        let selection = self.tree_selection.clone()?;
        // A selection change requests focus while the row is being painted
        // later in this frame. Allow the next key through during that handoff
        // so rapid key presses do not lose navigation events.
        if !self.tree_keyboard_active {
            return None;
        }
        // A tree selection can outlive focus moving back to the document.
        // Only let the tree consume navigation keys while its selected row
        // still owns keyboard focus (or is waiting to receive it after a
        // selection change).
        let items = self.visible_tree_items(tree);
        let tree_has_focus = items.iter().any(|item| {
            ui.memory(|memory| memory.has_focus(Id::new(("sidebar-tree-row", item.key()))))
        });
        let tree_focus_pending = self.tree_focus_request.as_deref() == Some(selection.as_str());
        if !tree_has_focus && !tree_focus_pending && ui.input(|input| input.key_pressed(Key::Enter))
        {
            return None;
        }
        if ui.input(|input| input.modifiers.any()) {
            return None;
        }
        let index = items.iter().position(|item| item.key() == selection)?;
        let key = |key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));

        if key(Key::ArrowDown) {
            if let Some(next) = items.get(index + 1) {
                let key = next.key();
                self.tree_selection = Some(key.clone());
                self.tree_focus_request = Some(key);
            }
        } else if key(Key::ArrowUp) {
            if let Some(previous) = index.checked_sub(1).and_then(|i| items.get(i)) {
                let key = previous.key();
                self.tree_selection = Some(key.clone());
                self.tree_focus_request = Some(key);
            }
        } else if key(Key::ArrowRight) {
            if let TreeItem::Folder { path, .. } = &items[index] {
                if !self.expanded.contains(path) {
                    self.expanded.insert(path.clone());
                } else if let Some(child) = items.get(index + 1) {
                    if child.is_descendant_of(path) {
                        let key = child.key();
                        self.tree_selection = Some(key.clone());
                        self.tree_focus_request = Some(key);
                    }
                }
            }
        } else if key(Key::ArrowLeft) {
            match &items[index] {
                TreeItem::Folder { path, .. } if self.expanded.contains(path) => {
                    self.expanded.remove(path);
                }
                TreeItem::Folder { path, .. } | TreeItem::Note { id: path, .. } => {
                    let parent = store::parent(path);
                    if parent.is_empty() {
                        self.tree_selection = None;
                    } else {
                        let key = TreeItem::folder_key(parent);
                        self.tree_selection = Some(key.clone());
                        self.tree_focus_request = Some(key);
                    }
                }
            }
        } else if key(Key::Enter) {
            match &items[index] {
                TreeItem::Folder { path, .. } => {
                    if self.expanded.contains(path) {
                        self.expanded.remove(path);
                    } else {
                        self.expanded.insert(path.clone());
                    }
                }
                TreeItem::Note { id, .. } => {
                    self.tree_keyboard_active = false;
                    return Some(SidebarAction::Open(id.clone()));
                }
            }
        }
        None
    }

    fn visible_tree_items(&self, tree: &Folder) -> Vec<TreeItem> {
        fn add(items: &mut Vec<TreeItem>, folder: &Folder, expanded: &HashSet<String>) {
            for sub in &folder.folders {
                items.push(TreeItem::Folder {
                    path: sub.path.clone(),
                    has_children: !sub.is_empty(),
                });
                if expanded.contains(&sub.path) {
                    add(items, sub, expanded);
                }
            }
            for note in &folder.notes {
                items.push(TreeItem::Note {
                    id: note.id.clone(),
                    folder: folder.path.clone(),
                });
            }
        }

        let mut items = Vec::new();
        add(&mut items, tree, &self.expanded);
        items
    }

    fn new_folder_in(&mut self, parent: &str) {
        self.reveal(parent);
        self.tree_keyboard_active = false;
        self.renaming = None;
        self.editing = Some(FolderEdit {
            parent: parent.to_string(),
            renaming: None,
            name: String::new(),
            focus: true,
        });
    }

    /// Subfolders, then notes, of `folder`, indented to `depth`.
    fn folder_contents(
        &mut self,
        ui: &mut Ui,
        folder: &Folder,
        depth: usize,
        notes: &Notes,
        palette: &Palette,
    ) -> Option<SidebarAction> {
        let mut action = None;
        let creating_here = self
            .editing
            .as_ref()
            .is_some_and(|e| e.renaming.is_none() && e.parent == folder.path);
        if creating_here {
            action = self.folder_name_field(ui, depth, palette);
        }
        for sub in &folder.folders {
            let renaming = self
                .editing
                .as_ref()
                .is_some_and(|e| e.renaming.as_deref() == Some(sub.path.as_str()));
            let a = if renaming {
                self.folder_name_field(ui, depth, palette)
            } else {
                self.folder_row(ui, sub, depth, palette)
            };
            action = action.or(a);
            if self.expanded.contains(&sub.path) {
                let a = self.folder_contents(ui, sub, depth + 1, notes, palette);
                action = action.or(a);
            }
        }
        for note in &folder.notes {
            let a = self.tree_note_row(ui, note, depth, notes, palette);
            action = action.or(a);
        }
        action
    }

    fn folder_row(
        &mut self,
        ui: &mut Ui,
        folder: &Folder,
        depth: usize,
        palette: &Palette,
    ) -> Option<SidebarAction> {
        let mut action = None;
        let expanded = self.expanded.contains(&folder.path);
        let (rect, row) = tree_row(ui, ROW_HEIGHT, TreeItem::folder_key(&folder.path));
        row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &folder.name));
        let selected = self.tree_selection.as_deref() == Some(&TreeItem::folder_key(&folder.path));
        paint_row_bg(ui, rect, &row, selected, palette);
        let key = TreeItem::folder_key(&folder.path);
        if self.tree_focus_request.as_deref() == Some(key.as_str()) {
            row.request_focus();
            self.tree_focus_request = None;
        }

        let x = rect.left() + 10.0 + depth as f32 * INDENT;
        let y = rect.center().y;
        icons::chevron(
            ui.painter(),
            egui::pos2(x + 4.0, y),
            expanded,
            palette.faint,
        );
        icons::folder(ui.painter(), egui::pos2(x + 24.0, y), palette.text);
        let count = folder.note_count();
        let count = ui.painter().layout_no_wrap(
            if count > 0 {
                count.to_string()
            } else {
                String::new()
            },
            FontId::proportional(12.0),
            palette.faint,
        );
        let name_left = x + 40.0;
        let name_width = rect.right() - 10.0 - count.size().x - 8.0 - name_left;
        let name = elided(ui, &folder.name, 14.0, palette.text, name_width);
        ui.painter().galley(
            egui::pos2(name_left, y - name.size().y / 2.0),
            name,
            palette.text,
        );
        let count_pos = egui::pos2(
            rect.right() - 10.0 - count.size().x,
            y - count.size().y / 2.0,
        );
        ui.painter().galley(count_pos, count, palette.faint);

        if row.clicked() {
            self.tree_selection = Some(key.clone());
            self.tree_focus_request = Some(key);
            self.tree_keyboard_active = true;
            row.request_focus();
            if expanded {
                self.expanded.remove(&folder.path);
            } else {
                self.expanded.insert(folder.path.clone());
            }
        }
        if let Some(a) = drop_into(ui, &row, rect, &folder.path, palette) {
            self.expanded.insert(folder.path.clone());
            action = Some(a);
        }
        menu::context_menu(&row, palette, |ui| {
            let new_note = Item::new("New note")
                .icon(icons::compose)
                .shortcut(NEW_NOTE);
            if new_note.show(ui, palette) {
                self.expanded.insert(folder.path.clone());
                action = Some(SidebarAction::NewIn(folder.path.clone()));
            }
            let new_folder = Item::new("New folder")
                .icon(icons::new_folder)
                .shortcut(NEW_FOLDER);
            if new_folder.show(ui, palette) {
                self.new_folder_in(&folder.path);
            }
            if Item::new("Rename")
                .icon(icons::pencil)
                .shortcut(RENAME)
                .show(ui, palette)
            {
                self.tree_keyboard_active = false;
                self.renaming = None;
                self.editing = Some(FolderEdit {
                    parent: store::parent(&folder.path).to_string(),
                    renaming: Some(folder.path.clone()),
                    name: folder.name.clone(),
                    focus: true,
                });
            }
            let reveal = Item::new(REVEAL_LABEL).icon(icons::reveal).shortcut(REVEAL);
            if reveal.show(ui, palette) {
                action = Some(SidebarAction::RevealFolder(folder.path.clone()));
            }
            if Item::new(MOVE_OUT_LABEL)
                .icon(icons::move_to)
                .show(ui, palette)
            {
                action = Some(SidebarAction::MoveFolderOut(folder.path.clone()));
            }
            menu::separator(ui, palette);
            let delete = Item::new("Delete folder")
                .icon(icons::trash)
                .shortcut(DELETE_NOTE)
                .danger()
                .enabled(folder.is_empty())
                .disabled_hint("Move or delete the notes inside first");
            if delete.show(ui, palette) {
                action = Some(SidebarAction::DeleteFolder(folder.path.clone()));
            }
        });
        action
    }

    /// The inline text field for naming a new folder or renaming one.
    fn folder_name_field(
        &mut self,
        ui: &mut Ui,
        depth: usize,
        palette: &Palette,
    ) -> Option<SidebarAction> {
        let edit = self.editing.as_mut()?;
        let id = Id::new("folder-name");
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
        let rect = rect.shrink2(vec2(PAD_X - 4.0, 0.0));
        paint_field_bg(ui, rect, palette);
        let x = rect.left() + 10.0 + depth as f32 * INDENT;
        icons::folder(
            ui.painter(),
            egui::pos2(x + 24.0, rect.center().y),
            palette.text,
        );
        let field = Rect::from_min_max(
            egui::pos2(x + 40.0, rect.top()),
            egui::pos2(rect.right() - 8.0, rect.bottom()),
        );

        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(field)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let response = TextEdit::singleline(&mut edit.name)
            .id(id)
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .desired_width(field.width())
            .font(FontId::proportional(14.0))
            .text_color(palette.text)
            .hint_text(egui::RichText::new("Folder name").color(palette.faint))
            .event_filter(egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
                ..Default::default()
            })
            .show(&mut child);
        let response = response.response;
        if std::mem::take(&mut edit.focus) {
            focus_and_select_all(ui, &response, &edit.name);
        }

        let cancel =
            response.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let submit =
            response.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let name = edit.name.trim().to_string();
        let commit = submit || (response.lost_focus() && !cancel);
        if cancel || (commit && name.is_empty()) {
            self.editing = None;
            return None;
        }
        if !commit {
            return None;
        }
        if !is_valid_folder_name(&name) {
            // Keep the field open so the name can be fixed.
            edit.focus = true;
            return None;
        }
        let edit = self.editing.take()?;
        let path = store::join(&edit.parent, &name);
        match edit.renaming {
            Some(from) if from == path => None,
            Some(from) => {
                self.folder_renamed(&from, &path);
                Some(SidebarAction::RenameFolder { from, to: path })
            }
            None => {
                self.expanded.insert(path.clone());
                Some(SidebarAction::CreateFolder(path))
            }
        }
    }

    /// A note in the folder tree: draggable onto folders.
    fn tree_note_row(
        &mut self,
        ui: &mut Ui,
        note: &store::NoteMeta,
        depth: usize,
        notes: &Notes,
        palette: &Palette,
    ) -> Option<SidebarAction> {
        let is_current = note.id == notes.current;
        let title = if is_current {
            notes.current_title
        } else {
            note.title.as_str()
        };
        let (rect, row) = tree_row(ui, ROW_HEIGHT, TreeItem::note_key(&note.id));
        let x = rect.left() + 10.0 + depth as f32 * INDENT + 18.0;
        let y = rect.center().y;

        if self.is_renaming(&note.id) {
            let field = Rect::from_min_max(
                egui::pos2(x + 22.0, rect.top()),
                egui::pos2(rect.right() - 10.0, rect.bottom()),
            );
            let action = self.note_title_field(ui, rect, field, palette);
            icons::page(ui.painter(), egui::pos2(x + 6.0, y), palette.faint);
            return action;
        }

        let row = row.interact(Sense::click_and_drag());
        row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
        let selected = self.tree_selection.as_deref() == Some(&TreeItem::note_key(&note.id));
        paint_row_bg(ui, rect, &row, is_current || selected, palette);
        let key = TreeItem::note_key(&note.id);
        if self.tree_focus_request.as_deref() == Some(key.as_str()) {
            row.request_focus();
            self.tree_focus_request = None;
        }
        icons::page(ui.painter(), egui::pos2(x + 6.0, y), palette.faint);
        let galley = elided(
            ui,
            title,
            14.0,
            palette.text,
            rect.right() - 10.0 - (x + 22.0),
        );
        ui.painter().galley(
            egui::pos2(x + 22.0, y - galley.size().y / 2.0),
            galley,
            palette.text,
        );

        row.dnd_set_drag_payload(DraggedNote(note.id.clone()));
        let mut action = None;
        if row.double_clicked() {
            self.rename_note(&note.id, title);
        } else if row.clicked() {
            self.tree_selection = Some(key.clone());
            self.tree_focus_request = Some(key);
            // Opening a note transfers keyboard ownership back to the document.
            self.tree_keyboard_active = false;
            action = Some(SidebarAction::Open(note.id.clone()));
        }
        menu::context_menu(&row, palette, |ui| {
            if let Some(a) = self.note_menu(ui, &note.id, title, notes.tree, palette) {
                action = Some(a);
            }
        });
        action
    }

    fn is_renaming(&self, id: &str) -> bool {
        self.renaming.as_ref().is_some_and(|r| r.id == id)
    }

    /// The inline text field for renaming a note: `row` is the whole row,
    /// `field` the part the title is typed in.
    fn note_title_field(
        &mut self,
        ui: &mut Ui,
        row: Rect,
        field: Rect,
        palette: &Palette,
    ) -> Option<SidebarAction> {
        let edit = self.renaming.as_mut()?;
        paint_field_bg(ui, row, palette);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(field)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let output = TextEdit::singleline(&mut edit.title)
            .id(Id::new("note-title"))
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .desired_width(field.width())
            .font(FontId::proportional(14.0))
            .text_color(palette.text)
            .hint_text(egui::RichText::new(store::UNTITLED).color(palette.faint))
            .event_filter(egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
                ..Default::default()
            })
            .show(&mut child);
        let response = output.response.response;
        if std::mem::take(&mut edit.focus) {
            focus_and_select_all(ui, &response, &edit.title);
        }

        // A single-line field gives up focus on Enter itself; consume the key
        // either way so it doesn't reach the editor.
        let active = response.has_focus() || response.lost_focus();
        let cancel = active && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let submit = active && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        if !(cancel || submit || response.lost_focus()) {
            return None;
        }
        let edit = self.renaming.take()?;
        let title = edit.title.trim();
        (!cancel && !title.is_empty()).then(|| SidebarAction::RenameNote {
            id: edit.id,
            title: title.to_string(),
        })
    }

    /// Right-click menu for a note.
    fn note_menu(
        &mut self,
        ui: &mut Ui,
        id: &str,
        title: &str,
        tree: &Folder,
        palette: &Palette,
    ) -> Option<SidebarAction> {
        let mut action = None;
        if Item::new("Rename")
            .icon(icons::pencil)
            .shortcut(RENAME)
            .show(ui, palette)
        {
            self.rename_note(id, title);
        }
        menu::submenu(ui, palette, "Move to", icons::move_to, |ui| {
            let current = store::parent(id);
            let mut target = |ui: &mut Ui, item: Item, path: &str| {
                if item.checked(path == current).show(ui, palette) {
                    action = Some(SidebarAction::Move {
                        id: id.to_string(),
                        folder: path.to_string(),
                    });
                }
            };
            target(ui, Item::new("Top level").icon(icons::notes), "");
            if !tree.folders.is_empty() {
                menu::separator(ui, palette);
            }
            fn each(folder: &Folder, depth: usize, f: &mut dyn FnMut(&Folder, usize)) {
                for sub in &folder.folders {
                    f(sub, depth);
                    each(sub, depth + 1, f);
                }
            }
            each(tree, 0, &mut |sub, depth| {
                let item = Item::new(&sub.name).icon(icons::folder).indent(depth);
                target(ui, item, &sub.path);
            });
        });
        let reveal = Item::new(REVEAL_LABEL).icon(icons::reveal).shortcut(REVEAL);
        if reveal.show(ui, palette) {
            action = Some(SidebarAction::RevealNote(id.to_string()));
        }
        if Item::new(MOVE_OUT_LABEL)
            .icon(icons::move_to)
            .show(ui, palette)
        {
            action = Some(SidebarAction::MoveNoteOut(id.to_string()));
        }
        menu::separator(ui, palette);
        let delete = Item::new("Delete")
            .icon(icons::trash)
            .shortcut(DELETE_NOTE)
            .danger();
        if delete.show(ui, palette) {
            action = Some(SidebarAction::Delete(id.to_string()));
        }
        action
    }
}

/// Fill and focus ring behind an inline name field.
fn paint_field_bg(ui: &Ui, rect: Rect, palette: &Palette) {
    ui.painter().rect_filled(rect, 7.0, palette.menu_selected);
    ui.painter().rect_stroke(
        rect,
        7.0,
        egui::Stroke::new(1.0, palette.accent),
        egui::StrokeKind::Inside,
    );
}

/// Focuses a text field with all of `text` selected, ready to be replaced.
fn focus_and_select_all(ui: &Ui, response: &Response, text: &str) {
    response.request_focus();
    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), response.id).unwrap_or_default();
    let end = egui::text::CCursor::new(text.chars().count());
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            end,
        )));
    state.store(ui.ctx(), response.id);
}

/// Highlights `rect` while a note is dragged over it; returns a move when
/// one is dropped there.
fn drop_into(
    ui: &Ui,
    response: &Response,
    rect: Rect,
    folder: &str,
    palette: &Palette,
) -> Option<SidebarAction> {
    let dragged = response.dnd_hover_payload::<DraggedNote>()?;
    if store::parent(&dragged.0) == folder {
        return None;
    }
    ui.painter().rect_stroke(
        rect,
        7.0,
        egui::Stroke::new(1.5, palette.accent),
        egui::StrokeKind::Inside,
    );
    let dropped = response.dnd_release_payload::<DraggedNote>()?;
    Some(SidebarAction::Move {
        id: dropped.0.clone(),
        folder: folder.to_string(),
    })
}

/// A full-width clickable row. Returns the inset rect to draw it in.
fn row(ui: &mut Ui, height: f32) -> (Rect, Response) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    (rect.shrink2(vec2(PAD_X - 4.0, 0.0)), response)
}

/// A tree row with a stable ID, allowing focus to follow keyboard selection
/// when the visible tree changes after expanding or collapsing a folder.
fn tree_row(ui: &mut Ui, height: f32, key: String) -> (Rect, Response) {
    let (full_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let response = ui.interact(
        full_rect,
        Id::new(("sidebar-tree-row", key)),
        Sense::click(),
    );
    (full_rect.shrink2(vec2(PAD_X - 4.0, 0.0)), response)
}

fn paint_row_bg(ui: &Ui, rect: Rect, response: &Response, selected: bool, palette: &Palette) {
    let fill = if selected {
        palette.menu_selected
    } else if response.hovered() {
        palette.hover
    } else {
        return;
    };
    ui.painter().rect_filled(rect, 7.0, fill);
}

fn section_label(ui: &mut Ui, text: &str, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    ui.painter().text(
        rect.left_center() + vec2(PAD_X + 6.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.0),
        palette.faint,
    );
}

fn icon_button(
    ui: &mut Ui,
    rect: Rect,
    label: &str,
    shortcut: Option<KeyboardShortcut>,
    active: bool,
    palette: &Palette,
    paint: fn(&egui::Painter, egui::Pos2, egui::Color32),
) -> Response {
    let response = ui.interact(rect, Id::new(("icon-button", label)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if active {
        ui.painter().rect_filled(rect, 8.0, palette.menu_selected);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, palette.hover);
    }
    let color = if active || response.hovered() {
        palette.text
    } else {
        palette.faint
    };
    paint(ui.painter(), rect.center(), color);
    let shortcut = shortcut.map(|shortcut| ui.ctx().format_shortcut(&shortcut));
    rail_tooltip(&response, label, shortcut.as_deref(), palette);
    response
}

/// A small popover to the side of a rail button, with its name and shortcut.
fn rail_tooltip(response: &Response, label: &str, shortcut: Option<&str>, palette: &Palette) {
    let palette = *palette;
    let mut tip = egui::Tooltip::for_enabled(response).gap(10.0).width(220.0);
    tip.popup = tip
        .popup
        .align(egui::RectAlign::RIGHT)
        .align_alternatives(&[egui::RectAlign::LEFT])
        .style(move |style: &mut egui::Style| {
            menu::apply_style(style, &palette);
            style.spacing.menu_margin = Margin::symmetric(12, 8);
            style.spacing.item_spacing = vec2(10.0, 0.0);
        });
    tip.show(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(label).size(13.5).color(palette.text));
            if let Some(shortcut) = shortcut {
                ui.label(
                    egui::RichText::new(shortcut)
                        .size(12.0)
                        .color(palette.faint),
                );
            }
        });
    });
}

/// One line of text, cut off with "…" if it's wider than `width`.
pub fn elided(ui: &Ui, text: &str, size: f32, color: egui::Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(
        text.to_string(),
        TextFormat {
            font_id: FontId::proportional(size),
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping {
        max_width: width.max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

/// The sidebar-toggle button that sits next to the window controls.
pub fn toggle_button(ui: &mut Ui, rect: Rect, palette: &Palette) -> Response {
    icon_button(
        ui,
        rect,
        "Toggle sidebar",
        Some(TOGGLE_SIDEBAR),
        false,
        palette,
        icons::sidebar,
    )
}
