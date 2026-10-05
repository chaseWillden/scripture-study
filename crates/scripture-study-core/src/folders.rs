//! The folder tree shown when organizing notes.

use crate::store::{self, NoteMeta};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    /// Full path, `""` for the top level.
    pub path: String,
    pub name: String,
    /// Subfolders, sorted by name.
    pub folders: Vec<Folder>,
    /// Notes directly in this folder, most recent first.
    pub notes: Vec<NoteMeta>,
}

impl Folder {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            name: store::name(path).to_string(),
            folders: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Notes in this folder and all its subfolders.
    pub fn note_count(&self) -> usize {
        self.notes.len() + self.folders.iter().map(Folder::note_count).sum::<usize>()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty() && self.folders.is_empty()
    }

    fn child(&mut self, path: &str) -> &mut Folder {
        let index = match self.folders.iter().position(|f| f.path == path) {
            Some(i) => i,
            None => {
                self.folders.push(Folder::new(path));
                self.folders.len() - 1
            }
        };
        &mut self.folders[index]
    }

    /// The folder at `path`, creating it and any missing parents.
    fn at(&mut self, path: &str) -> &mut Folder {
        if path.is_empty() {
            return self;
        }
        let mut folder = self;
        let mut current = String::new();
        for part in path.split('/') {
            current = store::join(&current, part);
            folder = folder.child(&current);
        }
        folder
    }

    fn sort(&mut self) {
        self.folders.sort_by_key(|f| f.name.to_lowercase());
        for folder in &mut self.folders {
            folder.sort();
        }
    }
}

/// Builds the tree from every folder path and every note. Notes keep the
/// order they're given in (the store lists newest first).
pub fn tree(folders: &[String], notes: &[NoteMeta]) -> Folder {
    let mut root = Folder::new("");
    for path in folders {
        root.at(path);
    }
    for note in notes {
        root.at(note.folder()).notes.push(note.clone());
    }
    root.sort();
    root
}

/// `path` and every folder above it, e.g. `a/b/c` → `a`, `a/b`, `a/b/c`.
pub fn ancestors(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        current = store::join(&current, part);
        out.push(current.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn note(id: &str) -> NoteMeta {
        NoteMeta {
            id: id.into(),
            title: id.into(),
            modified: SystemTime::UNIX_EPOCH,
            created: SystemTime::UNIX_EPOCH,
        }
    }

    fn names(folder: &Folder) -> Vec<&str> {
        folder.folders.iter().map(|f| f.name.as_str()).collect()
    }

    #[test]
    fn builds_nested_sorted_folders() {
        let root = tree(
            &["work".into(), "Personal".into(), "work/Old".into()],
            &[
                note("top"),
                note("work/a"),
                note("work/Old/b"),
                note("work/c"),
            ],
        );
        assert_eq!(names(&root), ["Personal", "work"]);
        assert_eq!(root.notes, [note("top")]);

        let work = &root.folders[1];
        assert_eq!(work.path, "work");
        assert_eq!(names(work), ["Old"]);
        assert_eq!(
            work.notes,
            [note("work/a"), note("work/c")],
            "keeps input order"
        );
        assert_eq!(work.folders[0].path, "work/Old");
        assert_eq!(work.note_count(), 3);
        assert!(root.folders[0].is_empty());
    }

    #[test]
    fn creates_folders_implied_by_notes() {
        let root = tree(&[], &[note("a/b/c")]);
        assert_eq!(root.folders[0].path, "a");
        assert_eq!(root.folders[0].folders[0].path, "a/b");
        assert_eq!(root.folders[0].folders[0].notes, [note("a/b/c")]);
    }

    #[test]
    fn ancestors_of_a_path() {
        assert_eq!(ancestors("a/b/c"), ["a", "a/b", "a/b/c"]);
        assert!(ancestors("").is_empty());
    }
}
