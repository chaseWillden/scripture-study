//! Which directory of notes the app opens: `$SCRIPTURE_STUDY_DIR`, else the
//! one last opened with File → Open Folder, else the platform data directory
//! (e.g. `~/Library/Application Support/scripture-study`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn dir() -> PathBuf {
    std::env::var_os("SCRIPTURE_STUDY_DIR")
        .map(PathBuf::from)
        .or_else(saved)
        .or_else(|| dirs::data_dir().map(|d| d.join("scripture-study")))
        .unwrap_or_else(|| PathBuf::from("scripture-study"))
}

/// Remembers `dir` as the directory to open next launch.
pub fn remember(dir: &Path) -> io::Result<()> {
    let file = settings_file().ok_or_else(|| io::Error::other("no settings directory"))?;
    fs::create_dir_all(file.parent().unwrap_or(&file))?;
    fs::write(file, dir.to_string_lossy().as_bytes())
}

/// The remembered directory, if it's still there. One that has since been
/// moved or deleted is skipped rather than recreated empty.
fn saved() -> Option<PathBuf> {
    let text = fs::read_to_string(settings_file()?).ok()?;
    let dir = PathBuf::from(text.trim());
    dir.is_dir().then_some(dir)
}

/// Kept apart from the default notes directory, which it may point away from.
fn settings_file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("scripture-study").join("library"))
}
