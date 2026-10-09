//! Note lifecycle (which note is open, autosave, note commands) and the
//! window layout: title bar, sidebar, editor, and the scriptures,
//! conference talks, and settings pages.

use std::time::SystemTime;

use eframe::egui::{self, pos2, vec2, Frame, Rect, UiBuilder};
use scripture_study_core::{
    commands::{self, Action, Command},
    drive_sync::LocalNote,
    editor::Caret,
    folders::{self, Folder},
    search::IndexedNote,
    settings::NoteSettings,
    store,
    store::UNTITLED,
    Document, FsStore, NoteMeta, NoteStore,
};

use crate::editor::{Editor, Event};
use crate::google_drive::GoogleDrive;
use crate::meta::NoteTimes;
use crate::reader::Reader;
use crate::settings_page::SettingsPage;
use crate::shortcuts::{
    DELETE_NOTE, FIND, NEW_FOLDER, NEW_NOTE, OPEN_FOLDER, RENAME, REVEAL, SAVE, SEARCH, SETTINGS,
    TOGGLE_SIDEBAR, TOGGLE_SIDEBAR_SHIFT_B,
};
use crate::sidebar::{self, Page, Sidebar, SidebarAction};
use crate::talk_library::TalkLibrary;
use crate::talks::TalksPage;
use crate::theme::{self, Palette};

/// Height of the strip at the top of the window that holds the window
/// controls and can be dragged to move the window.
pub const TITLEBAR_HEIGHT: f32 = 44.0;

/// Where the sidebar toggle sits: right of the macOS traffic lights, whose
/// centers are ~14pt from the top when the content fills the title bar.
fn toggle_rect() -> Rect {
    let left = if cfg!(target_os = "macos") {
        78.0
    } else {
        10.0
    };
    Rect::from_min_size(pos2(left, 0.0), vec2(28.0, 28.0))
}

#[cfg(test)]
mod tests;

/// Seconds of inactivity before autosaving.
const SAVE_DELAY: f64 = 0.5;

pub struct ScriptureStudyApp {
    store: FsStore,
    notes: Vec<NoteMeta>,
    /// Every note's text, for the sidebar's search.
    index: Vec<IndexedNote>,
    /// Folders and the notes in them, for organizing.
    tree: Folder,
    current: String,
    sidebar: Sidebar,
    scriptures: Reader,
    talks: TalksPage,
    /// Downloaded conference talks, and the thread that fetches more.
    talk_library: TalkLibrary,
    settings: SettingsPage,
    /// Copies notes to Google Drive when that connector is on.
    drive: GoogleDrive,
    /// Sync hash of the open note as saved, to tell whether Drive has it.
    saved_hash: String,
    editor: Editor,
    commands: Vec<Command>,
    /// egui time of the first unsaved edit.
    dirty_since: Option<f64>,
    /// Reads a picture from the clipboard as PNG bytes (swapped out in tests).
    clipboard_image: fn() -> Option<Vec<u8>>,
    /// Shows a file in the system file manager (swapped out in tests).
    reveal: fn(&std::path::Path) -> std::io::Result<()>,
    /// Asks for a folder on disk with this title, starting in the given
    /// one (swapped out in tests).
    pick_folder: fn(&str, &std::path::Path) -> Option<std::path::PathBuf>,
    /// Remembers the notes directory for next launch (swapped out in tests).
    remember_library: fn(&std::path::Path) -> std::io::Result<()>,
    /// Whether the V key's press was seen (see [`ScriptureStudyApp::image_paste`]).
    v_down: bool,
    /// Whether text was pasted since the V key went down.
    text_pasted: bool,
    error: Option<String>,
    /// Eased width of the page column. `0` until the first frame.
    page_width: f32,
    /// When outline folds changed because the text did, not because of a click.
    /// Written with the note, so a fold click doesn't wait for this.
    outline_dirty_since: Option<f64>,
    /// Misspellings, ignored words, and the user's dictionary.
    spelling: crate::spell::Spelling,
}

impl ScriptureStudyApp {
    pub fn new(cc: &eframe::CreationContext<'_>, store: FsStore) -> std::io::Result<Self> {
        theme::install_fonts(&cc.egui_ctx);
        scripture_study_core::scriptures::warm();
        let notes = store.list()?;
        let current = match notes.first() {
            Some(note) => note.id.clone(),
            None => store.create()?,
        };
        let doc = store.load(&current)?;
        // Tests never read or write the real connector settings.
        let config = if cfg!(test) {
            None
        } else {
            crate::google_drive::config_file()
        };
        let drive = GoogleDrive::start(config, store.root().to_path_buf(), cc.egui_ctx.clone());
        // Tests never touch the network or the real downloads.
        let talk_library = if cfg!(test) {
            TalkLibrary::start(
                None,
                std::sync::Arc::new(|_: &str| Err("offline".to_string())),
                cc.egui_ctx.clone(),
            )
        } else {
            TalkLibrary::start(
                crate::talk_library::dir(),
                crate::talk_library::web(),
                cc.egui_ctx.clone(),
            )
        };
        let mut app = Self {
            store,
            notes,
            index: Vec::new(),
            tree: folders::tree(&[], &[]),
            editor: Editor::new(&current, doc),
            current,
            sidebar: Sidebar::default(),
            scriptures: Reader::default(),
            talks: TalksPage::default(),
            talk_library,
            settings: SettingsPage::default(),
            drive,
            saved_hash: String::new(),
            commands: Vec::new(),
            dirty_since: None,
            error: None,
            clipboard_image: clipboard_png,
            reveal: reveal_in_file_manager,
            pick_folder,
            remember_library: crate::library::remember,
            v_down: false,
            text_pasted: false,
            page_width: 0.0,
            outline_dirty_since: None,
            spelling: crate::spell::Spelling::new(),
        };
        app.load_outline();
        app.rehash();
        app.refresh_notes();
        Ok(app)
    }

    /// Folds whatever `.scripture-study` recorded for the open note.
    fn load_outline(&mut self) {
        match self.store.note_settings(&self.current) {
            Ok(settings) => self.editor.restore_collapsed(&settings.collapsed),
            Err(e) => self.error = Some(format!("Couldn't read note settings: {e}")),
        }
    }

    /// Writes folded outline items into the note's folder. A click is written
    /// immediately; a change that came from editing waits out the autosave
    /// delay, and saving or leaving the note flushes it.
    fn persist_outline(&mut self, now: f64, ui: &egui::Ui) {
        if self.editor.outline_needs_flush() {
            self.flush_outline();
            return;
        }
        if self.editor.outline_changes().is_none() {
            self.outline_dirty_since = None;
            return;
        }
        let since = *self.outline_dirty_since.get_or_insert(now);
        let elapsed = now - since;
        if elapsed >= SAVE_DELAY {
            self.flush_outline();
        } else {
            ui.ctx()
                .request_repaint_after_secs((SAVE_DELAY - elapsed) as f32);
        }
    }

    fn flush_outline(&mut self) {
        let Some(collapsed) = self.editor.outline_changes() else {
            self.outline_dirty_since = None;
            return;
        };
        let settings = NoteSettings { collapsed };
        if let Err(e) = self.store.set_note_settings(&self.current, &settings) {
            self.error = Some(format!("Couldn't save note settings: {e}"));
            return;
        }
        self.editor.mark_outline_persisted();
        self.outline_dirty_since = None;
    }

    fn refresh_notes(&mut self) {
        match self.store.list() {
            Ok(notes) => self.notes = notes,
            Err(e) => self.error = Some(format!("Couldn't list notes: {e}")),
        }
        self.index = self
            .notes
            .iter()
            .filter_map(|meta| {
                let doc = self.store.load(&meta.id).ok()?;
                Some(IndexedNote::new(meta.clone(), &doc))
            })
            .collect();
        match self.store.folders() {
            Ok(paths) => self.tree = folders::tree(&paths, &self.notes),
            Err(e) => self.error = Some(format!("Couldn't list folders: {e}")),
        }
        self.commands = commands::with_notes(&self.notes, Some(&self.current));
    }

    fn save(&mut self) {
        self.dirty_since = None;
        // Record created the first time a note grows properties, so the date
        // survives being copied to a machine that doesn't keep birth times.
        if self.editor.doc.properties.has_entries() && self.editor.doc.properties.created.is_none()
        {
            let created = self
                .notes
                .iter()
                .find(|note| note.id == self.current)
                .map(|note| note.created)
                .unwrap_or_else(SystemTime::now);
            self.editor.doc.properties.created = Some(created);
        }
        if let Err(e) = self.store.save(&self.current, &self.editor.doc) {
            self.error = Some(format!("Couldn't save: {e}"));
            return;
        }
        self.error = None;
        self.rehash();
        self.refresh_notes();
        self.flush_outline();
        self.drive.notes_changed();
    }

    /// Records what the open note looks like on disk, for its sync badge.
    /// Call after the editor's note is loaded or saved.
    fn rehash(&mut self) {
        self.saved_hash = LocalNote::new(&self.current, &self.editor.doc).hash;
    }

    /// Saves the open note (or discards it if it's empty) and opens another.
    /// An unchanged note isn't rewritten, so browsing doesn't reorder the
    /// most-recent list.
    fn open(&mut self, id: String) {
        if id != self.current {
            if self.editor.doc.is_blank() {
                let _ = self.store.delete(&self.current);
            } else if self.dirty_since.is_some() {
                self.save();
            } else {
                self.flush_outline();
            }
        }
        match self.store.load(&id) {
            Ok(doc) => {
                self.editor = Editor::new(&id, doc);
                self.sidebar.reveal(store::parent(&id));
                self.scriptures.close();
                self.settings.close();
                self.current = id;
                self.outline_dirty_since = None;
                self.load_outline();
                self.rehash();
            }
            Err(e) => self.error = Some(format!("Couldn't open note: {e}")),
        }
        self.refresh_notes();
    }

    fn run(&mut self, action: Action) {
        match action {
            Action::NewNote => match self.store.create() {
                Ok(id) => self.open(id),
                Err(e) => self.error = Some(format!("Couldn't create note: {e}")),
            },
            Action::OpenNote(id) => self.open(id),
            Action::DeleteNote => self.delete(self.current.clone()),
            // The editor handles these itself.
            Action::SetBlock(_)
            | Action::InsertCitation
            | Action::InsertScripture
            | Action::InsertFileLink => {}
        }
    }

    /// Deletes a note. Deleting the open note moves to the most recent one.
    fn delete(&mut self, id: String) {
        if let Err(e) = self.store.delete(&id) {
            self.error = Some(format!("Couldn't delete note: {e}"));
            return;
        }
        self.drive.notes_changed();
        if id != self.current {
            self.refresh_notes();
            return;
        }
        self.current_gone();
    }

    /// Opens another note after the current one left the store.
    fn current_gone(&mut self) {
        // Nothing left to save or discard for the note that's gone.
        self.editor.doc = Document::default();
        self.dirty_since = None;
        self.refresh_notes();
        let next = match self.notes.first() {
            Some(note) => Ok(note.id.clone()),
            None => self.store.create(),
        };
        match next {
            Ok(id) => self.open(id),
            Err(e) => self.error = Some(format!("Couldn't create note: {e}")),
        }
    }

    #[cfg(test)]
    fn sidebar_editing_is_none(&self) -> bool {
        self.sidebar.editing_folder().is_none()
    }

    fn note_times(&self) -> NoteTimes {
        let meta = self.notes.iter().find(|note| note.id == self.current);
        let created = self
            .editor
            .doc
            .properties
            .created
            .or_else(|| meta.map(|note| note.created))
            .unwrap_or_else(SystemTime::now);
        let updated = if self.dirty_since.is_some() {
            SystemTime::now()
        } else {
            meta.map(|note| note.modified)
                .unwrap_or_else(SystemTime::now)
        };
        // A blank note isn't sent anywhere, so it has nothing to show.
        let sync = if self.editor.doc.is_blank() {
            None
        } else {
            let saved = self
                .dirty_since
                .is_none()
                .then_some(self.saved_hash.as_str());
            self.drive.note_status(&self.current, saved)
        };
        NoteTimes {
            created,
            updated,
            sync,
        }
    }

    fn title(&self) -> String {
        self.editor
            .doc
            .title()
            .unwrap_or_else(|| UNTITLED.to_string())
    }

    fn move_selection(&mut self, destination: String, markdown: String) {
        // Persist the deletion before creating the destination so the move is
        // atomic from the user's point of view when the app switches pages.
        self.save();
        match self.store.load(&destination).and_then(|mut doc| {
            if doc.is_blank() {
                doc = Document::from_markdown(&markdown);
            } else {
                let last = doc.blocks.len() - 1;
                let end = Caret {
                    block: last,
                    char: doc.blocks[last].text.chars().count(),
                };
                let insert_at = if doc.blocks[last].text.is_empty() {
                    end
                } else {
                    scripture_study_core::editor::split_block(
                        &mut doc, end.block, end.char, end.char,
                    );
                    Caret {
                        block: end.block + 1,
                        char: 0,
                    }
                };
                scripture_study_core::selection::insert(&mut doc, insert_at, &markdown);
            }
            self.store.save(&destination, &doc)
        }) {
            Ok(()) => self.open(destination),
            Err(e) => self.error = Some(format!("Couldn't move selection: {e}")),
        }
    }

    fn window_title(&self) -> String {
        self.settings
            .title()
            .or_else(|| self.scriptures.title())
            .or_else(|| self.talks.title(&self.talk_library))
            .unwrap_or_else(|| self.title())
    }

    /// What fills the window beside the rail.
    fn page(&self) -> Page {
        if self.settings.is_open() {
            Page::Settings
        } else if self.scriptures.is_open() {
            Page::Scriptures
        } else if self.talks.is_open() {
            Page::Talks
        } else {
            Page::Notes
        }
    }

    /// Back to the open note, keeping whatever the sidebar shows.
    fn close_pages(&mut self) {
        self.scriptures.close();
        self.talks.close();
        self.settings.close();
    }
}

impl ScriptureStudyApp {
    fn shortcuts(&mut self, ui: &mut egui::Ui) {
        // While a context menu is open, its shortcuts act on what it was
        // opened for; the menu handles them.
        let menu_open = egui::Popup::is_any_open(ui.ctx());
        // Shift-modified shortcuts first: egui treats extra Shift as optional.
        if !menu_open && ui.input_mut(|i| i.consume_shortcut(&DELETE_NOTE)) {
            self.delete(self.current.clone());
        }
        if !menu_open && ui.input_mut(|i| i.consume_shortcut(&NEW_FOLDER)) {
            self.sidebar.new_folder();
            self.close_pages();
        }
        // ⌥⌘R before ⌘R, which also matches with Option held.
        if !menu_open && ui.input_mut(|i| i.consume_shortcut(&REVEAL)) {
            self.sidebar_action(SidebarAction::RevealNote(self.current.clone()));
        }
        if !menu_open && ui.input_mut(|i| i.consume_shortcut(&RENAME)) {
            self.sidebar.rename_note(&self.current, &self.title());
        }
        // ⌘⇧B before anything that matches ⌘B, so it toggles the sidebar
        // instead of bolding the current word.
        if ui.input_mut(|i| {
            i.consume_shortcut(&TOGGLE_SIDEBAR_SHIFT_B) || i.consume_shortcut(&TOGGLE_SIDEBAR)
        }) {
            self.sidebar.toggle();
        }
        if ui.input_mut(|i| i.consume_shortcut(&SEARCH)) {
            self.sidebar.start_search();
            self.close_pages();
        }
        // Cmd+F finds within the open note, or the open chapter on the
        // scriptures page. Cmd+K searches every note.
        if ui.input_mut(|i| i.consume_shortcut(&FIND)) {
            match self.page() {
                Page::Scriptures => self.scriptures.open_find(),
                Page::Talks => self.talks.open_find(),
                Page::Notes => self.editor.find.open(),
                Page::Settings => {}
            }
        }
        // On macOS the menu bar takes ⌘O and ⌘, and reports them here.
        let chosen = crate::app_menu::chosen();
        let mac = cfg!(target_os = "macos");
        let open_folder = !mac && ui.input_mut(|i| i.consume_shortcut(&OPEN_FOLDER));
        if open_folder || chosen.contains(&crate::app_menu::Command::OpenFolder) {
            self.choose_library();
        }
        let settings = !mac && ui.input_mut(|i| i.consume_shortcut(&SETTINGS));
        if settings || chosen.contains(&crate::app_menu::Command::Settings) {
            self.sidebar_action(if self.settings.is_open() {
                SidebarAction::ClosePage
            } else {
                SidebarAction::OpenSettings
            });
        }
        if !menu_open && ui.input_mut(|i| i.consume_shortcut(&NEW_NOTE)) {
            self.run(Action::NewNote);
        }
        if ui.input_mut(|i| i.consume_shortcut(&SAVE)) && self.dirty_since.is_some() {
            self.save();
        }
    }

    fn show_rail(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let action = egui::Panel::left("rail")
            .resizable(false)
            .exact_size(sidebar::RAIL_WIDTH)
            // The sidebar draws its own edge while it is on screen, including
            // during the slide. This is a frame behind `open` so the rail edge
            // doesn't flash in before the panel has left.
            .show_separator_line(!self.sidebar.panel_visible())
            .frame(Frame::new().fill(palette.rail))
            .show(ui, |ui| {
                ui.add_space(TITLEBAR_HEIGHT);
                let page = self.page();
                self.sidebar.rail(ui, palette, page)
            })
            .inner;
        if let Some(action) = action {
            self.sidebar_action(action);
        }
    }

    fn show_sidebar(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        // A copy so dragging the resize edge can't collapse the sidebar.
        // Opening and closing still slide: egui eases the width with
        // `Style::animation_time` (cubic out, 0.2s). Tests zero that time.
        let mut expanded = self.sidebar.open;
        let shown = egui::Panel::left("sidebar")
            .resizable(true)
            .drag_to_open(false)
            .default_size(sidebar::DEFAULT_WIDTH)
            .size_range(200.0..=420.0)
            .frame(Frame::new().fill(palette.sidebar))
            .show_collapsible(ui, &mut expanded, |ui| {
                ui.add_space(TITLEBAR_HEIGHT);
                if self.settings.is_open() {
                    self.settings.show_index(ui, palette);
                    None
                } else if self.scriptures.is_open() {
                    self.scriptures.show_index(ui, palette);
                    None
                } else if self.talks.is_open() {
                    self.talks.show_index(ui, palette, &mut self.talk_library);
                    None
                } else {
                    let title = self.title();
                    let notes = sidebar::Notes {
                        index: &self.index,
                        current: &self.current,
                        current_title: &title,
                        now: SystemTime::now(),
                        tree: &self.tree,
                    };
                    self.sidebar.show(ui, &notes, palette)
                }
            });
        let visible = shown.is_some();
        if visible != self.sidebar.panel_visible() {
            self.sidebar.set_panel_visible(visible);
            // The rail chooses its edge before this panel runs.
            ui.ctx().request_repaint();
        }
        if let Some(action) = shown.and_then(|response| response.inner) {
            self.sidebar_action(action);
        }
        if self.sidebar.is_typing() {
            self.editor.skip_initial_focus();
        }
    }

    fn sidebar_action(&mut self, action: SidebarAction) {
        let result = match action {
            SidebarAction::Open(id) => {
                self.open(id);
                return;
            }
            SidebarAction::New => {
                self.run(Action::NewNote);
                return;
            }
            SidebarAction::OpenScriptures => {
                self.settings.close();
                self.talks.close();
                self.scriptures.ensure_open();
                return;
            }
            SidebarAction::OpenTalks => {
                self.settings.close();
                self.scriptures.close();
                self.talks.ensure_open();
                return;
            }
            SidebarAction::OpenSettings => {
                self.scriptures.close();
                self.talks.close();
                self.settings.open();
                self.sidebar.open = true;
                return;
            }
            SidebarAction::ShowNotes => {
                self.close_pages();
                self.sidebar.show_recent();
                return;
            }
            SidebarAction::ClosePage => {
                self.close_pages();
                return;
            }
            SidebarAction::NewIn(folder) => self.store.create_in(&folder).map(|id| self.open(id)),
            SidebarAction::Delete(id) => {
                self.delete(id);
                return;
            }
            SidebarAction::RenameNote { id, title } => self.rename_note(&id, &title),
            SidebarAction::Move { id, folder } => self.move_note(&id, &folder),
            SidebarAction::CreateFolder(path) => self.store.create_folder(&path),
            SidebarAction::RenameFolder { from, to } => {
                self.store.rename_folder(&from, &to).map(|()| {
                    // The open note's id changes if it was inside.
                    if let Some(rest) = self.current.strip_prefix(&format!("{from}/")) {
                        self.current = store::join(&to, rest);
                    }
                })
            }
            SidebarAction::DeleteFolder(path) => self.store.delete_folder(&path),
            SidebarAction::RevealNote(id) => {
                // A new note only exists on disk once it's saved.
                if id == self.current && self.dirty_since.is_some() {
                    self.save();
                }
                self.store
                    .note_path(&id)
                    .and_then(|path| (self.reveal)(&path))
            }
            SidebarAction::RevealFolder(path) => self
                .store
                .folder_path(&path)
                .and_then(|path| (self.reveal)(&path)),
            SidebarAction::MoveNoteOut(id) => {
                let takes_current = id == self.current;
                self.move_out(takes_current, |store, dest| store.move_note_out(&id, dest));
                return;
            }
            SidebarAction::MoveFolderOut(path) => {
                let takes_current = self.current.starts_with(&format!("{path}/"));
                self.move_out(takes_current, |store, dest| {
                    store.move_folder_out(&path, dest)
                });
                return;
            }
        };
        match result {
            Ok(()) => self.error = None,
            Err(e) => self.error = Some(e.to_string()),
        }
        self.refresh_notes();
        self.drive.notes_changed();
    }

    /// Retitles a note by rewriting its title line.
    fn rename_note(&mut self, id: &str, title: &str) -> std::io::Result<()> {
        if id == self.current {
            let mut doc = self.editor.doc.clone();
            doc.set_title(title);
            self.editor.replace_doc(doc);
            self.save();
            return Ok(());
        }
        let mut doc = self.store.load(id)?;
        doc.set_title(title);
        self.store.save(id, &doc)
    }

    /// Saves the open note, or discards it if it's an empty extra.
    fn close_current(&mut self) {
        if self.editor.doc.is_blank() && self.notes.len() > 1 {
            let _ = self.store.delete(&self.current);
        } else if self.dirty_since.is_some() {
            self.save();
        } else {
            self.flush_outline();
        }
    }

    /// Asks for a directory of notes and switches to it.
    fn choose_library(&mut self) {
        let start = self.store.root().parent().unwrap_or(self.store.root());
        if let Some(dir) = (self.pick_folder)("Open Folder", start) {
            self.open_library(dir);
        }
    }

    /// Switches to another directory of notes, opening its most recent note,
    /// and opens it again next launch.
    fn open_library(&mut self, dir: std::path::PathBuf) {
        let opened = FsStore::open(&dir).and_then(|store| {
            let current = match store.list()?.first() {
                Some(note) => note.id.clone(),
                None => store.create()?,
            };
            let doc = store.load(&current)?;
            Ok((store, current, doc))
        });
        let (store, current, doc) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                self.error = Some(format!("Couldn't open {}: {e}", dir.display()));
                return;
            }
        };
        self.close_current();
        self.store = store;
        self.editor = Editor::new(&current, doc);
        self.current = current;
        self.dirty_since = None;
        self.outline_dirty_since = None;
        self.load_outline();
        self.rehash();
        self.sidebar.forget_notes();
        self.drive.set_library(dir.clone());
        self.error = (self.remember_library)(&dir)
            .err()
            .map(|e| format!("Couldn't remember the folder: {e}"));
        self.refresh_notes();
    }

    /// Moves a note or folder out of the store to a folder the user picks.
    /// `takes_current` says whether the open note leaves with it.
    fn move_out(
        &mut self,
        takes_current: bool,
        move_to: impl FnOnce(&FsStore, &std::path::Path) -> std::io::Result<std::path::PathBuf>,
    ) {
        // Flush pending edits so the file being moved is current.
        if takes_current && self.dirty_since.is_some() {
            self.save();
        } else if takes_current {
            self.flush_outline();
        }
        let start = self.store.root().parent().unwrap_or(self.store.root());
        let Some(dest) = (self.pick_folder)("Move to", start) else {
            return;
        };
        match move_to(&self.store, &dest) {
            Ok(_) => {
                self.error = None;
                self.drive.notes_changed();
            }
            Err(e) => {
                self.error = Some(format!("Couldn't move: {e}"));
                return;
            }
        }
        if takes_current {
            self.current_gone();
        } else {
            self.refresh_notes();
        }
    }

    fn move_note(&mut self, id: &str, folder: &str) -> std::io::Result<()> {
        // Flush pending edits so the file being moved is current.
        if id == self.current && self.dirty_since.is_some() {
            self.save();
        } else if id == self.current {
            self.flush_outline();
        }
        let new_id = self.store.move_note(id, folder)?;
        if id == self.current {
            self.current = new_id;
        }
        self.sidebar.reveal(folder);
        Ok(())
    }

    fn show_scriptures(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let index_open = self.sidebar.open;
        egui::CentralPanel::no_frame().show(ui, |ui| {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, palette.background);
            let full = ui.max_rect();
            let mut column = self.page_column(ui, full);
            let top = full.top() + TITLEBAR_HEIGHT + 16.0;
            column.min.y = top;
            column.max.y = full.bottom();
            ui.scope_builder(UiBuilder::new().max_rect(column), |ui| {
                self.scriptures.show_reading(ui, palette, index_open);
            });
            let top_right = pos2(full.right() - 16.0, full.top() + TITLEBAR_HEIGHT + 4.0);
            self.scriptures.show_find_bar(ui.ctx(), top_right, palette);
        });
    }

    fn show_talks(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        egui::CentralPanel::no_frame().show(ui, |ui| {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, palette.background);
            let full = ui.max_rect();
            let mut column = self.page_column(ui, full);
            column.min.y = full.top() + TITLEBAR_HEIGHT + 16.0;
            column.max.y = full.bottom();
            ui.scope_builder(UiBuilder::new().max_rect(column), |ui| {
                self.talks.show_page(ui, palette, &mut self.talk_library);
            });
            let top_right = pos2(full.right() - 16.0, full.top() + TITLEBAR_HEIGHT + 4.0);
            self.talks.show_find_bar(ui.ctx(), top_right, palette);
        });
    }

    fn show_settings(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        egui::CentralPanel::no_frame().show(ui, |ui| {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, palette.background);
            let full = ui.max_rect();
            let mut column = self.page_column(ui, full);
            column.min.y = full.top() + TITLEBAR_HEIGHT + 36.0;
            column.max.y = full.bottom();
            ui.scope_builder(UiBuilder::new().max_rect(column), |ui| {
                self.settings.show_page(ui, palette, &self.drive);
            });
        });
    }

    fn show_editor(&mut self, ui: &mut egui::Ui, palette: &Palette) -> Vec<Event> {
        let dir = self
            .store
            .note_dir(&self.current)
            .unwrap_or_else(|_| self.store.root().to_path_buf());
        let times = self.note_times();
        let events = egui::CentralPanel::no_frame()
            .show(ui, |ui| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, palette.background);
                let area = ui.max_rect();
                let events = egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.add_space(80.0);
                        let full = ui.available_rect_before_wrap();
                        let column = self.page_column(ui, full);
                        ui.scope_builder(UiBuilder::new().max_rect(column), |ui| {
                            self.editor.show(
                                ui,
                                &self.commands,
                                &self.notes,
                                times,
                                &dir,
                                &mut self.spelling,
                            )
                        })
                        .inner
                    })
                    .inner;
                let top_right = pos2(area.right() - 16.0, area.top() + TITLEBAR_HEIGHT + 4.0);
                self.editor.show_find_bar(ui.ctx(), top_right, palette);
                events
            })
            .inner;
        if let Some(message) = self.spelling.take_error() {
            self.error = Some(message);
        }
        events
    }

    /// Page column for `area`. The left edge stays put; the width eases toward
    /// the size that leaves [`theme::PAGE_MARGIN`] on both sides.
    fn page_column(&mut self, ui: &egui::Ui, area: Rect) -> Rect {
        let ppp = ui.pixels_per_point().max(0.01);
        let target = (theme::column_width(area.width()) * ppp).round() / ppp;
        let dt = ui.input(|i| i.stable_dt);
        if theme::ease_width(&mut self.page_width, target, dt) {
            ui.ctx().request_repaint();
        }
        let width = (self.page_width * ppp).round() / ppp;
        let margin = (area.width() - target).max(0.0) * 0.5;
        Rect::from_min_size(
            pos2(area.left() + margin, area.top()),
            vec2(width.max(0.0), area.height()),
        )
    }

    /// The window-control strip: drag to move, double-click to zoom, plus the
    /// sidebar toggle. On macOS the app draws into the title bar area.
    fn title_bar(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let strip =
            Rect::from_min_size(pos2(0.0, 0.0), vec2(ui.max_rect().width(), TITLEBAR_HEIGHT));
        let drag = ui.interact(
            strip,
            egui::Id::new("titlebar"),
            egui::Sense::click_and_drag(),
        );
        if drag.double_clicked() {
            let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        } else if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if sidebar::toggle_button(ui, toggle_rect(), palette).clicked() {
            self.sidebar.toggle();
        }
    }
}

/// Picture files that can be dropped into a note.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg"];

impl ScriptureStudyApp {
    /// Whether Cmd+V was pressed with a picture (and no text) on the
    /// clipboard. egui only turns a paste into an event when the clipboard
    /// holds text; otherwise it drops the key press entirely, so the paste
    /// shows up as a release of V whose press never arrived.
    fn image_paste(&mut self, ui: &egui::Ui) -> bool {
        let mut paste = false;
        ui.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Paste(_) => self.text_pasted = true,
                    egui::Event::Key {
                        key: egui::Key::V,
                        pressed,
                        ..
                    } => {
                        if *pressed {
                            self.v_down = true;
                        } else {
                            paste = !self.v_down && !self.text_pasted;
                            self.v_down = false;
                            self.text_pasted = false;
                        }
                    }
                    _ => {}
                }
            }
        });
        paste
    }

    /// Pastes a clipboard picture, or adds dropped picture files, to the note.
    fn add_images(&mut self, ui: &egui::Ui) {
        let mut images = Vec::new();
        // Not while a sidebar field has the keyboard.
        let sidebar_busy = self.sidebar.is_typing() || self.sidebar.is_searching();
        if self.image_paste(ui) && !sidebar_busy {
            images.extend((self.clipboard_image)().map(|png| ("png".to_string(), png)));
        }
        for file in ui.input(|i| i.raw.dropped_files.clone()) {
            let path = file.path();
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_lowercase)
                .unwrap_or_default();
            if !IMAGE_EXTENSIONS.contains(&extension.as_str()) {
                self.error = Some(format!(
                    "Only PNG and JPEG pictures can be added ({})",
                    path.display()
                ));
                continue;
            }
            match file.bytes() {
                Ok(bytes) => images.push((extension, bytes)),
                Err(e) => self.error = Some(format!("Couldn't read {}: {e}", path.display())),
            }
        }
        for (extension, bytes) in images {
            match self.store.save_asset(&self.current, &extension, &bytes) {
                Ok(src) => {
                    self.editor.insert_image(src);
                    let now = ui.input(|i| i.time);
                    self.dirty_since.get_or_insert(now);
                }
                Err(e) => self.error = Some(format!("Couldn't save the picture: {e}")),
            }
        }
    }

    /// A hint over the page while picture files are dragged over the window.
    fn drop_hint(&self, ui: &egui::Ui, palette: &Palette) {
        if ui.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let rect = ui.max_rect().shrink(12.0);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop-hint"),
        ));
        painter.rect_filled(rect, 12.0, palette.background.gamma_multiply(0.85));
        painter.rect_stroke(
            rect,
            12.0,
            egui::Stroke::new(2.0, palette.accent),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop pictures to add them to this note",
            egui::FontId::proportional(16.0),
            palette.text,
        );
    }
}

/// Opens the system file manager with `path` selected.
fn reveal_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    use std::process::Command;
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg("-R").arg(path);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    } else {
        // Most Linux file managers can't select a file; open its folder.
        let mut c = Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    let mut child = command.spawn()?;
    // Reap it once it exits so it doesn't linger as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Asks for a folder with the system's folder picker.
fn pick_folder(title: &str, start: &std::path::Path) -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .set_title(title)
        .set_directory(start)
        .pick_folder()
}

/// The clipboard's picture as PNG bytes, if it holds one.
fn clipboard_png() -> Option<Vec<u8>> {
    let image = arboard::Clipboard::new().ok()?.get_image().ok()?;
    let rgba = image::RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.into_owned(),
    )?;
    let mut png = std::io::Cursor::new(Vec::new());
    rgba.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

impl eframe::App for ScriptureStudyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let palette = Palette::for_ui(ui);
        let title_before = self.window_title();

        if self.talk_library.poll() {
            self.talks.library_changed();
        }
        self.shortcuts(ui);
        if self.page() == Page::Notes {
            self.add_images(ui);
        }
        self.show_rail(ui, &palette);
        self.show_sidebar(ui, &palette);
        let events = match self.page() {
            Page::Settings => {
                self.show_settings(ui, &palette);
                Vec::new()
            }
            Page::Scriptures => {
                self.show_scriptures(ui, &palette);
                Vec::new()
            }
            Page::Talks => {
                self.show_talks(ui, &palette);
                Vec::new()
            }
            Page::Notes => self.show_editor(ui, &palette),
        };
        self.title_bar(ui, &palette);
        if self.page() == Page::Notes {
            self.drop_hint(ui, &palette);
        }

        let now = ui.input(|i| i.time);
        for event in events {
            match event {
                Event::Changed => {
                    self.dirty_since.get_or_insert(now);
                }
                // Running a slash command removes its query from the note.
                Event::Run(action) => {
                    self.dirty_since.get_or_insert(now);
                    self.run(action);
                }
                Event::OpenNote(id) => {
                    if id != self.current {
                        self.open(id);
                    }
                }
                Event::OpenScripture(reference) => {
                    self.scriptures.open_reference(&reference);
                }
                Event::MoveSelection {
                    destination,
                    markdown,
                } => {
                    self.move_selection(destination, markdown);
                }
            }
        }

        if let Some(since) = self.dirty_since {
            let elapsed = now - since;
            if elapsed >= SAVE_DELAY {
                self.save();
            } else {
                ui.ctx()
                    .request_repaint_after_secs((SAVE_DELAY - elapsed) as f32);
            }
        }
        self.persist_outline(now, ui);

        if let Some(error) = &self.error {
            egui::Area::new(egui::Id::new("error"))
                .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -16.0))
                .show(ui.ctx(), |ui| {
                    ui.colored_label(egui::Color32::from_rgb(0xD4, 0x4C, 0x47), error)
                });
        }

        let title = self.window_title();
        if title != title_before || ui.ctx().cumulative_frame_nr() == 0 {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    fn on_exit(&mut self) {
        self.close_current();
    }
}
