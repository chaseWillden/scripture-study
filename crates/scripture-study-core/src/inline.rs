//! Inline Markdown: `**bold**`, `*italic*` / `_italic_`, `` `code` ``, `~~strike~~`,
//! `[links](https://…)`, and bare `https://…` URLs.
//!
//! [`parse`] splits text into styled spans that cover every byte of the input,
//! so a renderer can lay the raw text out with rich styling while the text
//! itself stays plain Markdown.

use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: bool,
    /// A citation reference's number (`[^1]`), shown raised.
    pub footnote: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// Byte range into the parsed text.
    pub range: Range<usize>,
    pub style: Style,
    /// Delimiter characters (`**`, `` ` ``, …) rather than content.
    pub marker: bool,
}

/// Splits `text` into contiguous spans covering the whole string.
pub fn parse(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    parse_range(text, 0..text.len(), Style::default(), &mut spans);
    spans
}

/// A link in inline text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// Byte range of everything that shows as the link (label or bare URL).
    pub range: Range<usize>,
    /// Byte range of the whole link, including `[`, `](url)` for a Markdown link.
    pub full: Range<usize>,
    pub url: String,
}

impl Link {
    /// The Markdown around the label (`[` and `](url)`), which is never shown.
    pub fn markup(&self) -> [Range<usize>; 2] {
        [
            self.full.start..self.range.start,
            self.range.end..self.full.end,
        ]
    }
}

/// Byte ranges of link and citation markup that stay hidden even while
/// editing.
pub fn hidden_markup(text: &str) -> Vec<Range<usize>> {
    hidden_pairs(text)
        .into_iter()
        .flatten()
        .filter(|r| !r.is_empty())
        .collect()
}

/// The hidden markup around each link label and citation number, as
/// (before, after) pairs in order: `[` and `](url)`, or `[^` and `]`.
pub fn hidden_pairs(text: &str) -> Vec<[Range<usize>; 2]> {
    let mut pairs: Vec<[Range<usize>; 2]> = links(text)
        .iter()
        .map(Link::markup)
        .filter(|[open, _]| !open.is_empty())
        .chain(crate::citations::refs(text).iter().map(|r| {
            let label = r.label();
            [r.range.start..label.start, label.end..r.range.end]
        }))
        .collect();
    pairs.sort_by_key(|[open, _]| open.start);
    pairs
}

/// Every link in `text`, in order.
pub fn links(text: &str) -> Vec<Link> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut code = false;
    while i < text.len() {
        if text[i..].starts_with('`') {
            code = !code;
        } else if !code {
            if let Some(m) = match_link(text, i, text.len()) {
                out.push(Link {
                    range: m.label,
                    full: i..m.end,
                    url: text[m.url].to_string(),
                });
                i = m.end;
                continue;
            }
            if let Some(url) = match_bare_url(text, i, text.len()) {
                out.push(Link {
                    range: url.clone(),
                    full: url.clone(),
                    url: text[url.clone()].to_string(),
                });
                i = url.end;
                continue;
            }
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// The URL of the link covering byte `at`, if any.
pub fn link_at(text: &str, at: usize) -> Option<String> {
    links(text)
        .into_iter()
        .find(|l| l.range.contains(&at))
        .map(|l| l.url)
}

struct LinkMatch {
    label: Range<usize>,
    url: Range<usize>,
    end: usize,
}

/// `[label](url)` starting at `i`. The URL may contain balanced parentheses.
fn match_link(text: &str, i: usize, end: usize) -> Option<LinkMatch> {
    let rest = &text[i..end];
    if !rest.starts_with('[') || text[..i].ends_with('!') {
        return None;
    }
    let close = rest.find("](")?;
    let label = i + 1..i + close;
    if label.is_empty() || text[label.clone()].contains(['[', ']', '\n']) {
        return None;
    }
    let url_start = i + close + 2;
    let mut depth = 0;
    for (n, c) in text[url_start..end].char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => {
                let url = url_start..url_start + n;
                if url.is_empty() {
                    return None;
                }
                return Some(LinkMatch {
                    label,
                    url,
                    end: url_start + n + 1,
                });
            }
            ')' => depth -= 1,
            c if c.is_whitespace() => return None,
            _ => {}
        }
    }
    None
}

/// A bare `http(s)://` URL starting at `i`, minus trailing punctuation.
fn match_bare_url(text: &str, i: usize, end: usize) -> Option<Range<usize>> {
    let rest = &text[i..end];
    if !(rest.starts_with("https://") || rest.starts_with("http://")) {
        return None;
    }
    if text[..i]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric())
    {
        return None;
    }
    let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let mut url = &rest[..len];
    loop {
        let trimmed =
            url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"', '*', '_', '~', '`']);
        // Keep a closing parenthesis only if the URL opened one.
        let trimmed = if trimmed.ends_with(')')
            && trimmed.matches('(').count() < trimmed.matches(')').count()
        {
            &trimmed[..trimmed.len() - 1]
        } else {
            trimmed
        };
        if trimmed.len() == url.len() {
            break;
        }
        url = trimmed;
    }
    let host = url.split_once("://").map_or("", |(_, h)| h);
    (!host.is_empty()).then(|| i..i + url.len())
}

/// The text with all inline delimiters (and citation numbers) removed.
pub fn plain_text(text: &str) -> String {
    parse(text)
        .into_iter()
        .filter(|s| !s.marker && !s.style.footnote)
        .map(|s| &text[s.range])
        .collect()
}

fn parse_range(text: &str, range: Range<usize>, style: Style, out: &mut Vec<Span>) {
    let mut plain_start = range.start;
    let mut i = range.start;

    while i < range.end {
        if !style.code && !style.link {
            if let Some(r) = crate::citations::match_ref(text, i, range.end) {
                push(out, plain_start..i, style, false);
                push(out, i..i + 2, style, true);
                let number = Style {
                    footnote: true,
                    ..style
                };
                push(out, i + 2..r.end - 1, number, false);
                push(out, r.end - 1..r.end, style, true);
                i = r.end;
                plain_start = i;
                continue;
            }
            if let Some(m) = match_link(text, i, range.end) {
                push(out, plain_start..i, style, false);
                push(out, i..i + 1, style, true);
                let inner = Style {
                    link: true,
                    ..style
                };
                parse_range(text, m.label.clone(), inner, out);
                push(out, m.label.end..m.end, style, true);
                i = m.end;
                plain_start = i;
                continue;
            }
            if let Some(url) = match_bare_url(text, i, range.end) {
                push(out, plain_start..i, style, false);
                push(
                    out,
                    url.clone(),
                    Style {
                        link: true,
                        ..style
                    },
                    false,
                );
                i = url.end;
                plain_start = i;
                continue;
            }
        }
        if let Some((delim, content, apply)) = match_delimited(text, i, range.end, style) {
            push(out, plain_start..i, style, false);
            push(out, i..i + delim, style, true);
            let inner = apply(style);
            if inner.code {
                push(out, content.clone(), inner, false);
            } else {
                parse_range(text, content.clone(), inner, out);
            }
            push(out, content.end..content.end + delim, style, true);
            i = content.end + delim;
            plain_start = i;
        } else {
            i += text[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    push(out, plain_start..range.end, style, false);
}

type Apply = fn(Style) -> Style;

/// If a delimited run starts at `i`, returns the delimiter length, the content
/// range, and how the run modifies the style.
fn match_delimited(
    text: &str,
    i: usize,
    end: usize,
    style: Style,
) -> Option<(usize, Range<usize>, Apply)> {
    let rest = &text[i..end];
    let candidates: [(&str, Apply); 5] = [
        ("`", |s| Style { code: true, ..s }),
        ("**", |s| Style { bold: true, ..s }),
        ("~~", |s| Style { strike: true, ..s }),
        ("*", |s| Style { italic: true, ..s }),
        ("_", |s| Style { italic: true, ..s }),
    ];

    for (delim, apply) in candidates {
        if !rest.starts_with(delim) || style.code {
            continue;
        }
        // `_` only delimits at word boundaries, so snake_case stays plain.
        if delim == "_"
            && text[..i]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
        {
            continue;
        }
        let content_start = i + delim.len();
        let body = &text[content_start..end];
        if body.is_empty() || body.starts_with(char::is_whitespace) || body.starts_with(delim) {
            continue;
        }
        if let Some(close) = find_closing(body, delim) {
            let content = content_start..content_start + close;
            if text[content.clone()].ends_with(char::is_whitespace) {
                continue;
            }
            return Some((delim.len(), content, apply));
        }
    }
    None
}

fn find_closing(body: &str, delim: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(pos) = body[search..].find(delim) {
        let at = search + pos;
        // A single `*` must not be half of a `**`.
        let doubled = delim.len() == 1
            && delim != "`"
            && (body[at + 1..].starts_with(delim) || (at > 0 && body[..at].ends_with(delim)));
        if at > 0 && !doubled {
            return Some(at);
        }
        search = at + delim.len();
    }
    None
}

fn push(out: &mut Vec<Span>, range: Range<usize>, style: Style, marker: bool) {
    if range.is_empty() {
        return;
    }
    // Merge adjacent plain runs with identical styling.
    if let Some(last) = out.last_mut() {
        if last.range.end == range.start && last.style == style && last.marker == marker && !marker
        {
            last.range.end = range.end;
            return;
        }
    }
    out.push(Span {
        range,
        style,
        marker,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styled(text: &str) -> Vec<(&str, Style, bool)> {
        parse(text)
            .into_iter()
            .map(|s| (&text[s.range.clone()], s.style, s.marker))
            .collect()
    }

    const PLAIN: Style = Style {
        bold: false,
        italic: false,
        code: false,
        strike: false,
        link: false,
        footnote: false,
    };
    const BOLD: Style = Style {
        bold: true,
        ..PLAIN
    };
    const ITALIC: Style = Style {
        italic: true,
        ..PLAIN
    };
    const CODE: Style = Style {
        code: true,
        ..PLAIN
    };

    #[test]
    fn plain_text_is_one_span() {
        assert_eq!(styled("hello"), vec![("hello", PLAIN, false)]);
        assert!(parse("").is_empty());
    }

    #[test]
    fn bold_and_italic() {
        assert_eq!(
            styled("a **b** *c*"),
            vec![
                ("a ", PLAIN, false),
                ("**", PLAIN, true),
                ("b", BOLD, false),
                ("**", PLAIN, true),
                (" ", PLAIN, false),
                ("*", PLAIN, true),
                ("c", ITALIC, false),
                ("*", PLAIN, true),
            ]
        );
    }

    #[test]
    fn nested_styles() {
        let spans = styled("**bold _both_**");
        assert!(spans.contains(&(
            "both",
            Style {
                bold: true,
                italic: true,
                ..PLAIN
            },
            false
        )));
    }

    #[test]
    fn code_is_not_parsed_inside() {
        assert_eq!(
            styled("`**x**`"),
            vec![
                ("`", PLAIN, true),
                ("**x**", CODE, false),
                ("`", PLAIN, true)
            ]
        );
    }

    #[test]
    fn unmatched_and_intraword_delimiters_stay_plain() {
        assert_eq!(styled("2 * 3 * 4"), vec![("2 * 3 * 4", PLAIN, false)]);
        assert_eq!(
            styled("snake_case_name"),
            vec![("snake_case_name", PLAIN, false)]
        );
        assert_eq!(styled("**open"), vec![("**open", PLAIN, false)]);
    }

    #[test]
    fn spans_cover_every_byte() {
        for text in [
            "a **b** c",
            "~~x~~ `y` _z_ é**ü**",
            "**",
            "*a**b*",
            "日本 *語*",
        ] {
            let spans = parse(text);
            let mut pos = 0;
            for span in &spans {
                assert_eq!(span.range.start, pos, "gap in {text:?}");
                pos = span.range.end;
            }
            assert_eq!(pos, text.len());
        }
    }

    const LINK: Style = Style {
        link: true,
        ..PLAIN
    };

    #[test]
    fn markdown_links_hide_their_url() {
        assert_eq!(
            styled("see [the **docs**](https://a.io/x_(y)) now"),
            vec![
                ("see ", PLAIN, false),
                ("[", PLAIN, true),
                ("the ", LINK, false),
                ("**", LINK, true),
                ("docs", Style { bold: true, ..LINK }, false),
                ("**", LINK, true),
                ("](https://a.io/x_(y))", PLAIN, true),
                (" now", PLAIN, false),
            ]
        );
        assert_eq!(plain_text("[Docs](https://a.io)"), "Docs");
        assert_eq!(
            hidden_markup("a [Docs](https://a.io) https://b.io"),
            [2..3, 7..22]
        );
        assert_eq!(styled("![img](x.png)")[0].1, PLAIN, "images aren't links");
    }

    #[test]
    fn bare_urls_are_links_without_trailing_punctuation() {
        let text = "Go to https://example.com/a_b_c?x=1. Or (https://w.io/p) `https://no`";
        let links = links(text);
        let urls: Vec<&str> = links.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(urls, ["https://example.com/a_b_c?x=1", "https://w.io/p"]);
        assert_eq!(
            link_at(text, text.find("example").unwrap()).as_deref(),
            Some("https://example.com/a_b_c?x=1")
        );
        assert_eq!(link_at(text, 0), None);
        assert!(styled(text).contains(&("https://example.com/a_b_c?x=1", LINK, false)));
    }

    #[test]
    fn plain_text_strips_markers() {
        assert_eq!(plain_text("My **big** `idea`"), "My big idea");
    }
}
