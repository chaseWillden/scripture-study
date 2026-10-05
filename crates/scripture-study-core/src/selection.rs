//! Selections that span blocks: select all, Shift+arrows across paragraphs,
//! and mouse drags over several lines.
//!
//! Positions are [`Caret`]s (block index + character index). A divider has no
//! text, so its only position is character 0.

use crate::document::{Block, BlockKind, Document};
use crate::editor::{char_to_byte, Caret};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Where the selection started; stays put while extending.
    pub anchor: Caret,
    /// The end that moves (under the pointer, or with Shift+arrows).
    pub head: Caret,
}

fn len(block: &Block) -> usize {
    block.text.chars().count()
}

impl Selection {
    pub fn new(anchor: Caret, head: Caret) -> Self {
        Self { anchor, head }
    }

    /// The whole document.
    pub fn all(doc: &Document) -> Self {
        let last = doc.blocks.len() - 1;
        Self::new(
            Caret { block: 0, char: 0 },
            Caret {
                block: last,
                char: len(&doc.blocks[last]),
            },
        )
    }

    /// Start and end, in document order.
    pub fn range(&self) -> (Caret, Caret) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn spans_blocks(&self) -> bool {
        self.anchor.block != self.head.block
    }

    /// The selected character range within block `index`, if any of it is
    /// selected. The flag is true when the selection continues past the
    /// block's end (so a renderer can show the line break as selected).
    pub fn in_block(&self, doc: &Document, index: usize) -> Option<(usize, usize, bool)> {
        let (start, end) = self.range();
        if index < start.block || index > end.block || self.is_empty() {
            return None;
        }
        let from = if index == start.block { start.char } else { 0 };
        let (to, continues) = if index == end.block {
            (end.char, false)
        } else {
            (len(&doc.blocks[index]), true)
        };
        Some((from, to, continues))
    }
}

/// The selected blocks, trimmed to the selection.
fn slice(doc: &Document, selection: &Selection) -> Vec<Block> {
    let (start, end) = selection.range();
    (start.block..=end.block)
        .map(|i| {
            let block = &doc.blocks[i];
            let (from, to, _) = selection.in_block(doc, i).unwrap_or((0, 0, false));
            let text = &block.text[char_to_byte(&block.text, from)..char_to_byte(&block.text, to)];
            Block::new(block.kind.clone(), text)
        })
        .collect()
}

/// The selection as Markdown, for the clipboard. Within one block this is
/// just the raw text (inline syntax included).
pub fn to_markdown(doc: &Document, selection: &Selection) -> String {
    let blocks = slice(doc, selection);
    if !selection.spans_blocks() {
        return blocks
            .into_iter()
            .next()
            .map(|b| b.text)
            .unwrap_or_default();
    }
    Document::new(blocks).to_markdown().trim_end().to_string()
}

/// Deletes the selected text, joining the first and last blocks. Returns the
/// caret, which lands where the selection started.
pub fn delete(doc: &mut Document, selection: &Selection) -> Caret {
    if *selection == Selection::all(doc) || selection.range() == Selection::all(doc).range() {
        // Keep the note's properties; only the content goes.
        doc.blocks = vec![Block::paragraph("")];
        return Caret { block: 0, char: 0 };
    }
    let (start, end) = selection.range();
    let last = &doc.blocks[end.block];
    let tail = last.text[char_to_byte(&last.text, end.char)..].to_string();
    let tail_kind = last.kind.clone();

    let first = &mut doc.blocks[start.block];
    first.text.truncate(char_to_byte(&first.text, start.char));
    first.text.push_str(&tail);
    if !first.kind.has_text() {
        // A divider can't hold the joined text.
        first.kind = if tail_kind.has_text() {
            tail_kind
        } else {
            BlockKind::Paragraph
        };
    }
    doc.blocks.drain(start.block + 1..=end.block);
    doc.ensure_not_empty();
    start
}

/// Inserts `text` at `caret`. Multi-paragraph text (e.g. a paste) becomes
/// blocks; its first and last paragraphs join the text around the caret.
pub fn insert(doc: &mut Document, caret: Caret, text: &str) -> Caret {
    let block = &mut doc.blocks[caret.block];
    if !block.kind.has_text() {
        block.kind = BlockKind::Paragraph;
    }
    let at = char_to_byte(&block.text, caret.char);
    if !text.contains('\n') || matches!(block.kind, BlockKind::Code { .. }) {
        block.text.insert_str(at, text);
        return Caret {
            block: caret.block,
            char: caret.char + text.chars().count(),
        };
    }

    let tail = block.text.split_off(at);
    let mut pasted = Document::from_markdown(text).blocks.into_iter();
    let first = pasted.next().unwrap_or_else(|| Block::paragraph(""));
    if block.text.is_empty() && block.kind == BlockKind::Paragraph {
        block.kind = first.kind.clone();
    }
    block.text.push_str(&first.text);

    let mut index = caret.block;
    for next in pasted {
        index += 1;
        doc.blocks.insert(index, next);
    }
    let last = &mut doc.blocks[index];
    // Parsing trims trailing spaces; keep them so words don't run together.
    let trailing = &text[text.trim_end_matches([' ', '\t']).len()..];
    if !last.text.ends_with(trailing) {
        last.text.push_str(trailing);
    }
    let char = len(last);
    last.text.push_str(&tail);
    Caret { block: index, char }
}

/// Replaces the selection with `text` (typing or pasting over a selection).
pub fn replace(doc: &mut Document, selection: &Selection, text: &str) -> Caret {
    let caret = delete(doc, selection);
    insert(doc, caret, text)
}

/// One word left or right, like Option+arrow on macOS: forward lands at the
/// end of the next word, backward at the start of the previous one. At a
/// block's edge it continues into the neighboring block.
pub fn word_step(doc: &Document, caret: Caret, forward: bool) -> Caret {
    let text: Vec<char> = doc.blocks[caret.block].text.chars().collect();
    let at_edge = if forward {
        caret.char >= text.len()
    } else {
        caret.char == 0
    };
    if at_edge {
        let crossed = step(doc, caret, forward);
        if crossed == caret {
            return caret;
        }
        // Land on the nearest word inside the neighboring block.
        let chars: Vec<char> = doc.blocks[crossed.block].text.chars().collect();
        return Caret {
            char: word_edge(&chars, crossed.char, forward),
            ..crossed
        };
    }
    Caret {
        char: word_edge(&text, caret.char, forward),
        ..caret
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

/// Skips any separators, then the word after (or before) `at`.
fn word_edge(chars: &[char], mut at: usize, forward: bool) -> usize {
    if forward {
        while at < chars.len() && !is_word_char(chars[at]) {
            at += 1;
        }
        while at < chars.len() && is_word_char(chars[at]) {
            at += 1;
        }
    } else {
        while at > 0 && !is_word_char(chars[at - 1]) {
            at -= 1;
        }
        while at > 0 && is_word_char(chars[at - 1]) {
            at -= 1;
        }
    }
    at
}

/// One character left or right, crossing into neighboring blocks.
pub fn step(doc: &Document, caret: Caret, forward: bool) -> Caret {
    let here = len(&doc.blocks[caret.block]);
    match (forward, caret.char) {
        (true, c) if c < here => Caret {
            char: c + 1,
            ..caret
        },
        (true, _) if caret.block + 1 < doc.blocks.len() => Caret {
            block: caret.block + 1,
            char: 0,
        },
        (false, c) if c > 0 => Caret {
            char: c - 1,
            ..caret
        },
        (false, _) if caret.block > 0 => Caret {
            block: caret.block - 1,
            char: len(&doc.blocks[caret.block - 1]),
        },
        _ => caret,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(block: usize, char: usize) -> Caret {
        Caret { block, char }
    }

    fn doc() -> Document {
        Document::new(vec![
            Block::new(BlockKind::Heading(1), "Title"),
            Block::paragraph("first para"),
            Block::new(BlockKind::Bullet, "item"),
            Block::paragraph("last"),
        ])
    }

    fn texts(doc: &Document) -> Vec<&str> {
        doc.blocks.iter().map(|b| b.text.as_str()).collect()
    }

    #[test]
    fn range_is_ordered_regardless_of_direction() {
        let backwards = Selection::new(c(2, 1), c(0, 3));
        assert_eq!(backwards.range(), (c(0, 3), c(2, 1)));
        assert!(backwards.spans_blocks());
    }

    #[test]
    fn per_block_ranges() {
        let d = doc();
        let sel = Selection::new(c(1, 6), c(3, 2));
        assert_eq!(sel.in_block(&d, 0), None);
        assert_eq!(sel.in_block(&d, 1), Some((6, 10, true)));
        assert_eq!(sel.in_block(&d, 2), Some((0, 4, true)));
        assert_eq!(sel.in_block(&d, 3), Some((0, 2, false)));
    }

    #[test]
    fn copies_markdown_across_blocks() {
        let d = doc();
        let sel = Selection::new(c(1, 6), c(3, 2));
        assert_eq!(to_markdown(&d, &sel), "para\n\n- item\n\nla");
        assert_eq!(
            to_markdown(&d, &Selection::all(&d)),
            d.to_markdown().trim_end()
        );
        // Inside one block it's just the text.
        assert_eq!(to_markdown(&d, &Selection::new(c(0, 0), c(0, 3))), "Tit");
    }

    #[test]
    fn deleting_joins_the_ends() {
        let mut d = doc();
        let caret = delete(&mut d, &Selection::new(c(3, 2), c(1, 6)));
        assert_eq!(caret, c(1, 6));
        assert_eq!(texts(&d), ["Title", "first st"]);
        assert_eq!(d.blocks[1].kind, BlockKind::Paragraph);
    }

    #[test]
    fn deleting_everything_leaves_a_blank_page() {
        let mut d = doc();
        d.properties = Document::from_markdown("---\ntags: [a]\n---\n\nx").properties;
        let props = d.properties.clone();
        let all = Selection::all(&d);
        assert_eq!(delete(&mut d, &all), c(0, 0));
        assert_eq!(d.blocks, Document::default().blocks);
        assert_eq!(d.properties, props, "properties survive");
    }

    #[test]
    fn deleting_from_a_divider_keeps_the_text() {
        let mut d = Document::new(vec![
            Block::new(BlockKind::Divider, ""),
            Block::new(BlockKind::Quote, "wise"),
        ]);
        delete(&mut d, &Selection::new(c(0, 0), c(1, 2)));
        assert_eq!(d.blocks, vec![Block::new(BlockKind::Quote, "se")]);
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut d = doc();
        let caret = replace(&mut d, &Selection::new(c(0, 2), c(2, 2)), "X");
        assert_eq!(texts(&d), ["TiXem", "last"]);
        assert_eq!(caret, c(0, 3));
    }

    #[test]
    fn pasting_paragraphs_makes_blocks() {
        let mut d = Document::new(vec![Block::paragraph("before after")]);
        let caret = insert(&mut d, c(0, 7), "one\n\n- two\n\nthree ");
        assert_eq!(texts(&d), ["before one", "two", "three after"]);
        assert_eq!(d.blocks[1].kind, BlockKind::Bullet);
        assert_eq!(caret, c(2, 6), "after the pasted space");
    }

    #[test]
    fn pasting_into_an_empty_line_takes_the_pasted_kind() {
        let mut d = Document::default();
        insert(&mut d, c(0, 0), "# Head\n\nbody");
        assert_eq!(
            d.blocks,
            vec![
                Block::new(BlockKind::Heading(1), "Head"),
                Block::paragraph("body")
            ]
        );
    }

    #[test]
    fn pasting_into_code_keeps_newlines() {
        let mut d = Document::new(vec![Block::new(
            BlockKind::Code {
                lang: String::new(),
            },
            "",
        )]);
        insert(&mut d, c(0, 0), "a\n\nb");
        assert_eq!(texts(&d), ["a\n\nb"]);
    }

    #[test]
    fn stepping_crosses_blocks() {
        let d = doc();
        assert_eq!(step(&d, c(0, 5), true), c(1, 0));
        assert_eq!(step(&d, c(1, 0), false), c(0, 5));
        assert_eq!(step(&d, c(0, 0), false), c(0, 0));
        assert_eq!(step(&d, c(3, 4), true), c(3, 4));
    }

    #[test]
    fn word_steps_within_a_block() {
        let d = Document::new(vec![Block::paragraph("one, two  three")]);
        assert_eq!(word_step(&d, c(0, 0), true), c(0, 3));
        assert_eq!(word_step(&d, c(0, 3), true), c(0, 8));
        assert_eq!(word_step(&d, c(0, 15), false), c(0, 10));
        assert_eq!(word_step(&d, c(0, 10), false), c(0, 5));
        assert_eq!(
            word_step(&d, c(0, 6), false),
            c(0, 5),
            "mid-word goes to its start"
        );
    }

    #[test]
    fn word_steps_cross_into_neighboring_blocks() {
        let d = doc(); // "Title", "first para", "item", "last"
        assert_eq!(word_step(&d, c(1, 0), false), c(0, 0), "start of Title");
        assert_eq!(word_step(&d, c(1, 10), true), c(2, 4), "end of item");
        assert_eq!(word_step(&d, c(0, 0), false), c(0, 0), "document start");
        assert_eq!(word_step(&d, c(3, 4), true), c(3, 4), "document end");
    }

    #[test]
    fn word_steps_handle_apostrophes_and_unicode() {
        let d = Document::new(vec![Block::paragraph("don't café")]);
        assert_eq!(word_step(&d, c(0, 0), true), c(0, 5));
        assert_eq!(word_step(&d, c(0, 5), true), c(0, 10));
    }
}
