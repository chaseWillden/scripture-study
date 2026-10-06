//! Applying, recoloring, and removing highlights and underlines.
//!
//! The tags live in the note's text, so saving the Markdown keeps them and
//! opening the file (or a folder of files) brings them back.

use crate::document::{Block, BlockKind, Document};
use crate::editor::{self, Caret};
use crate::inline::{self, Mark, MarkKind};
use crate::selection::Selection;

/// How a character range is annotated with one kind of mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// Nothing visible in the range.
    Empty,
    /// Every visible character has this color. `None` means no mark.
    Uniform(Option<u32>),
    Mixed,
}

/// Sets or clears `kind` on the character range `start..end`.
///
/// A collapsed range marks the annotation under the caret, or the word there
/// when there is none. `color` of `None` removes the annotation. When
/// `toggle` is set, asking for a color the range already has removes it.
/// Returns the character range of the text that was changed.
pub fn edit(
    text: &mut String,
    mut start: usize,
    mut end: usize,
    kind: MarkKind,
    color: Option<u32>,
    toggle: bool,
) -> (usize, usize) {
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    if start == end {
        let at = char_to_byte(text, start);
        if let Some(mark) = innermost_at(text, kind, at) {
            let from = byte_to_char(text, mark.content.start);
            let to = byte_to_char(text, mark.content.end);
            let width = to - from;
            if color == Some(mark.color) && toggle || color.is_none() {
                let content = text[mark.content.clone()].to_string();
                let at = mark.full.start;
                text.replace_range(mark.full.clone(), &content);
                let from = byte_to_char(text, at);
                return (from, from + width);
            }
            if let Some(c) = color {
                if c != mark.color {
                    let open_tag = kind.open_tag(c);
                    text.replace_range(mark.full.start..mark.content.start, &open_tag);
                }
            }
            return (from, to);
        }
        let (word_start, word_end) = word_bounds(text, start);
        if word_start == word_end {
            return (start, end);
        }
        start = word_start;
        end = word_end;
    }

    let color = match color {
        Some(c) if toggle && coverage(text, start, end, kind) == Coverage::Uniform(Some(c)) => None,
        other => other,
    };
    paint(text, start, end, kind, color)
}

/// `edit` across a selection. Blocks without prose (code, pictures, rules)
/// are left alone. The selection comes back around the annotated text.
pub fn edit_selection(
    doc: &mut Document,
    sel: &Selection,
    kind: MarkKind,
    color: Option<u32>,
    toggle: bool,
) -> Selection {
    let (start, end) = sel.range();
    let color = decide(doc, sel, kind, color, toggle);

    let orig_start = start.char;
    let orig_end = end.char;
    let mut start_char = orig_start;
    let mut end_char = orig_end;
    for index in (start.block..=end.block).rev() {
        if !markable(&doc.blocks[index]) {
            continue;
        }
        let from = if index == start.block { orig_start } else { 0 };
        let to = if index == end.block {
            orig_end
        } else {
            doc.blocks[index].text.chars().count()
        };
        if from >= to {
            continue;
        }
        let (new_from, new_to) = edit(&mut doc.blocks[index].text, from, to, kind, color, false);
        if index == start.block {
            start_char = new_from;
        }
        if index == end.block {
            end_char = new_to;
        }
    }
    Selection::new(
        Caret {
            block: start.block,
            char: start_char,
        },
        Caret {
            block: end.block,
            char: end_char,
        },
    )
}

fn decide(
    doc: &Document,
    sel: &Selection,
    kind: MarkKind,
    color: Option<u32>,
    toggle: bool,
) -> Option<u32> {
    match color {
        Some(c) if toggle && shared_color(doc, sel, kind) == Some(c) => None,
        other => other,
    }
}

/// The color every visible character in the selection shares, if there is one.
pub fn shared_color(doc: &Document, sel: &Selection, kind: MarkKind) -> Option<u32> {
    let (start, end) = sel.range();
    let mut found = None;
    let mut any = false;
    for index in start.block..=end.block {
        if !markable(&doc.blocks[index]) {
            continue;
        }
        let from = if index == start.block { start.char } else { 0 };
        let to = if index == end.block {
            end.char
        } else {
            doc.blocks[index].text.chars().count()
        };
        if from >= to {
            continue;
        }
        match coverage(&doc.blocks[index].text, from, to, kind) {
            Coverage::Empty => {}
            Coverage::Uniform(Some(c)) => {
                if any && found != Some(c) {
                    return None;
                }
                found = Some(c);
                any = true;
            }
            Coverage::Uniform(None) | Coverage::Mixed => return None,
        }
    }
    found.filter(|_| any)
}

fn markable(block: &Block) -> bool {
    block.kind.has_text() && !matches!(block.kind, BlockKind::Code { .. })
}

/// The color a collapsed caret or a selection would show as already applied.
pub fn active_color(text: &str, start: usize, end: usize, kind: MarkKind) -> Option<u32> {
    if start == end {
        let at = char_to_byte(text, start);
        if let Some(mark) = innermost_at(text, kind, at) {
            return Some(mark.color);
        }
        let (word_start, word_end) = word_bounds(text, start);
        if word_start == word_end {
            return None;
        }
        return uniform(text, word_start, word_end, kind);
    }
    uniform(text, start, end, kind)
}

fn uniform(text: &str, start: usize, end: usize, kind: MarkKind) -> Option<u32> {
    match coverage(text, start, end, kind) {
        Coverage::Uniform(color) => color,
        _ => None,
    }
}

pub fn coverage(text: &str, mut start: usize, mut end: usize, kind: MarkKind) -> Coverage {
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    let s = char_to_byte(text, start);
    let e = char_to_byte(text, end);
    if s >= e {
        return Coverage::Empty;
    }
    let mut found: Option<Option<u32>> = None;
    let mut any = false;
    for span in inline::parse(text) {
        if span.marker || span.range.end <= s || span.range.start >= e {
            continue;
        }
        let from = span.range.start.max(s);
        let to = span.range.end.min(e);
        if from >= to {
            continue;
        }
        any = true;
        let color = kind.color_of(span.style);
        match found {
            None => found = Some(color),
            Some(prev) if prev != color => return Coverage::Mixed,
            _ => {}
        }
    }
    if any {
        Coverage::Uniform(found.unwrap_or(None))
    } else {
        Coverage::Empty
    }
}

/// Close marks that the split cut open, and reopen on the next line the ones
/// whose closer is still there. `start` and `end` are byte offsets in
/// `original` (equal when the caret only splits).
pub fn repair_split(
    original: &str,
    start: usize,
    end: usize,
    left: &mut String,
    right: &mut String,
) {
    let all = inline::marks(original);
    let mut close_left = Vec::new();
    let mut open_right = Vec::new();
    for mark in &all {
        let opened_before = mark.content.start < start;
        let reaches_cut = mark.content.end > start;
        let closer_survives = mark.content.end > end;
        let opener_in_cut = mark.content.start >= start && mark.content.start < end;
        if opened_before && reaches_cut {
            close_left.push(mark);
            if closer_survives {
                open_right.push(mark);
            }
        } else if opener_in_cut && closer_survives {
            open_right.push(mark);
        }
    }
    // Innermost first, so the left gains `</u></mark>` and the right gains
    // `<mark><u>` (each opener is inserted at the front).
    close_left.sort_by_key(|m| std::cmp::Reverse(m.content.start));
    open_right.sort_by_key(|m| std::cmp::Reverse(m.content.start));
    for mark in &close_left {
        left.push_str(mark.kind.close_tag());
    }
    for mark in &open_right {
        right.insert_str(0, &mark.kind.open_tag(mark.color));
    }
}

/// Backspace or Delete at the edge of an annotation edits its text instead of
/// eating the hidden tag. Returns the new caret, in characters.
pub fn delete_at_edge(text: &mut String, caret: usize, forward: bool) -> Option<usize> {
    let at = char_to_byte(text, caret);
    let marks = inline::marks(text);
    if !forward {
        if let Some(mark) = innermost(&marks, |m| m.full.end == at) {
            return delete_inside(text, &mark, false);
        }
        if let Some(mark) = outermost(&marks, |m| m.content.start == at) {
            return delete_outside(text, mark.full.start, false);
        }
    } else {
        if let Some(mark) = innermost(&marks, |m| m.full.start == at) {
            return delete_inside(text, &mark, true);
        }
        if let Some(mark) = outermost(&marks, |m| m.content.end == at) {
            return delete_outside(text, mark.full.end, true);
        }
    }
    None
}

fn delete_inside(text: &mut String, mark: &Mark, forward: bool) -> Option<usize> {
    let visible = visible_chars(text, mark.content.clone());
    if visible.len() <= 1 {
        let start = byte_to_char(text, mark.full.start);
        text.replace_range(mark.full.clone(), "");
        return Some(start);
    }
    let range = if forward {
        visible[0].clone()
    } else {
        visible.last().unwrap().clone()
    };
    let caret = byte_to_char(text, range.start);
    text.replace_range(range, "");
    Some(caret)
}

/// Delete the next visible character before (`forward` is false) or after
/// `from`, skipping hidden tags.
fn delete_outside(text: &mut String, from: usize, forward: bool) -> Option<usize> {
    let hidden = inline::hidden_markup(text);
    let hidden_at = |i: usize| hidden.iter().any(|r| r.start <= i && i < r.end);
    if forward {
        let mut i = from;
        while i < text.len() {
            let n = text[i..].chars().next().map_or(1, char::len_utf8);
            if !hidden_at(i) {
                let caret = byte_to_char(text, i);
                text.replace_range(i..i + n, "");
                return Some(caret);
            }
            i += n;
        }
    } else {
        let mut i = from;
        while i > 0 {
            let ch = text[..i].chars().next_back()?;
            let start = i - ch.len_utf8();
            if !hidden_at(start) {
                text.replace_range(start..i, "");
                return Some(byte_to_char(text, start));
            }
            i = start;
        }
    }
    None
}

fn visible_chars(text: &str, content: std::ops::Range<usize>) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    for span in inline::parse(text) {
        if span.marker || span.range.end <= content.start || span.range.start >= content.end {
            continue;
        }
        let mut i = span.range.start.max(content.start);
        let to = span.range.end.min(content.end);
        while i < to {
            let n = text[i..].chars().next().map_or(1, char::len_utf8);
            if i + n <= to {
                out.push(i..i + n);
            }
            i += n;
        }
    }
    out
}

fn innermost_at(text: &str, kind: MarkKind, byte: usize) -> Option<Mark> {
    inline::marks(text)
        .into_iter()
        .filter(|m| m.kind == kind && m.content.start <= byte && byte < m.content.end)
        .min_by_key(|m| m.content.end - m.content.start)
}

fn innermost<'a>(marks: &'a [Mark], pred: impl Fn(&Mark) -> bool) -> Option<&'a Mark> {
    marks
        .iter()
        .filter(|m| pred(m))
        .min_by_key(|m| m.content.end - m.content.start)
}

fn outermost<'a>(marks: &'a [Mark], pred: impl Fn(&Mark) -> bool) -> Option<&'a Mark> {
    marks
        .iter()
        .filter(|m| pred(m))
        .max_by_key(|m| m.full.end - m.full.start)
}

/// Rewrite `start..end` (characters) so it wears `color`, or nothing when
/// `color` is `None`. Returns the character range of that text.
fn paint(
    text: &mut String,
    start: usize,
    end: usize,
    kind: MarkKind,
    color: Option<u32>,
) -> (usize, usize) {
    if coverage(text, start, end, kind) == Coverage::Uniform(color) && color.is_some() {
        return (start, end);
    }
    let mut cs = char_to_byte(text, start);
    let mut ce = char_to_byte(text, end);
    (cs, ce) = snap(text, cs, ce);
    (cs, ce) = absorb_edges(text, cs, ce);
    if cs >= ce {
        return (byte_to_char(text, cs), byte_to_char(text, ce));
    }
    if color.is_some() && coverage_bytes(text, cs, ce, kind) == Coverage::Uniform(color) {
        return (byte_to_char(text, cs), byte_to_char(text, ce));
    }

    for _ in 0..128 {
        let relevant: Vec<Mark> = inline::marks(text)
            .into_iter()
            .filter(|m| m.kind == kind)
            .collect();
        if let Some(mark) = relevant
            .iter()
            .filter(|m| overlaps(m, cs, ce) && (m.content.start < cs || m.content.end > ce))
            .max_by_key(|m| m.content.start)
            .cloned()
        {
            if mark.content.end > ce && mark.content.start < ce {
                let at = nudge_out(text, &mark, ce, true);
                if at != ce {
                    ce = at;
                    continue;
                }
                split_at(text, &mark, ce);
                continue;
            }
            if mark.content.start < cs && mark.content.end > cs {
                let at = nudge_out(text, &mark, cs, false);
                if at != cs {
                    cs = at;
                    continue;
                }
                let n = split_at(text, &mark, cs);
                cs += n;
                ce += n;
                continue;
            }
            break;
        }
        if let Some(mark) = relevant
            .iter()
            .filter(|m| {
                m.content.start >= cs && m.content.end <= ce && m.content.start < m.content.end
            })
            .max_by_key(|m| m.full.start)
            .cloned()
        {
            (cs, ce) = unwrap(text, &mark, cs, ce);
            continue;
        }
        break;
    }

    if let Some(c) = color {
        if cs < ce && has_visible(text, cs, ce) {
            let open = kind.open_tag(c);
            let close = kind.close_tag();
            text.insert_str(ce, close);
            text.insert_str(cs, &open);
            cs += open.len();
            ce += open.len();
        }
    }
    (cs, ce) = merge(text, kind, cs, ce);
    (byte_to_char(text, cs), byte_to_char(text, ce))
}

fn overlaps(mark: &Mark, cs: usize, ce: usize) -> bool {
    mark.content.start < ce && mark.content.end > cs
}

fn has_visible(text: &str, cs: usize, ce: usize) -> bool {
    !visible_chars(text, cs..ce).is_empty()
}

fn coverage_bytes(text: &str, cs: usize, ce: usize, kind: MarkKind) -> Coverage {
    coverage(text, byte_to_char(text, cs), byte_to_char(text, ce), kind)
}

/// Pull in the tags of an annotation whose text is already entirely in the
/// range. A mark that merely ends here (the selection stops at the end of a
/// larger annotation) stays put, so wrapping a word does not swallow the
/// outer tag.
fn absorb_edges(text: &str, mut cs: usize, mut ce: usize) -> (usize, usize) {
    for _ in 0..32 {
        let mut cs2 = cs;
        let mut ce2 = ce;
        for mark in inline::marks(text) {
            if mark.content.start >= cs && mark.content.end <= ce {
                cs2 = cs2.min(mark.full.start);
                ce2 = ce2.max(mark.full.end);
            }
        }
        if (cs2, ce2) == (cs, ce) {
            break;
        }
        cs = cs2;
        ce = ce2;
    }
    (cs, ce)
}

/// If `at` sits on a nested tag boundary, move just outside that tag so the
/// split does not separate the tag from its text.
fn nudge_out(text: &str, mark: &Mark, at: usize, end_side: bool) -> usize {
    let mut at = at;
    for _ in 0..32 {
        let nested = inline::marks(text).into_iter().find(|m| {
            m.full.start >= mark.full.start
                && m.full.end <= mark.full.end
                && m.full != mark.full
                && if end_side {
                    m.content.end == at && m.full.end > at
                } else {
                    m.content.start == at && m.full.start < at
                }
        });
        let Some(nested) = nested else {
            break;
        };
        at = if end_side {
            nested.full.end
        } else {
            nested.full.start
        };
    }
    at
}

fn snap(text: &str, mut s: usize, mut e: usize) -> (usize, usize) {
    for mark in inline::marks(text) {
        if mark.full.start < s && s < mark.content.start {
            s = mark.content.start;
        } else if mark.content.end < s && s < mark.full.end {
            s = mark.full.end;
        }
        if mark.full.start < e && e < mark.content.start {
            e = mark.full.start;
        } else if mark.content.end < e && e < mark.full.end {
            e = mark.content.end;
        }
    }
    if s > e {
        e = s;
    }
    (s, e)
}

/// Split `mark` at `at`, and any annotation nested inside it there too, so
/// the tags stay nested. Returns how many bytes were inserted.
fn split_at(text: &mut String, mark: &Mark, at: usize) -> usize {
    let mut chain: Vec<Mark> = inline::marks(text)
        .into_iter()
        .filter(|m| m.full.start >= mark.full.start && m.full.end <= mark.full.end)
        .filter(|m| m.content.start < at && at < m.content.end)
        .collect();
    // Innermost first, so closers end the inner annotation before the outer.
    chain.sort_by_key(|m| m.content.end - m.content.start);
    let close: String = chain.iter().map(|m| m.kind.close_tag()).collect();
    let open: String = chain
        .iter()
        .rev()
        .map(|m| m.kind.open_tag(m.color))
        .collect();
    let insert = format!("{close}{open}");
    text.insert_str(at, &insert);
    insert.len()
}

fn unwrap(text: &mut String, mark: &Mark, cs: usize, ce: usize) -> (usize, usize) {
    let content = text[mark.content.clone()].to_string();
    let open_len = mark.content.start - mark.full.start;
    let close_len = mark.full.end - mark.content.end;
    text.replace_range(mark.full.clone(), &content);
    let map = |p: usize| {
        if p >= mark.full.end {
            p - open_len - close_len
        } else if p >= mark.content.end {
            mark.full.start + content.len()
        } else if p >= mark.content.start {
            p - open_len
        } else if p > mark.full.start {
            mark.full.start
        } else {
            p
        }
    };
    (map(cs), map(ce))
}

fn merge(text: &mut String, kind: MarkKind, mut cs: usize, mut ce: usize) -> (usize, usize) {
    loop {
        let marks = inline::marks(text);
        let pair = marks.iter().find_map(|a| {
            marks
                .iter()
                .find(|b| {
                    a.kind == kind
                        && b.kind == kind
                        && a.color == b.color
                        && a.full.end == b.full.start
                })
                .map(|b| (a.clone(), b.clone()))
        });
        let Some((a, b)) = pair else {
            break;
        };
        let from = a.content.end;
        let to = b.content.start;
        let n = to - from;
        text.replace_range(from..to, "");
        let shift = |p: usize| if p >= to { p - n } else { p };
        cs = shift(cs);
        ce = shift(ce);
    }
    (cs, ce)
}

fn char_to_byte(text: &str, char: usize) -> usize {
    editor::char_to_byte(text, char)
}

fn byte_to_char(text: &str, byte: usize) -> usize {
    editor::byte_to_char(text, byte)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Block, Document};
    use crate::inline::{DEFAULT_HIGHLIGHT, DEFAULT_UNDERLINE};

    fn chars(text: &str, needle: &str) -> (usize, usize) {
        let start = text.find(needle).unwrap();
        let start_c = text[..start].chars().count();
        (start_c, start_c + needle.chars().count())
    }

    #[test]
    fn highlighting_a_word_and_recoloring_it() {
        let mut text = "remember this".to_string();
        let (start, end) = edit(
            &mut text,
            0,
            8,
            MarkKind::Highlight,
            Some(DEFAULT_HIGHLIGHT),
            true,
        );
        assert_eq!(text, "<mark #FFE08A>remember</mark> this");
        assert_eq!(
            &text[editor::char_to_byte(&text, start)..editor::char_to_byte(&text, end)],
            "remember"
        );

        let (start, end) = chars(&text, "remember");
        edit(
            &mut text,
            start,
            end,
            MarkKind::Highlight,
            Some(0x8ED6A8),
            true,
        );
        assert_eq!(text, "<mark #8ED6A8>remember</mark> this");

        let (start, end) = chars(&text, "remember");
        edit(
            &mut text,
            start,
            end,
            MarkKind::Highlight,
            Some(0x8ED6A8),
            true,
        );
        assert_eq!(text, "remember this", "the same color again removes it");
    }

    #[test]
    fn underline_nests_with_highlight_and_partial_recolor_splits() {
        let mut text = "<mark #FFE08A>hello world</mark>".to_string();
        let (start, end) = chars(&text, "world");
        edit(
            &mut text,
            start,
            end,
            MarkKind::Underline,
            Some(DEFAULT_UNDERLINE),
            true,
        );
        assert_eq!(text, "<mark #FFE08A>hello <u #9EC7F5>world</u></mark>");

        let (start, end) = chars(&text, "world");
        edit(
            &mut text,
            start,
            end,
            MarkKind::Highlight,
            Some(0xF5B3CE),
            true,
        );
        assert_eq!(
            text,
            "<mark #FFE08A>hello </mark><mark #F5B3CE><u #9EC7F5>world</u></mark>"
        );

        // Half of an underlined word still splits both annotations.
        let mut text = "<mark #FFE08A>hello <u #9EC7F5>world</u></mark>".to_string();
        let (start, end) = chars(&text, "orld");
        edit(
            &mut text,
            start,
            end,
            MarkKind::Highlight,
            Some(0xF5B3CE),
            true,
        );
        assert_eq!(
            text,
            "<mark #FFE08A>hello <u #9EC7F5>w</u></mark><mark #F5B3CE><u #9EC7F5>orld</u></mark>"
        );
    }

    #[test]
    fn the_caret_marks_or_recolors_the_whole_annotation() {
        let mut text = "say <mark #FFE08A>hello</mark>".to_string();
        let at = chars(&text, "hello").0 + 2;
        edit(&mut text, at, at, MarkKind::Highlight, Some(0xC9B6F5), true);
        assert_eq!(text, "say <mark #C9B6F5>hello</mark>");

        let at = chars(&text, "hello").0;
        edit(&mut text, at, at, MarkKind::Highlight, Some(0xC9B6F5), true);
        assert_eq!(text, "say hello");
    }

    #[test]
    fn a_new_word_at_the_caret_is_marked() {
        let mut text = "hello world".to_string();
        edit(
            &mut text,
            6,
            6,
            MarkKind::Underline,
            Some(DEFAULT_UNDERLINE),
            true,
        );
        assert_eq!(text, "hello <u #9EC7F5>world</u>");
    }

    #[test]
    fn adjacent_marks_of_one_color_join() {
        let mut text = "<mark #FFE08A>hello</mark> world".to_string();
        let (start, end) = chars(&text, " world");
        edit(
            &mut text,
            start,
            end,
            MarkKind::Highlight,
            Some(DEFAULT_HIGHLIGHT),
            true,
        );
        assert_eq!(text, "<mark #FFE08A>hello world</mark>");
    }

    #[test]
    fn splitting_a_line_keeps_the_annotation_on_both_sides() {
        let mut doc = Document::new(vec![Block::paragraph("<mark #FFE08A>hello world</mark>")]);
        let at = chars(&doc.blocks[0].text, "hello world").0 + "hello".len();
        editor::split_block(&mut doc, 0, at, at);
        assert_eq!(doc.blocks[0].text, "<mark #FFE08A>hello</mark>");
        assert_eq!(doc.blocks[1].text, "<mark #FFE08A> world</mark>");
    }

    #[test]
    fn backspace_edits_the_annotation_instead_of_its_tag() {
        let mut text = "a <mark #FFE08A>go</mark> z".to_string();
        let end = chars(&text, "go").1;
        // The caret rests after the closing tag.
        let caret = byte_to_char(&text, inline::marks(&text)[0].full.end);
        assert_eq!(delete_at_edge(&mut text, caret, false), Some(end - 1));
        assert_eq!(text, "a <mark #FFE08A>g</mark> z");
        let caret = byte_to_char(&text, inline::marks(&text)[0].full.end);
        assert_eq!(delete_at_edge(&mut text, caret, false), Some(2));
        assert_eq!(text, "a  z");
    }

    #[test]
    fn backspace_before_an_annotation_deletes_the_previous_letter() {
        let mut text = "ab<mark #FFE08A>cd</mark>".to_string();
        let caret = chars(&text, "cd").0;
        assert_eq!(delete_at_edge(&mut text, caret, false), Some(1));
        assert_eq!(text, "a<mark #FFE08A>cd</mark>");
    }

    #[test]
    fn annotations_round_trip_through_markdown() {
        let src = "Faith <mark #FFE08A>hope</mark> and <u #9EC7F5>charity</u>\n";
        let doc = Document::from_markdown(src);
        assert_eq!(doc.to_markdown(), src);
        assert_eq!(
            inline::plain_text(&doc.blocks[0].text),
            "Faith hope and charity"
        );
    }

    #[test]
    fn a_selection_across_blocks_is_marked_and_toggles_off() {
        let mut doc = Document::new(vec![Block::paragraph("one"), Block::paragraph("two")]);
        let sel = Selection::all(&doc);
        let sel = edit_selection(
            &mut doc,
            &sel,
            MarkKind::Highlight,
            Some(DEFAULT_HIGHLIGHT),
            true,
        );
        assert_eq!(doc.blocks[0].text, "<mark #FFE08A>one</mark>");
        assert_eq!(doc.blocks[1].text, "<mark #FFE08A>two</mark>");
        edit_selection(
            &mut doc,
            &sel,
            MarkKind::Highlight,
            Some(DEFAULT_HIGHLIGHT),
            true,
        );
        assert_eq!(doc.blocks[0].text, "one");
        assert_eq!(doc.blocks[1].text, "two");
    }

    #[test]
    fn find_still_lands_on_the_word_inside_a_mark() {
        let doc = Document::from_markdown("See <mark #FFE08A>hope</mark> here\n");
        let hits = crate::find::find(&doc, "hope", false);
        assert_eq!(hits.len(), 1);
        let text = &doc.blocks[0].text;
        let start = editor::char_to_byte(text, hits[0].start);
        let end = editor::char_to_byte(text, hits[0].end);
        assert_eq!(&text[start..end], "hope");
    }
}
