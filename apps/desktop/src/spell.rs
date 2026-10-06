//! Spell checking for notes.
//!
//! Misspelled words are the ones macOS does not know, minus words ignored
//! for this session and words added to the dictionary. Added words are
//! written to the app's dictionary file and learned by the system spelling
//! dictionary. "Ignore All" lasts until the app quits.
//!
//! The system dictionary runs on its own thread. The editor asks for words
//! and paints whatever is already known; a finished check asks egui to draw
//! again. A red line waits until the word is closed by a space, tab, or
//! newline, so it does not appear under the word still being typed.

#[cfg(any(test, target_os = "macos"))]
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::{mpsc, Arc, Mutex};
#[cfg(target_os = "macos")]
use std::thread;
#[cfg(target_os = "macos")]
use std::time::Duration;

use eframe::egui::{self, pos2, Color32, Galley, Id, Painter, Pos2, Stroke};
use scripture_study_core::spell::checkable_words;

use crate::icons;
use crate::link_menu::{self, LinkTarget};
use crate::menu::{self, Item};
use crate::theme::Palette;

const MAX_SUGGESTIONS: usize = 5;
const SQUIGGLE: Color32 = Color32::from_rgb(0xEB, 0x57, 0x57);

/// What the user picked from a misspelled word's menu.
pub enum SpellChoice {
    Replace(String),
    IgnoreAll,
    AddToDictionary,
    Link(link_menu::Choice),
    Mark(crate::marks::Choice),
}

/// The right-click menu for one misspelled word.
pub struct SpellMenu {
    pub block: usize,
    pub range: Range<usize>,
    pub word: String,
    suggestions: Vec<String>,
    pub link: Option<LinkTarget>,
    pos: Pos2,
    just_opened: bool,
}

impl SpellMenu {
    pub fn new(
        block: usize,
        range: Range<usize>,
        word: String,
        suggestions: Vec<String>,
        pos: Pos2,
        link: Option<LinkTarget>,
    ) -> Self {
        let suggestions = cased_suggestions(&word, &suggestions);
        Self {
            block,
            range,
            word,
            suggestions,
            link,
            pos,
            just_opened: true,
        }
    }

    /// Draws the menu. Returns the chosen item, and whether it stays open.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        palette: &Palette,
        highlight: Option<u32>,
        underline: Option<u32>,
    ) -> (Option<SpellChoice>, bool) {
        let (choice, open) = menu::menu_at(
            ctx,
            Id::new("spell-menu"),
            self.pos,
            palette,
            std::mem::take(&mut self.just_opened),
            |ui| {
                for suggestion in &self.suggestions {
                    if Item::new(suggestion).show(ui, palette) {
                        return Some(SpellChoice::Replace(suggestion.clone()));
                    }
                }
                if !self.suggestions.is_empty() {
                    menu::separator(ui, palette);
                }
                if Item::new("Ignore All").icon(icons::close).show(ui, palette) {
                    return Some(SpellChoice::IgnoreAll);
                }
                if Item::new("Add to Dictionary")
                    .icon(icons::book)
                    .show(ui, palette)
                {
                    return Some(SpellChoice::AddToDictionary);
                }
                if let Some(link) = &self.link {
                    menu::separator(ui, palette);
                    if let Some(choice) = link_menu::items(ui, palette, link.is_bare()) {
                        return Some(SpellChoice::Link(choice));
                    }
                }
                menu::separator(ui, palette);
                if let Some(choice) = crate::marks::items(ui, palette, highlight, underline) {
                    return Some(SpellChoice::Mark(choice));
                }
                None
            },
        );
        let open = open && choice.is_none();
        (choice, open)
    }
}

/// Rewrite `suggestion` in the same capitals as `source` ("helo" stays
/// "hello", "Helo" becomes "Hello", "HELO" becomes "HELLO").
pub fn match_case(source: &str, suggestion: &str) -> String {
    let letters: Vec<char> = source.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|c| c.is_uppercase()) {
        return suggestion.to_uppercase();
    }
    if letters.first().is_some_and(|c| c.is_uppercase()) {
        let mut chars = suggestion.chars();
        let Some(first) = chars.next() else {
            return suggestion.to_string();
        };
        let mut out: String = first.to_uppercase().collect();
        out.extend(chars.flat_map(|c| c.to_lowercase()));
        return out;
    }
    suggestion.to_string()
}

fn cased_suggestions(word: &str, suggestions: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for suggestion in suggestions {
        let cased = match_case(word, suggestion);
        let same = cased.to_lowercase() == word.to_lowercase();
        if !same && !out.iter().any(|have| have == &cased) {
            out.push(cased);
        }
        if out.len() == MAX_SUGGESTIONS {
            break;
        }
    }
    out
}

/// Red waves under each misspelled word. `ranges` are byte ranges in `text`.
pub fn paint_squiggles(
    painter: &Painter,
    galley: &Galley,
    origin: Pos2,
    text: &str,
    ranges: &[Range<usize>],
    text_height: f32,
) {
    for range in ranges {
        let from = scripture_study_core::editor::byte_to_char(text, range.start);
        let to = scripture_study_core::editor::byte_to_char(text, range.end);
        paint_word(painter, galley, origin, from, to, text_height, SQUIGGLE);
    }
}

fn paint_word(
    painter: &Painter,
    galley: &Galley,
    origin: Pos2,
    from: usize,
    to: usize,
    text_height: f32,
    color: Color32,
) {
    let mut row_start = 0;
    for row in &galley.rows {
        let row_end = row_start + row.char_count_excluding_newline().0;
        if from < row_end && to > row_start {
            let lo = from.max(row_start);
            let hi = to.min(row_end);
            let x0 = origin.x + row.pos.x + row.x_offset((lo - row_start).into());
            let x1 = origin.x + row.pos.x + row.x_offset((hi - row_start).into());
            if x1 - x0 > 0.5 {
                let row_h = row.max_y() - row.min_y();
                let y = origin.y + row.min_y() + text_height.min(row_h) - 1.0;
                squiggle(painter, x0, x1, y, color);
            }
        }
        row_start += row.char_count_including_newline().0;
    }
}

/// A space, tab, or newline after `end` closes the word. Punctuation in
/// between still counts ("helo," then a space), a following letter does not.
pub fn has_break_after(text: &str, end: usize) -> bool {
    for c in text[end..].chars() {
        match c {
            ' ' | '\t' | '\n' | '\r' => return true,
            c if c.is_alphabetic() => return false,
            _ => {}
        }
    }
    false
}

/// Byte offset where the unfinished word's trailing punctuation ends.
fn composing_end(text: &str, word_end: usize) -> usize {
    let mut i = word_end;
    for c in text[word_end..].chars() {
        if c == ' ' || c == '\t' || c == '\n' || c == '\r' || c.is_alphabetic() {
            break;
        }
        i += c.len_utf8();
    }
    i
}

/// The word the caret is still typing: no space, tab, or newline has closed it.
pub fn open_word(text: &str, caret_byte: usize) -> Option<Range<usize>> {
    checkable_words(text).into_iter().find(|range| {
        !has_break_after(text, range.end)
            && caret_byte >= range.start
            && caret_byte <= composing_end(text, range.end)
    })
}

/// Whether a misspelled `range` gets a red line.
///
/// The line stays off while the caret is in a word that space, tab, and
/// newline have not closed. `committed` is that key on the frame it was
/// pressed, and after Tab, which leaves the caret in the word.
pub fn ready_to_mark(
    text: &str,
    range: &Range<usize>,
    caret_byte: Option<usize>,
    committed: bool,
) -> bool {
    if has_break_after(text, range.end) {
        return true;
    }
    let typing = caret_byte
        .is_some_and(|caret| caret >= range.start && caret <= composing_end(text, range.end));
    if typing {
        return committed;
    }
    true
}

fn squiggle(painter: &Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = false;
    while x <= x1 {
        let amp = if up { -1.0 } else { 1.0 };
        points.push(pos2(x.min(x1), y + amp));
        if x >= x1 {
            break;
        }
        x += 2.2;
        up = !up;
    }
    if points.len() >= 2 {
        painter.line(points, Stroke::new(1.0, color));
    }
}

/// The dictionaries a note is checked against.
pub struct Spelling {
    /// Words the user added, lowercased.
    user: HashSet<String>,
    /// Words ignored for this session, lowercased.
    ignored: HashSet<String>,
    path: Option<PathBuf>,
    engine: Engine,
    error: Option<String>,
}

enum Engine {
    /// Nothing is misspelled. Tests use this so notes stay quiet, and so do
    /// builds without a system spelling dictionary.
    #[cfg(any(test, not(target_os = "macos")))]
    Inert,
    /// `known` is the whole dictionary. Used by tests.
    #[cfg(test)]
    List {
        known: HashSet<String>,
        suggestions: HashMap<String, Vec<String>>,
    },
    /// macOS spelling. Every lookup runs on [`SystemChecker`]'s thread.
    #[cfg(target_os = "macos")]
    System(SystemChecker),
}

/// Owns the system spelling thread. The UI thread only reads `cache`.
#[cfg(target_os = "macos")]
struct SystemChecker {
    tx: mpsc::Sender<Msg>,
    /// Folded word → misspelled. Filled by the spelling thread.
    cache: Arc<Mutex<HashMap<String, bool>>>,
    /// Words sent and not yet in `cache`, so a frame does not send them again.
    inflight: HashSet<String>,
    /// Wakes the editor when a check finishes. Set once the UI exists.
    ctx: Option<egui::Context>,
    thread: Option<thread::JoinHandle<()>>,
}

#[cfg(target_os = "macos")]
enum Msg {
    Check {
        words: Vec<String>,
        ctx: Option<egui::Context>,
    },
    Suggest {
        word: String,
        reply: mpsc::Sender<Vec<String>>,
    },
    Ignore(String),
    Learn(String),
    #[cfg(test)]
    Ping(mpsc::Sender<()>),
    Shutdown,
}

#[cfg(target_os = "macos")]
impl SystemChecker {
    fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let for_thread = Arc::clone(&cache);
        let thread = thread::Builder::new()
            .name("spellcheck".into())
            .spawn(move || system::serve(rx, for_thread))
            .expect("spellcheck thread");
        Self {
            tx,
            cache,
            inflight: HashSet::new(),
            ctx: None,
            thread: Some(thread),
        }
    }

    fn suggest(&self, word: &str) -> Vec<String> {
        let (reply, rx) = mpsc::channel();
        if self
            .tx
            .send(Msg::Suggest {
                word: word.to_string(),
                reply,
            })
            .is_err()
        {
            return Vec::new();
        }
        rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default()
    }
}

#[cfg(target_os = "macos")]
impl Drop for SystemChecker {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Spelling {
    /// The dictionary the app checks with. Tests get a checker that flags
    /// nothing, so fixtures stay quiet.
    pub fn new() -> Self {
        #[cfg(test)]
        {
            Self::inert()
        }
        #[cfg(not(test))]
        {
            Self::system()
        }
    }

    #[cfg(test)]
    fn inert() -> Self {
        Self::from_parts(HashSet::new(), None, Engine::Inert)
    }

    /// macOS spelling, plus the words saved in the app's dictionary file.
    #[cfg_attr(test, allow(dead_code))]
    pub fn system() -> Self {
        Self::system_at(dictionary_path())
    }

    fn system_at(path: Option<PathBuf>) -> Self {
        let (user, error) = match &path {
            Some(path) => match load_words(path) {
                Ok(words) => (words, None),
                Err(e) => (
                    HashSet::new(),
                    Some(format!("Couldn't read the dictionary: {e}")),
                ),
            },
            None => (HashSet::new(), None),
        };
        let mut spelling = Self::from_parts(user, path, system_engine());
        spelling.error = error;
        spelling
    }

    /// A small dictionary for tests. `known` words are spelled correctly.
    #[cfg(test)]
    pub fn for_test(
        path: Option<PathBuf>,
        known: &[&str],
        suggestions: &[(&str, &[&str])],
    ) -> Self {
        let (user, error) = match &path {
            Some(path) => match load_words(path) {
                Ok(words) => (words, None),
                Err(e) => (
                    HashSet::new(),
                    Some(format!("Couldn't read the dictionary: {e}")),
                ),
            },
            None => (HashSet::new(), None),
        };
        let known = known.iter().map(|word| fold(word)).collect();
        let suggestions = suggestions
            .iter()
            .map(|(word, options)| {
                (
                    fold(word),
                    options.iter().map(|option| (*option).to_string()).collect(),
                )
            })
            .collect();
        let mut spelling = Self::from_parts(user, path, Engine::List { known, suggestions });
        spelling.error = error;
        spelling
    }

    fn from_parts(user: HashSet<String>, path: Option<PathBuf>, engine: Engine) -> Self {
        Self {
            user,
            ignored: HashSet::new(),
            path,
            engine,
            error: None,
        }
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    /// Remember how to wake the UI when a background check finishes.
    pub fn set_repaint(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        if let Some(worker) = self.worker_mut() {
            if worker.ctx.is_none() {
                worker.ctx = Some(ctx.clone());
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = ctx;
    }

    /// The system-dictionary worker, when this checker is using macOS.
    #[cfg(target_os = "macos")]
    fn worker(&self) -> Option<&SystemChecker> {
        match &self.engine {
            Engine::System(worker) => Some(worker),
            #[cfg(test)]
            _ => None,
        }
    }

    /// The system-dictionary worker, when this checker is using macOS.
    #[cfg(target_os = "macos")]
    fn worker_mut(&mut self) -> Option<&mut SystemChecker> {
        match &mut self.engine {
            Engine::System(worker) => Some(worker),
            #[cfg(test)]
            _ => None,
        }
    }

    /// Byte ranges of misspelled words in `text`.
    ///
    /// With the system dictionary this returns only words already checked.
    /// Anything new is sent to the spelling thread and shows up on a later frame.
    pub fn misspellings(&mut self, text: &str) -> Vec<Range<usize>> {
        #[cfg(any(test, not(target_os = "macos")))]
        if matches!(self.engine, Engine::Inert) {
            return Vec::new();
        }
        #[cfg(test)]
        if matches!(self.engine, Engine::List { .. }) {
            return self.compute(text);
        }
        #[cfg(target_os = "macos")]
        {
            return self.system_misspellings(text);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = text;
            Vec::new()
        }
    }

    /// Blocks until checks queued before this call have been cached.
    #[cfg(all(test, target_os = "macos"))]
    pub fn flush(&self) {
        let (reply, rx) = mpsc::channel();
        if self
            .worker()
            .is_some_and(|worker| worker.tx.send(Msg::Ping(reply)).is_ok())
        {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
    }

    /// Misspellings already known, queueing the rest on the spelling thread.
    #[cfg(target_os = "macos")]
    fn system_misspellings(&mut self, text: &str) -> Vec<Range<usize>> {
        let mut pending = Vec::new();
        for range in checkable_words(text) {
            let word = &text[range.clone()];
            let key = fold(word);
            if self.user.contains(&key) || self.ignored.contains(&key) {
                continue;
            }
            pending.push((range, key, word.to_string()));
        }
        let mut out = Vec::new();
        let mut done = Vec::new();
        let mut waiting = Vec::new();
        {
            let Some(worker) = self.worker() else {
                return Vec::new();
            };
            let cache = worker
                .cache
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            for (range, key, word) in pending {
                match cache.get(&key) {
                    Some(bad) => {
                        if *bad {
                            out.push(range);
                        }
                        done.push(key);
                    }
                    None => waiting.push((key, word)),
                }
            }
        }
        let Some(worker) = self.worker_mut() else {
            return out;
        };
        for key in done {
            worker.inflight.remove(&key);
        }
        let mut send = Vec::new();
        for (key, word) in waiting {
            if worker.inflight.insert(key) {
                send.push(word);
            }
        }
        if !send.is_empty() {
            let ctx = worker.ctx.clone();
            let _ = worker.tx.send(Msg::Check { words: send, ctx });
        }
        out
    }

    /// Every checkable word the test dictionary does not already accept.
    #[cfg(test)]
    fn compute(&self, text: &str) -> Vec<Range<usize>> {
        checkable_words(text)
            .into_iter()
            .filter(|range| !self.accepted(&text[range.clone()]))
            .collect()
    }

    #[cfg(test)]
    fn accepted(&self, word: &str) -> bool {
        let key = fold(word);
        if self.user.contains(&key) || self.ignored.contains(&key) {
            return true;
        }
        #[cfg(test)]
        if let Engine::List { known, .. } = &self.engine {
            return known.contains(&key);
        }
        false
    }

    /// Replacements for `word`, in the system's preferred order.
    pub fn suggestions(&self, word: &str) -> Vec<String> {
        match &self.engine {
            #[cfg(test)]
            Engine::List { suggestions, .. } => {
                suggestions.get(&fold(word)).cloned().unwrap_or_default()
            }
            #[cfg(target_os = "macos")]
            Engine::System(worker) => worker.suggest(word),
            #[cfg(any(test, not(target_os = "macos")))]
            Engine::Inert => Vec::new(),
        }
    }

    /// Stop underlining `word` everywhere until the app quits.
    pub fn ignore_all(&mut self, word: &str) {
        self.ignored.insert(fold(word));
        #[cfg(target_os = "macos")]
        {
            if let Some(worker) = self.worker() {
                let _ = worker.tx.send(Msg::Ignore(word.to_string()));
            }
        }
    }

    /// Remember `word` as spelled correctly, including after a restart.
    pub fn add(&mut self, word: &str) {
        let key = fold(word);
        if !self.user.insert(key) {
            return;
        }
        if let Some(path) = &self.path {
            if let Err(e) = append_word(path, word) {
                self.error = Some(format!("Couldn't save the dictionary: {e}"));
            }
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(worker) = self.worker() {
                let _ = worker.tx.send(Msg::Learn(word.to_string()));
            }
        }
    }
}

fn fold(word: &str) -> String {
    word.to_lowercase()
}

#[cfg_attr(test, allow(dead_code))]
fn dictionary_path() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("scripture-study")
            .join("dictionary"),
    )
}

fn load_words(path: &Path) -> io::Result<HashSet<String>> {
    let text = match fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(e) => return Err(e),
        Ok(text) => text,
    };
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.chars().any(char::is_whitespace))
        .map(fold)
        .collect())
}

fn append_word(path: &Path, word: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{word}")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn system_engine() -> Engine {
    Engine::System(SystemChecker::spawn())
}

#[cfg(not(target_os = "macos"))]
fn system_engine() -> Engine {
    Engine::Inert
}

#[cfg(target_os = "macos")]
mod system {
    use std::collections::HashMap;
    use std::sync::{mpsc, Arc, Mutex};

    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSNotFound, NSRange, NSString};

    pub fn document_tag() -> isize {
        NSSpellChecker::uniqueSpellDocumentTag()
    }

    /// Runs until [`Msg::Shutdown`]. Every system-dictionary call stays on this thread.
    pub fn serve(rx: mpsc::Receiver<super::Msg>, cache: Arc<Mutex<HashMap<String, bool>>>) {
        let tag = document_tag();
        while let Ok(msg) = rx.recv() {
            match msg {
                super::Msg::Shutdown => break,
                super::Msg::Check { words, ctx } => {
                    let found = classify(tag, &words);
                    {
                        let mut cache = cache.lock().unwrap_or_else(|poison| poison.into_inner());
                        for (word, bad) in found {
                            cache.insert(super::fold(&word), bad);
                        }
                    }
                    if let Some(ctx) = ctx {
                        ctx.request_repaint();
                    }
                }
                super::Msg::Suggest { word, reply } => {
                    let _ = reply.send(suggestions(tag, &word));
                }
                super::Msg::Ignore(word) => ignore(tag, &word),
                super::Msg::Learn(word) => learn(&word),
                #[cfg(test)]
                super::Msg::Ping(reply) => {
                    let _ = reply.send(());
                }
            }
        }
    }

    fn classify(tag: isize, words: &[String]) -> Vec<(String, bool)> {
        autoreleasepool(|_| {
            let checker = NSSpellChecker::sharedSpellChecker();
            let language = checker.language();
            words
                .iter()
                .map(|word| (word.clone(), misspelled(&checker, &language, tag, word)))
                .collect()
        })
    }

    pub fn suggestions(tag: isize, word: &str) -> Vec<String> {
        autoreleasepool(|_| {
            let checker = NSSpellChecker::sharedSpellChecker();
            let language = checker.language();
            let ns = NSString::from_str(word);
            let range = NSRange::new(0, ns.length());
            let Some(guesses) = checker
                .guessesForWordRange_inString_language_inSpellDocumentWithTag(
                    range,
                    &ns,
                    Some(&language),
                    tag as _,
                )
            else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for i in 0..guesses.count() {
                out.push(guesses.objectAtIndex(i).to_string());
                if out.len() == super::MAX_SUGGESTIONS {
                    break;
                }
            }
            out
        })
    }

    pub fn ignore(tag: isize, word: &str) {
        autoreleasepool(|_| {
            let checker = NSSpellChecker::sharedSpellChecker();
            let word = NSString::from_str(word);
            checker.ignoreWord_inSpellDocumentWithTag(&word, tag as _);
        });
    }

    pub fn learn(word: &str) {
        autoreleasepool(|_| {
            let checker = NSSpellChecker::sharedSpellChecker();
            let word = NSString::from_str(word);
            checker.learnWord(&word);
        });
    }

    fn misspelled(checker: &NSSpellChecker, language: &NSString, tag: isize, word: &str) -> bool {
        let ns = NSString::from_str(word);
        let mut count = 0;
        let range = unsafe {
            checker.checkSpellingOfString_startingAt_language_wrap_inSpellDocumentWithTag_wordCount(
                &ns,
                0,
                Some(language),
                false,
                tag as _,
                &mut count,
            )
        };
        range.length > 0 && range.location != NSNotFound as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_all_hides_the_word_everywhere() {
        let mut spelling = Spelling::for_test(None, &["there", "again"], &[]);
        assert_eq!(spelling.misspellings("helo there"), vec![0..4]);
        spelling.ignore_all("Helo");
        assert!(spelling.misspellings("helo there").is_empty());
        assert!(spelling.misspellings("HELO again").is_empty());
    }

    #[test]
    fn add_to_dictionary_is_saved_and_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dictionary");
        let mut spelling =
            Spelling::for_test(Some(path.clone()), &["there"], &[("helo", &["hello"])]);
        assert_eq!(spelling.suggestions("Helo"), ["hello"]);
        spelling.add("Helo");
        assert!(spelling.misspellings("Helo there").is_empty());
        let saved = fs::read_to_string(&path).unwrap();
        assert_eq!(saved, "Helo\n");

        let mut again = Spelling::for_test(Some(path), &["there"], &[]);
        assert!(again.misspellings("helo there").is_empty());
        again.add("helo");
        assert_eq!(
            fs::read_to_string(dir.path().join("dictionary")).unwrap(),
            "Helo\n"
        );
    }

    #[test]
    fn match_case_follows_the_typed_word() {
        assert_eq!(match_case("helo", "hello"), "hello");
        assert_eq!(match_case("Helo", "hello"), "Hello");
        assert_eq!(match_case("HELO", "hello"), "HELLO");
    }

    #[test]
    fn underline_waits_until_space_tab_or_newline() {
        let helo = 0..4;
        assert!(
            !ready_to_mark("helo", &helo, Some(4), false),
            "still typing"
        );
        assert!(
            ready_to_mark("helo", &helo, Some(4), true),
            "tab leaves the caret in the word"
        );
        assert!(ready_to_mark("helo ", &helo, Some(5), false), "space");
        assert!(ready_to_mark("helo\n", &helo, None, false), "newline");
        assert!(ready_to_mark("helo\tnext", &helo, None, false), "tab");
        assert!(
            !ready_to_mark("helo.", &helo, Some(5), false),
            "a period is not a break"
        );
        assert!(
            ready_to_mark("helo.", &helo, None, false),
            "the caret has left the word"
        );
        let there = 5..10;
        assert!(
            ready_to_mark("helo there", &helo, Some(5), false),
            "the earlier word already has a space"
        );
        assert!(
            !ready_to_mark("helo there", &there, Some(5), false),
            "the word at the caret is still open"
        );
        assert_eq!(open_word("helo.", 5), Some(0..4));
        assert_eq!(open_word("helo ", 5), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_dictionary_flags_a_typo_and_accepts_a_word() {
        let dir = tempfile::tempdir().unwrap();
        let mut spelling = Spelling::system_at(Some(dir.path().join("dictionary")));
        assert!(checked(&mut spelling, "hello").is_empty(), "hello");
        assert!(checked(&mut spelling, "A").is_empty(), "A");
        assert_eq!(checked(&mut spelling, "helozzqx"), vec![0..8]);
        let suggestions = spelling.suggestions("helo");
        assert!(
            suggestions.iter().any(|s| s == "hello"),
            "suggestions: {suggestions:?}"
        );
        spelling.ignore_all("helozzqx");
        assert!(spelling.misspellings("helozzqx today").is_empty());
    }

    #[cfg(target_os = "macos")]
    fn checked(spelling: &mut Spelling, text: &str) -> Vec<Range<usize>> {
        let _ = spelling.misspellings(text);
        spelling.flush();
        spelling.misspellings(text)
    }
}
