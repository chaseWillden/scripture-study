//! Citations: references in the text (`[^1]`, or a scripture like
//! `[^Ether 2:1-4]`) and the list of sources they point to, kept as Markdown
//! footnotes.
//!
//! Numbered citations follow the order their references appear in the note;
//! [`renumber`] rewrites those numbers after any change, so a citation added
//! between two others takes their place in the sequence. A label that isn't a
//! number, such as a scripture reference, is left as written.

use std::collections::HashMap;
use std::ops::Range;
use std::time::SystemTime;

use crate::document::{BlockKind, Document};
use crate::editor::{byte_to_char, char_to_byte, Caret};
use crate::links;
use crate::time::utc_parts;

/// One source in the note's citation list: the footnote `[^id]: text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Citation {
    pub id: String,
    /// The formatted citation, as inline Markdown.
    pub text: String,
}

/// A reference to a citation in some text: `[^id]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref {
    /// Byte range of the whole `[^id]`.
    pub range: Range<usize>,
    pub id: String,
}

impl Ref {
    /// Byte range of just the id, which shows as the superscript number.
    pub fn label(&self) -> Range<usize> {
        self.range.start + 2..self.range.end - 1
    }
}

/// A citation id that [`renumber`] will rewrite to `1`, `2`, `3`, …
///
/// Scripture references (`1 Nephi 1:11`, `Ether 2:1-4`) are not numbered.
/// `new123` is the temporary id [`insert`] uses before the rewrite.
pub fn is_numbered(id: &str) -> bool {
    id.chars().all(|c| c.is_ascii_digit())
        || id
            .strip_prefix("new")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !valid_id_char(first) || first == ' ' || first == ':' {
        return false;
    }
    let mut last = first;
    for c in chars {
        if !valid_id_char(c) {
            return false;
        }
        last = c;
    }
    last != ' ' && last != ':'
}

fn valid_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ' | ':' | '.')
}

/// `[^id]` at byte `i`, if there is one.
pub(crate) fn match_ref(text: &str, i: usize, end: usize) -> Option<Range<usize>> {
    let rest = text[i..end].strip_prefix("[^")?;
    let len = rest.find(']')?;
    let id = &rest[..len];
    // `[^id]:` starts a definition, not a reference.
    let definition = rest[len + 1..].starts_with(':') && i == 0;
    (valid_id(id) && !definition).then(|| i..i + 2 + len + 1)
}

/// Every citation reference in `text`, in order (none inside inline code).
pub fn refs(text: &str) -> Vec<Ref> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut code = false;
    while i < text.len() {
        if text[i..].starts_with('`') {
            code = !code;
        } else if !code {
            if let Some(range) = match_ref(text, i, text.len()) {
                let id = text[range.start + 2..range.end - 1].to_string();
                i = range.end;
                out.push(Ref { range, id });
                continue;
            }
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// The id of the reference covering byte `at`, if any.
pub fn ref_at(text: &str, at: usize) -> Option<String> {
    refs(text)
        .into_iter()
        .find(|r| r.range.contains(&at))
        .map(|r| r.id)
}

/// Numbers citations in the order their references appear, rewriting the
/// references and the list to match. Sources nothing refers to any more are
/// dropped; references to unknown sources are left alone.
pub fn renumber(doc: &mut Document) {
    let known: HashMap<String, String> = doc
        .citations
        .iter()
        .map(|c| (c.id.clone(), c.text.clone()))
        .collect();
    let mut order: Vec<String> = Vec::new();
    for block in &doc.blocks {
        if matches!(block.kind, BlockKind::Code { .. }) {
            continue;
        }
        for r in refs(&block.text) {
            if known.contains_key(&r.id) && !order.contains(&r.id) {
                order.push(r.id);
            }
        }
    }
    let mut next = 1usize;
    let assigned: Vec<String> = order
        .iter()
        .map(|id| {
            if is_numbered(id) {
                let n = next.to_string();
                next += 1;
                n
            } else {
                id.clone()
            }
        })
        .collect();
    let already = doc.citations.len() == order.len()
        && doc
            .citations
            .iter()
            .zip(&order)
            .zip(&assigned)
            .all(|((citation, id), label)| &citation.id == id && &citation.id == label);
    if already {
        return;
    }
    let number: HashMap<&str, &str> = order
        .iter()
        .zip(&assigned)
        .map(|(id, label)| (id.as_str(), label.as_str()))
        .collect();
    for block in &mut doc.blocks {
        if matches!(block.kind, BlockKind::Code { .. }) {
            continue;
        }
        let found = refs(&block.text);
        for r in found.iter().rev() {
            if let Some(n) = number.get(r.id.as_str()) {
                if *n != r.id {
                    block
                        .text
                        .replace_range(r.range.clone(), &format!("[^{n}]"));
                }
            }
        }
    }
    doc.citations = order
        .iter()
        .zip(&assigned)
        .map(|(id, label)| Citation {
            id: label.clone(),
            text: known[id].clone(),
        })
        .collect();
}

/// Adds a citation at character `char` of block `block`, numbered in place.
/// Returns the caret just after its reference.
pub fn insert(doc: &mut Document, block: usize, char: usize, text: &str) -> Caret {
    // A temporary id no reference can already use.
    let id = format!("new{}", doc.citations.len() + 1000);
    let target = &mut doc.blocks[block].text;
    let at = char_to_byte(target, char);
    target.insert_str(at, &format!("[^{id}]"));
    doc.citations.push(Citation {
        id: id.clone(),
        text: text.trim().to_string(),
    });
    renumber(doc);
    // Find where our reference ended up (its number may have changed length).
    let text = &doc.blocks[block].text;
    let end = refs(text)
        .into_iter()
        .find(|r| r.range.start == at)
        .map_or(at, |r| r.range.end);
    Caret {
        block,
        char: byte_to_char(text, end),
    }
}

/// Adds a citation whose superscript is `label` (a scripture reference, not a
/// number) at character `char` of block `block`. Citing the same label again
/// reuses the existing footnote. Returns the caret just after the reference.
pub fn insert_labeled(
    doc: &mut Document,
    block: usize,
    char: usize,
    label: &str,
    text: &str,
) -> Caret {
    let target = &mut doc.blocks[block].text;
    let at = char_to_byte(target, char);
    let marker = format!("[^{label}]");
    let prior = target[..at].matches(&marker).count();
    target.insert_str(at, &marker);
    if !doc.citations.iter().any(|citation| citation.id == label) {
        doc.citations.push(Citation {
            id: label.to_string(),
            text: text.trim().to_string(),
        });
    }
    renumber(doc);
    let text = &doc.blocks[block].text;
    let mut search = 0;
    let mut found = None;
    for _ in 0..=prior {
        found = text[search..].find(&marker).map(|index| search + index);
        if let Some(start) = found {
            search = start + marker.len();
        }
    }
    let end = found.map_or(at + marker.len(), |start| start + marker.len());
    Caret {
        block,
        char: byte_to_char(text, end.min(text.len())),
    }
}

/// Backspace right after a reference (or Delete right before one) removes
/// the whole reference. Returns the caret.
pub fn delete_ref_at_edge(text: &mut String, caret: usize, forward: bool) -> Option<usize> {
    let at = char_to_byte(text, caret);
    let r = refs(text).into_iter().find(|r| {
        if forward {
            r.range.start == at
        } else {
            r.range.end == at
        }
    })?;
    let start = byte_to_char(text, r.range.start);
    text.replace_range(r.range, "");
    Some(start)
}

/// `[^id]: text`, a citation in the list at the end of a note.
pub(crate) fn parse_definition(line: &str) -> Option<Citation> {
    let rest = line.strip_prefix("[^")?;
    let (id, text) = rest.split_once("]:")?;
    valid_id(id).then(|| Citation {
        id: id.to_string(),
        text: text.trim().to_string(),
    })
}

// ---------------------------------------------------------------------------
// Sources and MLA formatting

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceKind {
    #[default]
    Website,
    Book,
    Article,
}

impl SourceKind {
    pub const ALL: [SourceKind; 3] = [SourceKind::Website, SourceKind::Book, SourceKind::Article];

    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Website => "Web page",
            SourceKind::Book => "Book",
            SourceKind::Article => "Journal article",
        }
    }
}

/// What's known about a source, as typed in the citation form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    pub kind: SourceKind,
    /// People, separated by `;` or "and": "Jane Smith; John Doe".
    pub authors: String,
    pub title: String,
    /// The website, journal, or (for a book) nothing.
    pub container: String,
    pub publisher: String,
    /// Publication date: `2024-03-12`, `2024-03`, `2024`, or free text.
    pub published: String,
    pub volume: String,
    pub issue: String,
    pub pages: String,
    pub url: String,
    /// When a web page was read: `2026-09-29` or free text.
    pub accessed: String,
}

impl Source {
    /// The citation in MLA (9th edition) style, as inline Markdown.
    pub fn format_mla(&self) -> String {
        let mut out = String::new();
        let authors = mla_authors(&self.authors);
        if !authors.is_empty() {
            out.push_str(&authors);
            out.push(' ');
        }
        let title = self.title.trim();
        match self.kind {
            SourceKind::Book if !title.is_empty() => {
                out.push_str(&format!("*{}*", title));
                out.push_str(if ends_sentence(title) { " " } else { ". " });
            }
            _ if !title.is_empty() => {
                let punct = if ends_sentence(title) { "" } else { "." };
                out.push_str(&format!("\u{201c}{title}{punct}\u{201d} "));
            }
            _ => {}
        }

        // The "container": where the work appears, and how to find it.
        let mut parts: Vec<String> = Vec::new();
        let container = self.container.trim();
        if !container.is_empty() && self.kind != SourceKind::Book {
            parts.push(format!("*{container}*"));
        }
        if self.kind == SourceKind::Article {
            if !self.volume.trim().is_empty() {
                parts.push(format!("vol. {}", self.volume.trim()));
            }
            if !self.issue.trim().is_empty() {
                parts.push(format!("no. {}", self.issue.trim()));
            }
        }
        let publisher = self.publisher.trim();
        if !publisher.is_empty() && !publisher.eq_ignore_ascii_case(container) {
            parts.push(publisher.to_string());
        }
        if !self.published.trim().is_empty() {
            parts.push(mla_date(&self.published));
        }
        if !self.pages.trim().is_empty() && self.kind != SourceKind::Website {
            let pages = self.pages.trim();
            let prefix = if pages.contains(['-', '–', ',']) {
                "pp."
            } else {
                "p."
            };
            parts.push(format!("{prefix} {}", pages.replace('-', "–")));
        }
        if let Some(url) = links::as_url(&self.url) {
            let url = links::clean(&url);
            // MLA leaves off the scheme; the link still opens the page.
            let shown = url
                .split_once("://")
                .map_or(url.as_str(), |(_, rest)| rest)
                .trim_end_matches('/');
            parts.push(format!("[{}]({})", shown.replace(['[', ']'], ""), url));
        }
        if !parts.is_empty() {
            out.push_str(&parts.join(", "));
            out.push('.');
        }
        if self.kind == SourceKind::Website && !self.accessed.trim().is_empty() {
            out.push_str(&format!(" Accessed {}.", mla_date(&self.accessed)));
        }
        out.trim().to_string()
    }

    /// Fills the fields that are still empty from `other` (e.g. what was
    /// found on the web page), leaving anything already typed alone.
    pub fn fill_from(&mut self, other: &Source) {
        let pairs = [
            (&mut self.authors, &other.authors),
            (&mut self.title, &other.title),
            (&mut self.container, &other.container),
            (&mut self.publisher, &other.publisher),
            (&mut self.published, &other.published),
            (&mut self.volume, &other.volume),
            (&mut self.issue, &other.issue),
            (&mut self.pages, &other.pages),
        ];
        for (mine, theirs) in pairs {
            if mine.trim().is_empty() {
                mine.clone_from(theirs);
            }
        }
    }

    /// Reads what a web page says about itself: the citation, Open Graph,
    /// and schema.org metadata most sites publish, then its `<title>`.
    pub fn from_html(html: &str, url: &str) -> Source {
        let meta = meta_tags(html);
        let get = |keys: &[&str]| -> String {
            keys.iter()
                .find_map(|k| meta.get(*k).and_then(|v| v.first()).cloned())
                .unwrap_or_default()
        };
        let all = |key: &str| -> Vec<String> { meta.get(key).cloned().unwrap_or_default() };

        let journal = get(&["citation_journal_title", "prism.publicationname"]);
        let is_book = get(&["og:type"]).eq_ignore_ascii_case("book")
            || !get(&["citation_isbn", "books:isbn"]).is_empty();
        let kind = if !journal.is_empty() {
            SourceKind::Article
        } else if is_book {
            SourceKind::Book
        } else {
            SourceKind::Website
        };

        let host = url
            .split_once("://")
            .map_or(url, |(_, rest)| rest)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .trim_start_matches("www.")
            .to_string();
        let mut site = get(&["og:site_name", "application-name"]);
        if site.is_empty() {
            site = known_site(&host).unwrap_or_default().to_string();
        }

        let mut authors = all("citation_author");
        if authors.is_empty() {
            authors = all("author")
                .into_iter()
                .chain(all("article:author"))
                .chain(all("parsely-author"))
                .chain(all("dc.creator"))
                .filter(|a| !a.starts_with("http"))
                .collect();
        }
        if authors.is_empty() {
            authors.extend(json_ld_author(html));
        }
        if authors.is_empty() {
            authors.extend(byline(html));
        }
        let mut authors: Vec<String> = authors.iter().map(|a| without_honorific(a)).collect();
        authors.dedup();

        let mut title = get(&["citation_title", "og:title", "twitter:title", "dc.title"]);
        if title.is_empty() {
            title = tag_text(html, "title").unwrap_or_default();
        }
        // "Page title | Site name" → "Page title", learning the site's name
        // from the suffix if the page didn't say.
        let squashed = |s: &str| -> String {
            s.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        };
        for sep in [" | ", " - ", " – ", " — ", " :: "] {
            if let Some((head, tail)) = title.rsplit_once(sep) {
                let tail = tail.trim();
                let names_site = tail.eq_ignore_ascii_case(site.trim())
                    || (!squashed(tail).is_empty() && squashed(&host).contains(&squashed(tail)));
                if names_site {
                    if site.is_empty() {
                        site = tail.to_string();
                    }
                    title = head.trim().to_string();
                }
            }
        }
        // MLA puts the title in quotes itself.
        let mut title = title
            .trim()
            .trim_matches(['\u{201c}', '\u{201d}', '"'])
            .trim()
            .to_string();
        // A page that only names its domain (often a bot check) has no title.
        let domain_name = host.split('.').next().unwrap_or("");
        if [host.as_str(), site.as_str(), domain_name]
            .iter()
            .any(|name| squashed(&title) == squashed(name))
        {
            title.clear();
        }

        let container = if !journal.is_empty() {
            journal
        } else if kind == SourceKind::Book {
            String::new()
        } else if !site.is_empty() {
            site.clone()
        } else {
            host
        };

        let mut published = get(&[
            "citation_publication_date",
            "citation_date",
            "article:published_time",
            "dc.date",
            "date",
            "pubdate",
        ]);
        if published.is_empty() {
            published = json_ld_string(html, "datePublished").unwrap_or_default();
        }
        let published = published
            .split('T')
            .next()
            .unwrap_or("")
            .replace('/', "-")
            .trim()
            .to_string();

        let first = get(&["citation_firstpage"]);
        let last = get(&["citation_lastpage"]);
        let pages = match (first.is_empty(), last.is_empty()) {
            (false, false) => format!("{first}-{last}"),
            (false, true) => first,
            _ => String::new(),
        };

        Source {
            kind,
            authors: authors.join("; "),
            title,
            container,
            publisher: get(&["citation_publisher", "dc.publisher", "og:book:publisher"]),
            published,
            volume: get(&["citation_volume", "prism.volume"]),
            issue: get(&["citation_issue", "prism.number"]),
            pages,
            url: url.to_string(),
            accessed: String::new(),
        }
    }
}

/// Proper names for sites that don't state their own.
fn known_site(host: &str) -> Option<&'static str> {
    const SITES: [(&str, &str); 5] = [
        (
            "churchofjesuschrist.org",
            "The Church of Jesus Christ of Latter-day Saints",
        ),
        ("wikipedia.org", "Wikipedia"),
        ("youtube.com", "YouTube"),
        ("byu.edu", "Brigham Young University"),
        ("github.com", "GitHub"),
    ];
    SITES
        .iter()
        .find(|(domain, _)| host == *domain || host.ends_with(&format!(".{domain}")))
        .map(|(_, name)| *name)
}

/// The author from a byline like `<p class="author-name">By Jane Doe</p>`.
fn byline(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    for class in ["author-name", "byline__name", "byline", "author"] {
        let Some(n) = lower.find(&format!("class=\"{class}")) else {
            continue;
        };
        let start = n + lower[n..].find('>')? + 1;
        let end = start + lower[start..].find('<')?;
        let text = decode_entities(&html[start..end]);
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let text = text
            .strip_prefix("By ")
            .or_else(|| text.strip_prefix("by "))
            .unwrap_or(&text)
            .to_string();
        if !text.is_empty() && text.len() < 80 {
            return Some(text);
        }
    }
    None
}

/// MLA leaves off titles like "Elder" or "Dr.".
fn without_honorific(name: &str) -> String {
    const TITLES: [&str; 10] = [
        "Elder ",
        "President ",
        "Sister ",
        "Brother ",
        "Bishop ",
        "Dr. ",
        "Dr ",
        "Rev. ",
        "Professor ",
        "Prof. ",
    ];
    let name = name.trim();
    TITLES
        .iter()
        .find_map(|t| name.strip_prefix(t))
        .unwrap_or(name)
        .trim()
        .to_string()
}

/// Today's date as `YYYY-MM-DD` (UTC), for "Accessed".
pub fn today() -> String {
    let (year, month, day, ..) = utc_parts(SystemTime::now());
    format!("{year:04}-{month:02}-{day:02}")
}

fn ends_sentence(text: &str) -> bool {
    text.ends_with(['.', '?', '!'])
}

/// MLA author list: "Last, First." / "Last, First, and First Last." /
/// "Last, First, et al."
fn mla_authors(authors: &str) -> String {
    let names: Vec<String> = authors
        .split([';', '\n'])
        .flat_map(|part| part.split(" and "))
        .map(|name| name.trim().trim_end_matches(',').to_string())
        .filter(|name| !name.is_empty())
        .collect();
    let inverted = |name: &str| -> String {
        if name.contains(',') {
            return name.to_string();
        }
        match name.rsplit_once(' ') {
            Some((first, last)) => format!("{last}, {first}"),
            None => name.to_string(),
        }
    };
    let list = match names.as_slice() {
        [] => return String::new(),
        [one] => inverted(one),
        [one, two] => format!("{}, and {two}", inverted(one)),
        [one, ..] => format!("{}, et al", inverted(one)),
    };
    if list.ends_with('.') {
        list
    } else {
        format!("{list}.")
    }
}

/// `2024-03-12` → `12 Mar. 2024`; `2024-03` → `Mar. 2024`; anything else
/// stays as typed.
fn mla_date(date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan.", "Feb.", "Mar.", "Apr.", "May", "June", "July", "Aug.", "Sept.", "Oct.", "Nov.",
        "Dec.",
    ];
    let date = date.trim();
    let parts: Vec<&str> = date.split('-').collect();
    let number = |s: &str| s.parse::<u32>().ok();
    match parts.as_slice() {
        [y, m, d] if y.len() == 4 => match (number(y), number(m), number(d)) {
            (Some(y), Some(m @ 1..=12), Some(d @ 1..=31)) => {
                format!("{d} {} {y}", MONTHS[m as usize - 1])
            }
            _ => date.to_string(),
        },
        [y, m] if y.len() == 4 => match (number(y), number(m)) {
            (Some(y), Some(m @ 1..=12)) => format!("{} {y}", MONTHS[m as usize - 1]),
            _ => date.to_string(),
        },
        _ => date.to_string(),
    }
}

// ---------------------------------------------------------------------------
// A tiny, forgiving HTML metadata reader.

/// `<meta name|property|itemprop="key" content="value">`, keys lowercased.
fn meta_tags(html: &str) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(n) = lower[from..].find("<meta") {
        let start = from + n;
        let Some(len) = lower[start..].find('>') else {
            break;
        };
        let tag = &html[start..start + len];
        from = start + len;
        let key = ["name", "property", "itemprop"]
            .iter()
            .find_map(|attr| attribute(tag, attr));
        if let (Some(key), Some(content)) = (key, attribute(tag, "content")) {
            let content = decode_entities(&content).trim().to_string();
            if !content.is_empty() {
                out.entry(key.to_ascii_lowercase())
                    .or_default()
                    .push(content);
            }
        }
    }
    out
}

/// The value of attribute `name` in a tag, quoted or not.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(n) = lower[from..].find(name) {
        let at = from + n;
        from = at + name.len();
        let before_ok = lower[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_whitespace());
        let rest = lower[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let offset = tag.len() - rest.len() + 1;
        let value = tag[offset..].trim_start();
        return Some(match value.chars().next()? {
            q @ ('"' | '\'') => value[1..].split(q).next()?.to_string(),
            _ => value
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()?
                .trim_end_matches('/')
                .to_string(),
        });
    }
    None
}

/// The text inside the first `<tag>…</tag>`.
fn tag_text(html: &str, tag: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find(&format!("<{tag}"))?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find(&format!("</{tag}"))?;
    let text = decode_entities(&html[start..end]);
    Some(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// A string value for `"key": "value"` in the page's JSON-LD.
fn json_ld_string(html: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let at = html.find(&pattern)? + pattern.len();
    let rest = html[at..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    Some(decode_entities(&rest[..rest.find('"')?]))
}

/// The author's name in the page's JSON-LD (`"author": {"name": …}`).
fn json_ld_author(html: &str) -> Option<String> {
    let at = html.find("\"author\"")?;
    let window = &html[at..html.len().min(at + 400)];
    json_ld_string(window, "name").filter(|n| !n.is_empty())
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(n) = rest.find('&') {
        out.push_str(&rest[..n]);
        rest = &rest[n..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "rsquo" => Some('\u{2019}'),
            "lsquo" => Some('\u{2018}'),
            "rdquo" => Some('\u{201d}'),
            "ldquo" => Some('\u{201c}'),
            "mdash" => Some('\u{2014}'),
            "ndash" => Some('\u{2013}'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Block;

    fn doc(texts: &[&str], citations: &[(&str, &str)]) -> Document {
        let mut d = Document::new(texts.iter().map(|t| Block::paragraph(*t)).collect());
        d.citations = citations
            .iter()
            .map(|(id, text)| Citation {
                id: id.to_string(),
                text: text.to_string(),
            })
            .collect();
        d
    }

    fn texts(d: &Document) -> Vec<&str> {
        d.blocks.iter().map(|b| b.text.as_str()).collect()
    }

    fn list(d: &Document) -> Vec<(&str, &str)> {
        d.citations
            .iter()
            .map(|c| (c.id.as_str(), c.text.as_str()))
            .collect()
    }

    #[test]
    fn finds_references_but_not_in_code() {
        let text = "a[^1] b[^x-2] `[^3]` [^] [^4";
        let ids: Vec<String> = refs(text).into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["1", "x-2"]);
        assert_eq!(ref_at(text, 2).as_deref(), Some("1"));
        assert_eq!(ref_at(text, 0), None);
    }

    #[test]
    fn numbers_follow_the_text_and_renumber_when_one_is_added_between() {
        let mut d = doc(
            &["First[^1] and second[^2].", "Third[^3]."],
            &[("1", "A"), ("2", "B"), ("3", "C")],
        );
        // Add one between the first and second.
        let caret = insert(&mut d, 0, 5 + 4 + " and".len(), "New");
        assert_eq!(texts(&d), ["First[^1] and[^2] second[^3].", "Third[^4]."]);
        assert_eq!(list(&d), [("1", "A"), ("2", "New"), ("3", "B"), ("4", "C")]);
        assert_eq!(caret, Caret { block: 0, char: 17 });

        // Deleting a reference drops its source and closes the gap.
        d.blocks[0].text = "First[^1] and second[^3].".into();
        renumber(&mut d);
        assert_eq!(texts(&d), ["First[^1] and second[^2].", "Third[^3]."]);
        assert_eq!(list(&d), [("1", "A"), ("2", "B"), ("3", "C")]);
    }

    #[test]
    fn scripture_labels_are_not_renumbered() {
        let mut d = doc(&["One[^1] three[^2]"], &[("1", "A"), ("2", "B")]);
        let at = "One[^1]".chars().count();
        insert_labeled(&mut d, 0, at, "Ether 2:1-4", "Jared and Nimrod.");
        assert_eq!(texts(&d), ["One[^1][^Ether 2:1-4] three[^2]"]);
        assert_eq!(
            list(&d),
            [("1", "A"), ("Ether 2:1-4", "Jared and Nimrod."), ("2", "B"),]
        );

        // A numbered citation before it shifts the numbers and leaves the reference.
        insert(&mut d, 0, 0, "First");
        assert!(texts(&d)[0].starts_with("[^1]"), "{}", texts(&d)[0]);
        assert!(texts(&d)[0].contains("[^Ether 2:1-4]"), "{}", texts(&d)[0]);
        assert!(texts(&d)[0].contains("[^3]"), "{}", texts(&d)[0]);
        let ether = d
            .citations
            .iter()
            .find(|citation| citation.id == "Ether 2:1-4")
            .unwrap();
        assert_eq!(ether.text, "Jared and Nimrod.");
        assert_eq!(d.citations[0].id, "1");
        assert_eq!(d.citations[1].id, "2");

        // Citing it again reuses the one footnote.
        let before = d.citations.len();
        let end = texts(&d)[0].chars().count();
        insert_labeled(&mut d, 0, end, "Ether 2:1-4", "other");
        assert_eq!(d.citations.len(), before);
        assert_eq!(texts(&d)[0].matches("[^Ether 2:1-4]").count(), 2);

        let parsed = Document::from_markdown(&d.to_markdown());
        let ether = parsed
            .citations
            .iter()
            .find(|citation| citation.id == "Ether 2:1-4")
            .unwrap();
        assert_eq!(ether.text, "Jared and Nimrod.");
    }

    #[test]
    fn scripture_references_parse_in_the_text_and_the_list() {
        let ids: Vec<String> = refs("see [^1 Nephi 1:11] and [^Ether 2:1-4].")
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["1 Nephi 1:11", "Ether 2:1-4"]);
        let citation = parse_definition("[^Joseph Smith--History 1:2]: In this history.").unwrap();
        assert_eq!(citation.id, "Joseph Smith--History 1:2");
        assert_eq!(citation.text, "In this history.");
    }

    #[test]
    fn backspace_removes_a_whole_reference() {
        let mut text = "word[^12] more".to_string();
        assert_eq!(delete_ref_at_edge(&mut text, 9, false), Some(4));
        assert_eq!(text, "word more");
        let mut text = "word[^1]".to_string();
        assert_eq!(delete_ref_at_edge(&mut text, 4, true), Some(4));
        assert_eq!(text, "word");
        assert_eq!(delete_ref_at_edge(&mut text, 2, false), None);
    }

    #[test]
    fn mla_web_page() {
        let source = Source {
            kind: SourceKind::Website,
            authors: "Jane Q. Smith".into(),
            title: "How Tides Work".into(),
            container: "Ocean Today".into(),
            publisher: "NOAA".into(),
            published: "2024-03-12".into(),
            url: "https://oceantoday.noaa.gov/tides/".into(),
            accessed: "2026-09-29".into(),
            ..Default::default()
        };
        assert_eq!(
            source.format_mla(),
            "Smith, Jane Q. \u{201c}How Tides Work.\u{201d} *Ocean Today*, NOAA, 12 Mar. 2024, \
             [oceantoday.noaa.gov/tides](https://oceantoday.noaa.gov/tides/). Accessed 29 Sept. 2026."
        );
    }

    #[test]
    fn mla_book_and_article() {
        let book = Source {
            kind: SourceKind::Book,
            authors: "Terryl Givens and Fiona Givens".into(),
            title: "The God Who Weeps".into(),
            publisher: "Ensign Peak".into(),
            published: "2012".into(),
            ..Default::default()
        };
        assert_eq!(
            book.format_mla(),
            "Givens, Terryl, and Fiona Givens. *The God Who Weeps*. Ensign Peak, 2012."
        );
        let article = Source {
            kind: SourceKind::Article,
            authors: "A One; B Two; C Three".into(),
            title: "Why Study?".into(),
            container: "BYU Studies".into(),
            volume: "58".into(),
            issue: "2".into(),
            published: "2019-06".into(),
            pages: "45-67".into(),
            ..Default::default()
        };
        assert_eq!(
            article.format_mla(),
            "One, A, et al. \u{201c}Why Study?\u{201d} *BYU Studies*, vol. 58, no. 2, June 2019, pp. 45\u{2013}67."
        );
    }

    #[test]
    fn reads_page_metadata() {
        let html = r#"<html><head>
            <title>Faith | Church News</title>
            <meta property="og:site_name" content="Church News">
            <meta property="og:title" content="Faith &amp; Works | Church News" />
            <meta name="author" content="Mary Jones">
            <meta property="article:published_time" content="2023-10-05T14:00:00Z">
            </head></html>"#;
        let s = Source::from_html(html, "https://www.thechurchnews.com/faith");
        assert_eq!(s.kind, SourceKind::Website);
        assert_eq!(s.title, "Faith & Works");
        assert_eq!(s.container, "Church News");
        assert_eq!(s.authors, "Mary Jones");
        assert_eq!(s.published, "2023-10-05");

        let journal = r#"<meta name="citation_title" content="On Grace">
            <meta name="citation_author" content="Doe, Jane">
            <meta name="citation_author" content="Roe, Rick">
            <meta name="citation_journal_title" content="Journal of Things">
            <meta name="citation_volume" content="12"><meta name="citation_issue" content="3">
            <meta name="citation_firstpage" content="1"><meta name="citation_lastpage" content="20">
            <meta name="citation_publication_date" content="2020/05/01">"#;
        let s = Source::from_html(journal, "https://j.org/a");
        assert_eq!(s.kind, SourceKind::Article);
        assert_eq!(s.authors, "Doe, Jane; Roe, Rick");
        assert_eq!(
            (s.volume.as_str(), s.issue.as_str(), s.pages.as_str()),
            ("12", "3", "1-20")
        );
        assert_eq!(s.published, "2020-05-01");

        // A Church talk: quoted title, byline with a title, known site name.
        let talk = r#"<meta name="title" content="“Abide in Me, and I in You; Therefore Walk with Me”"/>
            <meta property="og:title" content="“Abide in Me, and I in You; Therefore Walk with Me”"/>
            <p class="author-name" data-aid="1" id="author1">By Elder David A. Bednar</p>
            <script>{"@type":"WebPage","datePublished":"2023-04-02T00:00:00.000Z"}</script>"#;
        let s = Source::from_html(
            talk,
            "https://www.churchofjesuschrist.org/study/general-conference/2023/04/57bednar",
        );
        assert_eq!(s.title, "Abide in Me, and I in You; Therefore Walk with Me");
        assert_eq!(s.authors, "David A. Bednar");
        assert_eq!(
            s.container,
            "The Church of Jesus Christ of Latter-day Saints"
        );
        assert_eq!(s.published, "2023-04-02");

        // "Title - Wikipedia": the suffix names the site.
        let wiki = Source::from_html(
            "<title>Grace in Christianity - Wikipedia</title>",
            "https://en.wikipedia.org/wiki/Grace",
        );
        assert_eq!(
            (wiki.title.as_str(), wiki.container.as_str()),
            ("Grace in Christianity", "Wikipedia")
        );

        // A bot check that only names the domain gives no title.
        let blocked = Source::from_html("<title>nytimes.com</title>", "https://www.nytimes.com/x");
        assert_eq!(blocked.title, "");
        let missing = Source::from_html("<title>BBC</title>", "https://www.bbc.com/news/nope");
        assert_eq!(missing.title, "");

        // Nothing but a <title>: fall back to it and the host.
        let bare = Source::from_html(
            "<title>  Plain\n page </title>",
            "https://www.example.com/x",
        );
        assert_eq!(
            (bare.title.as_str(), bare.container.as_str()),
            ("Plain page", "example.com")
        );
    }

    #[test]
    fn filling_keeps_what_was_typed() {
        let mut mine = Source {
            title: "My title".into(),
            ..Default::default()
        };
        mine.fill_from(&Source {
            title: "Their title".into(),
            authors: "Someone".into(),
            ..Default::default()
        });
        assert_eq!(
            (mine.title.as_str(), mine.authors.as_str()),
            ("My title", "Someone")
        );
    }
}
