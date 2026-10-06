//! Persistence. Each note is one Markdown file.
//!
//! [`NoteStore`] is the seam other platforms plug into (e.g. browser storage
//! on the web); [`FsStore`] keeps notes as `.md` files in a directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::document::{BlockKind, Document};
use crate::settings::{self, NoteSettings};
use crate::time::civil_from_days;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteMeta {
    /// Path of the note within the store, without extension, using `/`
    /// between folders: `2026-09-28-142233` or `Work/Ideas/2026-09-28-142233`.
    pub id: String,
    pub title: String,
    pub modified: SystemTime,
    /// Frontmatter `created`, or the file's birth time when that isn't set.
    pub created: SystemTime,
}

impl NoteMeta {
    /// The folder holding this note (`""` for the top level).
    pub fn folder(&self) -> &str {
        parent(&self.id)
    }
}

pub const UNTITLED: &str = "Untitled";

/// Hidden folder, next to a note, that holds its pasted images. Hidden so it
/// never shows up as a folder of notes.
pub const ASSETS: &str = ".assets";

/// The folder part of a note id or folder path (`""` for the top level).
pub fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// The last component of a note id or folder path.
pub fn name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

/// Joins a folder path and a name (`""` is the top level).
pub fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_string()
    } else {
        format!("{folder}/{name}")
    }
}

/// Whether `name` can be a folder name: no path separators, not hidden, no
/// surrounding whitespace.
pub fn is_valid_folder_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 120
        && name.trim() == name
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':') || c.is_control())
}

pub trait NoteStore {
    /// All notes in every folder, most recently modified first.
    fn list(&self) -> io::Result<Vec<NoteMeta>>;
    fn load(&self, id: &str) -> io::Result<Document>;
    fn save(&self, id: &str, doc: &Document) -> io::Result<()>;
    /// Creates an empty note in `folder` (`""` for the top level) and
    /// returns its id.
    fn create_in(&self, folder: &str) -> io::Result<String>;
    fn delete(&self, id: &str) -> io::Result<()>;
    /// Moves a note into `folder`, returning its new id.
    fn move_note(&self, id: &str, folder: &str) -> io::Result<String>;

    /// Every folder path, including empty folders, sorted.
    fn folders(&self) -> io::Result<Vec<String>>;
    /// Creates a folder (and any missing parents).
    fn create_folder(&self, path: &str) -> io::Result<()>;
    /// Renames or moves a folder with everything in it.
    fn rename_folder(&self, from: &str, to: &str) -> io::Result<()>;
    /// Deletes a folder. Refuses unless it's empty, so notes are never lost.
    fn delete_folder(&self, path: &str) -> io::Result<()>;

    /// Creates an empty note at the top level.
    fn create(&self) -> io::Result<String> {
        self.create_in("")
    }
}

pub struct FsStore {
    root: PathBuf,
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

impl FsStore {
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The Markdown file of a note.
    pub fn note_path(&self, id: &str) -> io::Result<PathBuf> {
        self.path(id)
    }

    /// The directory of a folder (`""` is the notes directory itself).
    pub fn folder_path(&self, folder: &str) -> io::Result<PathBuf> {
        self.dir(folder)
    }

    /// The directory holding a note, which its relative image paths start from.
    pub fn note_dir(&self, id: &str) -> io::Result<PathBuf> {
        self.dir(parent(id))
    }

    /// Settings kept beside the note, in the folder's `.scripture-study` file.
    pub fn note_settings(&self, id: &str) -> io::Result<NoteSettings> {
        let _ = self.path(id)?;
        settings::load_note(&self.note_dir(id)?, name(id))
    }

    /// Replaces the note's settings. Blank settings remove its entry.
    pub fn set_note_settings(&self, id: &str, note_settings: &NoteSettings) -> io::Result<()> {
        let _ = self.path(id)?;
        settings::store_note(&self.note_dir(id)?, name(id), note_settings)
    }

    /// Saves an image (or other file) for a note and returns the relative
    /// path to link it with, like `.assets/<note>-<time>.png`.
    pub fn save_asset(&self, id: &str, extension: &str, bytes: &[u8]) -> io::Result<String> {
        let dir = self.note_dir(id)?.join(ASSETS);
        fs::create_dir_all(&dir)?;
        let stem = format!("{}-{}", name(id), timestamp_id(SystemTime::now()));
        let file = free_file(&dir, &stem, extension);
        fs::write(dir.join(&file), bytes)?;
        Ok(format!("{ASSETS}/{file}"))
    }

    /// Moves a note out of the store into `dest`, any directory on disk,
    /// taking its images along. Returns where the note's file ended up.
    pub fn move_note_out(&self, id: &str, dest: &Path) -> io::Result<PathBuf> {
        let source = self.path(id)?;
        let target = dest.join(format!("{}.md", name(id)));
        if target.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} already exists", target.display()),
            ));
        }
        let mut doc = self.load(id)?;
        let from_dir = self.note_dir(id)?;
        let stem = name(id).to_string();
        move_path(&source, &target)?;
        if self.move_assets(&mut doc, &from_dir, dest)? {
            fs::write(&target, doc.to_markdown())?;
        }
        // The settings file stays with the note. A failure here leaves the
        // note moved; the entry is still in the old folder to retry.
        settings::move_note(&from_dir, &stem, dest, &stem)?;
        Ok(target)
    }

    /// Moves a folder with everything in it out of the store into `dest`,
    /// any directory on disk. Returns where the folder ended up.
    pub fn move_folder_out(&self, folder: &str, dest: &Path) -> io::Result<PathBuf> {
        if folder.is_empty() {
            return Err(invalid("can't move the top level".into()));
        }
        let source = self.dir(folder)?;
        let target = dest.join(name(folder));
        if dest.canonicalize()?.starts_with(source.canonicalize()?) {
            return Err(invalid("can't move a folder into itself".into()));
        }
        if target.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} already exists", target.display()),
            ));
        }
        move_path(&source, &target)?;
        Ok(target)
    }

    /// Moves the images a note links to from `from` into `to`, rewriting the
    /// links if a name was taken. Returns whether the document changed.
    fn move_assets(&self, doc: &mut Document, from: &Path, to: &Path) -> io::Result<bool> {
        let mut changed = false;
        for block in &mut doc.blocks {
            let BlockKind::Image { src, .. } = &mut block.kind else {
                continue;
            };
            let Some(file) = src.strip_prefix(&format!("{ASSETS}/")) else {
                continue;
            };
            let source = from.join(ASSETS).join(file);
            if file.contains(['/', '\\']) || !source.is_file() {
                continue;
            }
            fs::create_dir_all(to.join(ASSETS))?;
            let (stem, extension) = file.rsplit_once('.').unwrap_or((file, ""));
            let target = free_file(&to.join(ASSETS), stem, extension);
            move_path(&source, &to.join(ASSETS).join(&target))?;
            if target != file {
                *src = format!("{ASSETS}/{target}");
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Resolves a folder path, rejecting anything that could escape the root.
    fn dir(&self, folder: &str) -> io::Result<PathBuf> {
        if folder.is_empty() {
            return Ok(self.root.clone());
        }
        let mut path = self.root.clone();
        for part in folder.split('/') {
            if !is_valid_folder_name(part) {
                return Err(invalid(format!("bad folder name {part:?}")));
            }
            path.push(part);
        }
        Ok(path)
    }

    fn path(&self, id: &str) -> io::Result<PathBuf> {
        let stem = name(id);
        let valid = !stem.is_empty()
            && stem
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return Err(invalid(format!("bad note id {id:?}")));
        }
        Ok(self.dir(parent(id))?.join(format!("{stem}.md")))
    }

    /// A free note id in `folder` starting from `stem`.
    fn free_id(&self, folder: &str, stem: &str) -> io::Result<String> {
        let mut id = join(folder, stem);
        let mut n = 1;
        while self.path(&id)?.exists() {
            n += 1;
            id = join(folder, &format!("{stem}-{n}"));
        }
        Ok(id)
    }

    fn walk(&self, dir: &Path, folder: &str, visit: &mut dyn FnMut(&str, &Path)) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if file_name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if is_valid_folder_name(file_name) {
                    let sub = join(folder, file_name);
                    visit(&sub, &path);
                    self.walk(&path, &sub, visit)?;
                }
            } else {
                visit(folder, &path);
            }
        }
        Ok(())
    }
}

impl NoteStore for FsStore {
    fn list(&self) -> io::Result<Vec<NoteMeta>> {
        let mut notes = Vec::new();
        self.walk(&self.root, "", &mut |folder, path| {
            if path.is_dir() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                return;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                return;
            };
            let Ok(markdown) = fs::read_to_string(path) else {
                return;
            };
            let Ok(meta) = fs::metadata(path) else {
                return;
            };
            let Ok(modified) = meta.modified() else {
                return;
            };
            let doc = Document::from_markdown(&markdown);
            let created = doc
                .properties
                .created
                .or_else(|| meta.created().ok())
                .unwrap_or(modified);
            notes.push(NoteMeta {
                id: join(folder, stem),
                title: doc.title().unwrap_or_else(|| UNTITLED.to_string()),
                modified,
                created,
            });
        })?;
        notes.sort_by(|a, b| b.modified.cmp(&a.modified).then(b.id.cmp(&a.id)));
        Ok(notes)
    }

    fn load(&self, id: &str) -> io::Result<Document> {
        Ok(Document::from_markdown(&fs::read_to_string(
            self.path(id)?,
        )?))
    }

    fn save(&self, id: &str, doc: &Document) -> io::Result<()> {
        let path = self.path(id)?;
        // Captured before the rename replaces the inode.
        let created = fs::metadata(&path)
            .ok()
            .and_then(|meta| meta.created().ok());
        // Write-then-rename so a crash never leaves a half-written note.
        let tmp = path.with_extension("md.tmp");
        fs::write(&tmp, doc.to_markdown())?;
        fs::rename(tmp, &path)?;
        if let Some(created) = created {
            restore_created(&path, created);
        }
        Ok(())
    }

    fn create_in(&self, folder: &str) -> io::Result<String> {
        fs::create_dir_all(self.dir(folder)?)?;
        let id = self.free_id(folder, &timestamp_id(SystemTime::now()))?;
        fs::write(self.path(&id)?, "")?;
        Ok(id)
    }

    fn delete(&self, id: &str) -> io::Result<()> {
        let dir = self.note_dir(id)?;
        let stem = name(id).to_string();
        fs::remove_file(self.path(id)?)?;
        settings::store_note(&dir, &stem, &NoteSettings::default())
    }

    fn move_note(&self, id: &str, folder: &str) -> io::Result<String> {
        if parent(id) == folder {
            return Ok(id.to_string());
        }
        let dir = self.dir(folder)?;
        if !dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no folder {folder:?}"),
            ));
        }
        let new_id = self.free_id(folder, name(id))?;
        let from_dir = self.note_dir(id)?;
        let from_stem = name(id).to_string();
        fs::rename(self.path(id)?, self.path(&new_id)?)?;
        // Images live next to the note, so they move with it.
        let mut doc = self.load(&new_id)?;
        if self.move_assets(&mut doc, &from_dir, &dir)? {
            self.save(&new_id, &doc)?;
        }
        settings::move_note(&from_dir, &from_stem, &dir, name(&new_id))?;
        Ok(new_id)
    }

    fn folders(&self) -> io::Result<Vec<String>> {
        let mut folders = Vec::new();
        self.walk(&self.root, "", &mut |folder, path| {
            if path.is_dir() {
                folders.push(folder.to_string());
            }
        })?;
        folders.sort_by_key(|f| f.to_lowercase());
        Ok(folders)
    }

    fn create_folder(&self, path: &str) -> io::Result<()> {
        if path.is_empty() {
            return Err(invalid("folder needs a name".into()));
        }
        let dir = self.dir(path)?;
        if dir.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{:?} already exists", name(path)),
            ));
        }
        fs::create_dir_all(dir)
    }

    fn rename_folder(&self, from: &str, to: &str) -> io::Result<()> {
        if from.is_empty() || to.is_empty() {
            return Err(invalid("can't rename the top level".into()));
        }
        if to == from || to.starts_with(&format!("{from}/")) {
            return Err(invalid("can't move a folder into itself".into()));
        }
        let (source, target) = (self.dir(from)?, self.dir(to)?);
        // Case-only renames ("work" -> "Work") are the same path on macOS.
        let case_only = from.to_lowercase() == to.to_lowercase();
        if target.exists() && !case_only {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{:?} already exists", name(to)),
            ));
        }
        fs::create_dir_all(self.dir(parent(to))?)?;
        fs::rename(source, target)
    }

    fn delete_folder(&self, path: &str) -> io::Result<()> {
        if path.is_empty() {
            return Err(invalid("can't delete the top level".into()));
        }
        let dir = self.dir(path)?;
        // Hidden files (e.g. Finder's .DS_Store) don't count as content.
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if !entry.file_name().to_string_lossy().starts_with('.') {
                return Err(io::Error::other(format!("{:?} isn't empty", name(path))));
            }
        }
        fs::remove_dir_all(dir)
    }
}

/// Renames a file or directory, copying then deleting when `to` is on
/// another drive, where a rename can't reach.
fn move_path(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            copy_all(from, to)?;
            if from.is_dir() {
                fs::remove_dir_all(from)
            } else {
                fs::remove_file(from)
            }
        }
        result => result,
    }
}

/// Copies a file, or a directory and everything in it.
fn copy_all(from: &Path, to: &Path) -> io::Result<()> {
    if !from.is_dir() {
        return fs::copy(from, to).map(|_| ());
    }
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        copy_all(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

/// `stem.extension` in `dir`, or `stem-2.extension` and so on if taken.
fn free_file(dir: &Path, stem: &str, extension: &str) -> String {
    let file = |suffix: String| {
        if extension.is_empty() {
            format!("{stem}{suffix}")
        } else {
            format!("{stem}{suffix}.{extension}")
        }
    };
    let mut name = file(String::new());
    let mut n = 1;
    while dir.join(&name).exists() {
        n += 1;
        name = file(format!("-{n}"));
    }
    name
}

/// A sortable, filename-safe id like `2026-09-28-142233` (UTC).
fn timestamp_id(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Puts `created` back after an atomic replace. macOS and Windows can store
/// a birth time; elsewhere the filesystem doesn't, and frontmatter holds it.
fn restore_created(path: &Path, created: SystemTime) {
    let Ok(file) = fs::OpenOptions::new().write(true).open(path) else {
        return;
    };
    #[cfg(target_os = "macos")]
    {
        use std::os::darwin::fs::FileTimesExt;
        let _ = file.set_times(fs::FileTimes::new().set_created(created));
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::FileTimesExt;
        let _ = file.set_times(fs::FileTimes::new().set_created(created));
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (file, created);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Block;
    use std::time::Duration;

    #[test]
    fn timestamp_ids_are_dates() {
        assert_eq!(timestamp_id(UNIX_EPOCH), "1970-01-01-000000");
        // 2026-09-28 14:22:33 UTC
        let t = UNIX_EPOCH + Duration::from_secs(1_790_605_353);
        assert_eq!(timestamp_id(t), "2026-09-28-142233");
    }

    #[test]
    fn create_save_load_list_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::open(dir.path()).unwrap();

        let id = store.create().unwrap();
        assert_eq!(store.load(&id).unwrap(), Document::default());

        let doc = Document::new(vec![
            Block::new(BlockKind::Heading(1), "Hello"),
            Block::paragraph("world"),
        ]);
        store.save(&id, &doc).unwrap();
        assert_eq!(store.load(&id).unwrap(), doc);
        assert_eq!(
            fs::read_to_string(dir.path().join(format!("{id}.md"))).unwrap(),
            "# Hello\n\nworld\n"
        );

        let notes = store.list().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "Hello");

        store.delete(&id).unwrap();
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn ids_are_unique_within_a_second() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::open(dir.path()).unwrap();
        let a = store.create().unwrap();
        let b = store.create().unwrap();
        assert_ne!(a, b);
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::open(dir.path()).unwrap();
        assert!(store.load("../secret").is_err());
    }

    #[test]
    fn untitled_notes_get_a_placeholder_title() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::open(dir.path()).unwrap();
        store.create().unwrap();
        assert_eq!(store.list().unwrap()[0].title, UNTITLED);
    }

    fn store() -> (tempfile::TempDir, FsStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::open(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn notes_live_in_nested_folders() {
        let (dir, store) = store();
        store.create_folder("Work/Projects").unwrap();
        let id = store.create_in("Work/Projects").unwrap();
        assert!(id.starts_with("Work/Projects/"));
        store
            .save(&id, &Document::from_markdown("# Plan\n"))
            .unwrap();
        assert!(dir.path().join(format!("{id}.md")).exists());

        let notes = store.list().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].folder(), "Work/Projects");
        assert_eq!(notes[0].title, "Plan");
        assert_eq!(store.folders().unwrap(), ["Work", "Work/Projects"]);
    }

    #[test]
    fn moving_notes_between_folders() {
        let (_dir, store) = store();
        store.create_folder("Archive").unwrap();
        let id = store.create().unwrap();
        let moved = store.move_note(&id, "Archive").unwrap();
        assert_eq!(moved, format!("Archive/{id}"));
        assert!(store.load(&id).is_err());
        assert!(store.load(&moved).is_ok());

        // Back to the top level.
        assert_eq!(store.move_note(&moved, "").unwrap(), id);
        // Moving into a folder that doesn't exist fails without losing the note.
        assert!(store.move_note(&id, "Nope").is_err());
        assert!(store.load(&id).is_ok());
    }

    #[test]
    fn pasted_images_live_next_to_the_note_and_move_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::open(dir.path()).unwrap();
        store.create_folder("Work").unwrap();
        let id = store.create().unwrap();
        let src = store.save_asset(&id, "png", b"png bytes").unwrap();
        assert!(src.starts_with(".assets/") && src.ends_with(".png"));
        assert_eq!(fs::read(dir.path().join(&src)).unwrap(), b"png bytes");
        let image = BlockKind::Image {
            src: src.clone(),
            alt: String::new(),
        };
        let doc = Document::new(vec![Block::paragraph("Shot"), Block::new(image, "")]);
        store.save(&id, &doc).unwrap();
        // The hidden folder isn't a folder of notes.
        assert_eq!(store.folders().unwrap(), ["Work"]);

        // A clashing name in the destination gets a new one.
        fs::create_dir_all(dir.path().join("Work/.assets")).unwrap();
        fs::write(dir.path().join("Work").join(&src), b"other").unwrap();
        let moved = store.move_note(&id, "Work").unwrap();
        let doc = store.load(&moved).unwrap();
        let BlockKind::Image { src: new_src, .. } = &doc.blocks[1].kind else {
            panic!("image block lost");
        };
        assert_ne!(new_src, &src);
        assert_eq!(
            fs::read(dir.path().join("Work").join(new_src)).unwrap(),
            b"png bytes"
        );
        assert!(!dir.path().join(&src).exists());
    }

    #[test]
    fn moving_notes_out_of_the_store_takes_their_images() {
        let (_dir, store) = store();
        let elsewhere = tempfile::tempdir().unwrap();
        let id = store.create().unwrap();
        let src = store.save_asset(&id, "png", b"png bytes").unwrap();
        let image = BlockKind::Image {
            src: src.clone(),
            alt: String::new(),
        };
        store
            .save(&id, &Document::new(vec![Block::new(image, "")]))
            .unwrap();

        let target = store.move_note_out(&id, elsewhere.path()).unwrap();
        assert_eq!(target, elsewhere.path().join(format!("{id}.md")));
        assert!(store.list().unwrap().is_empty());
        assert!(!store.root().join(&src).exists());
        assert_eq!(fs::read(elsewhere.path().join(&src)).unwrap(), b"png bytes");

        // Never overwrites what's already there.
        fs::write(store.root().join(format!("{id}.md")), "again").unwrap();
        assert!(store.move_note_out(&id, elsewhere.path()).is_err());
        assert!(store.load(&id).is_ok());
    }

    #[test]
    fn moving_folders_out_of_the_store() {
        let (_dir, store) = store();
        let elsewhere = tempfile::tempdir().unwrap();
        store.create_folder("Work/Sub").unwrap();
        let id = store.create_in("Work/Sub").unwrap();

        assert!(store
            .move_folder_out("Work", &store.root().join("Work/Sub"))
            .is_err());
        let target = store.move_folder_out("Work", elsewhere.path()).unwrap();
        assert_eq!(target, elsewhere.path().join("Work"));
        assert!(target.join(format!("Sub/{}.md", name(&id))).exists());
        assert!(store.folders().unwrap().is_empty());
        assert!(store.move_folder_out("", elsewhere.path()).is_err());
    }

    #[test]
    fn moving_avoids_name_clashes() {
        let (_dir, store) = store();
        store.create_folder("A").unwrap();
        fs::write(store.root().join("same.md"), "top").unwrap();
        fs::write(store.root().join("A/same.md"), "in a").unwrap();
        let moved = store.move_note("same", "A").unwrap();
        assert_eq!(moved, "A/same-2");
        assert_eq!(
            fs::read_to_string(store.root().join("A/same.md")).unwrap(),
            "in a"
        );
    }

    #[test]
    fn renaming_folders_keeps_their_notes() {
        let (_dir, store) = store();
        store.create_folder("Old/Sub").unwrap();
        let id = store.create_in("Old/Sub").unwrap();
        store.rename_folder("Old", "New").unwrap();
        assert_eq!(store.folders().unwrap(), ["New", "New/Sub"]);
        assert!(store.load(&id.replacen("Old", "New", 1)).is_ok());

        store.create_folder("Other").unwrap();
        assert!(store.rename_folder("New", "Other").is_err(), "no overwrite");
        assert!(
            store.rename_folder("New", "New/Sub/Deeper").is_err(),
            "no cycles"
        );
        store.rename_folder("New", "new").unwrap(); // case-only rename
    }

    #[test]
    fn only_empty_folders_can_be_deleted() {
        let (_dir, store) = store();
        store.create_folder("Keep").unwrap();
        let id = store.create_in("Keep").unwrap();
        assert!(store.delete_folder("Keep").is_err());
        assert!(store.load(&id).is_ok());

        store.delete(&id).unwrap();
        fs::write(store.root().join("Keep/.DS_Store"), "").unwrap();
        store.delete_folder("Keep").unwrap();
        assert!(store.folders().unwrap().is_empty());
    }

    #[test]
    fn rejects_bad_folder_names() {
        let (_dir, store) = store();
        for bad in [
            "",
            "..",
            ".hidden",
            "a/../b",
            " padded",
            "back\\slash",
            "a//b",
        ] {
            assert!(store.create_folder(bad).is_err(), "{bad:?}");
        }
        assert!(store.load("../outside/x").is_err());
        assert!(store.create_folder("Café & Notes (2026)").is_ok());
        assert!(
            store.create_folder("Café & Notes (2026)").is_err(),
            "exists"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn saving_preserves_the_created_time() {
        let (dir, store) = store();
        let id = store.create().unwrap();
        let path = dir.path().join(format!("{id}.md"));
        let past = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        {
            use std::os::darwin::fs::FileTimesExt;
            let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_times(fs::FileTimes::new().set_created(past))
                .unwrap();
        }
        store
            .save(&id, &Document::from_markdown("# Hello\n"))
            .unwrap();
        let created = fs::metadata(&path).unwrap().created().unwrap();
        assert_eq!(created, past);
        assert_eq!(store.list().unwrap()[0].created, past);
    }

    #[test]
    fn note_settings_live_beside_the_note_and_follow_it() {
        use crate::settings::{CollapsedOutline, NoteSettings, FILE_NAME};

        let (dir, store) = store();
        let id = store.create().unwrap();
        let other = store.create().unwrap();
        let folded = NoteSettings {
            collapsed: vec![CollapsedOutline {
                key: "p:Parent".into(),
                nth: 0,
            }],
        };
        store.set_note_settings(&id, &folded).unwrap();
        store
            .set_note_settings(
                &other,
                &NoteSettings {
                    collapsed: vec![CollapsedOutline {
                        key: "p:Other".into(),
                        nth: 1,
                    }],
                },
            )
            .unwrap();

        let path = dir.path().join(FILE_NAME);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 1"));
        assert!(text.contains("p:Parent"));
        assert!(text.contains("p:Other"));
        assert_eq!(store.note_settings(&id).unwrap(), folded);
        // The settings file is not a note or a folder.
        assert_eq!(store.list().unwrap().len(), 2);
        assert!(store.folders().unwrap().is_empty());

        // Clearing the last entry removes the file. Clearing one keeps the other.
        store
            .set_note_settings(&id, &NoteSettings::default())
            .unwrap();
        assert!(path.exists());
        assert!(store.note_settings(&id).unwrap().is_blank());
        store
            .set_note_settings(&other, &NoteSettings::default())
            .unwrap();
        assert!(!path.exists());

        store.set_note_settings(&id, &folded).unwrap();
        store.create_folder("Work").unwrap();
        let moved = store.move_note(&id, "Work").unwrap();
        assert_eq!(
            store.note_settings(&moved).unwrap().collapsed[0].key,
            "p:Parent"
        );
        assert!(!dir.path().join(FILE_NAME).exists());
        assert!(dir.path().join("Work").join(FILE_NAME).exists());

        store.delete(&moved).unwrap();
        assert!(!dir.path().join("Work").join(FILE_NAME).exists());

        // A file this version doesn't understand is left in place.
        store.set_note_settings(&other, &folded).unwrap();
        fs::write(
            dir.path().join(FILE_NAME),
            "{\"version\": 2, \"notes\": {}}",
        )
        .unwrap();
        assert!(store.set_note_settings(&other, &folded).is_err());
        assert!(fs::read_to_string(dir.path().join(FILE_NAME))
            .unwrap()
            .contains("\"version\": 2"));
    }
}
