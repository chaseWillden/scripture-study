//! Browsing and full-text search over notes.

use std::time::{Duration, SystemTime};

use crate::document::Document;
use crate::inline;
use crate::store::NoteMeta;

/// A note's metadata plus its plain text, ready to search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedNote {
    pub meta: NoteMeta,
    /// Plain text (no Markdown syntax), one block per line.
    pub text: String,
}

impl IndexedNote {
    pub fn new(meta: NoteMeta, doc: &Document) -> Self {
        let mut lines: Vec<String> = doc
            .blocks
            .iter()
            .map(|b| inline::plain_text(&b.text))
            .filter(|t| !t.trim().is_empty())
            .collect();
        lines.extend(doc.properties.tags.iter().cloned());
        lines.extend(doc.properties.mentions.iter().cloned());
        lines.extend(
            doc.properties
                .extra
                .iter()
                .map(|(key, value)| format!("{key} {value}")),
        );
        Self {
            meta,
            text: lines.join("\n"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    /// The matching line of the body, if the match wasn't only in the title.
    pub snippet: Option<String>,
}

const SNIPPET_CHARS: usize = 80;

/// Notes containing every word of `query` (case-insensitive), best first:
/// title matches outrank body matches, then newer notes win ties.
/// An empty query returns every note in case-insensitive title order.
pub fn search(notes: &[IndexedNote], query: &str) -> Vec<SearchHit> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut hits: Vec<(i32, SystemTime, SearchHit)> = notes
        .iter()
        .filter_map(|note| {
            let title = note.meta.title.to_lowercase();
            let body = note.text.to_lowercase();
            let mut score = 0;
            for word in &words {
                if title.contains(word.as_str()) {
                    score += 10;
                } else if body.contains(word.as_str()) {
                    score += 1;
                } else {
                    return None;
                }
            }
            if !words.is_empty() && title.starts_with(&query.trim().to_lowercase()) {
                score += 5;
            }
            let snippet = (!words.is_empty()).then(|| snippet(note, &words)).flatten();
            Some((
                score,
                note.meta.modified,
                SearchHit {
                    id: note.meta.id.clone(),
                    title: note.meta.title.clone(),
                    snippet,
                },
            ))
        })
        .collect();
    if words.is_empty() {
        hits.sort_by(|a, b| {
            a.2.title
                .to_lowercase()
                .cmp(&b.2.title.to_lowercase())
                .then(a.2.id.cmp(&b.2.id))
        });
    } else {
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    }
    hits.into_iter().map(|(_, _, hit)| hit).collect()
}

/// The first body line (after the title line) containing a query word,
/// trimmed to a window around the match.
fn snippet(note: &IndexedNote, words: &[String]) -> Option<String> {
    note.text.lines().skip(1).find_map(|line| {
        let lower = line.to_lowercase();
        let at = words.iter().filter_map(|w| lower.find(w.as_str())).min()?;
        // Lowercasing can change byte lengths; map back via char counts.
        let at_char = lower[..at].chars().count();
        let chars: Vec<char> = line.chars().collect();
        let start = at_char.saturating_sub(24).min(chars.len());
        let end = (start + SNIPPET_CHARS).min(chars.len());
        let mut out: String = chars[start..end].iter().collect();
        out = out.trim().to_string();
        if start > 0 {
            out.insert(0, '…');
        }
        if end < chars.len() {
            out.push('…');
        }
        Some(out)
    })
}

/// Compact age like Codex's sidebar: "now", "5m", "3h", "2d", "4w", "7mo", "2y".
pub fn relative_time(then: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(then).unwrap_or(Duration::ZERO).as_secs();
    let (minute, hour, day) = (60, 3600, 86_400);
    match secs {
        s if s < minute => "now".into(),
        s if s < hour => format!("{}m", s / minute),
        s if s < day => format!("{}h", s / hour),
        s if s < 7 * day => format!("{}d", s / day),
        s if s < 30 * day => format!("{}w", s / (7 * day)),
        s if s < 365 * day => format!("{}mo", s / (30 * day)),
        s => format!("{}y", s / (365 * day)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn note(id: &str, markdown: &str, age_secs: u64) -> IndexedNote {
        let doc = Document::from_markdown(markdown);
        IndexedNote::new(
            NoteMeta {
                id: id.into(),
                title: doc.title().unwrap_or_default(),
                modified: UNIX_EPOCH + Duration::from_secs(1_000_000 - age_secs),
                created: UNIX_EPOCH,
            },
            &doc,
        )
    }

    fn ids(hits: &[SearchHit]) -> Vec<&str> {
        hits.iter().map(|h| h.id.as_str()).collect()
    }

    fn notes() -> Vec<IndexedNote> {
        vec![
            note("groceries", "# Groceries\n\n- Milk\n- **Eggs**\n", 100),
            note(
                "trip",
                "# Trip plan\n\nBuy eggs for the road. Pack milk too.\n",
                50,
            ),
            note("ideas", "# Ideas\n\nA notes app with a slash menu.\n", 10),
        ]
    }

    #[test]
    fn empty_query_lists_titles_alphabetically() {
        assert_eq!(ids(&search(&notes(), "  ")), ["groceries", "ideas", "trip"]);
        assert!(search(&notes(), "").iter().all(|h| h.snippet.is_none()));
    }

    #[test]
    fn title_matches_outrank_body_matches() {
        assert_eq!(ids(&search(&notes(), "groc")), ["groceries"]);
        // "eggs" is in both bodies; the newer note wins the tie.
        assert_eq!(ids(&search(&notes(), "EGGS")), ["trip", "groceries"]);
    }

    #[test]
    fn every_word_must_match() {
        assert_eq!(ids(&search(&notes(), "trip milk")), ["trip"]);
        assert!(search(&notes(), "milk zebra").is_empty());
    }

    #[test]
    fn searches_plain_text_not_markdown() {
        assert!(search(&notes(), "**").is_empty());
        assert!(search(&notes(), "#").is_empty());
    }

    #[test]
    fn snippets_show_the_matching_line() {
        let hits = search(&notes(), "slash");
        assert_eq!(
            hits[0].snippet.as_deref(),
            Some("A notes app with a slash menu.")
        );
        // Title-only matches have no body line to show.
        assert_eq!(search(&notes(), "ideas")[0].snippet, None);
    }

    #[test]
    fn tags_and_mentions_are_searchable() {
        let tagged = note(
            "launch",
            "---\ntags: design\nmentions: alex\nstatus: draft\n---\n\n# Launch\n",
            1,
        );
        assert_eq!(
            ids(&search(std::slice::from_ref(&tagged), "design")),
            ["launch"]
        );
        assert_eq!(
            ids(&search(std::slice::from_ref(&tagged), "alex")),
            ["launch"]
        );
        assert_eq!(ids(&search(&[tagged], "draft")), ["launch"]);
    }

    #[test]
    fn long_snippets_are_windowed() {
        let long = format!("# T\n\n{} needle {}\n", "a".repeat(100), "b".repeat(100));
        let hits = search(&[note("t", &long, 0)], "needle");
        let snippet = hits[0].snippet.as_deref().unwrap();
        assert!(snippet.starts_with('…') && snippet.ends_with('…'));
        assert!(snippet.contains("needle"));
        assert!(snippet.chars().count() <= SNIPPET_CHARS + 2);
    }

    #[test]
    fn relative_times() {
        let now = UNIX_EPOCH + Duration::from_secs(100_000_000);
        let ago = |s| relative_time(now - Duration::from_secs(s), now);
        assert_eq!(ago(5), "now");
        assert_eq!(ago(300), "5m");
        assert_eq!(ago(3 * 3600), "3h");
        assert_eq!(ago(2 * 86_400), "2d");
        assert_eq!(ago(15 * 86_400), "2w");
        assert_eq!(ago(90 * 86_400), "3mo");
        assert_eq!(ago(800 * 86_400), "2y");
        assert_eq!(relative_time(now + Duration::from_secs(9), now), "now");
    }
}
