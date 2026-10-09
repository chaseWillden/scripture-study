//! One-way sync of every note to a folder in a cloud drive (Google Drive
//! today).
//!
//! The front end lists notes and talks to the service; this module decides
//! what has to change. [`SyncState`] remembers which remote file each note
//! became and what it held, so an unchanged note isn't uploaded again.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::properties::Properties;
use crate::store::{self, UNTITLED};

/// A note as it will look remotely.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalNote {
    pub id: String,
    pub title: String,
    /// The note without its frontmatter, which would show up as stray text.
    pub markdown: String,
    /// Changes whenever `title` or `markdown` does.
    pub hash: String,
}

impl LocalNote {
    pub fn new(id: &str, doc: &Document) -> Self {
        let mut body = doc.clone();
        body.properties = Properties::default();
        let markdown = body.to_markdown();
        let title = doc.title().unwrap_or_else(|| UNTITLED.to_string());
        let hash = content_hash(&title, &markdown);
        Self {
            id: id.to_string(),
            title,
            markdown,
            hash,
        }
    }
}

/// What has already been sent, for one directory of notes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// The remote folder everything below lives in. When it changes, the
    /// rest no longer applies and every note is uploaded again.
    #[serde(default)]
    pub folder_id: Option<String>,
    /// Note id → the remote file it was uploaded as.
    #[serde(default)]
    pub notes: BTreeMap<String, SyncedNote>,
    /// Folder path → remote folder id.
    #[serde(default)]
    pub folders: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedNote {
    pub file_id: String,
    pub hash: String,
}

impl SyncState {
    /// Starts over in another remote folder.
    pub fn retarget(&mut self, folder_id: &str) {
        if self.folder_id.as_deref() != Some(folder_id) {
            *self = Self {
                folder_id: Some(folder_id.to_string()),
                ..Self::default()
            };
        }
    }

    /// The remote folder a note in `folder` goes in: a mirrored subfolder,
    /// or the sync folder itself for the top level.
    pub fn parent_id(&self, folder: &str) -> Option<&str> {
        if folder.is_empty() {
            self.folder_id.as_deref()
        } else {
            self.folders.get(folder).map(String::as_str)
        }
    }

    /// Drops a remote folder and everything recorded inside it.
    pub fn forget_folder(&mut self, path: &str) {
        let inside = |p: &str| p == path || p.starts_with(&format!("{path}/"));
        self.folders.retain(|p, _| !inside(p));
        self.notes.retain(|id, _| !inside(store::parent(id)));
    }

    /// Forgets folders and files that are no longer in the remote folder
    /// (`live` lists every remote id that is), so the next plan makes them
    /// again. Trashing a folder remotely trashes what's inside without
    /// saying so, and writing to a trashed file still succeeds.
    pub fn keep_live(&mut self, live: &BTreeSet<String>) {
        let gone: Vec<String> = self
            .folders
            .iter()
            .filter(|(_, id)| !live.contains(*id))
            .map(|(path, _)| path.clone())
            .collect();
        for path in gone {
            self.forget_folder(&path);
        }
        self.notes.retain(|_, note| live.contains(&note.file_id));
    }
}

/// One change to make remotely, in the order [`plan`] returns them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Mirror a local folder. Its parent is always created first.
    CreateFolder { path: String },
    /// A note that has never been uploaded.
    Upload { id: String },
    /// A note whose text changed since it was uploaded.
    Update { id: String, file_id: String },
    /// A note that was deleted, moved, or renamed away locally.
    Trash { id: String, file_id: String },
    /// A folder that's gone locally. Only the outermost one is listed;
    /// trashing it takes everything inside along.
    TrashFolder { path: String, folder_id: String },
}

/// The steps that bring the remote folder in line with `notes` and
/// `folders` (every local folder path, as [`crate::NoteStore::folders`]
/// lists them).
pub fn plan(notes: &[LocalNote], folders: &[String], state: &SyncState) -> Vec<Step> {
    let mut local_folders: BTreeSet<String> = BTreeSet::new();
    for path in folders
        .iter()
        .map(String::as_str)
        .chain(notes.iter().map(|n| store::parent(&n.id)))
    {
        let mut path = path;
        while !path.is_empty() {
            local_folders.insert(path.to_string());
            path = store::parent(path);
        }
    }

    // A set iterates in order, so "a" comes before "a/b".
    let mut steps: Vec<Step> = local_folders
        .iter()
        .filter(|path| !state.folders.contains_key(*path))
        .map(|path| Step::CreateFolder { path: path.clone() })
        .collect();

    let mut sorted: Vec<&LocalNote> = notes.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    for note in &sorted {
        match state.notes.get(&note.id) {
            None => steps.push(Step::Upload {
                id: note.id.clone(),
            }),
            Some(synced) if synced.hash != note.hash => steps.push(Step::Update {
                id: note.id.clone(),
                file_id: synced.file_id.clone(),
            }),
            Some(_) => {}
        }
    }

    let local_ids: BTreeSet<&str> = notes.iter().map(|n| n.id.as_str()).collect();
    let gone_folders: BTreeSet<&str> = state
        .folders
        .keys()
        .map(String::as_str)
        .filter(|path| !local_folders.contains(*path))
        .collect();
    let in_gone_folder = |path: &str| {
        let mut path = path;
        while !path.is_empty() {
            if gone_folders.contains(path) {
                return true;
            }
            path = store::parent(path);
        }
        false
    };
    for (id, synced) in &state.notes {
        // Trashing the folder takes these along.
        if !local_ids.contains(id.as_str()) && !in_gone_folder(store::parent(id)) {
            steps.push(Step::Trash {
                id: id.clone(),
                file_id: synced.file_id.clone(),
            });
        }
    }
    for path in &gone_folders {
        if !in_gone_folder(store::parent(path)) {
            steps.push(Step::TrashFolder {
                path: path.to_string(),
                folder_id: state.folders[*path].clone(),
            });
        }
    }
    steps
}

/// FNV-1a over the title and text: stable across runs and platforms.
fn content_hash(title: &str, markdown: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in title.bytes().chain([0]).chain(markdown.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, markdown: &str) -> LocalNote {
        LocalNote::new(id, &Document::from_markdown(markdown))
    }

    fn synced(state: &mut SyncState, note: &LocalNote, file_id: &str) {
        state.notes.insert(
            note.id.clone(),
            SyncedNote {
                file_id: file_id.to_string(),
                hash: note.hash.clone(),
            },
        );
    }

    #[test]
    fn frontmatter_is_left_out_and_the_title_comes_from_the_text() {
        let n = note("a", "---\ntags: faith\n---\n# Alma 32\n\nPlant the seed.\n");
        assert_eq!(n.title, "Alma 32");
        assert!(!n.markdown.contains("tags"));
        assert!(n.markdown.contains("Plant the seed."));
        // Properties alone don't count as a change.
        assert_eq!(n.hash, note("a", "# Alma 32\n\nPlant the seed.\n").hash);
        assert_ne!(n.hash, note("a", "# Alma 32\n\nNourish it.\n").hash);
    }

    #[test]
    fn first_sync_creates_folders_parents_first_then_uploads() {
        let notes = [note("Talks/2026/a", "# A\n"), note("b", "# B\n")];
        let steps = plan(&notes, &["Empty".to_string()], &SyncState::default());
        assert_eq!(
            steps,
            [
                Step::CreateFolder {
                    path: "Empty".into()
                },
                Step::CreateFolder {
                    path: "Talks".into()
                },
                Step::CreateFolder {
                    path: "Talks/2026".into()
                },
                Step::Upload {
                    id: "Talks/2026/a".into()
                },
                Step::Upload { id: "b".into() },
            ]
        );
    }

    #[test]
    fn only_changed_notes_are_updated() {
        let a = note("a", "# A\n");
        let b = note("b", "# B\n");
        let mut state = SyncState::default();
        synced(&mut state, &a, "file-a");
        synced(&mut state, &b, "file-b");
        assert_eq!(plan(&[a.clone(), b.clone()], &[], &state), []);

        let b2 = note("b", "# B\n\nMore.\n");
        assert_eq!(
            plan(&[a, b2], &[], &state),
            [Step::Update {
                id: "b".into(),
                file_id: "file-b".into()
            }]
        );
    }

    #[test]
    fn deleted_notes_are_trashed_and_gone_folders_take_their_notes_along() {
        let keep = note("keep", "# Keep\n");
        let mut state = SyncState::default();
        synced(&mut state, &keep, "file-keep");
        synced(&mut state, &note("gone", "# Gone\n"), "file-gone");
        synced(&mut state, &note("Old/Deep/x", "# X\n"), "file-x");
        state.folders.insert("Old".into(), "folder-old".into());
        state
            .folders
            .insert("Old/Deep".into(), "folder-deep".into());

        assert_eq!(
            plan(&[keep], &[], &state),
            [
                Step::Trash {
                    id: "gone".into(),
                    file_id: "file-gone".into()
                },
                Step::TrashFolder {
                    path: "Old".into(),
                    folder_id: "folder-old".into()
                },
            ]
        );

        state.forget_folder("Old");
        assert!(state.folders.is_empty());
        assert!(!state.notes.contains_key("Old/Deep/x"));
        assert!(state.notes.contains_key("keep"));
    }

    #[test]
    fn a_moved_note_is_uploaded_in_its_new_folder_and_trashed_in_the_old() {
        let mut state = SyncState::default();
        synced(&mut state, &note("a", "# A\n"), "file-a");
        let moved = note("Talks/a", "# A\n");
        assert_eq!(
            plan(&[moved], &["Talks".to_string()], &state),
            [
                Step::CreateFolder {
                    path: "Talks".into()
                },
                Step::Upload {
                    id: "Talks/a".into()
                },
                Step::Trash {
                    id: "a".into(),
                    file_id: "file-a".into()
                },
            ]
        );
    }

    #[test]
    fn files_gone_remotely_are_forgotten_and_planned_again() {
        let a = note("a", "# A\n");
        let b = note("Talks/b", "# B\n");
        let mut state = SyncState::default();
        state.retarget("root");
        synced(&mut state, &a, "file-a");
        synced(&mut state, &b, "file-b");
        state.folders.insert("Talks".into(), "folder-talks".into());

        // The Talks folder was trashed in Drive, taking file-b with it.
        let live = ["root", "file-a", "file-b"].map(String::from).into();
        state.keep_live(&live);
        assert!(state.folders.is_empty());
        assert_eq!(state.notes.keys().collect::<Vec<_>>(), ["a"]);
        assert_eq!(
            plan(&[a, b], &[], &state),
            [
                Step::CreateFolder {
                    path: "Talks".into()
                },
                Step::Upload {
                    id: "Talks/b".into()
                },
            ]
        );
    }

    #[test]
    fn a_new_remote_folder_starts_over() {
        let mut state = SyncState::default();
        state.retarget("root-1");
        synced(&mut state, &note("a", "# A\n"), "file-a");
        state.retarget("root-1");
        assert_eq!(state.notes.len(), 1);
        state.retarget("root-2");
        assert!(state.notes.is_empty());
        assert_eq!(state.parent_id(""), Some("root-2"));
    }
}
