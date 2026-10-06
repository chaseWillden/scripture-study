//! Find within a note (Cmd+F): exact or fuzzy matches over the text as it
//! reads, so `**bold**` is found by typing "bold".

use crate::document::{BlockKind, Document};
use crate::inline;

/// A match in block `block`, as character positions in the block's raw
/// (Markdown) text: `start..end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub block: usize,
    pub start: usize,
    pub end: usize,
}

/// Every match of `query` in reading order. Matching ignores case. Fuzzy
/// matching also tolerates small typos: about one wrong, missing, or extra
/// character per four typed.
pub fn find(doc: &Document, query: &str, fuzzy: bool) -> Vec<Match> {
    let Some((pattern, errors)) = pattern_of(query, fuzzy) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, block) in doc.blocks.iter().enumerate() {
        let (visible, raw_of) = visible_text(&block.kind, &block.text);
        for (start, end) in matches(&visible, &pattern, errors) {
            out.push(Match {
                block: index,
                start: raw_of[start],
                end: raw_of[end - 1] + 1,
            });
        }
    }
    out
}

/// Every match of `query` in plain strings, in order. [`Match::block`] is the
/// string's index and the range is in characters. Same rules as [`find`].
pub fn find_plain(texts: &[&str], query: &str, fuzzy: bool) -> Vec<Match> {
    let Some((pattern, errors)) = pattern_of(query, fuzzy) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, text) in texts.iter().enumerate() {
        let chars: Vec<char> = text.chars().map(fold).collect();
        for (start, end) in matches(&chars, &pattern, errors) {
            out.push(Match {
                block: index,
                start,
                end,
            });
        }
    }
    out
}

fn pattern_of(query: &str, fuzzy: bool) -> Option<(Vec<char>, usize)> {
    let pattern: Vec<char> = query.trim().chars().map(fold).collect();
    if pattern.is_empty() {
        return None;
    }
    let errors = if fuzzy {
        allowed_errors(pattern.len())
    } else {
        0
    };
    Some((pattern, errors))
}

/// Typos allowed for a query of `len` characters. Very short queries stay
/// exact, or they'd match nearly everything.
pub fn allowed_errors(len: usize) -> usize {
    if len < 4 {
        0
    } else {
        (len / 4).max(1)
    }
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// The block's text without inline Markdown markers (lowercased), and for
/// each of its characters, the matching character index in the raw text.
fn visible_text(kind: &BlockKind, raw: &str) -> (Vec<char>, Vec<usize>) {
    if matches!(kind, BlockKind::Code { .. }) {
        return (
            raw.chars().map(fold).collect(),
            (0..raw.chars().count()).collect(),
        );
    }
    let mut chars = Vec::new();
    let mut raw_of = Vec::new();
    for span in inline::parse(raw).into_iter().filter(|s| !s.marker) {
        let first = raw[..span.range.start].chars().count();
        for (n, c) in raw[span.range].chars().enumerate() {
            chars.push(fold(c));
            raw_of.push(first + n);
        }
    }
    (chars, raw_of)
}

/// Non-overlapping `start..end` ranges in `text` within `errors` edits of
/// `pattern` (approximate substring matching, Sellers' algorithm).
fn matches(text: &[char], pattern: &[char], errors: usize) -> Vec<(usize, usize)> {
    let m = pattern.len();
    // cost[i]: fewest edits matching pattern[..i] to a substring ending here;
    // start[i]: where that substring starts.
    let mut cost: Vec<usize> = (0..=m).collect();
    let mut start: Vec<usize> = vec![0; m + 1];
    let mut found = Vec::new();
    // The best candidate in the current run of acceptable end positions.
    let mut best: Option<(usize, usize, usize)> = None; // (cost, start, end)

    for (j, &c) in text.iter().enumerate() {
        let (mut diag_cost, mut diag_start) = (0, j); // row 0: free start here
        cost[0] = 0;
        start[0] = j + 1;
        for i in 1..=m {
            let (up_cost, up_start) = (cost[i], start[i]);
            let substitute = diag_cost + usize::from(pattern[i - 1] != c);
            let insert = up_cost + 1; // extra character in the text
            let delete = cost[i - 1] + 1; // character missing from the text
            let (new_cost, new_start) = if substitute <= insert && substitute <= delete {
                (substitute, diag_start)
            } else if delete <= insert {
                (delete, start[i - 1])
            } else {
                (insert, up_start)
            };
            diag_cost = up_cost;
            diag_start = up_start;
            cost[i] = new_cost;
            start[i] = new_start;
        }

        if cost[m] <= errors && start[m] <= j {
            let candidate = (cost[m], start[m], j + 1);
            match best {
                // A fresh match right after the current one (e.g. "aa" twice
                // in "aaaa"): keep both.
                Some((_, begin, end)) if candidate.1 >= end => {
                    found.push((begin, end));
                    best = Some(candidate);
                }
                // Prefer fewer edits, then the longer span (whole words).
                Some((c, s, e)) if (candidate.0, e - s) < (c, candidate.2 - candidate.1) => {
                    best = Some(candidate);
                }
                None => best = Some(candidate),
                _ => {}
            }
        } else if let Some((_, s, e)) = best.take() {
            found.push((s, e));
        }
    }
    if let Some((_, s, e)) = best {
        found.push((s, e));
    }

    // Keep them apart: a later match can't reuse earlier characters.
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (s, e) in found {
        if out.last().is_none_or(|&(_, prev_end)| s >= prev_end) {
            out.push((s, e));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Block;

    fn doc() -> Document {
        Document::new(vec![
            Block::new(BlockKind::Heading(1), "Brethren"),
            Block::paragraph("The brethren shall be **inasmuch** as the teacher."),
            Block::new(
                BlockKind::Code {
                    lang: String::new(),
                },
                "let **x** = 1;",
            ),
        ])
    }

    fn texts(doc: &Document, found: &[Match]) -> Vec<String> {
        found
            .iter()
            .map(|m| {
                doc.blocks[m.block]
                    .text
                    .chars()
                    .skip(m.start)
                    .take(m.end - m.start)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn exact_matches_ignore_case_in_reading_order() {
        let d = doc();
        let found = find(&d, "BRETHREN", false);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].block, found[1].block), (0, 1));
        assert_eq!(texts(&d, &found), ["Brethren", "brethren"]);
    }

    #[test]
    fn matches_skip_markdown_but_point_at_the_raw_text() {
        let d = doc();
        let found = find(&d, "be inasmuch", false);
        assert_eq!(texts(&d, &found), ["be **inasmuch"], "covers the raw span");
        // In code, the asterisks are real characters.
        assert_eq!(texts(&d, &find(&d, "**x**", false)), ["**x**"]);
    }

    #[test]
    fn exact_mode_needs_exact_text() {
        assert!(find(&doc(), "inasmch", false).is_empty());
        assert!(find(&doc(), "   ", false).is_empty());
    }

    #[test]
    fn fuzzy_tolerates_typos() {
        let d = doc();
        assert_eq!(
            texts(&d, &find(&d, "inasmch", true)),
            ["inasmuch"],
            "missing letter"
        );
        assert_eq!(
            texts(&d, &find(&d, "teachr", true)),
            ["teacher"],
            "whole word"
        );
        assert_eq!(
            texts(&d, &find(&d, "brehtren", true)).len(),
            2,
            "swapped letters"
        );
    }

    #[test]
    fn short_fuzzy_queries_stay_exact() {
        assert_eq!(allowed_errors(3), 0);
        assert_eq!(allowed_errors(4), 1);
        assert_eq!(allowed_errors(9), 2);
        assert!(find(&doc(), "thx", true).is_empty());
    }

    #[test]
    fn plain_text_matches_like_a_note() {
        let texts = ["In the beginning God created the heaven and the earth."];
        assert!(find_plain(&texts, "begining", false).is_empty());
        assert_eq!(
            find_plain(&texts, "begining", true)
                .iter()
                .map(|m| (m.block, m.start, m.end))
                .collect::<Vec<_>>(),
            [(0, 7, 16)]
        );
        let repeated = ["aa aa"];
        assert_eq!(
            find_plain(&repeated, "aa", false)
                .iter()
                .map(|m| (m.start, m.end))
                .collect::<Vec<_>>(),
            [(0, 2), (3, 5)]
        );
    }

    #[test]
    fn repeated_matches_do_not_overlap() {
        let d = Document::new(vec![Block::paragraph("aaaa")]);
        let found = find(&d, "aa", false);
        assert_eq!(
            found.iter().map(|m| (m.start, m.end)).collect::<Vec<_>>(),
            [(0, 2), (2, 4)]
        );
    }
}
