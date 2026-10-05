//! Tags, mentions, and other note properties, stored as frontmatter.
//!
//! A note with properties starts like this:
//!
//! ```markdown
//! ---
//! tags: design, ideas
//! mentions: alex
//! created: 2026-09-28T14:22:33Z
//! status: draft
//! ---
//! ```
//!
//! `created` is written once, in UTC. `updated` is the file's modification
//! time and is not stored here. Unknown keys are kept so hand-edited fields
//! survive a save.

use std::time::SystemTime;

use crate::time::{self, format_rfc3339, parse_rfc3339};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Properties {
    pub tags: Vec<String>,
    pub mentions: Vec<String>,
    /// When the note was first created. Absent until the note has properties
    /// worth saving, or until a file already recorded it.
    pub created: Option<SystemTime>,
    /// Other `key: value` lines, in file order.
    pub extra: Vec<(String, String)>,
}

impl Properties {
    /// User-entered fields. `created` is assigned on save and does not, by
    /// itself, keep an otherwise empty note.
    pub fn has_entries(&self) -> bool {
        !self.tags.is_empty() || !self.mentions.is_empty() || !self.extra.is_empty()
    }

    /// Adds every comma-separated tag in `raw`. A leading `#` is ignored.
    /// Returns whether the list grew.
    pub fn add_tags(&mut self, raw: &str) -> bool {
        add_tokens(&mut self.tags, raw, '#')
    }

    /// Adds every comma-separated mention in `raw`. A leading `@` is ignored.
    pub fn add_mentions(&mut self, raw: &str) -> bool {
        add_tokens(&mut self.mentions, raw, '@')
    }

    pub fn remove_tag(&mut self, tag: &str) {
        self.tags.retain(|t| !eq_ignore(t, tag));
    }

    pub fn remove_mention(&mut self, name: &str) {
        self.mentions.retain(|t| !eq_ignore(t, name));
    }

    pub fn remove_last_tag(&mut self) -> bool {
        self.tags.pop().is_some()
    }

    pub fn remove_last_mention(&mut self) -> bool {
        self.mentions.pop().is_some()
    }

    /// Starts an extra property. Rejects blank names and the reserved keys
    /// (`tags`, `mentions`, `created`, `updated`).
    pub fn add_extra(&mut self, name: &str) -> bool {
        let name = name.trim();
        if !is_property_name(name) || self.extra.iter().any(|(k, _)| eq_ignore(k, name)) {
            return false;
        }
        self.extra.push((name.to_string(), String::new()));
        true
    }

    pub fn remove_extra(&mut self, name: &str) {
        self.extra.retain(|(k, _)| !eq_ignore(k, name));
    }

    /// Frontmatter block including the closing fence, or `None` when the note
    /// has nothing to record.
    pub fn to_frontmatter(&self) -> Option<String> {
        if !self.has_entries() {
            return None;
        }
        let mut out = String::from("---\n");
        if !self.tags.is_empty() {
            out.push_str("tags: ");
            out.push_str(&self.tags.join(", "));
            out.push('\n');
        }
        if !self.mentions.is_empty() {
            out.push_str("mentions: ");
            out.push_str(&self.mentions.join(", "));
            out.push('\n');
        }
        if let Some(created) = self.created {
            out.push_str("created: ");
            out.push_str(&format_rfc3339(created));
            out.push('\n');
        }
        for (key, value) in &self.extra {
            out.push_str(key);
            out.push_str(": ");
            out.push_str(&value.replace(['\n', '\r'], " "));
            out.push('\n');
        }
        out.push_str("---\n");
        Some(out)
    }
}

/// Pulls a leading frontmatter block off `markdown`. Without a closing
/// fence the text is left untouched, so a divider (`---`) stays a divider.
pub fn split_frontmatter(markdown: &str) -> (Properties, &str) {
    let rest = markdown
        .strip_prefix("---\n")
        .or_else(|| markdown.strip_prefix("---\r\n"));
    let Some(rest) = rest else {
        return (Properties::default(), markdown);
    };

    let mut pos = 0;
    let mut content_end = None;
    for line in rest.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        if content == "---" {
            content_end = Some(pos);
            pos += line.len();
            break;
        }
        pos += line.len();
    }
    let Some(content_end) = content_end else {
        return (Properties::default(), markdown);
    };
    (parse_block(&rest[..content_end]), &rest[pos..])
}

fn parse_block(block: &str) -> Properties {
    let mut props = Properties::default();
    for line in block.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key.to_ascii_lowercase().as_str() {
            "tags" => props.tags.extend(split_tokens(value, '#')),
            "mentions" => props.mentions.extend(split_tokens(value, '@')),
            "created" => props.created = parse_rfc3339(value),
            "updated" => {}
            _ if is_property_name(key) => {
                if let Some((_, existing)) = props.extra.iter_mut().find(|(k, _)| eq_ignore(k, key))
                {
                    *existing = value.to_string();
                } else {
                    props.extra.push((key.to_string(), value.to_string()));
                }
            }
            _ => {}
        }
    }
    dedupe(&mut props.tags);
    dedupe(&mut props.mentions);
    props
}

fn is_property_name(name: &str) -> bool {
    let len = name.chars().count();
    (1..=40).contains(&len)
        && name.trim() == name
        && !name.chars().any(|c| c.is_control() || c == ':')
        && !matches!(
            name.to_ascii_lowercase().as_str(),
            "tags" | "mentions" | "created" | "updated"
        )
}

fn add_tokens(list: &mut Vec<String>, raw: &str, marker: char) -> bool {
    let before = list.len();
    list.extend(split_tokens(raw, marker));
    dedupe(list);
    list.len() > before
}

fn split_tokens(raw: &str, marker: char) -> Vec<String> {
    raw.split(',')
        .filter_map(|part| normalize(part, marker))
        .collect()
}

fn normalize(raw: &str, marker: char) -> Option<String> {
    let mut text = raw.trim().to_string();
    if text.starts_with(marker) {
        text = text[marker.len_utf8()..].trim().to_string();
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() || text.chars().count() > 80 {
        None
    } else {
        Some(text)
    }
}

fn dedupe(list: &mut Vec<String>) {
    let mut kept: Vec<String> = Vec::new();
    for item in list.drain(..) {
        if !kept.iter().any(|k| eq_ignore(k, &item)) {
            kept.push(item);
        }
    }
    *list = kept;
}

fn eq_ignore(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Month name for a 1-based month, used when drawing dates.
pub fn month_name(month: u32) -> &'static str {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    MONTHS.get(month as usize - 1).copied().unwrap_or("")
}

/// `September 28, 2026` from UTC parts. The desktop app passes local parts.
pub fn format_date(year: i64, month: u32, day: u32) -> String {
    format!("{} {day}, {year}", month_name(month))
}

/// `2:22 PM` from an hour in `0..24`.
pub fn format_time(hour: u32, minute: u32) -> String {
    let (h, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        h => (h - 12, "PM"),
    };
    format!("{h}:{minute:02} {suffix}")
}

/// Re-export so callers can format a stored timestamp without reaching into `time`.
pub fn created_utc_parts(time: SystemTime) -> (i64, u32, u32, u32, u32, u32) {
    time::utc_parts(time)
}

/// `Sep 28`, with the year added when it isn't `current_year`.
pub fn format_short_date(year: i64, month: u32, day: u32, current_year: i64) -> String {
    let month = &month_name(month)[..3];
    if year == current_year {
        format!("{month} {day}")
    } else {
        format!("{month} {day}, {year}")
    }
}

/// A stable color slot in `0..slots` for a tag or name, so the same tag gets
/// the same color on every platform and every run (case-insensitive).
pub fn color_slot(name: &str, slots: usize) -> usize {
    // FNV-1a: tiny, and unlike std's hasher, guaranteed stable.
    let hash = name
        .to_lowercase()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    (hash % slots.max(1) as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn sample_time() -> SystemTime {
        // 2026-09-28 14:22:33 UTC
        UNIX_EPOCH + Duration::from_secs(1_790_605_353)
    }

    #[test]
    fn timestamps_roundtrip() {
        assert_eq!(format_rfc3339(sample_time()), "2026-09-28T14:22:33Z");
        assert_eq!(parse_rfc3339("2026-09-28T14:22:33Z"), Some(sample_time()));
        assert!(parse_rfc3339("2026-02-31T00:00:00Z").is_none());
        assert!(parse_rfc3339("yesterday").is_none());
    }

    #[test]
    fn parses_and_writes_frontmatter() {
        let markdown = "\
---
tags: design, #ideas, design
mentions: @alex, Sam
created: 2026-09-28T14:22:33Z
status: draft
updated: ignore me
---

# Hello
";
        let (props, body) = split_frontmatter(markdown);
        assert_eq!(props.tags, ["design", "ideas"]);
        assert_eq!(props.mentions, ["alex", "Sam"]);
        assert_eq!(props.created, Some(sample_time()));
        assert_eq!(props.extra, vec![("status".into(), "draft".into())]);
        assert_eq!(body, "\n# Hello\n");
        assert_eq!(
            props.to_frontmatter().unwrap(),
            "\
---
tags: design, ideas
mentions: alex, Sam
created: 2026-09-28T14:22:33Z
status: draft
---
"
        );
    }

    #[test]
    fn a_divider_is_not_frontmatter() {
        let (props, body) = split_frontmatter("---\n\nhello\n");
        assert!(!props.has_entries());
        assert_eq!(body, "---\n\nhello\n");
    }

    #[test]
    fn tokens_are_trimmed_deduped_and_bounded() {
        let mut props = Properties::default();
        assert!(props.add_tags("  #Design, design, ideas "));
        assert!(!props.add_tags("DESIGN"));
        assert_eq!(props.tags, ["Design", "ideas"]);
        assert!(props.add_mentions("@alex, alex"));
        assert_eq!(props.mentions, ["alex"]);
        assert!(!props.add_extra("tags"));
        assert!(!props.add_extra("  "));
        assert!(props.add_extra("Status"));
        assert!(!props.add_extra("status"));
        props.remove_tag("design");
        assert_eq!(props.tags, ["ideas"]);
    }

    #[test]
    fn date_formatting() {
        assert_eq!(format_date(2026, 9, 28), "September 28, 2026");
        assert_eq!(format_time(0, 5), "12:05 AM");
        assert_eq!(format_time(14, 22), "2:22 PM");
    }

    #[test]
    fn short_dates_drop_the_current_year() {
        assert_eq!(format_short_date(2026, 9, 28, 2026), "Sep 28");
        assert_eq!(format_short_date(2025, 1, 3, 2026), "Jan 3, 2025");
    }

    #[test]
    fn color_slots_are_stable_and_case_insensitive() {
        assert_eq!(color_slot("Design", 8), color_slot("design", 8));
        assert!(color_slot("anything", 8) < 8);
        let slots: std::collections::HashSet<_> =
            ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
                .iter()
                .map(|t| color_slot(t, 8))
                .collect();
        assert!(slots.len() > 3, "names spread across colors");
        assert_eq!(color_slot("x", 0), 0);
    }
}
