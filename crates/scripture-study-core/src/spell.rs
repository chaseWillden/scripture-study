//! Words in a note that are worth checking against a dictionary.
//!
//! Markup is not prose: inline code, link addresses, and citation markers
//! are left out. The caller decides which of the remaining words are
//! misspelled.

use std::ops::Range;

use crate::inline;

/// Byte ranges of the words in `text` a spell checker should look at, in order.
pub fn checkable_words(text: &str) -> Vec<Range<usize>> {
    let skipped = skip_ranges(text);
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if skipped.iter().any(|r| r.contains(&i)) {
            i += utf8_len(text, i);
            continue;
        }
        let ch = text[i..].chars().next().unwrap();
        if !ch.is_alphabetic() {
            i += ch.len_utf8();
            continue;
        }
        let start = i;
        let mut end = i + ch.len_utf8();
        i = end;
        while i < text.len() && !skipped.iter().any(|r| r.contains(&i)) {
            let c = text[i..].chars().next().unwrap();
            let n = c.len_utf8();
            if c.is_alphabetic() {
                i += n;
                end = i;
                continue;
            }
            // An apostrophe or hyphen stays inside the word when a letter follows
            // ("don't", "well-known"). A trailing one does not.
            let inner = c == '\'' || c == '\u{2019}' || c == '-';
            if inner && letter_at(text, &skipped, i + n) {
                i += n;
                continue;
            }
            break;
        }
        if !touches_digit(text, start, end) {
            out.push(start..end);
        }
    }
    out
}

/// Inline code, hidden markup, citation numbers, and bare web addresses.
fn skip_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = inline::parse(text)
        .into_iter()
        .filter(|span| span.marker || span.style.code || span.style.footnote)
        .map(|span| span.range)
        .collect();
    ranges.extend(
        inline::links(text)
            .into_iter()
            .filter(|link| link.full == link.range)
            .map(|link| link.full),
    );
    ranges
}

fn letter_at(text: &str, skipped: &[Range<usize>], i: usize) -> bool {
    i < text.len()
        && !skipped.iter().any(|r| r.contains(&i))
        && text[i..].chars().next().is_some_and(|c| c.is_alphabetic())
}

/// "1st" and "mp3" are not words to look up.
fn touches_digit(text: &str, start: usize, end: usize) -> bool {
    let prev = text[..start].chars().next_back();
    let next = text[end..].chars().next();
    prev.is_some_and(|c| c.is_ascii_digit()) || next.is_some_and(|c| c.is_ascii_digit())
}

fn utf8_len(text: &str, i: usize) -> usize {
    text[i..].chars().next().map_or(1, char::len_utf8)
}

#[cfg(test)]
mod tests {
    use super::checkable_words;

    fn words(text: &str) -> Vec<&str> {
        checkable_words(text)
            .iter()
            .map(|r| &text[r.clone()])
            .collect()
    }

    #[test]
    fn splits_plain_prose() {
        assert_eq!(words("helo there"), ["helo", "there"]);
    }

    #[test]
    fn keeps_apostrophes_and_hyphens_inside_a_word() {
        assert_eq!(
            words("don't stop the well-known path"),
            ["don't", "stop", "the", "well-known", "path",]
        );
        assert_eq!(words("boys' toys"), ["boys", "toys"]);
    }

    #[test]
    fn skips_markup_code_urls_and_citations() {
        assert_eq!(words("a **helo** world"), ["a", "helo", "world"]);
        assert_eq!(words("a `helo` word"), ["a", "word"]);
        assert_eq!(words("go https://example.com/helo now"), ["go", "now"]);
        assert_eq!(
            words("read [helo](https://x.test/helo) please"),
            ["read", "helo", "please",]
        );
        assert_eq!(words("see Alma[^Ether 2:1] now"), ["see", "Alma", "now"]);
    }

    #[test]
    fn skips_numbers_stuck_to_letters() {
        assert_eq!(words("the 1st day of mp3"), ["the", "day", "of"]);
    }
}
