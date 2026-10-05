//! Undo and redo for a whole note.
//!
//! The editor reports the document after every frame; [`History`] notices
//! when it changed and keeps the state from before. Typing in one block is
//! grouped into one step per word (or per pause), while formatting, pasting,
//! and other commands are always steps of their own.

use crate::document::Document;
use crate::editor::Caret;

/// Typing pauses longer than this start a new undo step.
const PAUSE: f64 = 1.0;
/// Oldest steps are dropped beyond this many.
const LIMIT: usize = 200;

/// A document and where the caret was in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub doc: Document,
    pub caret: Option<Caret>,
}

/// A run of typing in one block, which undoes as a single step.
#[derive(Clone, Copy, Debug)]
struct Typing {
    block: usize,
    at: f64,
    /// The last keystroke ended a word; the next one starts a new step.
    ended_word: bool,
}

#[derive(Clone, Debug)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The document as last recorded, and the caret in it.
    current: Snapshot,
    typing: Option<Typing>,
}

impl History {
    pub fn new(doc: &Document) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            current: Snapshot {
                doc: doc.clone(),
                caret: None,
            },
            typing: None,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Records the document as it is now. `separate` marks a change that
    /// must be its own step (a command, a paste) even if it looks like typing.
    pub fn record(&mut self, doc: &Document, caret: Option<Caret>, now: f64, separate: bool) {
        if *doc == self.current.doc {
            // Nothing changed; remember where the caret is, so undoing the
            // next change puts it back here.
            self.current.caret = caret;
            return;
        }
        let edit = (!separate).then(|| typed(&self.current.doc, doc)).flatten();
        let continues = match (edit, self.typing) {
            (Some(edit), Some(typing)) => {
                edit.block == typing.block
                    && now - typing.at < PAUSE
                    // After a space, the next word starts a new step.
                    && (!typing.ended_word || edit.ends_word)
            }
            _ => false,
        };
        if !continues {
            self.undo.push(self.current.clone());
            if self.undo.len() > LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.typing = edit.map(|edit| Typing {
            block: edit.block,
            at: now,
            ended_word: edit.ends_word,
        });
        self.current = Snapshot {
            doc: doc.clone(),
            caret,
        };
    }

    /// Steps back, returning the document (and caret) to show.
    pub fn undo(&mut self, doc: &Document, caret: Option<Caret>) -> Option<Snapshot> {
        self.record(doc, caret, f64::INFINITY, true);
        let previous = self.undo.pop()?;
        self.redo
            .push(std::mem::replace(&mut self.current, previous.clone()));
        self.typing = None;
        Some(previous)
    }

    /// Steps forward again after an undo.
    pub fn redo(&mut self, doc: &Document, caret: Option<Caret>) -> Option<Snapshot> {
        self.record(doc, caret, f64::INFINITY, true);
        let next = self.redo.pop()?;
        self.undo
            .push(std::mem::replace(&mut self.current, next.clone()));
        self.typing = None;
        Some(next)
    }
}

#[derive(Clone, Copy, Debug)]
struct Typed {
    block: usize,
    /// The edit inserted whitespace (the end of a word).
    ends_word: bool,
}

/// Stands in for a block index when typing is in the note's properties.
const PROPERTIES: usize = usize::MAX;

/// If `after` differs from `before` only by text typed or deleted in one
/// block (or only in the properties), where.
fn typed(before: &Document, after: &Document) -> Option<Typed> {
    if before.properties != after.properties {
        // Typing a property value changes only the properties.
        return (before.blocks == after.blocks).then_some(Typed {
            block: PROPERTIES,
            ends_word: false,
        });
    }
    if before.blocks.len() != after.blocks.len() {
        return None;
    }
    let mut changed = before
        .blocks
        .iter()
        .zip(&after.blocks)
        .enumerate()
        .filter(|(_, (a, b))| a != b);
    let (block, (a, b)) = changed.next()?;
    if changed.next().is_some() || a.kind != b.kind {
        return None;
    }
    let inserted = b.text.chars().count() == a.text.chars().count() + 1;
    let ends_word = inserted && inserted_char(&a.text, &b.text).is_some_and(char::is_whitespace);
    Some(Typed { block, ends_word })
}

/// The one character `after` has that `before` doesn't.
fn inserted_char(before: &str, after: &str) -> Option<char> {
    let prefix = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .count();
    after.chars().nth(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Block, BlockKind};

    fn doc(texts: &[&str]) -> Document {
        Document::new(texts.iter().map(|t| Block::paragraph(*t)).collect())
    }

    fn caret(char: usize) -> Option<Caret> {
        Some(Caret { block: 0, char })
    }

    /// Types `text` one character per recorded frame, 0.1s apart.
    fn type_into(h: &mut History, before: &str, text: &str, start: f64) -> String {
        let mut s = before.to_string();
        for (n, c) in text.chars().enumerate() {
            s.push(c);
            h.record(
                &doc(&[&s]),
                caret(s.chars().count()),
                start + n as f64 * 0.1,
                false,
            );
        }
        s
    }

    #[test]
    fn typing_undoes_a_word_at_a_time() {
        let mut h = History::new(&doc(&[""]));
        let now = type_into(&mut h, "", "hello world", 0.0);
        assert_eq!(now, "hello world");

        let back = h.undo(&doc(&[&now]), caret(11)).unwrap();
        assert_eq!(back.doc, doc(&["hello "]));
        assert_eq!(back.caret, caret(6));
        let back = h.undo(&back.doc, back.caret).unwrap();
        assert_eq!(back.doc, doc(&[""]));
        assert!(h.undo(&back.doc, back.caret).is_none());

        let forward = h.redo(&back.doc, back.caret).unwrap();
        assert_eq!(forward.doc, doc(&["hello "]));
        let forward = h.redo(&forward.doc, forward.caret).unwrap();
        assert_eq!(forward.doc, doc(&["hello world"]));
        assert!(!h.can_redo());
    }

    #[test]
    fn a_pause_or_another_block_starts_a_new_step() {
        let mut h = History::new(&doc(&["", ""]));
        h.record(&doc(&["a", ""]), None, 0.0, false);
        h.record(&doc(&["ab", ""]), None, 2.0, false);
        h.record(&doc(&["ab", "c"]), None, 2.1, false);
        let back = h.undo(&doc(&["ab", "c"]), None).unwrap();
        assert_eq!(back.doc, doc(&["ab", ""]));
        let back = h.undo(&back.doc, None).unwrap();
        assert_eq!(back.doc, doc(&["a", ""]));
    }

    #[test]
    fn commands_are_their_own_steps_and_new_edits_clear_redo() {
        let mut h = History::new(&doc(&["word"]));
        // Bolding looks like a one-block text edit, but is marked separate.
        h.record(&doc(&["**word**"]), caret(2), 0.0, true);
        h.record(&doc(&["**word**!"]), caret(9), 0.1, false);
        let back = h.undo(&doc(&["**word**!"]), caret(9)).unwrap();
        assert_eq!(back.doc, doc(&["**word**"]));
        let back = h.undo(&back.doc, back.caret).unwrap();
        assert_eq!(back.doc, doc(&["word"]));

        // Editing after an undo drops what could have been redone.
        h.record(&doc(&["words"]), caret(5), 1.0, false);
        assert!(!h.can_redo());
    }

    #[test]
    fn typing_a_property_value_is_one_step() {
        let start = doc(&["body"]);
        let mut h = History::new(&start);
        let mut d = start.clone();
        for (n, value) in ["d", "dr", "dra", "draft"].into_iter().enumerate() {
            d.properties.extra = vec![("Status".into(), value.into())];
            h.record(&d, None, n as f64 * 0.2, false);
        }
        assert_eq!(h.undo(&d, None).unwrap().doc, start);
    }

    #[test]
    fn structural_changes_are_steps() {
        let mut h = History::new(&doc(&["ab"]));
        let split = doc(&["a", "b"]);
        h.record(&split, None, 0.0, false);
        let mut heading = split.clone();
        heading.blocks[0].kind = BlockKind::Heading(1);
        h.record(&heading, None, 0.1, false);
        assert_eq!(h.undo(&heading, None).unwrap().doc, split);
        assert_eq!(h.undo(&split, None).unwrap().doc, doc(&["ab"]));
    }
}
