//! Note lifecycle (which note is open, autosave, note commands) and the
//! window layout: title bar, sidebar, and editor.

use std::time::SystemTime;

use eframe::egui::{self, pos2, vec2, Frame, Rect, UiBuilder};
use scripture_study_core::{
    commands::{self, Action, Command},
    folders::{self, Folder},
    search::IndexedNote,
    store,
    store::UNTITLED,
    Document, FsStore, NoteMeta, NoteStore,
};

use crate::editor::{Editor, Event};
use crate::meta::NoteTimes;
use crate::shortcuts::{
    DELETE_NOTE, FIND, NEW_FOLDER, NEW_NOTE, OPEN_FOLDER, RENAME, REVEAL, SAVE, SEARCH,
    TOGGLE_SIDEBAR, TOGGLE_SIDEBAR_SHIFT_B,
};
use crate::sidebar::{self, Sidebar, SidebarAction};
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
}

impl ScriptureStudyApp {
    pub fn new(cc: &eframe::CreationContext<'_>, store: FsStore) -> std::io::Result<Self> {
        theme::install_fonts(&cc.egui_ctx);
        let notes = store.list()?;
        let current = match notes.first() {
            Some(note) => note.id.clone(),
            None => store.create()?,
        };
        let doc = store.load(&current)?;
        let mut app = Self {
            store,
            notes,
            index: Vec::new(),
            tree: folders::tree(&[], &[]),
            editor: Editor::new(&current, doc),
            current,
            sidebar: Sidebar::default(),
            commands: Vec::new(),
            dirty_since: None,
            error: None,
            clipboard_image: clipboard_png,
            reveal: reveal_in_file_manager,
            pick_folder,
            remember_library: crate::library::remember,
            v_down: false,
            text_pasted: false,
        };
        app.refresh_notes();
        Ok(app)
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
        self.refresh_notes();
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
            }
        }
        match self.store.load(&id) {
            Ok(doc) => {
                self.editor = Editor::new(&id, doc);
                self.sidebar.reveal(store::parent(&id));
                self.current = id;
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
            Action::SetBlock(_) | Action::InsertCitation | Action::InsertScripture => {}
        }
    }

    /// Deletes a note. Deleting the open note moves to the most recent one.
    fn delete(&mut self, id: String) {
        if let Err(e) = self.store.delete(&id) {
            self.error = Some(format!("Couldn't delete note: {e}"));
            return;
        }
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
        NoteTimes { created, updated }
    }

    fn title(&self) -> String {
        self.editor
            .doc
            .title()
            .unwrap_or_else(|| UNTITLED.to_string())
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
        }
        // Cmd+F finds within the open note; Cmd+K searches every note.
        if ui.input_mut(|i| i.consume_shortcut(&FIND)) {
            self.editor.find.open();
        }
        // On macOS the menu bar takes ⌘O and reports it here.
        let open_folder =
            !cfg!(target_os = "macos") && ui.input_mut(|i| i.consume_shortcut(&OPEN_FOLDER));
        if open_folder || crate::app_menu::chosen().contains(&crate::app_menu::Command::OpenFolder)
        {
            self.choose_library();
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
                self.sidebar.rail(ui, palette)
            })
            .inner;
        if let Some(SidebarAction::New) = action {
            self.run(Action::NewNote);
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
                let title = self.title();
                let notes = sidebar::Notes {
                    index: &self.index,
                    current: &self.current,
                    current_title: &title,
                    now: SystemTime::now(),
                    tree: &self.tree,
                };
                self.sidebar.show(ui, &notes, palette)
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
        self.sidebar.forget_notes();
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
        }
        let start = self.store.root().parent().unwrap_or(self.store.root());
        let Some(dest) = (self.pick_folder)("Move to", start) else {
            return;
        };
        match move_to(&self.store, &dest) {
            Ok(_) => self.error = None,
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
        }
        let new_id = self.store.move_note(id, folder)?;
        if id == self.current {
            self.current = new_id;
        }
        self.sidebar.reveal(folder);
        Ok(())
    }

    fn show_editor(&mut self, ui: &mut egui::Ui, palette: &Palette) -> Vec<Event> {
        let dir = self
            .store
            .note_dir(&self.current)
            .unwrap_or_else(|_| self.store.root().to_path_buf());
        let times = self.note_times();
        egui::CentralPanel::no_frame()
            .show(ui, |ui| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, palette.background);
                let area = ui.max_rect();
                let events = egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.add_space(80.0);
                        let full = ui.available_rect_before_wrap();
                        let width = theme::CONTENT_WIDTH.min(full.width() - 48.0);
                        let column = Rect::from_min_size(
                            pos2(full.center().x - width / 2.0, full.top()),
                            vec2(width, full.height()),
                        );
                        ui.scope_builder(UiBuilder::new().max_rect(column), |ui| {
                            self.editor.show(ui, &self.commands, times, &dir)
                        })
                        .inner
                    })
                    .inner;
                let top_right = pos2(area.right() - 16.0, area.top() + TITLEBAR_HEIGHT + 4.0);
                self.editor.show_find_bar(ui.ctx(), top_right, palette);
                events
            })
            .inner
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
        let title_before = self.title();

        self.shortcuts(ui);
        self.add_images(ui);
        self.show_rail(ui, &palette);
        self.show_sidebar(ui, &palette);
        let events = self.show_editor(ui, &palette);
        self.title_bar(ui, &palette);
        self.drop_hint(ui, &palette);

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

        if let Some(error) = &self.error {
            egui::Area::new(egui::Id::new("error"))
                .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -16.0))
                .show(ui.ctx(), |ui| {
                    ui.colored_label(egui::Color32::from_rgb(0xD4, 0x4C, 0x47), error)
                });
        }

        let title = self.title();
        if title != title_before || ui.ctx().cumulative_frame_nr() == 0 {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    fn on_exit(&mut self) {
        self.close_current();
    }
}
