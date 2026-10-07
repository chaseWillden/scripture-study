//! The folder tree shown when organizing notes.

use std::cmp::Ordering;

use crate::store::{self, NoteMeta};

/// Compare leading numbers by value, then the remaining text alphabetically.
/// Comparing digit strings avoids overflow for arbitrarily long numbers.
fn compare_names(a: &str, b: &str) -> Ordering {
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    let split_number = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
    let a_end = split_number(&a);
    let b_end = split_number(&b);
    match (a_end > 0, b_end > 0) {
        (true, true) => {
            let a_number = a[..a_end].trim_start_matches('0');
            let b_number = b[..b_end].trim_start_matches('0');
            a_number
                .len()
                .cmp(&b_number.len())
                .then_with(|| a_number.cmp(b_number))
                .then_with(|| a[a_end..].cmp(&b[b_end..]))
        }
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.cmp(&b),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    /// Full path, `""` for the top level.
    pub path: String,
    pub name: String,
    /// Subfolders, sorted by name with leading numbers compared numerically.
    pub folders: Vec<Folder>,
    /// Notes sorted by title with leading numbers compared numerically.
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
        self.folders
            .sort_by(|a, b| compare_names(&a.name, &b.name).then(a.path.cmp(&b.path)));
        self.notes
            .sort_by(|a, b| compare_names(&a.title, &b.title).then(a.id.cmp(&b.id)));
        for folder in &mut self.folders {
            folder.sort();
        }
    }
}

/// Builds the tree from every folder path and every note. Notes are sorted by
/// title within each folder, comparing leading numbers numerically.
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
                note("work/c"),
                note("work/Old/b"),
                note("work/a"),
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
            "sorts note titles"
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
    fn numbered_titles_sort_numerically_then_alphabetically() {
        let titles = [
            "10 - Laman",
            "2 - Zebra",
            "1 - Destruction",
            "2 - apple",
            "1 Nephi Prophecies",
            "banana",
            "Apple",
            "02 - Banana",
        ];
        let notes: Vec<_> = titles
            .iter()
            .enumerate()
            .map(|(i, title)| {
                let mut meta = note(&format!("Prophecies/{i}"));
                meta.title = (*title).into();
                meta
            })
            .collect();
        let root = tree(&[], &notes);
        let actual: Vec<_> = root.folders[0]
            .notes
            .iter()
            .map(|n| n.title.as_str())
            .collect();
        assert_eq!(
            actual,
            [
                "1 - Destruction",
                "1 Nephi Prophecies",
                "2 - apple",
                "02 - Banana",
                "2 - Zebra",
                "10 - Laman",
                "Apple",
                "banana",
            ]
        );
    }

    #[test]
    fn numbered_folders_sort_at_every_level() {
        let root = tree(
            &[
                "10 Books/10 Zebra".into(),
                "2 Books".into(),
                "1 Books".into(),
                "10 Books/2 Zebra".into(),
                "10 Books/2 apple".into(),
                "Apple".into(),
            ],
            &[],
        );
        assert_eq!(names(&root), ["1 Books", "2 Books", "10 Books", "Apple"]);
        assert_eq!(names(&root.folders[2]), ["2 apple", "2 Zebra", "10 Zebra"]);
    }

    #[test]
    fn numeric_sort_handles_zero_large_numbers_and_equal_titles() {
        let titles = [
            "184467440737095516160 apple",
            "184467440737095516159 Zebra",
            "00 Zebra",
            "0 apple",
            "2 Apple",
            "02 apple",
            "Chapter 2",
            "Chapter 10",
            "",
        ];
        let notes: Vec<_> = titles
            .iter()
            .enumerate()
            .rev()
            .map(|(i, title)| {
                let mut meta = note(&format!("{i}"));
                meta.title = (*title).into();
                meta
            })
            .collect();
        let root = tree(&[], &notes);
        let ids: Vec<_> = root.notes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["3", "2", "4", "5", "1", "0", "8", "7", "6"]);
    }

    #[test]
    fn ancestors_of_a_path() {
        assert_eq!(ancestors("a/b/c"), ["a", "a/b", "a/b/c"]);
        assert!(ancestors("").is_empty());
    }
}
