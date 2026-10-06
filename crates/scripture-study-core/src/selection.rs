//! Selections that span blocks: select all, Shift+arrows across paragraphs,
//! and mouse drags over several lines.
//!
//! Positions are [`Caret`]s (block index + character index). A divider has no
//! text, so its only position is character 0.

use crate::citations;
use crate::document::{Block, BlockKind, Document, MAX_INDENT};
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
            Block::new(block.kind.clone(), text).indented(block.indent)
        })
        .collect()
}

/// The selection as Markdown, for the clipboard. Within one block this is
/// just the raw text (inline syntax included). Across blocks, indents are
/// relative to the shallowest selected block so a nested list pastes at the
/// level of wherever it lands. Citation superscripts bring their sources
/// along as footnote lines.
pub fn to_markdown(doc: &Document, selection: &Selection) -> String {
    let mut blocks = slice(doc, selection);
    let body = if !selection.spans_blocks() {
        blocks.first().map(|b| b.text.clone()).unwrap_or_default()
    } else {
        let base = blocks.iter().map(|b| b.indent).min().unwrap_or(0);
        if base > 0 {
            for block in &mut blocks {
                block.indent -= base;
            }
        }
        Document::new(blocks.clone())
            .to_markdown()
            .trim_end()
            .to_string()
    };
    citations::append_definitions(doc, &body, &blocks)
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
/// Footnote lines in `text` become citations on `doc`.
///
/// A list pasted into a list item is the exception. The item splits at the
/// caret, the pasted items keep their nesting and line up with this item,
/// and the text after the caret stays its own item.
pub fn insert(doc: &mut Document, caret: Caret, text: &str) -> Caret {
    let kind_has_text = doc.blocks[caret.block].kind.has_text();
    if !kind_has_text {
        doc.blocks[caret.block].kind = BlockKind::Paragraph;
    }
    let at = char_to_byte(&doc.blocks[caret.block].text, caret.char);
    if !text.contains('\n') || matches!(doc.blocks[caret.block].kind, BlockKind::Code { .. }) {
        doc.blocks[caret.block].text.insert_str(at, text);
        return citations::renumber_caret(
            doc,
            Caret {
                block: caret.block,
                char: caret.char + text.chars().count(),
            },
        );
    }

    let mut parsed = Document::from_markdown(text);
    citations::adopt(doc, &mut parsed.blocks, &parsed.citations);
    let pasted = parsed.blocks;
    let structural = pasted.len() > 1 || pasted.iter().any(|b| b.kind.continues_on_enter());
    let blank = matches!(
        pasted.as_slice(),
        [block] if block.kind == BlockKind::Paragraph && block.text.trim().is_empty()
    );
    if doc.blocks[caret.block].kind.continues_on_enter() && structural && !blank {
        let caret = insert_into_list(doc, caret, pasted);
        return citations::renumber_caret(doc, caret);
    }

    let tail = doc.blocks[caret.block].text.split_off(at);
    let mut pasted = pasted.into_iter();
    let first = pasted.next().unwrap_or_else(|| Block::paragraph(""));
    if doc.blocks[caret.block].text.is_empty()
        && doc.blocks[caret.block].kind == BlockKind::Paragraph
    {
        doc.blocks[caret.block].kind = first.kind.clone();
    }
    doc.blocks[caret.block].text.push_str(&first.text);

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
    citations::renumber_caret(doc, Caret { block: index, char })
}

/// Splits the list item at `caret` and puts `pasted` between the halves.
fn insert_into_list(doc: &mut Document, caret: Caret, mut pasted: Vec<Block>) -> Caret {
    let dest_indent = doc.blocks[caret.block].indent;
    let dest_kind = doc.blocks[caret.block].kind.clone();
    let base = pasted.iter().map(|b| b.indent).min().unwrap_or(0);
    for block in &mut pasted {
        let levels = block.indent.saturating_sub(base);
        block.indent = dest_indent.saturating_add(levels).min(MAX_INDENT);
    }

    let block = &mut doc.blocks[caret.block];
    let at = char_to_byte(&block.text, caret.char.min(block.text.chars().count()));
    let tail = block.text.split_off(at);
    let head_empty = block.text.is_empty();

    let mut index = caret.block;
    if head_empty {
        let first = pasted.remove(0);
        let slot = &mut doc.blocks[index];
        slot.kind = first.kind;
        slot.text = first.text;
        slot.indent = first.indent;
    }
    for block in pasted {
        index += 1;
        doc.blocks.insert(index, block);
    }

    let char = doc.blocks[index].text.chars().count();
    if !tail.is_empty() {
        doc.blocks
            .insert(index + 1, Block::new(dest_kind, tail).indented(dest_indent));
    }
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
    use crate::citations::Citation;

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
    fn copying_a_nested_list_keeps_relative_indents() {
        let d = Document::new(vec![
            Block::new(BlockKind::Numbered, "Prophecies"),
            Block::new(BlockKind::Numbered, "Jeremiah").indented(1),
            Block::new(BlockKind::Numbered, "Isaiah").indented(2),
            Block::new(BlockKind::Numbered, "Fulfillment"),
        ]);
        let sel = Selection::new(c(1, 0), c(2, 6));
        assert_eq!(to_markdown(&d, &sel), "1. Jeremiah\n\t1. Isaiah");
    }

    #[test]
    fn pasting_a_list_into_the_middle_of_a_list_keeps_both() {
        let mut d = Document::new(vec![
            Block::paragraph("\"...that they must\nbe destroyed,\nmany should be\""),
            Block::new(BlockKind::Numbered, "Prophecies"),
            Block::new(BlockKind::Numbered, "Jeremiah").indented(1),
            Block::new(BlockKind::Numbered, "Isaiah").indented(1),
            Block::new(BlockKind::Numbered, "Zedekiah").indented(1),
            Block::new(BlockKind::Numbered, "Fulfillment"),
        ]);
        // At the start of an item in the middle of the inner list.
        let caret = insert(
            &mut d,
            c(4, 0),
            "1. Jehoiakim\n\t1. Son of Josiah\n2. Another",
        );
        let shape: Vec<_> = d
            .blocks
            .iter()
            .map(|b| (b.kind.clone(), b.indent, b.text.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (
                    BlockKind::Paragraph,
                    0,
                    "\"...that they must\nbe destroyed,\nmany should be\""
                ),
                (BlockKind::Numbered, 0, "Prophecies"),
                (BlockKind::Numbered, 1, "Jeremiah"),
                (BlockKind::Numbered, 1, "Isaiah"),
                (BlockKind::Numbered, 1, "Jehoiakim"),
                (BlockKind::Numbered, 2, "Son of Josiah"),
                (BlockKind::Numbered, 1, "Another"),
                (BlockKind::Numbered, 1, "Zedekiah"),
                (BlockKind::Numbered, 0, "Fulfillment"),
            ]
        );
        assert_eq!(caret, c(6, "Another".len()));
        assert_eq!(d.list_number(1), 1, "Prophecies");
        assert_eq!(d.list_number(4), 3, "Jehoiakim continues the inner list");
        assert_eq!(d.list_number(5), 1, "nested item starts at 1");
        assert_eq!(d.list_number(7), 5, "Zedekiah stays an item");
        assert_eq!(d.list_number(8), 2, "Fulfillment");
    }

    #[test]
    fn pasting_into_the_middle_of_an_item_splits_it() {
        let mut d = Document::new(vec![
            Block::new(BlockKind::Bullet, "Alpha"),
            Block::new(BlockKind::Bullet, "Beta side").indented(1),
            Block::new(BlockKind::Bullet, "Gamma").indented(1),
        ]);
        let copied = "- One\n\t- Nested\n- Two";
        insert(&mut d, c(1, 4), copied);
        let shape: Vec<_> = d
            .blocks
            .iter()
            .map(|b| (b.kind.clone(), b.indent, b.text.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (BlockKind::Bullet, 0, "Alpha"),
                (BlockKind::Bullet, 1, "Beta"),
                (BlockKind::Bullet, 1, "One"),
                (BlockKind::Bullet, 2, "Nested"),
                (BlockKind::Bullet, 1, "Two"),
                (BlockKind::Bullet, 1, " side"),
                (BlockKind::Bullet, 1, "Gamma"),
            ]
        );
    }

    #[test]
    fn pasting_at_the_end_of_an_item_inserts_siblings() {
        let mut d = Document::new(vec![
            Block::new(BlockKind::Numbered, "Before"),
            Block::new(BlockKind::Numbered, "Into").indented(1),
            Block::new(BlockKind::Numbered, "After").indented(1),
        ]);
        insert(&mut d, c(1, 4), "1. Parent\n\t1. Child");
        let shape: Vec<_> = d
            .blocks
            .iter()
            .map(|b| (b.indent, b.text.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (0, "Before"),
                (1, "Into"),
                (1, "Parent"),
                (2, "Child"),
                (1, "After")
            ]
        );
        assert_eq!(d.list_number(2), 2);
        assert_eq!(d.list_number(3), 1);
        assert_eq!(d.list_number(4), 3);
    }

    #[test]
    fn cut_and_paste_a_nested_list_into_another() {
        let src = Document::new(vec![
            Block::new(BlockKind::Bullet, "Keep"),
            Block::new(BlockKind::Bullet, "Parent").indented(1),
            Block::new(BlockKind::Bullet, "Child").indented(2),
            Block::new(BlockKind::Todo { checked: false }, "Task").indented(2),
        ]);
        let sel = Selection::new(c(1, 0), c(3, 4));
        let copied = to_markdown(&src, &sel);
        let mut src = src;
        delete(&mut src, &sel);
        assert_eq!(src.blocks.len(), 2, "the cut leaves an empty item behind");
        assert_eq!(src.blocks[0].text, "Keep");

        let mut dest = Document::new(vec![
            Block::new(BlockKind::Numbered, "One"),
            Block::new(BlockKind::Numbered, "Two middle"),
            Block::new(BlockKind::Numbered, "Three"),
        ]);
        insert(&mut dest, c(1, 3), &copied);
        let shape: Vec<_> = dest
            .blocks
            .iter()
            .map(|b| (b.kind.clone(), b.indent, b.text.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (BlockKind::Numbered, 0, "One"),
                (BlockKind::Numbered, 0, "Two"),
                (BlockKind::Bullet, 0, "Parent"),
                (BlockKind::Bullet, 1, "Child"),
                (BlockKind::Todo { checked: false }, 1, "Task"),
                (BlockKind::Numbered, 0, " middle"),
                (BlockKind::Numbered, 0, "Three"),
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

    fn source(id: &str, text: &str) -> Citation {
        Citation {
            id: id.to_string(),
            text: text.to_string(),
        }
    }

    fn cited(blocks: Vec<Block>, sources: &[(&str, &str)]) -> Document {
        let mut doc = Document::new(blocks);
        doc.citations = sources.iter().map(|(id, text)| source(id, text)).collect();
        doc
    }

    fn citation_list(doc: &Document) -> Vec<(&str, &str)> {
        doc.citations
            .iter()
            .map(|c| (c.id.as_str(), c.text.as_str()))
            .collect()
    }

    #[test]
    fn copying_a_superscript_includes_its_citation() {
        let d = cited(
            vec![
                Block::new(BlockKind::Numbered, "Besieged[^1]"),
                Block::new(BlockKind::Numbered, "Carried[^2 Kings 24:15]"),
                Block::new(BlockKind::Numbered, "Untouched[^2]"),
            ],
            &[
                ("1", "Tablets."),
                ("2 Kings 24:15", "The king."),
                ("2", "Left behind."),
            ],
        );
        let copied = to_markdown(
            &d,
            &Selection::new(c(0, 0), c(1, "Carried[^2 Kings 24:15]".len())),
        );
        assert_eq!(
            copied,
            "1. Besieged[^1]\n2. Carried[^2 Kings 24:15]\n\n[^1]: Tablets.\n[^2 Kings 24:15]: The king."
        );

        // A superscript outside the selection stays behind, and so does its source.
        let only = to_markdown(&d, &Selection::new(c(0, 0), c(0, "Besieged[^1]".len())));
        assert_eq!(only, "Besieged[^1]\n\n[^1]: Tablets.");
    }

    #[test]
    fn copying_code_does_not_take_a_citation() {
        let d = cited(
            vec![Block::new(
                BlockKind::Code {
                    lang: String::new(),
                },
                "use[^1]",
            )],
            &[("1", "Nope.")],
        );
        assert_eq!(to_markdown(&d, &Selection::all(&d)), "use[^1]");
    }

    #[test]
    fn pasting_citations_onto_another_page_keeps_every_source() {
        let copied = "Besieged[^2]\n\nCarried[^2 Kings 24:15]\n\n[^2]: Tablets.\n[^2 Kings 24:15]: The king.";
        let mut dest = Document::default();
        let caret = insert(&mut dest, c(0, 0), copied);
        assert_eq!(texts(&dest), ["Besieged[^1]", "Carried[^2 Kings 24:15]"]);
        assert_eq!(
            citation_list(&dest),
            [("1", "Tablets."), ("2 Kings 24:15", "The king.")]
        );
        assert_eq!(caret, c(1, "Carried[^2 Kings 24:15]".len()));

        // A numbered superscript whose source is already cited keeps that one entry.
        let again = insert(&mut dest, c(1, caret.char), "Again[^9]\n\n[^9]: Tablets.");
        assert_eq!(
            texts(&dest),
            ["Besieged[^1]", "Carried[^2 Kings 24:15]Again[^1]"]
        );
        assert_eq!(citation_list(&dest).len(), 2, "{:?}", citation_list(&dest));
        assert_eq!(again.char, "Carried[^2 Kings 24:15]Again[^1]".len());

        // A different source that arrives as `[^1]` does not replace the one already there.
        let mut other = cited(
            vec![Block::paragraph("Already[^1].")],
            &[("1", "Dest source.")],
        );
        let caret = insert(
            &mut other,
            c(0, "Already[^1].".len()),
            "See[^1]\n\n[^1]: Tablets.",
        );
        assert_eq!(texts(&other), ["Already[^1].See[^2]"]);
        assert_eq!(
            citation_list(&other),
            [("1", "Dest source."), ("2", "Tablets.")]
        );
        assert_eq!(caret, c(0, "Already[^1].See[^2]".len()));
    }

    #[test]
    fn pasting_a_cited_list_into_a_list_keeps_the_sources() {
        let mut dest = Document::new(vec![
            Block::new(BlockKind::Numbered, "Keep"),
            Block::new(BlockKind::Numbered, "Here"),
        ]);
        insert(
            &mut dest,
            c(1, 4),
            "1. Besieged[^1]\n2. Next[^2]\n\n[^1]: Tablets.\n[^2]: Chronicle.",
        );
        let shape: Vec<_> = dest
            .blocks
            .iter()
            .map(|b| (b.kind.clone(), b.text.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (BlockKind::Numbered, "Keep"),
                (BlockKind::Numbered, "Here"),
                (BlockKind::Numbered, "Besieged[^1]"),
                (BlockKind::Numbered, "Next[^2]"),
            ]
        );
        assert_eq!(
            citation_list(&dest),
            [("1", "Tablets."), ("2", "Chronicle.")]
        );
    }
}
