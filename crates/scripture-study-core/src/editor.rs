//! UI-independent editing operations on a [`Document`].
//!
//! Positions are *character* indices (not bytes), matching what text widgets
//! report for cursors.

use crate::document::{Block, BlockKind, Document, MAX_INDENT};
use crate::inline;

/// Where the caret should go after an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Caret {
    pub block: usize,
    pub char: usize,
}

pub fn char_to_byte(text: &str, char: usize) -> usize {
    text.char_indices().nth(char).map_or(text.len(), |(b, _)| b)
}

pub fn byte_to_char(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

/// Enter: splits the block at the caret. Empty list items turn back into
/// plain text instead, which is how you leave a list (a nested item moves
/// out a level first). The new block keeps the indent.
pub fn split_block(doc: &mut Document, index: usize, start: usize, end: usize) -> Caret {
    let block = &mut doc.blocks[index];
    if block.kind.continues_on_enter() && block.text.is_empty() {
        if block.indent > 0 {
            block.indent -= 1;
            return Caret {
                block: index,
                char: 0,
            };
        }
        block.kind = BlockKind::Paragraph;
        return Caret {
            block: index,
            char: 0,
        };
    }

    let (start, end) = (
        char_to_byte(&block.text, start),
        char_to_byte(&block.text, end),
    );
    // A paragraph's new line starts as indented as the line being split.
    let tabs = if block.kind == BlockKind::Paragraph {
        "\t".repeat(leading_tabs(line_at(&block.text, start)))
    } else {
        String::new()
    };
    let original = block.text.clone();
    let mut left = original[..start].to_string();
    let tail = original[end..].trim_start_matches('\t').to_string();
    let mut right = tail.clone();
    // A code block shows its tags literally, so a split must not rewrite them.
    if !matches!(block.kind, BlockKind::Code { .. }) {
        crate::marks::repair_split(&original, start, end, &mut left, &mut right);
    }
    // Opening tags belong after the indent, and the caret belongs after those
    // tags so typing continues inside the annotation.
    let caret_byte = tabs.len() + right.len() - tail.len();
    right = format!("{tabs}{right}");
    block.text = left;

    let kind = match &block.kind {
        BlockKind::Todo { .. } => BlockKind::Todo { checked: false },
        k if k.continues_on_enter() => k.clone(),
        _ => BlockKind::Paragraph,
    };
    let indent = block.indent;
    doc.blocks
        .insert(index + 1, Block::new(kind, right).indented(indent));
    Caret {
        block: index + 1,
        char: byte_to_char(&doc.blocks[index + 1].text, caret_byte),
    }
}

/// The line (between soft line breaks) that byte `at` is on.
fn line_at(text: &str, at: usize) -> &str {
    let start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let end = text[at..].find('\n').map_or(text.len(), |n| at + n);
    &text[start..end]
}

/// How many tabs a line starts with: how far it's indented.
pub fn leading_tabs(line: &str) -> usize {
    line.chars().take_while(|&c| c == '\t').count()
}

/// Tab / Shift+Tab inside a paragraph: indents (or outdents) just the lines
/// from `start` to `end` (characters) by adding or removing a leading tab.
/// Returns the moved selection.
pub fn indent_lines(text: &mut String, start: usize, end: usize, deeper: bool) -> (usize, usize) {
    let (from, to) = (char_to_byte(text, start), char_to_byte(text, end));
    let first = text[..from].rfind('\n').map_or(0, |n| n + 1);
    // Starts of the lines the selection touches, last first so earlier
    // offsets stay valid while editing.
    let mut starts: Vec<usize> = std::iter::once(first)
        .chain(
            text[first..to]
                .match_indices('\n')
                .map(|(n, _)| first + n + 1),
        )
        .filter(|&s| s <= to)
        .collect();
    starts.reverse();
    let (mut start, mut end) = (start, end);
    for line in starts {
        let line_char = byte_to_char(text, line);
        if deeper {
            text.insert(line, '\t');
            start += usize::from(line_char <= start);
            end += 1;
        } else if text[line..].starts_with('\t') {
            text.remove(line);
            start -= usize::from(line_char < start);
            end -= usize::from(line_char < end);
        }
    }
    (start, end)
}

/// Shift+Enter: a line break that keeps the current line's indent.
/// Returns the caret after it.
pub fn soft_break(text: &mut String, start: usize, end: usize) -> usize {
    let (from, to) = (char_to_byte(text, start), char_to_byte(text, end));
    let tabs = leading_tabs(line_at(text, from));
    let insert = format!("\n{}", "\t".repeat(tabs));
    text.replace_range(from..to, &insert);
    start + 1 + tabs
}

/// Tab / Shift+Tab: moves blocks `first..=last` in or out a level.
pub fn indent_blocks(doc: &mut Document, first: usize, last: usize, deeper: bool) {
    let last = last.min(doc.blocks.len() - 1);
    for block in &mut doc.blocks[first..=last] {
        if block.kind == BlockKind::Paragraph {
            // Paragraphs indent line by line, with tabs in their text.
            let len = block.text.chars().count();
            indent_lines(&mut block.text, 0, len, deeper);
            continue;
        }
        block.indent = if deeper {
            (block.indent + 1).min(MAX_INDENT)
        } else {
            block.indent.saturating_sub(1)
        };
    }
}

/// Backspace with the caret at the very start of a block. An indented block
/// moves out a level first; styled blocks then become plain text; plain text
/// merges into the block above.
pub fn backspace_at_start(doc: &mut Document, index: usize) -> Option<Caret> {
    if doc.blocks[index].indent > 0 {
        doc.blocks[index].indent -= 1;
        return Some(Caret {
            block: index,
            char: 0,
        });
    }
    if doc.blocks[index].kind != BlockKind::Paragraph {
        doc.blocks[index].kind = BlockKind::Paragraph;
        return Some(Caret {
            block: index,
            char: 0,
        });
    }
    if index == 0 {
        return None;
    }
    if !doc.blocks[index - 1].kind.has_text() {
        doc.blocks.remove(index - 1);
        return Some(Caret {
            block: index - 1,
            char: 0,
        });
    }
    let current = doc.blocks.remove(index);
    let prev = &mut doc.blocks[index - 1];
    let char = prev.text.chars().count();
    prev.text.push_str(&current.text);
    Some(Caret {
        block: index - 1,
        char,
    })
}

/// Delete with the caret at the very end of a block: pulls the next block up.
pub fn delete_at_end(doc: &mut Document, index: usize) -> Option<Caret> {
    if index + 1 >= doc.blocks.len() {
        return None;
    }
    let char = doc.blocks[index].text.chars().count();
    let next = doc.blocks.remove(index + 1);
    doc.blocks[index].text.push_str(&next.text);
    Some(Caret { block: index, char })
}

/// If the line the caret is on starts with a Markdown block prefix
/// (`# `, `- `, `* `, `[] `, …), turns that line into its own block.
/// Leading tabs become the block's indent. Returns where the caret goes.
pub fn apply_markdown_shortcut(doc: &mut Document, index: usize, caret: usize) -> Option<Caret> {
    if doc
        .blocks
        .get(index)
        .is_none_or(|b| b.kind != BlockKind::Paragraph)
    {
        return None;
    }
    let text = doc.blocks[index].text.clone();
    let caret = caret.min(text.chars().count());
    let at = char_to_byte(&text, caret);
    let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let line_end = text[at..].find('\n').map_or(text.len(), |n| at + n);
    let line = &text[line_start..line_end];
    let tabs = leading_tabs(line);
    let spaced = &line[tabs..];
    let spaces = spaced.len() - spaced.trim_start_matches(' ').len();
    let content = &spaced[spaces..];
    let (kind, rest) = shortcut_kind(content)?;

    let marker = content.len() - rest.len();
    let rest_byte = (at - line_start)
        .saturating_sub(tabs + spaces + marker)
        .min(rest.len());
    let char_in_rest = byte_to_char(rest, rest_byte);
    let para_indent = doc.blocks[index].indent;
    let indent = (para_indent + tabs as u8).min(MAX_INDENT);
    let before = if line_start == 0 {
        ""
    } else {
        &text[..line_start - 1]
    };
    let after = if line_end < text.len() {
        &text[line_end + 1..]
    } else {
        ""
    };

    if before.is_empty() {
        let block = &mut doc.blocks[index];
        block.kind = kind;
        block.text = rest.to_string();
        block.indent = indent;
        if !after.is_empty() {
            doc.blocks
                .insert(index + 1, Block::paragraph(after).indented(para_indent));
        }
        return Some(Caret {
            block: index,
            char: char_in_rest,
        });
    }

    doc.blocks[index].text = before.to_string();
    doc.blocks
        .insert(index + 1, Block::new(kind, rest).indented(indent));
    if !after.is_empty() {
        doc.blocks
            .insert(index + 2, Block::paragraph(after).indented(para_indent));
    }
    Some(Caret {
        block: index + 1,
        char: char_in_rest,
    })
}

/// A Markdown prefix at the start of a line, and the text after it.
fn shortcut_kind(content: &str) -> Option<(BlockKind, &str)> {
    const CODE: fn() -> BlockKind = || BlockKind::Code {
        lang: String::new(),
    };
    let shortcuts: [(&str, BlockKind); 12] = [
        ("### ", BlockKind::Heading(3)),
        ("## ", BlockKind::Heading(2)),
        ("# ", BlockKind::Heading(1)),
        ("- ", BlockKind::Bullet),
        ("* ", BlockKind::Bullet),
        ("+ ", BlockKind::Bullet),
        ("1. ", BlockKind::Numbered),
        ("[] ", BlockKind::Todo { checked: false }),
        ("[ ] ", BlockKind::Todo { checked: false }),
        ("[x] ", BlockKind::Todo { checked: true }),
        ("> ", BlockKind::Quote),
        ("```", CODE()),
    ];
    for (prefix, kind) in shortcuts {
        if let Some(rest) = content.strip_prefix(prefix) {
            return Some((kind, rest));
        }
    }
    (content == "---").then_some((BlockKind::Divider, ""))
}

/// Applies a block-kind command from the slash menu to block `index`.
/// An empty block is converted in place; otherwise a new block is inserted
/// below. Returns where the caret should go.
pub fn set_block_kind(doc: &mut Document, index: usize, kind: BlockKind) -> Caret {
    let target = if doc.blocks[index].text.is_empty() {
        doc.blocks[index].kind = kind.clone();
        index
    } else {
        let indent = doc.blocks[index].indent;
        doc.blocks
            .insert(index + 1, Block::new(kind.clone(), "").indented(indent));
        index + 1
    };
    if kind == BlockKind::Divider {
        // Dividers hold no text, so keep typing in a fresh block below.
        if doc
            .blocks
            .get(target + 1)
            .is_none_or(|b| !b.text.is_empty())
        {
            doc.blocks.insert(target + 1, Block::paragraph(""));
        }
        return Caret {
            block: target + 1,
            char: 0,
        };
    }
    Caret {
        block: target,
        char: 0,
    }
}

/// An open slash menu within a block's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlashQuery {
    /// Byte offset of the `/`.
    pub start: usize,
    /// Byte offset just past the query (the caret).
    pub end: usize,
    pub query: String,
}

/// Finds a slash command being typed just before the caret: a `/` at the
/// start of the text or after whitespace, followed by the query.
pub fn slash_query(text: &str, caret_char: usize) -> Option<SlashQuery> {
    let end = char_to_byte(text, caret_char);
    let before = &text[..end];
    let start = before.rfind('/')?;
    let query = &before[start + 1..];
    let at_boundary = before[..start]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    let reasonable = !query.contains('\n') && query.chars().count() <= 32;
    (at_boundary && reasonable).then(|| SlashQuery {
        start,
        end,
        query: query.to_string(),
    })
}

/// Turns `block` into `kind` without moving its text.
///
/// A to-do keeps its checked state. A code block keeps its language.
pub fn set_kind_in_place(block: &mut Block, kind: BlockKind) {
    block.kind = match kind {
        BlockKind::Code { .. } => BlockKind::Code {
            lang: match &block.kind {
                BlockKind::Code { lang } => lang.clone(),
                _ => String::new(),
            },
        },
        BlockKind::Todo { .. } => match &block.kind {
            BlockKind::Todo { checked } => BlockKind::Todo { checked: *checked },
            _ => BlockKind::Todo { checked: false },
        },
        other => other,
    };
}

/// Toggles Markdown `marker` (`**`, `*`, `` ` ``, `~~`) around a character
/// range. An empty range styles the word under the caret, or inserts the
/// markers there when there is no word. Returns the new selection.
pub fn toggle_inline(
    text: &mut String,
    mut start: usize,
    mut end: usize,
    marker: &str,
) -> (usize, usize) {
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    if start == end {
        let byte = char_to_byte(text, start);
        if let Some((from, to)) = enclosing_marker(text, byte, marker) {
            return remove_marker(text, from, to, marker);
        }
        let (word_start, word_end) = word_bounds(text, start);
        if word_start != word_end {
            return toggle_inline(text, word_start, word_end, marker);
        }
    }

    let start_b = char_to_byte(text, start);
    let end_b = char_to_byte(text, end);
    if surrounded(text, start_b, end_b, marker) {
        return remove_marker(text, start_b - marker.len(), end_b + marker.len(), marker);
    }
    if covers_marker(text, start_b, end_b, marker) {
        return remove_marker(text, start_b, end_b, marker);
    }

    text.insert_str(end_b, marker);
    text.insert_str(start_b, marker);
    let width = marker.chars().count();
    (start + width, end + width)
}

/// Byte range of a `marker` pair that contains `byte`, including the markers.
fn enclosing_marker(text: &str, byte: usize, marker: &str) -> Option<(usize, usize)> {
    let spans = crate::inline::parse(text);
    let mut open: Option<usize> = None;
    let mut found = None;
    for span in &spans {
        if !span.marker || text.get(span.range.clone()) != Some(marker) {
            continue;
        }
        if let Some(start) = open.take() {
            let end = span.range.end;
            if start <= byte && byte < end {
                found = Some((start, end));
            }
        } else {
            open = Some(span.range.start);
        }
    }
    found
}

fn surrounded(text: &str, start: usize, end: usize, marker: &str) -> bool {
    let m = marker.len();
    if start < m || end + m > text.len() {
        return false;
    }
    if &text[start - m..start] != marker || &text[end..end + m] != marker {
        return false;
    }
    // A single `*` must not treat `**` as an italic wrapper.
    if m == 1 {
        let bytes = text.as_bytes();
        let ch = bytes[start - 1];
        if start >= 2 && bytes[start - 2] == ch {
            return false;
        }
        if end + 1 < bytes.len() && bytes[end + 1] == ch {
            return false;
        }
    }
    true
}

fn covers_marker(text: &str, start: usize, end: usize, marker: &str) -> bool {
    let m = marker.len();
    if end < start + 2 * m {
        return false;
    }
    let slice = &text[start..end];
    if !slice.starts_with(marker) || !slice.ends_with(marker) {
        return false;
    }
    if m == 1 {
        let bytes = slice.as_bytes();
        if bytes[1] == bytes[0] || bytes[bytes.len() - 2] == bytes[bytes.len() - 1] {
            return false;
        }
    }
    true
}

fn remove_marker(text: &mut String, start: usize, end: usize, marker: &str) -> (usize, usize) {
    let m = marker.len();
    let content = text[start + m..end - m].to_string();
    text.replace_range(start..end, &content);
    let char_start = byte_to_char(text, start);
    (char_start, char_start + content.chars().count())
}

/// Character range of the word touching `char_index`, or an empty range.
fn word_bounds(text: &str, char_index: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let i = char_index.min(chars.len());
    let is_word = |c: char| c.is_alphanumeric();
    let on_word = i < chars.len() && is_word(chars[i]);
    let after_word = i > 0 && is_word(chars[i - 1]);
    if !on_word && !after_word {
        return (char_index, char_index);
    }
    let mut start = i;
    let mut end = i;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    (start, end)
}

/// Removes the `/query` text once a command is chosen, along with the space
/// before it if it ended the text. Returns the caret.
pub fn remove_slash_query(block: &mut Block, query: &SlashQuery) -> usize {
    block.text.replace_range(query.start..query.end, "");
    let mut caret = query.start;
    if caret == block.text.len() {
        block.text.truncate(block.text.trim_end().len());
        caret = block.text.len();
    }
    byte_to_char(&block.text, caret)
}

/// Keeps the caret out of hidden link markup (`[` and `](url)`, or a
/// citation's `[^` and `]`), whose
/// positions all look the same on screen. `prev` is where the caret was;
/// a one-character step past hidden markup skips it, so every arrow press
/// visibly moves. The caret rests before `[` and after `)`, so typing at
/// either edge of a link adds plain text rather than extending it.
pub fn skip_link_markup(text: &str, prev: usize, caret: usize) -> usize {
    let at = char_to_byte(text, caret);
    let step = caret.abs_diff(prev) == 1;
    let forward = caret > prev;
    let char = |byte| byte_to_char(text, byte);
    for [open, close] in inline::hidden_pairs(text) {
        if at > open.start && at <= open.end {
            return match (step, forward) {
                (true, true) => (char(open.end) + 1).min(char(close.start)),
                _ => char(open.start),
            };
        }
        if at >= close.start && at < close.end {
            return match (step, forward) {
                (true, false) => char(close.start).saturating_sub(1).max(char(open.end)),
                _ => char(close.end),
            };
        }
    }
    caret
}

/// [`skip_link_markup`], plus the tabs that indent a paragraph's lines,
/// which are hidden too: the caret rests after them, where the line's text
/// starts, and an arrow step skips over them.
pub fn skip_hidden(text: &str, prev: usize, caret: usize, indent_tabs: bool) -> usize {
    let caret = skip_link_markup(text, prev, caret);
    if !indent_tabs {
        return caret;
    }
    let at = char_to_byte(text, caret);
    let line = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let tabs = leading_tabs(&text[line..]);
    let run_end = line + tabs; // All tabs, so bytes and characters agree.
    if tabs == 0 || at >= run_end {
        return caret;
    }
    let (line_char, end_char) = (byte_to_char(text, line), byte_to_char(text, run_end));
    let backward_step = caret.abs_diff(prev) == 1 && caret < prev;
    if backward_step && line_char > 0 {
        line_char - 1 // To the end of the line above.
    } else {
        end_char
    }
}

/// Backspace right after a link (or Delete right before one) removes the
/// label's last (or first) visible character instead of hidden markup. A
/// link whose label would become empty is removed. Returns the new caret.
pub fn delete_at_link_edge(text: &mut String, caret: usize, forward: bool) -> Option<usize> {
    let at = char_to_byte(text, caret);
    let link = inline::links(text).into_iter().find(|l| {
        l.full != l.range
            && if forward {
                l.full.start == at
            } else {
                l.full.end == at
            }
    })?;
    let label = &text[link.range.clone()];
    if label.chars().count() <= 1 {
        let start = byte_to_char(text, link.full.start);
        text.replace_range(link.full, "");
        return Some(start);
    }
    let remove = if forward {
        let len = label.chars().next().map_or(0, char::len_utf8);
        link.range.start..link.range.start + len
    } else {
        let len = label.chars().next_back().map_or(0, char::len_utf8);
        link.range.end - len..link.range.end
    };
    text.replace_range(remove, "");
    Some(if forward { caret } else { caret - 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(blocks: Vec<Block>) -> Document {
        Document::new(blocks)
    }

    fn texts(doc: &Document) -> Vec<&str> {
        doc.blocks.iter().map(|b| b.text.as_str()).collect()
    }

    #[test]
    fn indenting_carries_over_and_backspace_or_enter_outdents() {
        let mut d = doc(vec![
            Block::paragraph("a\nb"),
            Block::new(BlockKind::Bullet, "x"),
            Block::new(BlockKind::Bullet, ""),
        ]);
        // Paragraphs indent every line with a tab; other blocks as a whole.
        indent_blocks(&mut d, 0, 2, true);
        indent_blocks(&mut d, 2, 2, true);
        assert_eq!(d.blocks[0].text, "\ta\n\tb");
        assert_eq!(d.blocks[0].indent, 0);
        assert_eq!((d.blocks[1].indent, d.blocks[2].indent), (1, 2));

        // Enter keeps a list item's indent.
        let caret = split_block(&mut d, 1, 1, 1);
        assert_eq!(d.blocks[2].indent, 1);
        assert_eq!(caret.block, 2);

        // Backspace at the start moves out before merging.
        assert!(backspace_at_start(&mut d, 2).is_some());
        assert_eq!(d.blocks[2].indent, 0);
        assert_eq!(d.blocks.len(), 4);

        // Enter on an empty nested item moves it out a level.
        split_block(&mut d, 3, 0, 0);
        assert_eq!(d.blocks[3].indent, 1);
        assert_eq!(d.blocks[3].kind, BlockKind::Bullet);

        indent_blocks(&mut d, 0, 3, false);
        indent_blocks(&mut d, 0, 3, false);
        assert!(d.blocks.iter().all(|b| b.indent == 0));
        assert_eq!(d.blocks[0].text, "a\nb");
    }

    #[test]
    fn tab_indents_only_the_lines_at_the_caret() {
        let mut text = "quote\nnote".to_string();
        // Caret at the start of "note".
        assert_eq!(indent_lines(&mut text, 6, 6, true), (7, 7));
        assert_eq!(text, "quote\n\tnote");
        assert_eq!(indent_lines(&mut text, 8, 8, true), (9, 9));
        assert_eq!(text, "quote\n\t\tnote");
        assert_eq!(indent_lines(&mut text, 9, 9, false), (8, 8));
        assert_eq!(text, "quote\n\tnote");
        // A selection over both lines indents both.
        assert_eq!(indent_lines(&mut text, 0, 11, true), (1, 13));
        assert_eq!(text, "\tquote\n\t\tnote");
    }

    #[test]
    fn line_breaks_keep_the_lines_indent() {
        let mut text = "quote\n\tnote".to_string();
        let caret = soft_break(&mut text, 11, 11);
        assert_eq!(text, "quote\n\tnote\n\t");
        assert_eq!(caret, 13);

        let mut d = doc(vec![Block::paragraph("quote\n\tnote")]);
        let caret = split_block(&mut d, 0, 11, 11);
        assert_eq!(texts(&d), ["quote\n\tnote", "\t"]);
        assert_eq!(caret, Caret { block: 1, char: 1 });
    }

    #[test]
    fn the_caret_skips_indent_tabs() {
        let text = "ab\n\t\tcd";
        // Clicking or arriving before the tabs lands where the text starts.
        assert_eq!(skip_hidden(text, 0, 3, true), 5);
        assert_eq!(skip_hidden(text, 0, 4, true), 5);
        // Right from the end of "ab" lands at the indented text.
        assert_eq!(skip_hidden(text, 2, 3, true), 5);
        // Left from the indented text goes up to the end of "ab".
        assert_eq!(skip_hidden(text, 5, 4, true), 2);
        // Code keeps its tabs visible.
        assert_eq!(skip_hidden(text, 0, 4, false), 4);
    }

    #[test]
    fn arrows_skip_hidden_link_markup() {
        // "a " then "[go](u)" then " z": `[` at 2, label 3..5, `](u)` 5..9.
        let text = "a [go](u) z";
        // Right from before `[` lands after the first label character.
        assert_eq!(skip_link_markup(text, 2, 3), 4);
        // Right from the label's last character jumps past `)`.
        assert_eq!(skip_link_markup(text, 4, 5), 9);
        // Left from after `)` lands before the label's last character.
        assert_eq!(skip_link_markup(text, 9, 8), 4);
        // Left from after the first label character rests before `[`.
        assert_eq!(skip_link_markup(text, 4, 3), 2);
        // Clicks (jumps) land on the outside edges.
        assert_eq!(skip_link_markup(text, 0, 3), 2);
        assert_eq!(skip_link_markup(text, 11, 5), 9);
        // Positions outside markup and bare URLs are left alone.
        assert_eq!(skip_link_markup(text, 0, 4), 4);
        assert_eq!(skip_link_markup("see https://a.io", 3, 6), 6);
    }

    #[test]
    fn deleting_at_a_link_edge_edits_its_label() {
        let mut text = "a [go](u) z".to_string();
        assert_eq!(delete_at_link_edge(&mut text, 9, false), Some(8));
        assert_eq!(text, "a [g](u) z");
        assert_eq!(delete_at_link_edge(&mut text, 8, false), Some(2));
        assert_eq!(text, "a  z", "an emptied link goes away");

        let mut text = "[go](u)".to_string();
        assert_eq!(delete_at_link_edge(&mut text, 0, true), Some(0));
        assert_eq!(text, "[o](u)");
        assert_eq!(delete_at_link_edge(&mut text, 3, true), None);
    }

    #[test]
    fn split_in_middle_of_paragraph() {
        let mut d = doc(vec![Block::paragraph("héllo world")]);
        let caret = split_block(&mut d, 0, 5, 5);
        assert_eq!(texts(&d), ["héllo", " world"]);
        assert_eq!(caret, Caret { block: 1, char: 0 });
    }

    #[test]
    fn split_replaces_selection() {
        let mut d = doc(vec![Block::paragraph("abcdef")]);
        split_block(&mut d, 0, 2, 4);
        assert_eq!(texts(&d), ["ab", "ef"]);
    }

    #[test]
    fn split_continues_lists_but_not_headings() {
        let mut d = doc(vec![
            Block::new(BlockKind::Todo { checked: true }, "done"),
            Block::new(BlockKind::Heading(1), "Title"),
        ]);
        split_block(&mut d, 0, 4, 4);
        assert_eq!(d.blocks[1].kind, BlockKind::Todo { checked: false });
        split_block(&mut d, 2, 5, 5);
        assert_eq!(d.blocks[3].kind, BlockKind::Paragraph);
    }

    #[test]
    fn enter_on_empty_list_item_exits_list() {
        let mut d = doc(vec![Block::new(BlockKind::Bullet, "")]);
        split_block(&mut d, 0, 0, 0);
        assert_eq!(d.blocks, vec![Block::paragraph("")]);
    }

    #[test]
    fn backspace_unstyles_then_merges() {
        let mut d = doc(vec![
            Block::paragraph("ab"),
            Block::new(BlockKind::Bullet, "cd"),
        ]);
        assert_eq!(
            backspace_at_start(&mut d, 1),
            Some(Caret { block: 1, char: 0 })
        );
        assert_eq!(d.blocks[1].kind, BlockKind::Paragraph);
        assert_eq!(
            backspace_at_start(&mut d, 1),
            Some(Caret { block: 0, char: 2 })
        );
        assert_eq!(texts(&d), ["abcd"]);
        assert_eq!(backspace_at_start(&mut d, 0), None);
    }

    #[test]
    fn backspace_removes_divider_above() {
        let mut d = doc(vec![
            Block::new(BlockKind::Divider, ""),
            Block::paragraph("x"),
        ]);
        backspace_at_start(&mut d, 1);
        assert_eq!(d.blocks, vec![Block::paragraph("x")]);
    }

    #[test]
    fn delete_pulls_next_block_up() {
        let mut d = doc(vec![Block::paragraph("ab"), Block::paragraph("cd")]);
        assert_eq!(delete_at_end(&mut d, 0), Some(Caret { block: 0, char: 2 }));
        assert_eq!(texts(&d), ["abcd"]);
        assert_eq!(delete_at_end(&mut d, 0), None);
    }

    #[test]
    fn markdown_shortcuts_convert_paragraphs() {
        let mut d = doc(vec![Block::paragraph("## Hi")]);
        assert_eq!(
            apply_markdown_shortcut(&mut d, 0, 3),
            Some(Caret { block: 0, char: 0 })
        );
        assert_eq!(d.blocks[0], Block::new(BlockKind::Heading(2), "Hi"));

        let mut d = doc(vec![Block::paragraph("[] task")]);
        assert!(apply_markdown_shortcut(&mut d, 0, 3).is_some());
        assert_eq!(d.blocks[0].kind, BlockKind::Todo { checked: false });
        assert_eq!(d.blocks[0].text, "task");

        let mut d = doc(vec![Block::paragraph("* ")]);
        assert_eq!(
            apply_markdown_shortcut(&mut d, 0, 2),
            Some(Caret { block: 0, char: 0 })
        );
        assert_eq!(d.blocks[0], Block::new(BlockKind::Bullet, ""));

        let mut d = doc(vec![Block::paragraph("- milk")]);
        assert_eq!(
            apply_markdown_shortcut(&mut d, 0, 2),
            Some(Caret { block: 0, char: 0 })
        );
        assert_eq!(d.blocks[0], Block::new(BlockKind::Bullet, "milk"));

        let mut d = doc(vec![Block::paragraph("\t+ child")]);
        assert!(apply_markdown_shortcut(&mut d, 0, 3).is_some());
        assert_eq!(
            d.blocks[0],
            Block::new(BlockKind::Bullet, "child").indented(1)
        );

        let mut d = doc(vec![Block::paragraph("Keep this\n* item\nand this")]);
        assert_eq!(
            apply_markdown_shortcut(&mut d, 0, "Keep this\n* ".chars().count()),
            Some(Caret { block: 1, char: 0 })
        );
        assert_eq!(
            d.blocks,
            vec![
                Block::paragraph("Keep this"),
                Block::new(BlockKind::Bullet, "item"),
                Block::paragraph("and this"),
            ]
        );

        let mut d = doc(vec![Block::paragraph("*italic* and - not a list")]);
        assert!(apply_markdown_shortcut(&mut d, 0, 1).is_none());

        let mut d = doc(vec![Block::paragraph("plain")]);
        assert!(apply_markdown_shortcut(&mut d, 0, 5).is_none());

        let mut d = doc(vec![Block::new(BlockKind::Bullet, "- no")]);
        assert!(apply_markdown_shortcut(&mut d, 0, 2).is_none());
    }

    #[test]
    fn set_block_kind_converts_empty_or_inserts_below() {
        let mut d = doc(vec![Block::paragraph("")]);
        assert_eq!(set_block_kind(&mut d, 0, BlockKind::Heading(1)).block, 0);
        assert_eq!(d.blocks[0].kind, BlockKind::Heading(1));

        let mut d = doc(vec![Block::paragraph("text")]);
        assert_eq!(set_block_kind(&mut d, 0, BlockKind::Quote).block, 1);
        assert_eq!(d.blocks[1], Block::new(BlockKind::Quote, ""));
    }

    #[test]
    fn divider_leaves_a_paragraph_to_type_in() {
        let mut d = doc(vec![Block::paragraph("")]);
        let caret = set_block_kind(&mut d, 0, BlockKind::Divider);
        assert_eq!(d.blocks[0].kind, BlockKind::Divider);
        assert_eq!(caret, Caret { block: 1, char: 0 });
    }

    #[test]
    fn slash_query_detection() {
        let q = slash_query("/head", 5).unwrap();
        assert_eq!((q.start, q.end, q.query.as_str()), (0, 5, "head"));
        assert_eq!(slash_query("note /to", 8).unwrap().query, "to");
        assert_eq!(slash_query("/", 1).unwrap().query, "");
        assert!(slash_query("and/or", 6).is_none());
        assert!(slash_query("/head", 0).is_none());
        assert!(slash_query("no slash", 8).is_none());
    }

    #[test]
    fn removing_slash_query_restores_text() {
        let mut b = Block::paragraph("hi /quote there");
        let q = slash_query(&b.text, 9).unwrap();
        assert_eq!(remove_slash_query(&mut b, &q), 3);
        assert_eq!(b.text, "hi  there");

        let mut b = Block::paragraph("done /new");
        let q = slash_query(&b.text, 9).unwrap();
        assert_eq!(remove_slash_query(&mut b, &q), 4);
        assert_eq!(b.text, "done");
    }

    #[test]
    fn inline_style_wraps_a_word_and_toggles_off() {
        let mut text = "hello world".to_string();
        let (start, end) = toggle_inline(&mut text, 6, 6, "**");
        assert_eq!(text, "hello **world**");
        assert_eq!((start, end), (8, 13));

        let (start, end) = toggle_inline(&mut text, start, end, "**");
        assert_eq!(text, "hello world");
        assert_eq!((start, end), (6, 11));
    }

    #[test]
    fn inline_style_wraps_a_selection_and_keeps_bold_distinct_from_italic() {
        let mut text = "abcdef".to_string();
        toggle_inline(&mut text, 2, 4, "*");
        assert_eq!(text, "ab*cd*ef");

        let mut text = "**bold**".to_string();
        let (start, end) = toggle_inline(&mut text, 2, 6, "*");
        assert_eq!(text, "***bold***");
        let (start, end) = toggle_inline(&mut text, start, end, "**");
        assert_eq!(text, "*bold*");
        assert_eq!((start, end), (1, 5));
    }

    #[test]
    fn inline_style_inserts_markers_when_there_is_no_word() {
        let mut text = "hello ".to_string();
        let (start, end) = toggle_inline(&mut text, 6, 6, "`");
        assert_eq!(text, "hello ``");
        assert_eq!((start, end), (7, 7));
    }

    #[test]
    fn set_kind_in_place_keeps_text_and_todo_state() {
        let mut block = Block::new(BlockKind::Todo { checked: true }, "pay rent");
        set_kind_in_place(&mut block, BlockKind::Bullet);
        assert_eq!(block, Block::new(BlockKind::Bullet, "pay rent"));

        let mut block = Block::new(BlockKind::Todo { checked: true }, "pay rent");
        set_kind_in_place(&mut block, BlockKind::Todo { checked: false });
        assert_eq!(block.kind, BlockKind::Todo { checked: true });

        let mut block = Block::new(
            BlockKind::Code {
                lang: "rust".into(),
            },
            "fn main() {}",
        );
        set_kind_in_place(
            &mut block,
            BlockKind::Code {
                lang: String::new(),
            },
        );
        assert_eq!(
            block.kind,
            BlockKind::Code {
                lang: "rust".into()
            }
        );
    }
}
