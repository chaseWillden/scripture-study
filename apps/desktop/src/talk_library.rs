//! Downloaded conference talks. Each conference is saved as one JSON file
//! in a hidden folder beside the default notes, and fetched from the Church's
//! study site on a background thread.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use scripture_study_core::talks::{self, Conference, ConferenceTalks, ListedTalk, Session};

/// Where the study site serves a page's content as JSON.
const API: &str = "https://www.churchofjesuschrist.org/study/api/v3/language-pages/type/content";
/// Talk pages fetched at once while downloading a conference.
const PARALLEL: usize = 4;

/// Fetches the HTML body of a study-site page, e.g. `/general-conference/2024/10`.
pub type Fetch = Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// Where downloaded talks live. Hidden, so a notes folder it sits in
/// doesn't list it.
pub fn dir() -> Option<PathBuf> {
    Some(
        dirs::data_dir()?
            .join("scripture-study")
            .join(".conference-talks"),
    )
}

/// What the download thread is doing, for the UI to show.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// Waiting their turn, in order.
    pub queued: Vec<Conference>,
    pub current: Option<Progress>,
    /// The last conference that couldn't be downloaded, and why.
    pub error: Option<(Conference, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub conference: Conference,
    pub done: usize,
    /// `0` until the conference's table of contents has been read.
    pub total: usize,
}

impl Status {
    pub fn is_busy(&self) -> bool {
        self.current.is_some() || !self.queued.is_empty()
    }

    pub fn is_pending(&self, conference: Conference) -> bool {
        self.queued.contains(&conference)
            || self.current.is_some_and(|p| p.conference == conference)
    }
}

/// State shared with the download thread.
#[derive(Default)]
struct Shared {
    status: Status,
    /// Conferences read from disk or just downloaded, not yet picked up.
    arrived: Vec<ConferenceTalks>,
}

/// The downloaded talks and the thread that fetches more.
pub struct TalkLibrary {
    /// Newest first.
    conferences: Vec<ConferenceTalks>,
    index: talks::Index,
    dir: Option<PathBuf>,
    shared: Arc<Mutex<Shared>>,
    tx: Sender<Conference>,
    cancel: Arc<AtomicBool>,
}

impl TalkLibrary {
    /// Loads what's saved in `dir` and starts the download thread. With no
    /// `dir` nothing is read or saved, which is what the UI tests use.
    pub fn start(dir: Option<PathBuf>, fetch: Fetch, ctx: egui::Context) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let worker = Worker {
            dir: dir.clone(),
            fetch,
            shared: shared.clone(),
            cancel: cancel.clone(),
            ctx,
        };
        std::thread::Builder::new()
            .name("conference-talks".into())
            .spawn(move || worker.run(rx))
            .expect("start the conference talks thread");
        Self {
            conferences: Vec::new(),
            index: talks::Index::new(&[]),
            dir,
            shared,
            tx,
            cancel,
        }
    }

    /// Takes in conferences the thread has loaded or downloaded. Returns
    /// whether any arrived, since indexes into [`Self::conferences`] shift.
    pub fn poll(&mut self) -> bool {
        let arrived = std::mem::take(&mut self.shared.lock().unwrap().arrived);
        if arrived.is_empty() {
            return false;
        }
        for conference in arrived {
            self.conferences
                .retain(|c| c.conference != conference.conference);
            self.conferences.push(conference);
        }
        self.conferences
            .sort_by_key(|c| std::cmp::Reverse(c.conference));
        self.index = talks::Index::new(&self.conferences);
        true
    }

    pub fn conferences(&self) -> &[ConferenceTalks] {
        &self.conferences
    }

    pub fn get(&self, conference: Conference) -> Option<&ConferenceTalks> {
        self.conferences.iter().find(|c| c.conference == conference)
    }

    pub fn has(&self, conference: Conference) -> bool {
        self.get(conference).is_some()
    }

    pub fn talk_count(&self) -> usize {
        self.conferences
            .iter()
            .map(ConferenceTalks::talk_count)
            .sum()
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<talks::TalkHit> {
        self.index.search(&self.conferences, query, limit)
    }

    pub fn status(&self) -> Status {
        self.shared.lock().unwrap().status.clone()
    }

    /// Queues `conferences` for download, skipping any already here or on the way.
    pub fn download(&mut self, conferences: impl IntoIterator<Item = Conference>) {
        let mut shared = self.shared.lock().unwrap();
        self.cancel.store(false, Ordering::Relaxed);
        shared.status.error = None;
        for conference in conferences {
            if self.has(conference) || shared.status.is_pending(conference) {
                continue;
            }
            shared.status.queued.push(conference);
            let _ = self.tx.send(conference);
        }
    }

    /// Stops the download in progress and forgets the queue.
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.shared.lock().unwrap().status.queued.clear();
    }

    /// Deletes a downloaded conference.
    pub fn remove(&mut self, conference: Conference) -> std::io::Result<()> {
        if let Some(dir) = &self.dir {
            match fs::remove_file(file(dir, conference)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        self.conferences.retain(|c| c.conference != conference);
        self.index = talks::Index::new(&self.conferences);
        Ok(())
    }

    /// Adds a conference without downloading it.
    #[cfg(test)]
    pub fn insert(&mut self, conference: ConferenceTalks) {
        self.shared.lock().unwrap().arrived.push(conference);
        self.poll();
    }
}

struct Worker {
    dir: Option<PathBuf>,
    fetch: Fetch,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    ctx: egui::Context,
}

impl Worker {
    fn run(self, rx: Receiver<Conference>) {
        if let Some(dir) = &self.dir {
            let saved = load_all(dir);
            if !saved.is_empty() {
                self.shared.lock().unwrap().arrived.extend(saved);
                self.ctx.request_repaint();
            }
        }
        while let Ok(conference) = rx.recv() {
            {
                let mut shared = self.shared.lock().unwrap();
                // Canceled while it waited.
                let Some(at) = shared.status.queued.iter().position(|c| *c == conference) else {
                    continue;
                };
                shared.status.queued.remove(at);
                shared.status.current = Some(Progress {
                    conference,
                    done: 0,
                    total: 0,
                });
            }
            self.ctx.request_repaint();
            let result = self.download(conference).and_then(|talks| {
                if let Some(dir) = &self.dir {
                    save(dir, &talks).map_err(|e| format!("couldn't save: {e}"))?;
                }
                Ok(talks)
            });
            let mut shared = self.shared.lock().unwrap();
            shared.status.current = None;
            match result {
                Ok(talks) => shared.arrived.push(talks),
                Err(_) if self.cancel.load(Ordering::Relaxed) => {}
                Err(e) => shared.status.error = Some((conference, e)),
            }
            drop(shared);
            self.ctx.request_repaint();
        }
    }

    fn download(&self, conference: Conference) -> Result<ConferenceTalks, String> {
        let manifest = fetch_twice(&self.fetch, &conference.uri())?;
        // The site answers an unknown conference with another page, so keep
        // only talks that belong to this one.
        let prefix = format!("{}/", conference.uri());
        let mut listed = talks::parse_manifest(&manifest);
        for session in &mut listed {
            session.talks.retain(|talk| talk.uri.starts_with(&prefix));
        }
        listed.retain(|session| !session.talks.is_empty());
        let all: Vec<&ListedTalk> = listed.iter().flat_map(|s| &s.talks).collect();
        if all.is_empty() {
            // Only the newest conference can be waiting on its talks.
            return Err(
                if conference >= Conference::latest(std::time::SystemTime::now()) {
                    "its talks aren't published yet"
                } else {
                    "no talks were found on its page"
                }
                .to_string(),
            );
        }
        self.set_progress(conference, 0, all.len());

        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let pages: Vec<Mutex<Option<Result<String, String>>>> =
            all.iter().map(|_| Mutex::new(None)).collect();
        std::thread::scope(|scope| {
            for _ in 0..PARALLEL.min(all.len()) {
                scope.spawn(|| loop {
                    if self.cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let n = next.fetch_add(1, Ordering::Relaxed);
                    let Some(talk) = all.get(n) else {
                        return;
                    };
                    *pages[n].lock().unwrap() = Some(fetch_twice(&self.fetch, &talk.uri));
                    let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                    self.set_progress(conference, finished, all.len());
                });
            }
        });
        if self.cancel.load(Ordering::Relaxed) {
            return Err("canceled".to_string());
        }

        let mut pages = pages.into_iter().map(|page| page.into_inner().unwrap());
        let mut sessions = Vec::new();
        for session in &listed {
            let mut talks = Vec::new();
            for listed in &session.talks {
                let html = pages
                    .next()
                    .flatten()
                    .unwrap_or(Err("not fetched".into()))?;
                talks.push(talks::parse_talk(listed, &html));
            }
            sessions.push(Session {
                title: session.title.clone(),
                talks,
            });
        }
        Ok(ConferenceTalks {
            conference,
            sessions,
        })
    }

    fn set_progress(&self, conference: Conference, done: usize, total: usize) {
        self.shared.lock().unwrap().status.current = Some(Progress {
            conference,
            done,
            total,
        });
        self.ctx.request_repaint();
    }
}

/// One retry, for a dropped connection in the middle of a long download.
fn fetch_twice(fetch: &Fetch, uri: &str) -> Result<String, String> {
    fetch(uri).or_else(|_| {
        std::thread::sleep(Duration::from_millis(500));
        fetch(uri)
    })
}

fn file(dir: &Path, conference: Conference) -> PathBuf {
    dir.join(format!("{}.json", conference.key()))
}

fn save(dir: &Path, talks: &ConferenceTalks) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let json = serde_json::to_vec(talks).map_err(std::io::Error::other)?;
    // Written whole, then moved in place, so a crash never leaves half a file.
    let target = file(dir, talks.conference);
    let partial = target.with_extension("json.partial");
    fs::write(&partial, json)?;
    fs::rename(partial, target)
}

/// Every conference saved in `dir`. Unreadable files are skipped.
fn load_all(dir: &Path) -> Vec<ConferenceTalks> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "json")
                && path
                    .file_stem()
                    .and_then(|stem| Conference::from_key(&stem.to_string_lossy()))
                    .is_some()
        })
        .filter_map(|path| serde_json::from_slice(&fs::read(path).ok()?).ok())
        .collect()
}

/// Fetches pages from the study site over HTTPS.
pub fn web() -> Fetch {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(concat!("scripture-study/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    Arc::new(move |uri: &str| {
        let url = format!("{API}?lang=eng&uri={uri}");
        let mut response = agent.get(&url).call().map_err(|e| e.to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("the site answered {status}"));
        }
        let page: serde_json::Value = response.body_mut().read_json().map_err(|e| e.to_string())?;
        page["content"]["body"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "the page had no text".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const MANIFEST: &str = r#"<li><h2 class="label"><p class="title">Sunday Morning Session</p></h2><ul>
<li><a href="/study/general-conference/2024/10/sunday-morning-session?lang=eng" class="list-tile"><p class="title">Sunday Morning Session</p></a></li>
<li><a href="/study/general-conference/2024/10/41holland?lang=eng" class="list-tile"><p class="primaryMeta">Jeffrey R. Holland</p><p class="title">I Am He</p></a></li>
<li><a href="/study/general-conference/2024/10/42browning?lang=eng" class="list-tile"><p class="primaryMeta">Tracy Y. Browning</p><p class="title">Seeking Answers</p></a></li>
<li><a href="/study/general-conference/2023/04/elsewhere?lang=eng" class="list-tile"><p class="title">From another conference</p></a></li>
</ul></li>"#;

    fn fake(fails: &'static [&'static str]) -> (Fetch, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let log = calls.clone();
        let fetch: Fetch = Arc::new(move |uri: &str| {
            log.lock().unwrap().push(uri.to_string());
            if fails.contains(&uri) {
                return Err("offline".into());
            }
            if uri == "/general-conference/2024/10" {
                return Ok(MANIFEST.to_string());
            }
            if uri.starts_with("/general-conference/2024/10/") {
                return Ok(format!(
                    r#"<h1>Talk</h1><div class="body-block"><p>Words of {uri}.</p></div>"#
                ));
            }
            Ok("<p>Some other page</p>".to_string())
        });
        (fetch, calls)
    }

    fn wait_until(library: &mut TalkLibrary, done: impl Fn(&TalkLibrary) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            library.poll();
            if done(library) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn downloads_a_conference_saves_it_and_loads_it_next_time() {
        let dir = tempfile::tempdir().unwrap();
        let (fetch, calls) = fake(&[]);
        let mut library = TalkLibrary::start(
            Some(dir.path().into()),
            fetch.clone(),
            egui::Context::default(),
        );
        library.download([Conference::october(2024)]);
        wait_until(&mut library, |l| l.has(Conference::october(2024)));
        assert!(!library.status().is_busy());

        let talks = library.get(Conference::october(2024)).unwrap();
        assert_eq!(talks.sessions.len(), 1);
        assert_eq!(talks.sessions[0].title, "Sunday Morning Session");
        let titles: Vec<_> = talks.sessions[0].talks.iter().map(|t| &t.speaker).collect();
        assert_eq!(titles, ["Jeffrey R. Holland", "Tracy Y. Browning"]);
        assert_eq!(
            talks.sessions[0].talks[1].paragraphs[0].text,
            "Words of /general-conference/2024/10/42browning."
        );
        // The talk from another conference was never fetched.
        assert!(!calls
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.contains("elsewhere")));
        assert_eq!(library.search("browning", 10).len(), 2);
        assert!(dir.path().join("2024-10.json").exists());

        // Asking again for what's here fetches nothing.
        let before = calls.lock().unwrap().len();
        library.download([Conference::october(2024)]);
        assert_eq!(calls.lock().unwrap().len(), before);

        let mut reopened =
            TalkLibrary::start(Some(dir.path().into()), fetch, egui::Context::default());
        wait_until(&mut reopened, |l| l.has(Conference::october(2024)));

        reopened.remove(Conference::october(2024)).unwrap();
        assert!(!reopened.has(Conference::october(2024)));
        assert!(!dir.path().join("2024-10.json").exists());
    }

    #[test]
    fn a_conference_with_no_talks_yet_reports_an_error() {
        let (fetch, _) = fake(&[]);
        let mut library = TalkLibrary::start(None, fetch, egui::Context::default());
        library.download([Conference::april(2030)]);
        wait_until(&mut library, |l| l.status().error.is_some());
        let (conference, error) = library.status().error.unwrap();
        assert_eq!(conference, Conference::april(2030));
        assert!(error.contains("published"), "{error}");
        assert!(library.conferences().is_empty());
    }

    #[test]
    fn a_talk_that_keeps_failing_fails_the_conference() {
        let (fetch, calls) = fake(&["/general-conference/2024/10/42browning"]);
        let mut library = TalkLibrary::start(None, fetch, egui::Context::default());
        library.download([Conference::october(2024)]);
        wait_until(&mut library, |l| l.status().error.is_some());
        assert!(!library.has(Conference::october(2024)));
        let tries = calls
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.ends_with("42browning"))
            .count();
        assert_eq!(tries, 2);
    }
}
