//! Per-folder settings that aren't part of a note's Markdown.
//!
//! Each directory of notes may hold a hidden `.scripture-study` file. It is
//! keyed by note filename (without `.md`) so every note in that folder can
//! keep its own settings, and moving the folder takes the file along.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Name of the settings file inside a notes directory.
pub const FILE_NAME: &str = ".scripture-study";

const VERSION: u32 = 1;

/// Settings for one note. Empty settings are left out of the file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteSettings {
    /// Outline items whose sub items are folded, in document order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collapsed: Vec<CollapsedOutline>,
}

impl NoteSettings {
    pub fn is_blank(&self) -> bool {
        self.collapsed.is_empty()
    }
}

/// One folded outline item.
///
/// `key` is [`crate::document::outline_key`]. `nth` counts blocks with that
/// key from the top of the note, so two items with the same first line stay
/// distinct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollapsedOutline {
    pub key: String,
    #[serde(default)]
    pub nth: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FolderSettings {
    #[serde(default = "current_version")]
    version: u32,
    #[serde(default)]
    notes: BTreeMap<String, NoteSettings>,
}

impl Default for FolderSettings {
    fn default() -> Self {
        Self {
            version: VERSION,
            notes: BTreeMap::new(),
        }
    }
}

fn current_version() -> u32 {
    VERSION
}

/// Settings stored for the note named `stem` in `dir`. A missing file means
/// the note has none.
pub fn load_note(dir: &Path, stem: &str) -> io::Result<NoteSettings> {
    Ok(read_folder(dir)?
        .notes
        .get(stem)
        .cloned()
        .unwrap_or_default())
}

/// Writes `settings` for `stem`, keeping every other note in the file.
/// Blank settings drop that note's entry; the file goes away when nothing
/// is left. A file from a newer version is left untouched.
pub fn store_note(dir: &Path, stem: &str, settings: &NoteSettings) -> io::Result<()> {
    let mut folder = read_folder(dir)?;
    if folder.version > VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{FILE_NAME} is from a newer version"),
        ));
    }
    folder.version = VERSION;
    if settings.is_blank() {
        folder.notes.remove(stem);
    } else {
        folder.notes.insert(stem.to_string(), settings.clone());
    }
    write_folder(dir, &folder)
}

/// Moves one note's settings from one directory to another.
///
/// Does nothing when the note has no settings. If writing the destination
/// fails, the source is left as it was.
pub fn move_note(from_dir: &Path, from_stem: &str, to_dir: &Path, to_stem: &str) -> io::Result<()> {
    let settings = load_note(from_dir, from_stem)?;
    if settings.is_blank() {
        return Ok(());
    }
    let previous = load_note(to_dir, to_stem)?;
    store_note(to_dir, to_stem, &settings)?;
    if let Err(e) = store_note(from_dir, from_stem, &NoteSettings::default()) {
        let _ = store_note(to_dir, to_stem, &previous);
        return Err(e);
    }
    Ok(())
}

fn read_folder(dir: &Path) -> io::Result<FolderSettings> {
    let path = dir.join(FILE_NAME);
    let text = match fs::read_to_string(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(FolderSettings::default()),
        Err(e) => return Err(e),
        Ok(text) => text,
    };
    if text.trim().is_empty() {
        return Ok(FolderSettings::default());
    }
    serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn write_folder(dir: &Path, folder: &FolderSettings) -> io::Result<()> {
    let path = dir.join(FILE_NAME);
    if folder.notes.is_empty() {
        return match fs::remove_file(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        };
    }
    let mut body = serde_json::to_vec_pretty(folder)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    body.push(b'\n');
    let tmp = dir.join(format!("{FILE_NAME}.tmp"));
    fs::write(&tmp, &body)?;
    fs::rename(&tmp, &path)
}
