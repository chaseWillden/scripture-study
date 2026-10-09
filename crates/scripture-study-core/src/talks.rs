//! General conference talks: which conferences exist, reading the Church's
//! study pages into plain text, and searching the talks that were downloaded.
//!
//! Fetching is left to the front end. This module turns a conference's
//! table of contents ([`parse_manifest`]) and each talk page ([`parse_talk`])
//! into [`ConferenceTalks`], which the front end saves and later [`search`]es.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// The first general conference in the Church's online archive.
pub const FIRST: Conference = Conference {
    year: 1971,
    month: 4,
};

/// One general conference, held each April and October.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Conference {
    pub year: u16,
    /// `4` for April, `10` for October.
    pub month: u8,
}

impl Conference {
    pub fn april(year: u16) -> Self {
        Self { year, month: 4 }
    }

    pub fn october(year: u16) -> Self {
        Self { year, month: 10 }
    }

    /// `April` or `October`.
    pub fn season(&self) -> &'static str {
        if self.month == 4 {
            "April"
        } else {
            "October"
        }
    }

    /// `April 2024`.
    pub fn label(&self) -> String {
        format!("{} {}", self.season(), self.year)
    }

    /// Path on the study site, e.g. `/general-conference/2024/10`.
    pub fn uri(&self) -> String {
        format!("/general-conference/{}/{:02}", self.year, self.month)
    }

    /// Stable name for storage, e.g. `2024-10`.
    pub fn key(&self) -> String {
        format!("{}-{:02}", self.year, self.month)
    }

    pub fn from_key(key: &str) -> Option<Self> {
        let (year, month) = key.split_once('-')?;
        let conference = Self {
            year: year.parse().ok()?,
            month: month.parse().ok()?,
        };
        (conference.month == 4 || conference.month == 10).then_some(conference)
    }

    /// The most recent conference whose month has begun by `now`.
    pub fn latest(now: SystemTime) -> Self {
        let (year, month, ..) = crate::time::utc_parts(now);
        let year = year.clamp(FIRST.year as i64, u16::MAX as i64) as u16;
        match month {
            1..=3 => Self::october(year - 1),
            4..=9 => Self::april(year),
            _ => Self::october(year),
        }
    }

    /// Every conference from [`FIRST`] through `latest`, newest first.
    pub fn all_through(latest: Conference) -> Vec<Self> {
        let mut all = Vec::new();
        for year in (FIRST.year..=latest.year).rev() {
            for conference in [Self::october(year), Self::april(year)] {
                if conference <= latest && conference >= FIRST {
                    all.push(conference);
                }
            }
        }
        all
    }
}

/// A talk as listed in a conference's table of contents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListedTalk {
    /// Study-site path, e.g. `/general-conference/2024/10/12andersen`.
    pub uri: String,
    pub speaker: String,
    pub title: String,
}

/// A session in a conference's table of contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedSession {
    pub title: String,
    pub talks: Vec<ListedTalk>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paragraph {
    pub text: String,
    /// A section heading inside the talk.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub heading: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Talk {
    pub uri: String,
    pub title: String,
    pub speaker: String,
    /// Calling, e.g. `Of the Quorum of the Twelve Apostles`.
    #[serde(default)]
    pub role: String,
    /// The one-line summary under the byline.
    #[serde(default)]
    pub kicker: String,
    pub paragraphs: Vec<Paragraph>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub title: String,
    pub talks: Vec<Talk>,
}

/// Every talk of one conference, as saved after a download.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConferenceTalks {
    pub conference: Conference,
    pub sessions: Vec<Session>,
}

impl ConferenceTalks {
    pub fn talk_count(&self) -> usize {
        self.sessions.iter().map(|s| s.talks.len()).sum()
    }
}

/// Reads the sessions and talks out of a conference's table of contents.
///
/// Each session is an `h2.label` heading followed by `a.list-tile` links: one
/// to the session itself, then one per talk. Only some years also tag each
/// item with `data-content-type`, so that is not relied on.
pub fn parse_manifest(html: &str) -> Vec<ListedSession> {
    enum Field {
        SessionTitle,
        Speaker,
        Title,
    }
    let mut sessions: Vec<ListedSession> = Vec::new();
    let mut talk: Option<ListedTalk> = None;
    let mut field: Option<(Field, String)> = None;
    // The session's title is the first `p.title` after its heading.
    let mut wants_session_title = false;
    for token in tokens(html) {
        match token {
            Token::Start { name, attrs } => {
                let class = attr(&attrs, "class");
                match name.as_str() {
                    "h2" if has_class(class, "label") => {
                        sessions.push(ListedSession {
                            title: String::new(),
                            talks: Vec::new(),
                        });
                        wants_session_title = true;
                    }
                    "a" if has_class(class, "list-tile") => {
                        talk = Some(ListedTalk {
                            uri: study_uri(attr(&attrs, "href")),
                            ..ListedTalk::default()
                        });
                    }
                    "p" if has_class(class, "title") && wants_session_title => {
                        field = Some((Field::SessionTitle, String::new()));
                    }
                    "p" if talk.is_some() && has_class(class, "primaryMeta") => {
                        field = Some((Field::Speaker, String::new()));
                    }
                    "p" if talk.is_some() && has_class(class, "title") => {
                        field = Some((Field::Title, String::new()));
                    }
                    _ => {}
                }
            }
            Token::Text(text) => {
                if let Some((_, buffer)) = &mut field {
                    buffer.push_str(&decode(text));
                }
            }
            Token::End(name) => match name.as_str() {
                "p" => {
                    let Some((kind, text)) = field.take() else {
                        continue;
                    };
                    let text = collapse(&text);
                    match kind {
                        Field::SessionTitle => {
                            wants_session_title = false;
                            if let Some(session) = sessions.last_mut() {
                                session.title = text;
                            }
                        }
                        Field::Speaker => {
                            if let Some(talk) = &mut talk {
                                talk.speaker = text;
                            }
                        }
                        Field::Title => {
                            if let Some(talk) = &mut talk {
                                talk.title = text;
                            }
                        }
                    }
                }
                "a" => {
                    let Some(done) = talk.take() else {
                        continue;
                    };
                    if sessions.is_empty() {
                        sessions.push(ListedSession {
                            title: String::new(),
                            talks: Vec::new(),
                        });
                    }
                    let session = sessions.last_mut().unwrap();
                    // The session's own tile shares its title and has no speaker.
                    let overview = done.speaker.is_empty() && done.title == session.title;
                    if !done.uri.is_empty() && !overview {
                        session.talks.push(done);
                    }
                }
                _ => {}
            },
        }
    }
    sessions.retain(|s| !s.talks.is_empty());
    sessions
}

/// Reads a talk page's title, byline, and paragraphs. Footnotes, pictures,
/// and video are left out. `listed` fills in what the page leaves blank.
pub fn parse_talk(listed: &ListedTalk, html: &str) -> Talk {
    #[derive(PartialEq)]
    enum Field {
        Title,
        Speaker,
        Role,
        Kicker,
        Paragraph,
        Heading,
    }
    let mut talk = Talk {
        uri: listed.uri.clone(),
        ..Default::default()
    };
    let mut in_body = false;
    // Inside a figure or a footnote marker, nothing is part of the text.
    let mut skip_depth = 0usize;
    let mut field: Option<(Field, String)> = None;
    for token in tokens(html) {
        match token {
            Token::Start { name, attrs } => {
                let class = attr(&attrs, "class");
                if skip_depth > 0 {
                    if matches!(name.as_str(), "figure" | "sup") {
                        skip_depth += 1;
                    }
                    continue;
                }
                match name.as_str() {
                    "figure" | "sup" => skip_depth = 1,
                    "footer" => break,
                    "div" if has_class(class, "body-block") => in_body = true,
                    "br" => {
                        if let Some((_, buffer)) = &mut field {
                            buffer.push(' ');
                        }
                    }
                    "h1" if field.is_none() && talk.title.is_empty() => {
                        field = Some((Field::Title, String::new()));
                    }
                    "p" if field.is_none() && has_class(class, "author-name") => {
                        field = Some((Field::Speaker, String::new()));
                    }
                    "p" if field.is_none() && has_class(class, "author-role") => {
                        field = Some((Field::Role, String::new()));
                    }
                    "p" if field.is_none() && has_class(class, "kicker") => {
                        field = Some((Field::Kicker, String::new()));
                    }
                    "p" if field.is_none() && in_body => {
                        field = Some((Field::Paragraph, String::new()));
                    }
                    "h2" | "h3" | "h4" if field.is_none() && in_body => {
                        field = Some((Field::Heading, String::new()));
                    }
                    _ => {}
                }
            }
            Token::Text(text) => {
                if skip_depth == 0 {
                    if let Some((_, buffer)) = &mut field {
                        buffer.push_str(&decode(text));
                    }
                }
            }
            Token::End(name) => {
                if skip_depth > 0 {
                    if matches!(name.as_str(), "figure" | "sup") {
                        skip_depth -= 1;
                    }
                    continue;
                }
                let closes = match field.as_ref().map(|(kind, _)| kind) {
                    Some(Field::Title) => name == "h1",
                    Some(Field::Heading) => matches!(name.as_str(), "h2" | "h3" | "h4"),
                    Some(_) => name == "p",
                    None => false,
                };
                if !closes {
                    continue;
                }
                let (kind, text) = field.take().unwrap();
                let text = collapse(&text);
                if text.is_empty() {
                    continue;
                }
                match kind {
                    Field::Title => talk.title = text,
                    Field::Speaker => {
                        talk.speaker = text.strip_prefix("By ").unwrap_or(&text).to_string();
                    }
                    Field::Role => talk.role = text,
                    Field::Kicker => talk.kicker = text,
                    Field::Paragraph | Field::Heading => talk.paragraphs.push(Paragraph {
                        text,
                        heading: kind == Field::Heading,
                    }),
                }
            }
        }
    }
    if talk.title.is_empty() {
        talk.title = listed.title.clone();
    }
    if talk.speaker.is_empty() {
        talk.speaker = listed.speaker.clone();
    }
    talk
}

/// Where a search matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// The title or the speaker.
    Talk,
    /// A paragraph, by index into [`Talk::paragraphs`].
    Paragraph(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TalkHit {
    /// Index into the slice that was searched.
    pub conference: usize,
    pub session: usize,
    pub talk: usize,
    pub place: Place,
    /// Text around the match, for the results list.
    pub snippet: String,
}

/// Talks searched over and over, with their text folded once.
pub struct Index {
    talks: Vec<IndexedTalk>,
}

struct IndexedTalk {
    conference: usize,
    session: usize,
    talk: usize,
    heading: String,
    paragraphs: Vec<String>,
}

impl Index {
    pub fn new(conferences: &[ConferenceTalks]) -> Self {
        let mut talks = Vec::new();
        for (c, conference) in conferences.iter().enumerate() {
            for (s, session) in conference.sessions.iter().enumerate() {
                for (t, talk) in session.talks.iter().enumerate() {
                    talks.push(IndexedTalk {
                        conference: c,
                        session: s,
                        talk: t,
                        heading: fold(&format!("{}\n{}", talk.title, talk.speaker)),
                        paragraphs: talk.paragraphs.iter().map(|p| fold(&p.text)).collect(),
                    });
                }
            }
        }
        Self { talks }
    }

    /// Talks whose title or speaker match `query` come first, then matching
    /// paragraphs in conference order. Every word must appear; a query in
    /// quotes must appear as written. At most `limit` hits.
    pub fn search(
        &self,
        conferences: &[ConferenceTalks],
        query: &str,
        limit: usize,
    ) -> Vec<TalkHit> {
        let terms = terms(query);
        if terms.is_empty() {
            return Vec::new();
        }
        let matches = |text: &str| terms.iter().all(|term| text.contains(term.as_str()));
        let talk_of =
            |t: &IndexedTalk| &conferences[t.conference].sessions[t.session].talks[t.talk];
        let mut hits = Vec::new();
        for indexed in &self.talks {
            if hits.len() >= limit {
                return hits;
            }
            if matches(&indexed.heading) {
                let talk = talk_of(indexed);
                hits.push(TalkHit {
                    conference: indexed.conference,
                    session: indexed.session,
                    talk: indexed.talk,
                    place: Place::Talk,
                    snippet: if talk.kicker.is_empty() {
                        talk.paragraphs
                            .first()
                            .map(|p| p.text.clone())
                            .unwrap_or_default()
                    } else {
                        talk.kicker.clone()
                    },
                });
            }
        }
        for indexed in &self.talks {
            for (p, text) in indexed.paragraphs.iter().enumerate() {
                if hits.len() >= limit {
                    return hits;
                }
                if !matches(text) {
                    continue;
                }
                let original = &talk_of(indexed).paragraphs[p].text;
                let at = text.find(terms[0].as_str()).unwrap_or(0);
                hits.push(TalkHit {
                    conference: indexed.conference,
                    session: indexed.session,
                    talk: indexed.talk,
                    place: Place::Paragraph(p),
                    snippet: snippet(original, text[..at].chars().count()),
                });
            }
        }
        hits
    }
}

/// One-shot search; see [`Index::search`].
pub fn search(conferences: &[ConferenceTalks], query: &str, limit: usize) -> Vec<TalkHit> {
    Index::new(conferences).search(conferences, query, limit)
}

/// What a query asks for: one phrase if quoted, otherwise each word.
fn terms(query: &str) -> Vec<String> {
    let folded = fold(query.trim());
    if let Some(phrase) = folded
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        let phrase = collapse(phrase);
        return if phrase.is_empty() {
            Vec::new()
        } else {
            vec![phrase]
        };
    }
    folded
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\''))
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

/// Lowercase with typographic quotes made plain, one char for each char of
/// `text` so positions line up.
fn fold(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '‘' | '’' => '\'',
            '“' | '”' => '"',
            '\u{a0}' => ' ',
            c => c.to_lowercase().next().unwrap_or(c),
        })
        .collect()
}

/// About 150 characters of `text` around char index `at`, on word breaks.
fn snippet(text: &str, at: usize) -> String {
    const BEFORE: usize = 40;
    const LENGTH: usize = 150;
    let chars: Vec<char> = text.chars().collect();
    let mut start = at.saturating_sub(BEFORE);
    if start > 0 {
        while start < at && !chars[start - 1].is_whitespace() {
            start += 1;
        }
    }
    let mut end = (start + LENGTH).min(chars.len());
    if end < chars.len() {
        while end > at && !chars[end].is_whitespace() {
            end -= 1;
        }
    }
    let body: String = chars[start..end].iter().collect();
    let body = body.trim();
    format!(
        "{}{body}{}",
        if start > 0 { "…" } else { "" },
        if end < chars.len() { "…" } else { "" }
    )
}

/// `/study/general-conference/2024/10/12andersen?lang=eng` →
/// `/general-conference/2024/10/12andersen`.
fn study_uri(href: &str) -> String {
    let path = href.split(['?', '#']).next().unwrap_or("");
    path.strip_prefix("/study").unwrap_or(path).to_string()
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_class(class: &str, name: &str) -> bool {
    class.split_whitespace().any(|c| c == name)
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> &'a str {
    attrs
        .iter()
        .find(|(key, _)| key == name)
        .map_or("", |(_, value)| value.as_str())
}

/// Just enough HTML for the study site's pages, which are well formed.
enum Token<'a> {
    Start {
        name: String,
        attrs: Vec<(String, String)>,
    },
    End(String),
    Text(&'a str),
}

fn tokens(html: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            out.push(Token::Text(rest));
            break;
        };
        if open > 0 {
            out.push(Token::Text(&rest[..open]));
        }
        rest = &rest[open..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        let Some(close) = tag_end(rest) else {
            break;
        };
        let inner = &rest[1..close];
        rest = &rest[close + 1..];
        if let Some(name) = inner.strip_prefix('/') {
            out.push(Token::End(name.trim().to_ascii_lowercase()));
        } else if !inner.starts_with('!') && !inner.starts_with('?') {
            let (name, attrs) = parse_tag(inner);
            let void = matches!(
                name.as_str(),
                "br" | "img" | "source" | "hr" | "meta" | "link" | "input" | "wbr"
            );
            let self_closing = inner.trim_end().ends_with('/');
            out.push(Token::Start {
                name: name.clone(),
                attrs,
            });
            if self_closing && !void {
                out.push(Token::End(name));
            }
        }
    }
    out
}

/// Index of the `>` that ends the tag starting at `text[0]`, past any
/// quoted attribute values.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in text.char_indices().skip(1) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_tag(inner: &str) -> (String, Vec<(String, String)>) {
    let inner = inner.trim_end_matches('/');
    let name_end = inner
        .find(|c: char| c.is_whitespace())
        .unwrap_or(inner.len());
    let name = inner[..name_end].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let mut rest = inner[name_end..].trim_start();
    while !rest.is_empty() {
        let key_end = rest
            .find(|c: char| c == '=' || c.is_whitespace())
            .unwrap_or(rest.len());
        let key = rest[..key_end].to_ascii_lowercase();
        rest = rest[key_end..].trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (raw, remaining) = match after.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let body = &after[1..];
                    let end = body.find(q).unwrap_or(body.len());
                    (&body[..end], body.get(end + 1..).unwrap_or(""))
                }
                _ => {
                    let end = after
                        .find(|c: char| c.is_whitespace())
                        .unwrap_or(after.len());
                    (&after[..end], &after[end..])
                }
            };
            value = decode(raw);
            rest = remaining.trim_start();
        }
        if !key.is_empty() {
            attrs.push((key, value));
        }
    }
    (name, attrs)
}

/// Replaces character references like `&amp;`, `&#8217;`, and `&#x2019;`.
fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest.find(';').filter(|&end| end <= 10).and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                "mdash" => Some('—'),
                "ndash" => Some('–'),
                "hellip" => Some('…'),
                "lsquo" => Some('‘'),
                "rsquo" => Some('’'),
                "ldquo" => Some('“'),
                "rdquo" => Some('”'),
                _ => {
                    let number = name.strip_prefix('#')?;
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse().ok()?,
                    };
                    char::from_u32(code)
                }
            }?;
            Some((c, end))
        });
        match decoded {
            Some((c, end)) => {
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

    const MANIFEST: &str = r##"<header><h1 class="title">October 2024 general conference</h1></header>
<nav class="manifest"><ul class="doc-map">
<li data-content-type="general-conference-session"><h2 class="label"><p class="title">Saturday Morning Session</p></h2>
<ul class="doc-map"><li><a href="/study/general-conference/2024/10/saturday-morning-session?lang=eng" class="list-tile"><p class="title">Saturday Morning Session</p><p class="description">The Saturday morning session.</p></a></li>
<li data-content-type="general-conference-talk">
<a href="/study/general-conference/2024/10/12andersen?lang=eng" class="list-tile"><p class="primaryMeta">Neil L. Andersen</p><p class="title">The Triumph of Hope</p><p class="description">Elder Andersen teaches.</p><img class="thumbnail" src="x" srcset="a 60w,b 100w" alt="a.jpg"/></a>
</li>
<li data-content-type="general-conference-talk">
<a href="/study/general-conference/2024/10/13johnson?lang=eng" class="list-tile"><p class="primaryMeta">Tamara W. Runia</p><p class="title">Seeing God&#x2019;s Family &amp; Friends</p></a>
</li></ul></li>
<li data-content-type="general-conference-session"><h2 class="label"><p class="title">Saturday Afternoon Session</p></h2>
<ul class="doc-map"><li><a href="/study/general-conference/2024/10/saturday-afternoon-session?lang=eng" class="list-tile"><p class="title">Saturday Afternoon Session</p></a></li>
<li data-content-type="general-conference-talk"><a href="/study/general-conference/2024/10/21oaks?lang=eng" class="list-tile"><p class="primaryMeta">Dallin H. Oaks</p><p class="title">Sustaining</p></a></li>
</ul></li></ul></nav>"##;

    const TALK: &str = r##"<header>
<span class="page-break" data-page="4"></span>
<video preload="none" data-video-title="The Triumph of Hope"><source src="a.m3u8" type="application/vnd.apple.mpegurl"/></video><h1 data-aid="1" id="p_x">The Triumph of Hope</h1>
<div class="byline">
<p class="author-name" id="p1">By Elder Neil L. Andersen</p>
<p class="author-role" id="p2">Of the Quorum of the Twelve Apostles</p>
<div class="image-cropper"><img alt="Elder" class="headshot" src="x"/></div>
</div>
<p class="kicker" id="k">Hope is a living gift.</p>
</header>
<div class="body-block">
<p id="p3">My dear brothers and sisters, we will feel the “guiding, directing, [and] comforting”<a class="note-ref" href="#note1"><sup class="marker" data-value="1"></sup></a> influence.</p>
<section><header><h2 id="p4">Hope in <em>Christ</em></h2></header>
<p id="p5">The word <em>hope</em> is used for many&nbsp;things.<br/>Like rain.</p>
<figure class="image"><img src="x"/><figcaption><p class="title">A caption</p></figcaption></figure>
</section>
</div>
<footer class="notes"><ol><li id="note1"><p>Russell M. Nelson, “Revelation.”</p></li></ol></footer>"##;

    #[test]
    fn manifest_lists_sessions_and_their_talks() {
        let sessions = parse_manifest(MANIFEST);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].title, "Saturday Morning Session");
        assert_eq!(
            sessions[0].talks,
            vec![
                ListedTalk {
                    uri: "/general-conference/2024/10/12andersen".into(),
                    speaker: "Neil L. Andersen".into(),
                    title: "The Triumph of Hope".into(),
                },
                ListedTalk {
                    uri: "/general-conference/2024/10/13johnson".into(),
                    speaker: "Tamara W. Runia".into(),
                    title: "Seeing God’s Family & Friends".into(),
                },
            ]
        );
        assert_eq!(sessions[1].title, "Saturday Afternoon Session");
        assert_eq!(sessions[1].talks.len(), 1);
    }

    #[test]
    fn manifest_without_content_types_still_lists_talks() {
        // 2021–2023 drop the `data-content-type` on each item.
        let untagged = MANIFEST.replace(r#" data-content-type="general-conference-session""#, "");
        let untagged = untagged.replace(r#" data-content-type="general-conference-talk""#, "");
        assert!(!untagged.contains("data-content-type"));
        assert_eq!(parse_manifest(&untagged), parse_manifest(MANIFEST));
    }

    #[test]
    fn talk_keeps_byline_and_text_but_not_notes_or_figures() {
        let listed = ListedTalk {
            uri: "/general-conference/2024/10/12andersen".into(),
            speaker: "Neil L. Andersen".into(),
            title: "The Triumph of Hope".into(),
        };
        let talk = parse_talk(&listed, TALK);
        assert_eq!(talk.title, "The Triumph of Hope");
        assert_eq!(talk.speaker, "Elder Neil L. Andersen");
        assert_eq!(talk.role, "Of the Quorum of the Twelve Apostles");
        assert_eq!(talk.kicker, "Hope is a living gift.");
        assert_eq!(
            talk.paragraphs,
            vec![
                Paragraph {
                    text: "My dear brothers and sisters, we will feel the “guiding, directing, [and] comforting” influence.".into(),
                    heading: false,
                },
                Paragraph {
                    text: "Hope in Christ".into(),
                    heading: true,
                },
                Paragraph {
                    text: "The word hope is used for many things. Like rain.".into(),
                    heading: false,
                },
            ]
        );
    }

    #[test]
    fn talk_without_a_byline_uses_the_listing() {
        let listed = ListedTalk {
            uri: "/general-conference/2024/10/21oaks".into(),
            speaker: "Dallin H. Oaks".into(),
            title: "Sustaining".into(),
        };
        let talk = parse_talk(
            &listed,
            r#"<div class="body-block"><p>It is proposed.</p></div>"#,
        );
        assert_eq!(talk.title, "Sustaining");
        assert_eq!(talk.speaker, "Dallin H. Oaks");
        assert_eq!(talk.paragraphs.len(), 1);
    }

    #[test]
    fn conferences_run_from_1971_to_the_latest_newest_first() {
        let all = Conference::all_through(Conference::april(2026));
        assert_eq!(all.first(), Some(&Conference::april(2026)));
        assert_eq!(all[1], Conference::october(2025));
        assert_eq!(all.last(), Some(&FIRST));
        assert_eq!(all.len(), (2025 - 1971 + 1) * 2 + 1);
        assert_eq!(
            Conference::october(2024).uri(),
            "/general-conference/2024/10"
        );
        assert_eq!(
            Conference::from_key("1999-04"),
            Some(Conference::april(1999))
        );
        assert_eq!(Conference::from_key("1999-05"), None);
        assert_eq!(Conference::october(1999).label(), "October 1999");
    }

    #[test]
    fn latest_follows_the_calendar() {
        let at = |days: u64| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(days * 86_400);
        // 2024-03-31, 2024-04-01, 2024-10-08.
        assert_eq!(Conference::latest(at(19_813)), Conference::october(2023));
        assert_eq!(Conference::latest(at(19_814)), Conference::april(2024));
        assert_eq!(Conference::latest(at(20_004)), Conference::october(2024));
    }

    fn library() -> Vec<ConferenceTalks> {
        let talk = |title: &str, speaker: &str, paragraphs: &[&str]| Talk {
            uri: format!("/general-conference/2024/10/{title}"),
            title: title.into(),
            speaker: speaker.into(),
            role: String::new(),
            kicker: format!("About {title}"),
            paragraphs: paragraphs
                .iter()
                .map(|text| Paragraph {
                    text: text.to_string(),
                    heading: false,
                })
                .collect(),
        };
        vec![ConferenceTalks {
            conference: Conference::october(2024),
            sessions: vec![Session {
                title: "Saturday Morning Session".into(),
                talks: vec![
                    talk(
                        "Faith in Christ",
                        "Neil L. Andersen",
                        &["We speak of faith.", "The Lord’s hope is bright and sure."],
                    ),
                    talk(
                        "Covenants",
                        "Dale G. Renlund",
                        &["Faith and hope go together.", "Keep covenants."],
                    ),
                ],
            }],
        }]
    }

    #[test]
    fn search_finds_titles_first_then_paragraphs() {
        let library = library();
        let hits = search(&library, "faith", 50);
        assert_eq!(hits[0].place, Place::Talk);
        assert_eq!(
            (hits[0].talk, hits[0].snippet.as_str()),
            (0, "About Faith in Christ")
        );
        assert_eq!(hits[1].place, Place::Paragraph(0));
        assert_eq!(hits[2].talk, 1);
        assert_eq!(hits.len(), 3);
        assert_eq!(search(&library, "faith", 2).len(), 2);
    }

    #[test]
    fn every_word_must_match_and_quotes_match_a_phrase() {
        let library = library();
        let both = search(&library, "hope faith", 50);
        assert_eq!(both.len(), 1);
        assert_eq!((both[0].talk, &both[0].place), (1, &Place::Paragraph(0)));
        assert!(search(&library, "\"hope faith\"", 50).is_empty());
        assert_eq!(search(&library, "\"faith and hope\"", 50).len(), 1);
        // Straight quotes find curly ones.
        assert_eq!(search(&library, "lord's", 50)[0].place, Place::Paragraph(1));
        assert_eq!(search(&library, "renlund", 50)[0].place, Place::Talk);
        assert!(search(&library, "   ", 50).is_empty());
    }

    #[test]
    fn snippets_are_cut_on_word_breaks_around_the_match() {
        let long = format!("{} needle {}", "word ".repeat(40), "tail ".repeat(40));
        let at = long.find("needle").unwrap();
        let snip = snippet(&long, at);
        assert!(snip.starts_with('…') && snip.ends_with('…'));
        assert!(snip.contains("needle"));
        assert!(snip.chars().count() <= 152);
        assert_eq!(snippet("short text", 0), "short text");
    }

    #[test]
    fn saved_conferences_round_trip_as_json() {
        let library = library();
        let json = serde_json::to_string(&library[0]).unwrap();
        assert!(!json.contains("heading"));
        let back: ConferenceTalks = serde_json::from_str(&json).unwrap();
        assert_eq!(back, library[0]);
    }

    #[test]
    fn decode_handles_named_numeric_and_stray_ampersands() {
        assert_eq!(
            decode("a &amp; b &#8217; &#x201C; &bogus; & c"),
            "a & b ’ “ &bogus; & c"
        );
    }
}
