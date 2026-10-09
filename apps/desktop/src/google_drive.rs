//! The Google Drive connector: keeps a Google Doc copy of every note in one
//! Drive folder, mirroring the note folders inside it.
//!
//! Sync is one way, from the notes on disk to Drive. A background thread
//! owns everything that touches the network or the connector's settings
//! file; the UI sends it [`Msg`]s and draws the latest [`Snapshot`].
//!
//! Signing in uses Google's flow for installed apps: the browser opens the
//! consent page and redirects back to a one-shot server on 127.0.0.1, and
//! PKCE stands in for a confidential client secret. The app asks only for
//! `drive.file`, so it can see the files it created and nothing else.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use base64::Engine;
use eframe::egui;
use scripture_study_core::{
    drive_sync::{self, LocalNote, Step, SyncState, SyncedNote},
    store, FsStore, NoteStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const DEFAULT_FOLDER: &str = "Scripture Study";

/// Wait after an edit before syncing, so a burst of saves is one sync.
const SYNC_DELAY: Duration = Duration::from_secs(5);
/// How long the sign-in page may stay open before giving up.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
const FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD_URL: &str = "https://www.googleapis.com/upload/drive/v3/files";
const ABOUT_URL: &str = "https://www.googleapis.com/drive/v3/about";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const DOC_MIME: &str = "application/vnd.google-apps.document";

/// An OAuth client built into the binary (`SCRIPTURE_STUDY_GOOGLE_CLIENT_ID`
/// and `_SECRET` at build or run time). Without one, the settings page asks
/// for the user's own.
fn builtin_client() -> Option<(String, String)> {
    let id = std::env::var("SCRIPTURE_STUDY_GOOGLE_CLIENT_ID")
        .ok()
        .or(option_env!("SCRIPTURE_STUDY_GOOGLE_CLIENT_ID").map(str::to_string))?;
    let secret = std::env::var("SCRIPTURE_STUDY_GOOGLE_CLIENT_SECRET")
        .ok()
        .or(option_env!("SCRIPTURE_STUDY_GOOGLE_CLIENT_SECRET").map(str::to_string))
        .unwrap_or_default();
    (!id.trim().is_empty()).then_some((id, secret))
}

/// Where connector settings live: next to the remembered notes folder, not
/// inside it, so the sign-in never travels with the notes.
pub fn config_file() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("scripture-study")
            .join("connectors.json"),
    )
}

/// Everything the connector remembers between launches.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Connectors {
    #[serde(default)]
    google_drive: DriveConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DriveConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
    #[serde(default = "default_folder")]
    folder_name: String,
    /// The Drive folder made for the notes, once it exists.
    #[serde(default)]
    folder_id: Option<String>,
    /// `folder_name` was changed here and Drive hasn't caught up yet.
    #[serde(default)]
    rename_pending: bool,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    account: Option<String>,
    /// What each notes directory has sent, keyed by its path.
    #[serde(default)]
    libraries: BTreeMap<String, SyncState>,
}

fn default_folder() -> String {
    DEFAULT_FOLDER.to_string()
}

impl Default for DriveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            client_id: String::new(),
            client_secret: String::new(),
            folder_name: default_folder(),
            folder_id: None,
            rename_pending: false,
            refresh_token: None,
            account: None,
            libraries: BTreeMap::new(),
        }
    }
}

impl DriveConfig {
    fn client(&self) -> Option<(String, String)> {
        builtin_client().or_else(|| {
            let id = self.client_id.trim();
            (!id.is_empty()).then(|| (id.to_string(), self.client_secret.trim().to_string()))
        })
    }
}

fn load(path: &Path) -> Connectors {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Write-then-rename, readable only by the user: it holds a sign-in.
fn store_config(path: &Path, connectors: &Connectors) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_vec_pretty(connectors).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&tmp)?.write_all(&body)?;
    fs::rename(tmp, path)
}

/// What the connector is busy with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    /// Waiting for the user to finish in the browser.
    SigningIn,
    Syncing {
        done: usize,
        total: usize,
    },
}

/// The connector as the settings page shows it.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub enabled: bool,
    /// The OAuth client comes with the app, so there's nothing to fill in.
    pub builtin_client: bool,
    pub client_id: String,
    pub client_secret: String,
    pub folder_name: String,
    /// Link to the Drive folder, once it exists.
    pub folder_url: Option<String>,
    pub connected: bool,
    pub account: Option<String>,
    pub phase: Phase,
    pub last_synced: Option<SystemTime>,
    /// Notes in Drive after the last sync.
    pub synced_notes: usize,
    pub error: Option<String>,
    /// Note id → hash of the text last sent, for the open notes directory.
    pub notes: Arc<BTreeMap<String, String>>,
}

/// Where one note stands with Drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteSync {
    /// Drive has exactly what's saved.
    Synced,
    /// A sync is running and this note isn't done yet.
    Syncing,
    /// Changed since it was last sent; it goes with the next sync.
    Pending,
    /// The last sync failed, with this message.
    Failed(String),
}

impl Snapshot {
    pub fn has_client(&self) -> bool {
        self.builtin_client || !self.client_id.trim().is_empty()
    }
}

enum Msg {
    SetEnabled(bool),
    SetClient { id: String, secret: String },
    SetFolder(String),
    Connect,
    CancelConnect,
    Disconnect,
    Library(PathBuf),
    Changed,
    SyncNow,
    SignedIn(Result<SignIn, String>),
}

struct SignIn {
    refresh_token: String,
    access_token: String,
    expires: Instant,
    account: Option<String>,
}

/// The UI's handle on the connector thread.
pub struct GoogleDrive {
    tx: Sender<Msg>,
    shared: Arc<Mutex<Snapshot>>,
}

impl GoogleDrive {
    /// Starts the connector for the notes in `library`. With no `config`
    /// file nothing is saved, which is what the UI tests use.
    pub fn start(config: Option<PathBuf>, library: PathBuf, ctx: egui::Context) -> Self {
        let connectors = config.as_deref().map(load).unwrap_or_default();
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let worker = Worker {
            cfg: connectors.google_drive,
            config,
            library,
            shared: shared.clone(),
            ctx,
            tx: tx.clone(),
            access: None,
            due: None,
            cancel: None,
            phase: Phase::Idle,
            last_synced: None,
            synced_notes: 0,
            error: None,
            open_url: open_in_browser,
        };
        worker.publish();
        std::thread::Builder::new()
            .name("google-drive".into())
            .spawn(move || worker.run(rx))
            .expect("start the Google Drive thread");
        Self { tx, shared }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().unwrap().clone()
    }

    /// Where note `id` stands, or `None` while the connector isn't syncing.
    /// `saved` is the [`LocalNote::hash`] of what's on disk, or `None` while
    /// there are edits that haven't been saved yet.
    pub fn note_status(&self, id: &str, saved: Option<&str>) -> Option<NoteSync> {
        let s = self.shared.lock().unwrap();
        if !(s.enabled && s.connected) {
            return None;
        }
        let sent = s.notes.get(id).map(String::as_str);
        Some(if saved.is_some() && sent == saved {
            NoteSync::Synced
        } else if matches!(s.phase, Phase::Syncing { .. }) {
            NoteSync::Syncing
        } else if let Some(error) = &s.error {
            NoteSync::Failed(error.clone())
        } else {
            NoteSync::Pending
        })
    }

    /// Stands in for the connector thread, for UI tests.
    #[cfg(test)]
    pub fn set_snapshot(&self, snapshot: Snapshot) {
        *self.shared.lock().unwrap() = snapshot;
    }

    fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
    }

    pub fn set_enabled(&self, on: bool) {
        // Show the switch move this frame rather than when the thread
        // gets to it.
        self.shared.lock().unwrap().enabled = on;
        self.send(Msg::SetEnabled(on));
    }

    pub fn set_client(&self, id: String, secret: String) {
        self.send(Msg::SetClient { id, secret });
    }

    pub fn set_folder(&self, name: String) {
        self.send(Msg::SetFolder(name));
    }

    pub fn connect(&self) {
        self.send(Msg::Connect);
    }

    pub fn cancel_connect(&self) {
        self.send(Msg::CancelConnect);
    }

    pub fn disconnect(&self) {
        self.send(Msg::Disconnect);
    }

    pub fn sync_now(&self) {
        self.send(Msg::SyncNow);
    }

    /// A note or folder changed on disk.
    pub fn notes_changed(&self) {
        self.send(Msg::Changed);
    }

    /// The app switched to another directory of notes.
    pub fn set_library(&self, dir: PathBuf) {
        self.send(Msg::Library(dir));
    }
}

struct Worker {
    cfg: DriveConfig,
    config: Option<PathBuf>,
    library: PathBuf,
    shared: Arc<Mutex<Snapshot>>,
    ctx: egui::Context,
    /// Lets the sign-in thread report back.
    tx: Sender<Msg>,
    access: Option<(String, Instant)>,
    /// When the next sync is due after an edit.
    due: Option<Instant>,
    /// Stops a sign-in that's waiting on the browser.
    cancel: Option<Arc<AtomicBool>>,
    phase: Phase,
    last_synced: Option<SystemTime>,
    synced_notes: usize,
    error: Option<String>,
    open_url: fn(&str) -> io::Result<()>,
}

impl Worker {
    fn run(mut self, rx: Receiver<Msg>) {
        if self.active() {
            self.sync();
        }
        loop {
            let msg = match self.due {
                Some(due) => rx.recv_timeout(due.saturating_duration_since(Instant::now())),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match msg {
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => self.sync(),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Switched on and signed in.
    fn active(&self) -> bool {
        self.cfg.enabled && self.cfg.refresh_token.is_some()
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::SetEnabled(on) => {
                self.cfg.enabled = on;
                if !on {
                    self.due = None;
                }
                self.error = None;
                self.save();
                if self.active() {
                    self.sync();
                }
            }
            Msg::SetClient { id, secret } => {
                self.cfg.client_id = id;
                self.cfg.client_secret = secret;
                self.save();
            }
            Msg::SetFolder(name) => {
                let name = name.trim().to_string();
                if name.is_empty() || name == self.cfg.folder_name {
                    self.publish();
                    return;
                }
                self.cfg.folder_name = name;
                self.cfg.rename_pending = self.cfg.folder_id.is_some();
                self.save();
                if self.active() {
                    self.sync();
                }
            }
            Msg::Connect => self.sign_in(),
            Msg::CancelConnect => {
                if let Some(cancel) = self.cancel.take() {
                    cancel.store(true, Ordering::Relaxed);
                }
                self.phase = Phase::Idle;
                self.publish();
            }
            Msg::SignedIn(result) => {
                self.cancel = None;
                self.phase = Phase::Idle;
                match result {
                    Ok(sign_in) => {
                        self.cfg.refresh_token = Some(sign_in.refresh_token);
                        self.cfg.account = sign_in.account;
                        self.cfg.enabled = true;
                        self.access = Some((sign_in.access_token, sign_in.expires));
                        self.error = None;
                        self.save();
                        self.sync();
                    }
                    Err(e) => {
                        self.error = Some(e);
                        self.publish();
                    }
                }
            }
            Msg::Disconnect => {
                if let Some(token) = self.cfg.refresh_token.take() {
                    // Best effort: the token is forgotten here either way.
                    let _ = agent().post(REVOKE_URL).send_form([("token", token)]);
                }
                self.cfg.enabled = false;
                self.cfg.account = None;
                self.access = None;
                self.due = None;
                self.error = None;
                self.save();
            }
            Msg::Library(dir) => {
                self.library = dir;
                self.last_synced = None;
                self.synced_notes = 0;
                if self.active() {
                    self.sync();
                } else {
                    self.publish();
                }
            }
            Msg::Changed => {
                if self.active() {
                    // A later edit doesn't push the sync back, so steady
                    // typing still syncs every few seconds.
                    self.due.get_or_insert_with(|| Instant::now() + SYNC_DELAY);
                }
            }
            Msg::SyncNow => {
                if self.active() {
                    self.sync();
                }
            }
        }
    }

    fn publish(&self) {
        let builtin = builtin_client().is_some();
        let snapshot = Snapshot {
            enabled: self.cfg.enabled,
            builtin_client: builtin,
            client_id: self.cfg.client_id.clone(),
            client_secret: self.cfg.client_secret.clone(),
            folder_name: self.cfg.folder_name.clone(),
            folder_url: self
                .cfg
                .folder_id
                .as_ref()
                .map(|id| format!("https://drive.google.com/drive/folders/{id}")),
            connected: self.cfg.refresh_token.is_some(),
            account: self.cfg.account.clone(),
            phase: self.phase.clone(),
            last_synced: self.last_synced,
            synced_notes: self.synced_notes,
            error: self.error.clone(),
            notes: Arc::new(self.sent_notes()),
        };
        *self.shared.lock().unwrap() = snapshot;
        self.ctx.request_repaint();
    }

    /// What's in Drive for the open notes directory, if it still applies.
    fn sent_notes(&self) -> BTreeMap<String, String> {
        let key = self.library.to_string_lossy();
        match self.cfg.libraries.get(key.as_ref()) {
            Some(state) if state.folder_id.is_some() && state.folder_id == self.cfg.folder_id => {
                state
                    .notes
                    .iter()
                    .map(|(id, note)| (id.clone(), note.hash.clone()))
                    .collect()
            }
            _ => BTreeMap::new(),
        }
    }

    fn save(&mut self) {
        if let Some(path) = &self.config {
            let connectors = Connectors {
                google_drive: self.cfg.clone(),
            };
            if let Err(e) = store_config(path, &connectors) {
                self.error = Some(format!("Couldn't save connector settings: {e}"));
            }
        }
        self.publish();
    }

    fn sign_in(&mut self) {
        if self.phase == Phase::SigningIn {
            return;
        }
        let Some(client) = self.cfg.client() else {
            self.error = Some("Add an OAuth client ID first.".into());
            self.publish();
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.phase = Phase::SigningIn;
        self.error = None;
        self.publish();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let open_url = self.open_url;
        std::thread::spawn(move || {
            let result = sign_in(&client, &cancel, open_url);
            // A canceled sign-in has nothing left to report.
            if !cancel.load(Ordering::Relaxed) {
                let _ = tx.send(Msg::SignedIn(result));
            }
            ctx.request_repaint();
        });
    }

    /// A current access token, refreshed when it's about to run out.
    fn access_token(&mut self) -> Result<String, String> {
        if let Some((token, expires)) = &self.access {
            if Instant::now() + Duration::from_secs(60) < *expires {
                return Ok(token.clone());
            }
        }
        let refresh = self
            .cfg
            .refresh_token
            .clone()
            .ok_or("Connect a Google account first.")?;
        let (id, secret) = self.cfg.client().ok_or("Add an OAuth client ID first.")?;
        let mut response = agent()
            .post(TOKEN_URL)
            .send_form([
                ("client_id", id.as_str()),
                ("client_secret", secret.as_str()),
                ("refresh_token", refresh.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .map_err(|e| format!("Couldn't reach Google: {e}"))?;
        let status = response.status().as_u16();
        let body: Value = response.body_mut().read_json().unwrap_or(Value::Null);
        if body["error"] == "invalid_grant" {
            // Revoked from the Google account page, or expired. A bad
            // client ID or secret is reported below instead, keeping the
            // sign-in for when it's fixed.
            self.cfg.refresh_token = None;
            self.cfg.enabled = false;
            self.save();
            return Err("Google sign-in expired. Connect again to keep syncing.".into());
        }
        let token = body["access_token"]
            .as_str()
            .ok_or_else(|| {
                let reason = body["error_description"]
                    .as_str()
                    .or(body["error"].as_str())
                    .unwrap_or("no access token returned");
                format!("Google sign-in failed ({status}): {reason}")
            })?
            .to_string();
        let expires =
            Instant::now() + Duration::from_secs(body["expires_in"].as_u64().unwrap_or(3600));
        self.access = Some((token.clone(), expires));
        Ok(token)
    }

    fn sync(&mut self) {
        self.due = None;
        if !self.active() {
            return;
        }
        self.phase = Phase::Syncing { done: 0, total: 0 };
        self.publish();
        match self.try_sync() {
            Ok(count) => {
                self.last_synced = Some(SystemTime::now());
                self.synced_notes = count;
                self.error = None;
            }
            Err(e) => {
                if e.contains("401") {
                    self.access = None;
                }
                self.error = Some(e);
            }
        }
        self.phase = Phase::Idle;
        self.save();
    }

    fn try_sync(&mut self) -> Result<usize, String> {
        let token = self.access_token()?;
        let drive = Google::new(token);
        let folder_id = self.ensure_folder(&drive)?;

        let store = FsStore::open(&self.library).map_err(|e| e.to_string())?;
        let notes: Vec<LocalNote> = store
            .list()
            .map_err(|e| format!("Couldn't list notes: {e}"))?
            .iter()
            .filter_map(|meta| {
                let doc = store.load(&meta.id).ok()?;
                // An empty note is a page nobody has written on yet.
                (!doc.is_blank()).then(|| LocalNote::new(&meta.id, &doc))
            })
            .collect();
        let folders = store
            .folders()
            .map_err(|e| format!("Couldn't list folders: {e}"))?;

        let key = self.library.to_string_lossy().into_owned();
        let mut state = self.cfg.libraries.get(&key).cloned().unwrap_or_default();
        state.retarget(&folder_id);
        let live = drive.live_files().map_err(|e| e.to_string())?;
        state.keep_live(&live);
        // A second pass picks up after a folder that was removed in Drive.
        for _ in 0..2 {
            let steps = drive_sync::plan(&notes, &folders, &state);
            let total = steps.len();
            let mut retry = false;
            for (done, step) in steps.iter().enumerate() {
                self.phase = Phase::Syncing { done, total };
                self.publish();
                let result = apply(&drive, &mut state, step, &notes);
                self.cfg.libraries.insert(key.clone(), state.clone());
                self.save();
                match result {
                    Ok(()) => {}
                    Err(ApiError::Retry) => {
                        retry = true;
                        break;
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            if !retry {
                break;
            }
        }
        Ok(state.notes.len())
    }

    /// The Drive folder the notes go in, made (or renamed) as needed.
    fn ensure_folder(&mut self, drive: &dyn Drive) -> Result<String, String> {
        if let Some(id) = self.cfg.folder_id.clone() {
            match drive.folder_name(&id) {
                Ok(Some(name)) => {
                    if self.cfg.rename_pending && name != self.cfg.folder_name {
                        drive
                            .rename(&id, &self.cfg.folder_name)
                            .map_err(|e| e.to_string())?;
                    }
                    self.cfg.rename_pending = false;
                    return Ok(id);
                }
                // Deleted or trashed in Drive: make a new one.
                Ok(None) | Err(ApiError::NotFound) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        let id = drive
            .create_folder(&self.cfg.folder_name, None)
            .map_err(|e| e.to_string())?;
        self.cfg.folder_id = Some(id.clone());
        self.cfg.rename_pending = false;
        self.save();
        Ok(id)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ApiError {
    NotFound,
    /// Something it depended on was gone; plan again.
    Retry,
    Other(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("Not found in Google Drive"),
            Self::Retry => f.write_str("Google Drive changed during sync"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

/// The few Drive operations sync needs, so it can run against a fake.
trait Drive {
    /// The folder's name, or `None` if it's in the trash.
    fn folder_name(&self, id: &str) -> Result<Option<String>, ApiError>;
    fn create_folder(&self, name: &str, parent: Option<&str>) -> Result<String, ApiError>;
    fn rename(&self, id: &str, name: &str) -> Result<(), ApiError>;
    /// Uploads Markdown as a new Google Doc, returning its id.
    fn upload(&self, name: &str, markdown: &str, parent: &str) -> Result<String, ApiError>;
    fn update(&self, id: &str, name: &str, markdown: &str) -> Result<(), ApiError>;
    fn trash(&self, id: &str) -> Result<(), ApiError>;
    /// Every file and folder this app made that isn't in the trash.
    fn live_files(&self) -> Result<BTreeSet<String>, ApiError>;
}

/// Carries out one step of the plan, recording it in `state`.
fn apply(
    drive: &dyn Drive,
    state: &mut SyncState,
    step: &Step,
    notes: &[LocalNote],
) -> Result<(), ApiError> {
    let note = |id: &str| notes.iter().find(|n| n.id == id);
    // A missing parent folder was removed in Drive. Forget it so the next
    // pass makes it again.
    let parent_gone = |state: &mut SyncState, folder: &str| {
        if folder.is_empty() {
            ApiError::Other("The Google Drive folder was removed during sync".into())
        } else {
            state.forget_folder(folder);
            ApiError::Retry
        }
    };
    match step {
        Step::CreateFolder { path } => {
            let folder = store::parent(path);
            let parent = state.parent_id(folder).ok_or(ApiError::Retry)?.to_string();
            match drive.create_folder(store::name(path), Some(&parent)) {
                Ok(id) => {
                    state.folders.insert(path.clone(), id);
                    Ok(())
                }
                Err(ApiError::NotFound) => Err(parent_gone(state, folder)),
                Err(e) => Err(e),
            }
        }
        Step::Upload { id } => {
            upload(drive, state, note(id).ok_or(ApiError::Retry)?).map_err(|e| match e {
                ApiError::NotFound => parent_gone(state, store::parent(id)),
                e => e,
            })
        }
        Step::Update { id, file_id } => {
            let note = note(id).ok_or(ApiError::Retry)?;
            match drive.update(file_id, &note.title, &note.markdown) {
                Ok(()) => {
                    state.notes.insert(
                        id.clone(),
                        SyncedNote {
                            file_id: file_id.clone(),
                            hash: note.hash.clone(),
                        },
                    );
                    Ok(())
                }
                // Deleted in Drive: put it back.
                Err(ApiError::NotFound) => {
                    state.notes.remove(id);
                    upload(drive, state, note)
                }
                Err(e) => Err(e),
            }
        }
        Step::Trash { id, file_id } => match drive.trash(file_id) {
            Ok(()) | Err(ApiError::NotFound) => {
                state.notes.remove(id);
                Ok(())
            }
            Err(e) => Err(e),
        },
        Step::TrashFolder { path, folder_id } => match drive.trash(folder_id) {
            Ok(()) | Err(ApiError::NotFound) => {
                state.forget_folder(path);
                Ok(())
            }
            Err(e) => Err(e),
        },
    }
}

fn upload(drive: &dyn Drive, state: &mut SyncState, note: &LocalNote) -> Result<(), ApiError> {
    let parent = state
        .parent_id(store::parent(&note.id))
        .ok_or(ApiError::Retry)?
        .to_string();
    let file_id = drive.upload(&note.title, &note.markdown, &parent)?;
    state.notes.insert(
        note.id.clone(),
        SyncedNote {
            file_id,
            hash: note.hash.clone(),
        },
    );
    Ok(())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into()
}

/// Drive's REST API with one access token.
struct Google {
    agent: ureq::Agent,
    auth: String,
}

impl Google {
    fn new(token: String) -> Self {
        Self {
            agent: agent(),
            auth: format!("Bearer {token}"),
        }
    }

    fn multipart(&self, metadata: &Value, markdown: &str) -> (String, Vec<u8>) {
        let boundary = format!("scripture-study-{}", random_token(12));
        let mut body = Vec::new();
        let _ = write!(
            body,
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n\
             --{boundary}\r\nContent-Type: text/markdown; charset=UTF-8\r\n\r\n{markdown}\r\n\
             --{boundary}--\r\n"
        );
        (format!("multipart/related; boundary={boundary}"), body)
    }
}

/// Reads a Drive response, turning failures into a readable message.
fn reply(
    response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Value, ApiError> {
    let mut response =
        response.map_err(|e| ApiError::Other(format!("Couldn't reach Google Drive: {e}")))?;
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().unwrap_or_default();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    match status {
        200..=299 => Ok(body),
        404 => Err(ApiError::NotFound),
        _ => {
            let message = body["error"]["message"]
                .as_str()
                .or(body["error_description"].as_str())
                .unwrap_or("request failed");
            Err(ApiError::Other(format!("Google Drive {status}: {message}")))
        }
    }
}

fn file_id(body: Value) -> Result<String, ApiError> {
    body["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ApiError::Other("Google Drive didn't return a file id".into()))
}

impl Drive for Google {
    fn folder_name(&self, id: &str) -> Result<Option<String>, ApiError> {
        let body = reply(
            self.agent
                .get(format!("{FILES_URL}/{id}"))
                .query("fields", "name,trashed")
                .header("Authorization", &self.auth)
                .call(),
        )?;
        if body["trashed"].as_bool().unwrap_or(false) {
            return Ok(None);
        }
        Ok(Some(body["name"].as_str().unwrap_or_default().to_string()))
    }

    fn create_folder(&self, name: &str, parent: Option<&str>) -> Result<String, ApiError> {
        let mut metadata = json!({ "name": name, "mimeType": FOLDER_MIME });
        if let Some(parent) = parent {
            metadata["parents"] = json!([parent]);
        }
        file_id(reply(
            self.agent
                .post(FILES_URL)
                .query("fields", "id")
                .header("Authorization", &self.auth)
                .send_json(metadata),
        )?)
    }

    fn rename(&self, id: &str, name: &str) -> Result<(), ApiError> {
        reply(
            self.agent
                .patch(format!("{FILES_URL}/{id}"))
                .header("Authorization", &self.auth)
                .send_json(json!({ "name": name })),
        )
        .map(drop)
    }

    fn upload(&self, name: &str, markdown: &str, parent: &str) -> Result<String, ApiError> {
        // Drive converts the Markdown into a Google Doc.
        let metadata = json!({ "name": name, "mimeType": DOC_MIME, "parents": [parent] });
        let (content_type, body) = self.multipart(&metadata, markdown);
        file_id(reply(
            self.agent
                .post(UPLOAD_URL)
                .query("uploadType", "multipart")
                .query("fields", "id")
                .header("Authorization", &self.auth)
                .header("Content-Type", content_type)
                .send(&body[..]),
        )?)
    }

    fn update(&self, id: &str, name: &str, markdown: &str) -> Result<(), ApiError> {
        let (content_type, body) = self.multipart(&json!({ "name": name }), markdown);
        reply(
            self.agent
                .patch(format!("{UPLOAD_URL}/{id}"))
                .query("uploadType", "multipart")
                .query("fields", "id")
                .header("Authorization", &self.auth)
                .header("Content-Type", content_type)
                .send(&body[..]),
        )
        .map(drop)
    }

    fn trash(&self, id: &str) -> Result<(), ApiError> {
        reply(
            self.agent
                .patch(format!("{FILES_URL}/{id}"))
                .header("Authorization", &self.auth)
                .send_json(json!({ "trashed": true })),
        )
        .map(drop)
    }

    fn live_files(&self) -> Result<BTreeSet<String>, ApiError> {
        // `drive.file` only lists files this app made, so this stays small.
        let mut live = BTreeSet::new();
        let mut page: Option<String> = None;
        loop {
            let mut request = self
                .agent
                .get(FILES_URL)
                .query("q", "trashed = false")
                .query("fields", "nextPageToken,files(id)")
                .query("pageSize", "1000")
                .header("Authorization", &self.auth);
            if let Some(token) = &page {
                request = request.query("pageToken", token);
            }
            let body = reply(request.call())?;
            let files = body["files"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            live.extend(
                files
                    .iter()
                    .filter_map(|f| f["id"].as_str().map(str::to_string)),
            );
            match body["nextPageToken"].as_str() {
                Some(token) => page = Some(token.to_string()),
                None => return Ok(live),
            }
        }
    }
}

/// Runs Google's sign-in in the browser and trades the result for tokens.
fn sign_in(
    (client_id, client_secret): &(String, String),
    cancel: &AtomicBool,
    open_url: fn(&str) -> io::Result<()>,
) -> Result<SignIn, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Couldn't start sign-in: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Couldn't start sign-in: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    let verifier = random_token(32);
    let challenge = base64_url(&Sha256::digest(verifier.as_bytes()));
    let state = random_token(16);
    let url = format!(
        "{AUTH_URL}?{}",
        query(&[
            ("client_id", client_id),
            ("redirect_uri", &redirect),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
            // A refresh token, so sync keeps working after this session.
            ("access_type", "offline"),
            ("prompt", "consent"),
        ])
    );
    open_url(&url).map_err(|e| format!("Couldn't open the browser: {e}"))?;

    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    let code = loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Sign-in canceled".into());
        }
        if Instant::now() > deadline {
            return Err("Sign-in timed out. Try connecting again.".into());
        }
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(format!("Sign-in failed: {e}")),
        };
        match redirect_params(stream, &state) {
            Some(Ok(code)) => break code,
            Some(Err(e)) => return Err(e),
            // A favicon request or the like.
            None => continue,
        }
    };

    let mut response = agent()
        .post(TOKEN_URL)
        .send_form([
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect.as_str()),
        ])
        .map_err(|e| format!("Couldn't reach Google: {e}"))?;
    let status = response.status().as_u16();
    let body: Value = response.body_mut().read_json().unwrap_or(Value::Null);
    let (Some(access), Some(refresh)) = (
        body["access_token"].as_str(),
        body["refresh_token"].as_str(),
    ) else {
        let reason = body["error_description"]
            .as_str()
            .or(body["error"].as_str())
            .unwrap_or("no token returned");
        return Err(format!("Google sign-in failed ({status}): {reason}"));
    };
    let expires = Instant::now() + Duration::from_secs(body["expires_in"].as_u64().unwrap_or(3600));
    // Only for showing which account is connected.
    let account = agent()
        .get(ABOUT_URL)
        .query("fields", "user(emailAddress)")
        .header("Authorization", format!("Bearer {access}"))
        .call()
        .ok()
        .and_then(|mut r| r.body_mut().read_json::<Value>().ok())
        .and_then(|v| v["user"]["emailAddress"].as_str().map(str::to_string));
    Ok(SignIn {
        refresh_token: refresh.to_string(),
        access_token: access.to_string(),
        expires,
        account,
    })
}

/// Answers the browser's redirect. `None` when it isn't the redirect.
fn redirect_params(mut stream: TcpStream, state: &str) -> Option<Result<String, String>> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut request = Vec::new();
    let mut buf = [0u8; 2048];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 16 * 1024 {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let request = String::from_utf8_lossy(&request);
    let target = request.lines().next()?.split_whitespace().nth(1)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let params: BTreeMap<String, String> = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (decode(k), decode(v)))
        .collect();
    if path != "/" || (!params.contains_key("code") && !params.contains_key("error")) {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }
    let result = if params.get("state").map(String::as_str) != Some(state) {
        Err("Sign-in failed: the response didn't match. Try again.".to_string())
    } else if let Some(error) = params.get("error") {
        Err(if error == "access_denied" {
            "Sign-in canceled".to_string()
        } else {
            format!("Sign-in failed: {error}")
        })
    } else {
        Ok(params["code"].clone())
    };
    let message = match &result {
        Ok(_) => "Scripture Study is connected to Google Drive. You can close this tab.",
        Err(_) => "Scripture Study couldn't connect to Google Drive. You can close this tab and try again.",
    };
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>Scripture Study</title>\
         <body style=\"font:16px -apple-system,system-ui,sans-serif;margin:4em auto;max-width:32em;color:#37352f\">\
         <p>{message}</p>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    Some(result)
}

fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("system randomness");
    base64_url(&buf)
}

fn base64_url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// `a=b&c=d`, percent-encoded.
fn query(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Opens a web page in the default browser.
pub fn open_in_browser(url: &str) -> io::Result<()> {
    use std::process::Command;
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(windows) {
        // `start` would split the URL at each `&`.
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler").arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let mut child = command.spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use scripture_study_core::Document;

    use super::*;

    /// Name, parent, and whether it's trashed.
    type FakeFile = (String, Option<String>, bool);

    /// Drive in memory, by file id.
    #[derive(Default)]
    struct FakeDrive {
        files: RefCell<BTreeMap<String, FakeFile>>,
        calls: RefCell<Vec<String>>,
    }

    impl FakeDrive {
        fn add(&self, name: &str, parent: Option<&str>) -> Result<String, ApiError> {
            if let Some(parent) = parent {
                if !self.live(parent) {
                    return Err(ApiError::NotFound);
                }
            }
            let mut files = self.files.borrow_mut();
            let id = format!("id{}", files.len());
            files.insert(id.clone(), (name.into(), parent.map(Into::into), false));
            Ok(id)
        }

        /// Like Drive, a file in a trashed folder is trashed too.
        fn live(&self, id: &str) -> bool {
            let files = self.files.borrow();
            let mut id = Some(id.to_string());
            while let Some(current) = id {
                match files.get(&current) {
                    Some(file) if !file.2 => id = file.1.clone(),
                    _ => return false,
                }
            }
            true
        }

        fn names_in(&self, parent: &str) -> Vec<String> {
            let files = self.files.borrow();
            let mut names: Vec<String> = files
                .values()
                .filter(|f| !f.2 && f.1.as_deref() == Some(parent))
                .map(|f| f.0.clone())
                .collect();
            names.sort();
            names
        }
    }

    impl Drive for FakeDrive {
        fn folder_name(&self, id: &str) -> Result<Option<String>, ApiError> {
            let files = self.files.borrow();
            let file = files.get(id).ok_or(ApiError::NotFound)?;
            Ok((!file.2).then(|| file.0.clone()))
        }
        fn create_folder(&self, name: &str, parent: Option<&str>) -> Result<String, ApiError> {
            self.calls.borrow_mut().push(format!("folder {name}"));
            self.add(name, parent)
        }
        fn rename(&self, id: &str, name: &str) -> Result<(), ApiError> {
            let mut files = self.files.borrow_mut();
            files.get_mut(id).ok_or(ApiError::NotFound)?.0 = name.into();
            Ok(())
        }
        fn upload(&self, name: &str, _markdown: &str, parent: &str) -> Result<String, ApiError> {
            self.calls.borrow_mut().push(format!("upload {name}"));
            self.add(name, Some(parent))
        }
        fn update(&self, id: &str, name: &str, _markdown: &str) -> Result<(), ApiError> {
            self.calls.borrow_mut().push(format!("update {name}"));
            if !self.live(id) {
                return Err(ApiError::NotFound);
            }
            self.rename(id, name)
        }
        fn trash(&self, id: &str) -> Result<(), ApiError> {
            self.calls.borrow_mut().push(format!("trash {id}"));
            let mut files = self.files.borrow_mut();
            files.get_mut(id).ok_or(ApiError::NotFound)?.2 = true;
            Ok(())
        }
        fn live_files(&self) -> Result<BTreeSet<String>, ApiError> {
            let ids: Vec<String> = self.files.borrow().keys().cloned().collect();
            Ok(ids.into_iter().filter(|id| self.live(id)).collect())
        }
    }

    fn note(id: &str, markdown: &str) -> LocalNote {
        LocalNote::new(id, &Document::from_markdown(markdown))
    }

    /// Plans and applies until nothing is left, like a sync does.
    fn sync(drive: &FakeDrive, state: &mut SyncState, notes: &[LocalNote], folders: &[&str]) {
        let folders: Vec<String> = folders.iter().map(|f| f.to_string()).collect();
        state.keep_live(&drive.live_files().unwrap());
        for _ in 0..2 {
            let mut retry = false;
            for step in drive_sync::plan(notes, &folders, state) {
                match apply(drive, state, &step, notes) {
                    Ok(()) => {}
                    Err(ApiError::Retry) => {
                        retry = true;
                        break;
                    }
                    Err(e) => panic!("{e}"),
                }
            }
            if !retry {
                break;
            }
        }
        assert_eq!(drive_sync::plan(notes, &folders, state), []);
    }

    #[test]
    fn notes_land_in_mirrored_folders_and_follow_edits_and_deletes() {
        let drive = FakeDrive::default();
        let root = drive.create_folder("Scripture Study", None).unwrap();
        let mut state = SyncState::default();
        state.retarget(&root);

        let mut notes = vec![note("a", "# Alma 32\n"), note("Talks/b", "# Faith\n")];
        sync(&drive, &mut state, &notes, &["Talks"]);
        assert_eq!(drive.names_in(&root), ["Alma 32", "Talks"]);
        assert_eq!(drive.names_in(&state.folders["Talks"]), ["Faith"]);

        // Nothing changed, nothing sent.
        drive.calls.borrow_mut().clear();
        sync(&drive, &mut state, &notes, &["Talks"]);
        assert!(drive.calls.borrow().is_empty());

        // A retitled note is updated in place; a deleted one is trashed.
        notes = vec![note("a", "# Alma 32: The Seed\n")];
        sync(&drive, &mut state, &notes, &["Talks"]);
        assert_eq!(drive.names_in(&root), ["Alma 32: The Seed", "Talks"]);
        assert!(drive.names_in(&state.folders["Talks"]).is_empty());
    }

    #[test]
    fn files_removed_in_drive_are_put_back() {
        let drive = FakeDrive::default();
        let root = drive.create_folder("Scripture Study", None).unwrap();
        let mut state = SyncState::default();
        state.retarget(&root);
        let notes = vec![note("Talks/b", "# Faith\n")];
        sync(&drive, &mut state, &notes, &["Talks"]);

        // Someone trashes the subfolder in Drive, then the note changes.
        drive.trash(&state.folders["Talks"].clone()).unwrap();
        let notes = vec![note("Talks/b", "# Faith\n\nHope too.\n")];
        sync(&drive, &mut state, &notes, &["Talks"]);
        let talks = &state.folders["Talks"];
        assert!(drive.live(talks));
        assert_eq!(drive.names_in(talks), ["Faith"]);
    }

    #[test]
    fn the_sign_in_redirect_is_decoded() {
        assert_eq!(decode("4%2F0Ab-x%20y+z"), "4/0Ab-x y z");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(encode("http://127.0.0.1:80"), "http%3A%2F%2F127.0.0.1%3A80");
    }
}
