//! Latter-day Saint scriptures, bundled from `data/lds-scriptures.json`.
//!
//! [`search`] fuzzy-matches a reference (`Ether 2:1-4`, `1 Nephi 1:11`, `D&C 4:1`)
//! or words from the verse. A hit's [`Hit::label`] is what a citation shows
//! in place of a number.

use std::sync::OnceLock;

use serde::Deserialize;

/// One passage a person can cite: a verse, or a verse range in one chapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Superscript text, e.g. `1 Nephi 1:11` or `Ether 2:1-4`.
    pub label: String,
    /// Footnote body: the verse text, then the volume.
    pub text: String,
    /// Short verse text for the picker row.
    pub preview: String,
}

#[derive(Deserialize)]
struct RawVerse {
    volume_title: String,
    book_title: String,
    book_short_title: String,
    chapter_number: u16,
    verse_number: u16,
    scripture_text: String,
}

struct Verse {
    chapter: u16,
    number: u16,
    text: String,
    text_lower: String,
    title_lower: String,
    title_compact: String,
}

struct Book {
    title: String,
    volume: String,
    /// Lowercase names and abbreviations this book matches.
    aliases: Vec<String>,
    /// Index range into [`Catalog::verses`].
    start: usize,
    end: usize,
    /// Highest chapter number. Chapters in the bundled text run `1..=chapters`.
    chapters: u16,
}

struct Catalog {
    books: Vec<Book>,
    verses: Vec<Verse>,
}

enum Spec {
    Book,
    Chapter(u16),
    Verses { chapter: u16, start: u16, end: u16 },
    Rejected,
}

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(Catalog::load)
}

impl Catalog {
    fn load() -> Self {
        let raw: Vec<RawVerse> = serde_json::from_str(include_str!("../data/lds-scriptures.json"))
            .expect("bundled lds-scriptures.json");
        let mut books: Vec<Book> = Vec::new();
        let mut verses = Vec::with_capacity(raw.len());
        for item in raw {
            let new_book = books
                .last()
                .is_none_or(|book| book.title != item.book_title);
            if new_book {
                if let Some(book) = books.last_mut() {
                    close_book(book, &verses);
                }
                books.push(Book::new(
                    item.book_title,
                    item.book_short_title,
                    item.volume_title,
                    verses.len(),
                ));
            }
            let book = books.last().expect("book");
            let title = format!(
                "{} {}:{}",
                book.title, item.chapter_number, item.verse_number
            );
            let title_lower = title.to_lowercase();
            verses.push(Verse {
                chapter: item.chapter_number,
                number: item.verse_number,
                text_lower: item.scripture_text.to_lowercase(),
                text: item.scripture_text,
                title_compact: compact(&title_lower),
                title_lower,
            });
        }
        if let Some(book) = books.last_mut() {
            close_book(book, &verses);
        }
        Self { books, verses }
    }

    fn search(&self, query: &str) -> Vec<Hit> {
        let query = normalize(query);
        if query.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        if let Some((book, spec)) = self.parse_ref(&query) {
            self.structured_hits(book, spec, &mut hits);
        }
        if hits.is_empty() {
            self.fuzzy_hits(&query, &mut hits);
        }
        hits.truncate(40);
        hits
    }

    fn structured_hits(&self, book: usize, spec: Spec, hits: &mut Vec<Hit>) {
        match spec {
            Spec::Rejected | Spec::Book => {}
            Spec::Chapter(chapter) => {
                for number in self.verse_numbers(book, chapter) {
                    self.push_verse(hits, book, chapter, number);
                    if hits.len() >= 40 {
                        break;
                    }
                }
            }
            Spec::Verses {
                chapter,
                start,
                end,
            } => {
                if end > start {
                    self.push_range(hits, book, chapter, start, end);
                }
                let mut numbers: Vec<u16> = (start..=end).collect();
                if start == end {
                    let before = start.saturating_sub(2);
                    numbers.extend(before..start);
                    numbers.extend(start + 1..=start.saturating_add(6));
                }
                for number in numbers {
                    self.push_verse(hits, book, chapter, number);
                }
            }
        }
    }

    fn push_range(&self, hits: &mut Vec<Hit>, book: usize, chapter: u16, start: u16, end: u16) {
        let mut indexes = Vec::new();
        for number in start..=end {
            let Some(index) = self.find_verse(book, chapter, number) else {
                break;
            };
            indexes.push(index);
            if indexes.len() >= 80 {
                break;
            }
        }
        if indexes.len() < 2 {
            return;
        }
        let end = self.verses[*indexes.last().expect("verse")].number;
        hits.push(self.hit(book, chapter, start, end, &indexes));
    }

    fn push_verse(&self, hits: &mut Vec<Hit>, book: usize, chapter: u16, number: u16) {
        let Some(index) = self.find_verse(book, chapter, number) else {
            return;
        };
        let label = verse_label(&self.books[book].title, chapter, number, number);
        if hits.iter().any(|hit| hit.label == label) {
            return;
        }
        hits.push(self.hit(book, chapter, number, number, &[index]));
    }

    fn hit(&self, book: usize, chapter: u16, start: u16, end: u16, indexes: &[usize]) -> Hit {
        let book = &self.books[book];
        let label = verse_label(&book.title, chapter, start, end);
        let body = if indexes.len() == 1 {
            self.verses[indexes[0]].text.clone()
        } else {
            indexes
                .iter()
                .map(|&index| {
                    let verse = &self.verses[index];
                    format!("{} {}", verse.number, verse.text)
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        let preview = preview(&self.verses[indexes[0]].text);
        Hit {
            label,
            text: format!("{body} — *{}*", book.volume),
            preview,
        }
    }

    fn find_verse(&self, book: usize, chapter: u16, number: u16) -> Option<usize> {
        let book = &self.books[book];
        let verses = &self.verses[book.start..book.end];
        let offset = verses
            .binary_search_by(|verse| (verse.chapter, verse.number).cmp(&(chapter, number)))
            .ok()?;
        Some(book.start + offset)
    }

    fn verse_numbers(&self, book: usize, chapter: u16) -> Vec<u16> {
        let book = &self.books[book];
        self.verses[book.start..book.end]
            .iter()
            .filter(|verse| verse.chapter == chapter)
            .map(|verse| verse.number)
            .collect()
    }

    fn parse_ref(&self, query: &str) -> Option<(usize, Spec)> {
        if let Some((fragment, rest)) = split_ref(query) {
            if let Some(index) = self.book_match(fragment) {
                let spec = parse_spec(rest);
                if !matches!(spec, Spec::Rejected) {
                    return Some((index, spec));
                }
            }
        }
        self.prefix_ref(query)
    }

    /// Longest book alias that begins `query`, with whatever reference follows.
    fn prefix_ref(&self, query: &str) -> Option<(usize, Spec)> {
        let compact_query = compact(query);
        let mut best: Option<(usize, usize, Spec)> = None;
        for (index, book) in self.books.iter().enumerate() {
            for alias in &book.aliases {
                let mut consider = |len: usize, rest: &str| {
                    let spec = parse_spec(rest);
                    if matches!(spec, Spec::Rejected) {
                        return;
                    }
                    if best.as_ref().is_none_or(|(have, _, _)| len > *have) {
                        best = Some((len, index, spec));
                    }
                };
                if let Some((len, rest)) = prefix_rest(query, alias) {
                    consider(len, rest);
                }
                if let Some((len, rest)) = prefix_rest(&compact_query, alias) {
                    consider(len, rest);
                }
            }
        }
        best.map(|(_, index, spec)| (index, spec))
    }

    fn book_match(&self, fragment: &str) -> Option<usize> {
        let fragment = fragment.trim();
        let frag_compact = compact(fragment);
        if frag_compact
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .count()
            < 2
        {
            return None;
        }
        // score, alias length (shorter wins ties), book index
        let mut best: Option<(i32, usize, usize)> = None;
        let mut tie = false;
        for (index, book) in self.books.iter().enumerate() {
            let mut local: Option<(i32, usize)> = None;
            for alias in &book.aliases {
                let score = alias_score(alias, fragment, &frag_compact);
                if score == 0 {
                    continue;
                }
                let better = local
                    .is_none_or(|(have, len)| score > have || (score == have && alias.len() < len));
                if better {
                    local = Some((score, alias.len()));
                }
            }
            let Some((score, len)) = local else {
                continue;
            };
            match best {
                None => best = Some((score, len, index)),
                Some((have, have_len, _)) if score > have || (score == have && len < have_len) => {
                    best = Some((score, len, index));
                    tie = false;
                }
                Some((have, have_len, _)) if score == have && len == have_len => tie = true,
                _ => {}
            }
        }
        if tie {
            None
        } else {
            best.map(|(_, _, index)| index)
        }
    }

    fn fuzzy_hits(&self, query: &str, hits: &mut Vec<Hit>) {
        let compact_query = compact(query);
        let words: Vec<&str> = query
            .split_whitespace()
            .filter(|word| word.len() >= 3)
            .collect();
        let mut scored = Vec::new();
        for (index, verse) in self.verses.iter().enumerate() {
            let score = fuzzy_score(query, &compact_query, &words, verse);
            if score > 0 {
                scored.push((score, index));
            }
        }
        scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, index) in scored.into_iter().take(40) {
            let verse = &self.verses[index];
            let book = self
                .books
                .iter()
                .position(|book| (book.start..book.end).contains(&index))
                .expect("verse belongs to a book");
            self.push_verse(hits, book, verse.chapter, verse.number);
        }
    }
}

fn fuzzy_score(query: &str, compact_query: &str, words: &[&str], verse: &Verse) -> i32 {
    if verse.title_lower == query || verse.title_compact == compact_query {
        return 1000;
    }
    if verse.title_lower.starts_with(query) || verse.title_compact.starts_with(compact_query) {
        return 900;
    }
    if compact_query.len() >= 3 && verse.title_compact.contains(compact_query) {
        return 800;
    }
    if query.len() >= 3 && verse.text_lower.contains(query) {
        return 600;
    }
    if words.len() >= 2 && words.iter().all(|word| verse.text_lower.contains(word)) {
        return 400;
    }
    0
}

fn verse_label(book: &str, chapter: u16, start: u16, end: u16) -> String {
    if start == end {
        format!("{book} {chapter}:{start}")
    } else {
        format!("{book} {chapter}:{start}-{end}")
    }
}

fn preview(text: &str) -> String {
    let mut chars = text.chars();
    let shown: String = chars.by_ref().take(140).collect();
    if chars.next().is_some() {
        format!("{shown}…")
    } else {
        shown
    }
}

impl Book {
    fn new(title: String, short: String, volume: String, start: usize) -> Self {
        let mut aliases = Vec::new();
        add_alias(&mut aliases, &title.to_lowercase());
        let short = short.trim().trim_end_matches('.').to_lowercase();
        add_alias(&mut aliases, &short);
        add_alias(&mut aliases, &initials(&title));
        // Abbreviations that aren't a prefix of the name (John is "Jn.", not "Jo.").
        for alias in extra_aliases(&title) {
            add_alias(&mut aliases, alias);
        }
        if short.contains('&') {
            add_alias(&mut aliases, &short.replace('&', "and"));
            add_alias(&mut aliases, &short.replace('&', ""));
        }
        Self {
            title,
            volume,
            aliases,
            start,
            end: start,
            chapters: 0,
        }
    }
}

fn close_book(book: &mut Book, verses: &[Verse]) {
    book.end = verses.len();
    book.chapters = verses[book.start..book.end]
        .last()
        .map(|verse| verse.chapter)
        .unwrap_or(0);
}

fn extra_aliases(title: &str) -> &'static [&'static str] {
    match title {
        "John" => &["jn"],
        "Jonah" => &["jon"],
        "Joel" => &["jl"],
        "Job" => &["jb"],
        "James" => &["jas"],
        "Psalms" => &["psalm", "psalms"],
        "Song of Solomon" => &["song of songs"],
        "Doctrine and Covenants" => &["d and c"],
        "Joseph Smith--History" => &["joseph smith history"],
        "Joseph Smith--Matthew" => &["joseph smith matthew"],
        _ => &[],
    }
}

fn initials(title: &str) -> String {
    title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .filter_map(|word| word.chars().next())
        .collect::<String>()
        .to_lowercase()
}

fn add_alias(out: &mut Vec<String>, alias: &str) {
    let alias = alias.trim().to_lowercase();
    if alias.chars().filter(|c| c.is_ascii_alphanumeric()).count() < 2 {
        return;
    }
    for form in [alias.clone(), compact(&alias), alnum(&alias)] {
        if form.chars().filter(|c| c.is_ascii_alphanumeric()).count() >= 2 && !out.contains(&form) {
            out.push(form);
        }
    }
}

fn alnum(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn normalize(query: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    for c in query.trim().chars() {
        let c = match c {
            '—' | '–' => '-',
            c if c.is_whitespace() => ' ',
            c => c.to_ascii_lowercase(),
        };
        if c == ' ' {
            pending_space = true;
            continue;
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        out.push(c);
    }
    out.trim_end_matches('.').to_string()
}

/// `book` and the `chapter:verse` tail, split on the last colon.
fn split_ref(query: &str) -> Option<(&str, &str)> {
    let colon = query.rfind(':')?;
    let before = &query[..colon];
    let after = &query[colon + 1..];
    if after.is_empty() || !after.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let mut chapter_start = before.len();
    for (index, c) in before.char_indices().rev() {
        if c.is_ascii_digit() {
            chapter_start = index;
        } else {
            break;
        }
    }
    if chapter_start == before.len() {
        return None;
    }
    let book = before[..chapter_start].trim_end();
    if book.is_empty() {
        return None;
    }
    Some((book, &query[chapter_start..]))
}

fn prefix_rest<'a>(query: &'a str, alias: &str) -> Option<(usize, &'a str)> {
    let rest = query.strip_prefix(alias)?;
    if rest.is_empty() {
        return Some((alias.len(), rest));
    }
    let next = rest.chars().next()?;
    (next.is_whitespace() || next.is_ascii_digit() || next == ':')
        .then(|| (alias.len(), rest.trim_start()))
}

fn alias_score(alias: &str, fragment: &str, frag_compact: &str) -> i32 {
    let alias_compact = compact(alias);
    if alias == fragment || alias_compact == frag_compact {
        1000
    } else if fragment.len() >= 2
        && (alias.starts_with(fragment) || alias_compact.starts_with(frag_compact))
    {
        800
    } else if fragment.len() >= 4
        && (alias.contains(fragment) || alias_compact.contains(frag_compact))
    {
        500
    } else {
        0
    }
}

fn parse_spec(rest: &str) -> Spec {
    let rest = rest.trim().trim_end_matches('.');
    if rest.is_empty() {
        return Spec::Book;
    }
    let mut i = 0;
    let Some(chapter) = take_number(rest, &mut i) else {
        return Spec::Rejected;
    };
    skip_ws(rest, &mut i);
    if i >= rest.len() {
        return Spec::Chapter(chapter);
    }
    if rest.as_bytes()[i] == b':' {
        i += 1;
        skip_ws(rest, &mut i);
        if i >= rest.len() {
            return Spec::Chapter(chapter);
        }
    } else if !rest.as_bytes()[i].is_ascii_digit() {
        return Spec::Rejected;
    }
    let Some(start) = take_number(rest, &mut i) else {
        return Spec::Rejected;
    };
    skip_ws(rest, &mut i);
    if i >= rest.len() {
        return Spec::Verses {
            chapter,
            start,
            end: start,
        };
    }
    let dash = rest[i..].chars().next();
    if matches!(dash, Some('-' | '–' | '—')) {
        i += dash.expect("dash").len_utf8();
        skip_ws(rest, &mut i);
        if i >= rest.len() {
            return Spec::Verses {
                chapter,
                start,
                end: start,
            };
        }
        let Some(end) = take_number(rest, &mut i) else {
            return Spec::Rejected;
        };
        skip_ws(rest, &mut i);
        if i == rest.len() && end >= start {
            return Spec::Verses {
                chapter,
                start,
                end,
            };
        }
    }
    Spec::Rejected
}

fn take_number(text: &str, i: &mut usize) -> Option<u16> {
    let bytes = text.as_bytes();
    if *i >= bytes.len() || !bytes[*i].is_ascii_digit() {
        return None;
    }
    let mut n: u32 = 0;
    while *i < bytes.len() && bytes[*i].is_ascii_digit() {
        n = n
            .saturating_mul(10)
            .saturating_add((bytes[*i] - b'0') as u32);
        *i += 1;
        if n > 400 {
            return None;
        }
    }
    (n > 0).then_some(n as u16)
}

fn skip_ws(text: &str, i: &mut usize) {
    let bytes = text.as_bytes();
    while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
        *i += 1;
    }
}

/// Passages matching `query`, best first. An empty query matches nothing.
pub fn search(query: &str) -> Vec<Hit> {
    catalog().search(query)
}

/// One verse found by [`find_verses`], best matches first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerseHit {
    pub book: String,
    pub chapter: u16,
    pub number: u16,
    pub label: String,
    /// Short verse text for a result row.
    pub preview: String,
}

/// The best scripture matches for `query`, at most ten.
///
/// A reference (`Ether 2:1`, `D&C 4`) wins. Otherwise the words are matched
/// against the verse text, so a partial phrase still finds the verse.
pub fn find_verses(query: &str) -> Vec<VerseHit> {
    const LIMIT: usize = 10;
    search(query)
        .into_iter()
        .filter_map(|hit| {
            let (book, chapter, number) = locate(&hit.label)?;
            Some(VerseHit {
                book,
                chapter,
                number,
                label: hit.label,
                preview: hit.preview,
            })
        })
        .take(LIMIT)
        .collect()
}

/// `Genesis 1:1` or `Ether 2:1-4` into book, chapter, and the first verse.
fn locate(label: &str) -> Option<(String, u16, u16)> {
    let (head, verses) = label.rsplit_once(':')?;
    let number: u16 = verses.split(['-', '–', '—']).next()?.trim().parse().ok()?;
    let (book, chapter) = head.rsplit_once(' ')?;
    let chapter: u16 = chapter.parse().ok()?;
    if book.is_empty() {
        return None;
    }
    Some((book.to_string(), chapter, number))
}

/// One volume in standard order, with the books that belong to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub title: String,
    pub books: Vec<BookSummary>,
}

/// A book someone can open and read chapter by chapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookSummary {
    pub title: String,
    pub volume: String,
    /// Highest chapter (or section) number. Numbers run from 1.
    pub chapters: u16,
}

/// One verse of a chapter, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerseText {
    pub number: u16,
    pub text: String,
}

/// Every verse of one chapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Passage {
    pub book: String,
    pub volume: String,
    pub number: u16,
    /// How many chapters the book has, so the reader can move to the next one.
    pub count: u16,
    pub verses: Vec<VerseText>,
}

/// Volumes and their books, in the order the scriptures are usually printed.
pub fn volumes() -> Vec<Volume> {
    volumes_matching("")
}

/// Volumes, keeping only books whose name or abbreviation matches `query`.
/// An empty query keeps every book. Volumes with nothing left are dropped.
pub fn volumes_matching(query: &str) -> Vec<Volume> {
    let query = normalize(query);
    let mut out: Vec<Volume> = Vec::new();
    for book in &catalog().books {
        if !book_matches(book, &query) {
            continue;
        }
        if out.last().is_none_or(|volume| volume.title != book.volume) {
            out.push(Volume {
                title: book.volume.clone(),
                books: Vec::new(),
            });
        }
        out.last_mut().expect("volume").books.push(summarize(book));
    }
    out
}

fn book_matches(book: &Book, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let title = book.title.to_lowercase();
    if title.contains(query) {
        return true;
    }
    let compact_query = compact(query);
    if !compact_query.is_empty() && compact(&title).contains(&compact_query) {
        return true;
    }
    book.aliases.iter().any(|alias| {
        alias_score(alias, query, &compact_query) > 0
            || (!compact_query.is_empty() && alias.contains(&compact_query))
    })
}

/// One book, by its full title (`Genesis`, `Doctrine and Covenants`).
pub fn book(title: &str) -> Option<BookSummary> {
    catalog()
        .books
        .iter()
        .find(|book| book.title == title)
        .map(summarize)
}

/// Every verse of `number` in `title`, or `None` when the book or chapter is missing.
pub fn chapter(title: &str, number: u16) -> Option<Passage> {
    let catalog = catalog();
    let book = catalog.books.iter().find(|item| item.title == title)?;
    if number == 0 || number > book.chapters {
        return None;
    }
    let verses: Vec<VerseText> = catalog.verses[book.start..book.end]
        .iter()
        .filter(|verse| verse.chapter == number)
        .map(|verse| VerseText {
            number: verse.number,
            text: verse.text.clone(),
        })
        .collect();
    if verses.is_empty() {
        return None;
    }
    Some(Passage {
        book: book.title.clone(),
        volume: book.volume.clone(),
        number,
        count: book.chapters,
        verses,
    })
}

fn summarize(book: &Book) -> BookSummary {
    BookSummary {
        title: book.title.clone(),
        volume: book.volume.clone(),
        chapters: book.chapters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_nothing() {
        assert!(search("   ").is_empty());
    }

    #[test]
    fn exact_verse_and_range_use_the_reference_as_the_label() {
        let nephi = &search("1 Nephi 1:11")[0];
        assert_eq!(nephi.label, "1 Nephi 1:11");
        assert!(
            nephi.text.contains("gave unto him a book"),
            "{}",
            nephi.text
        );
        assert!(nephi.text.contains("*Book of Mormon*"), "{}", nephi.text);

        let ether = &search("ether 2:1-4")[0];
        assert_eq!(ether.label, "Ether 2:1-4");
        assert!(ether.text.contains("Jared"), "{}", ether.text);
        assert!(ether.text.contains("Nimrod"), "{}", ether.text);
        assert!(ether.text.starts_with("1 And it came"), "{}", ether.text);
    }

    #[test]
    fn abbreviations_and_verse_words_match() {
        assert_eq!(search("1 ne 1:11")[0].label, "1 Nephi 1:11");
        assert_eq!(search("dc 4:1")[0].label, "Doctrine and Covenants 4:1");
        assert_eq!(search("d&c 4:1")[0].label, "Doctrine and Covenants 4:1");
        assert_eq!(search("jn 3:16")[0].label, "John 3:16");
        assert!(
            search("jn 3:16")[0]
                .text
                .contains("For God so loved the world"),
            "{}",
            search("jn 3:16")[0].text
        );
        assert_eq!(
            search("in the beginning god created")[0].label,
            "Genesis 1:1"
        );
        assert_eq!(search("js-h 1:2")[0].label, "Joseph Smith--History 1:2");
    }

    #[test]
    fn a_chapter_lists_its_verses() {
        let hits = search("Ether 2");
        assert_eq!(hits[0].label, "Ether 2:1");
        assert!(hits.iter().any(|hit| hit.label == "Ether 2:4"));
    }

    #[test]
    fn volumes_list_every_book_in_print_order() {
        let volumes = volumes();
        assert_eq!(
            volumes
                .iter()
                .map(|volume| volume.title.as_str())
                .collect::<Vec<_>>(),
            [
                "Old Testament",
                "New Testament",
                "Book of Mormon",
                "Doctrine and Covenants",
                "Pearl of Great Price",
            ]
        );
        assert_eq!(volumes[0].books.len(), 39);
        assert_eq!(volumes[0].books[0].title, "Genesis");
        assert_eq!(volumes[0].books[0].chapters, 50);
        assert_eq!(volumes[2].books.len(), 15);
        assert_eq!(volumes[3].books[0].chapters, 138);
        assert_eq!(book("Obadiah").unwrap().chapters, 1);
        assert!(book("Missing").is_none());
    }

    #[test]
    fn chapter_returns_the_whole_chapter() {
        let genesis = chapter("Genesis", 1).unwrap();
        assert_eq!(genesis.volume, "Old Testament");
        assert_eq!(genesis.count, 50);
        assert_eq!(genesis.verses.len(), 31);
        assert_eq!(genesis.verses[0].number, 1);
        assert_eq!(
            genesis.verses[0].text,
            "In the beginning God created the heaven and the earth."
        );
        assert_eq!(
            genesis.verses[30].text,
            "And God saw every thing that he had made, and, behold, it was very good. And the evening and the morning were the sixth day."
        );
        assert!(chapter("Genesis", 51).is_none());
        assert!(chapter("Missing", 1).is_none());

        let found = find_verses("in the beginning god created");
        assert_eq!(found[0].label, "Genesis 1:1");
        assert_eq!(found[0].book, "Genesis");
        assert_eq!(found[0].chapter, 1);
        assert_eq!(found[0].number, 1);
        assert_eq!(find_verses("and the").len(), 10);
        assert!(find_verses("   ").is_empty());
        assert_eq!(
            find_verses("js-h 1:2")[0].label,
            "Joseph Smith--History 1:2"
        );

        let section = chapter("Doctrine and Covenants", 4).unwrap();
        assert_eq!(section.verses.len(), 7);
        assert!(section.verses[0].text.contains("a marvelous work"));
    }

    #[test]
    fn a_book_filter_matches_names_and_abbreviations() {
        assert_eq!(volumes_matching("").len(), 5);
        let genesis = volumes_matching("gen");
        assert_eq!(genesis.len(), 1);
        assert_eq!(genesis[0].books.len(), 1);
        assert_eq!(genesis[0].books[0].title, "Genesis");
        assert!(volumes_matching("zzzz").is_empty());
        let covenants = volumes_matching("dc");
        assert_eq!(covenants[0].books[0].title, "Doctrine and Covenants");
        assert!(volumes_matching("nephi")
            .iter()
            .flat_map(|volume| &volume.books)
            .any(|book| book.title == "1 Nephi"));
    }
}
