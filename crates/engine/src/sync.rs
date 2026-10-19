//! Optional settings sync between DeckCraft installs: a small JSON snapshot of the settings
//! categories the user picked, merged with last-writer-wins per category, stored in a folder, an
//! S3-compatible bucket or Google Drive.
//!
//! The local settings files are always the primary copy. Sync is opt-in, a failure leaves them
//! untouched, and a remote snapshot older than the one last seen is refused (stale protection)
//! instead of overwriting newer local work. Every merge keeps the losing value in the report so
//! the UI can show what changed.
//!
//! Transports use `curl` (like video export uses ffmpeg): no HTTP or TLS stack is linked into the
//! editor. S3 signing is curl's `--aws-sigv4`; Google Drive uses OAuth 2.0 refresh tokens. The
//! browser build has no sync: it has no file system or process to run curl.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{EngineError, Result};

/// A snapshot bigger than this is refused: settings are small, and a huge remote file would be a
/// denial of service rather than a settings file.
const MAX_SNAPSHOT_BYTES: usize = 1 << 20;

/// The settings a snapshot can carry, in the order the UI lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    /// Light or dark, interface size.
    Appearance,
    /// Ribbon tab, panel sizes, zoom and view.
    Layout,
    /// Author name, grid, smart guides, AutoCorrect and undo limit.
    Editing,
    /// The recent presentations list.
    Recent,
    /// Starred starter templates.
    Favourites,
    /// Quick Access Toolbar buttons.
    QuickAccess,
    /// Words added to the spelling dictionary.
    Dictionary,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Appearance,
        Category::Layout,
        Category::Editing,
        Category::Recent,
        Category::Favourites,
        Category::QuickAccess,
        Category::Dictionary,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Category::Appearance => "appearance",
            Category::Layout => "layout",
            Category::Editing => "editing",
            Category::Recent => "recent",
            Category::Favourites => "favourites",
            Category::QuickAccess => "quickAccess",
            Category::Dictionary => "dictionary",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::Layout => "Window layout and zoom",
            Category::Editing => "Editing preferences",
            Category::Recent => "Recent presentations",
            Category::Favourites => "Favourite templates",
            Category::QuickAccess => "Quick Access Toolbar",
            Category::Dictionary => "Spelling dictionary",
        }
    }

    pub fn parse(id: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(id.trim()))
    }

    /// The categories synced when the user turns sync on: everything but the recent list (which
    /// can name private files) and the dictionary (which can be large).
    pub fn defaults() -> Vec<Category> {
        vec![Category::Appearance, Category::Layout, Category::Editing, Category::Favourites, Category::QuickAccess]
    }
}

/// One category's value and when it last changed (Unix seconds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryValue {
    pub modified: i64,
    pub value: Value,
}

/// The document stored remotely (and built locally before a sync).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Format version; a newer one is refused rather than misread.
    pub version: u32,
    /// The install that wrote it (a stable random id).
    pub device: String,
    /// Counts merges, so a remote copy older than the last one seen can be spotted.
    pub revision: u64,
    /// When this snapshot was written (Unix seconds).
    pub modified: i64,
    pub categories: BTreeMap<String, CategoryValue>,
}

/// What the last sync saw, kept beside the local settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SyncState {
    pub device: String,
    pub revision: u64,
    /// Unix seconds; 0 when never synced.
    pub last_sync: i64,
    pub last_remote_revision: u64,
    /// FNV-1a of the remote bytes last merged, to notice a different file at the same revision.
    pub last_remote_hash: String,
    /// Each category's modified time when the last merge ended: the base for "both changed".
    pub base: BTreeMap<String, i64>,
    /// Each category's value when the last merge ended, so the next local snapshot keeps its
    /// modified time when the value has not changed.
    pub base_values: BTreeMap<String, Value>,
}

impl SyncState {
    pub fn sanitized(mut self) -> SyncState {
        self.device.retain(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
        if self.device.len() > 64 {
            self.device.truncate(64);
        }
        self.last_remote_hash.retain(|c| c.is_ascii_hexdigit());
        if self.last_remote_hash.len() > 32 {
            self.last_remote_hash.truncate(32);
        }
        self.base.retain(|k, _| Category::parse(k).is_some());
        self.base_values.retain(|k, _| Category::parse(k).is_some());
        self
    }
}

/// A category both sides changed differently: `kept` won, `lost` is the other value.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub category: Category,
    pub kept: Value,
    pub lost: Value,
    pub kept_device: String,
    pub lost_device: String,
}

/// The result of merging the local settings with the remote snapshot.
#[derive(Clone, Debug)]
pub struct Merge {
    /// The snapshot to keep locally and upload.
    pub snapshot: Snapshot,
    /// Categories whose remote value replaced the local one.
    pub applied: Vec<Category>,
    /// Categories whose local value won over a remote one.
    pub pushed: Vec<Category>,
    pub conflicts: Vec<Conflict>,
    /// The remote copy is older than the one last seen: nothing was applied.
    pub stale: bool,
}

/// FNV-1a, hex: a cheap stable fingerprint of the remote bytes (not a security hash).
pub fn fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn now_secs() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs().min(i64::MAX as u64) as i64).unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

/// Build a local snapshot from `values` (category id → value), keeping the previous modified
/// time for a category whose value did not change. A category not in `selected` is left out.
pub fn snapshot(device: &str, selected: &[Category], values: &BTreeMap<String, Value>, previous: Option<&Snapshot>) -> Snapshot {
    let now = now_secs();
    let mut categories = BTreeMap::new();
    for cat in selected {
        let Some(value) = values.get(cat.id()) else { continue };
        let modified = previous.and_then(|p| p.categories.get(cat.id())).filter(|old| old.value == *value).map(|old| old.modified).unwrap_or(now);
        categories.insert(cat.id().to_string(), CategoryValue { modified, value: value.clone() });
    }
    Snapshot { version: 1, device: device.to_string(), revision: previous.map(|p| p.revision).unwrap_or(0), modified: now, categories }
}

/// Merge `local` with `remote` for `selected`, using `state` as the common base.
///
/// - A remote revision below the last one seen is stale: return the local snapshot untouched.
/// - A category only one side changed takes that side.
/// - A category both sides changed takes the newer `modified`; a tie goes to the lexicographically
///   greater device id, so every install makes the same choice.
/// - A category absent remotely keeps the local value; a category only remote has is taken.
/// - Remote categories the user did not select are kept, so syncing one category never deletes
///   another install's settings.
pub fn merge(local: &Snapshot, remote: &Snapshot, selected: &[Category], state: &SyncState) -> Merge {
    if remote.version > 1 {
        return Merge { snapshot: local.clone(), applied: vec![], pushed: vec![], conflicts: vec![], stale: true };
    }
    if remote.revision < state.last_remote_revision {
        return Merge { snapshot: local.clone(), applied: vec![], pushed: vec![], conflicts: vec![], stale: true };
    }
    let mut categories = remote.categories.clone();
    let mut applied = vec![];
    let mut pushed = vec![];
    let mut conflicts = vec![];
    for cat in selected {
        let id = cat.id();
        let l = local.categories.get(id);
        let r = remote.categories.get(id);
        let base = state.base.get(id).copied();
        let l_changed = l.is_some_and(|v| base != Some(v.modified));
        let r_changed = r.is_some_and(|v| base != Some(v.modified));
        let chosen = match (l, r) {
            (Some(l), Some(r)) => {
                if l.value == r.value {
                    // Same value: keep the remote entry (and its modified time) so an unchanged
                    // sync does not invent a difference and rewrite the remote file.
                    r.clone()
                } else if l_changed && r_changed {
                    // Both changed since the last sync: newest wins, device breaks a tie.
                    let remote_wins = r.modified > l.modified || (r.modified == l.modified && remote.device > local.device);
                    let (kept, lost) = if remote_wins { (r, l) } else { (l, r) };
                    conflicts.push(Conflict {
                        category: *cat,
                        kept: kept.value.clone(),
                        lost: lost.value.clone(),
                        kept_device: if remote_wins { remote.device.clone() } else { local.device.clone() },
                        lost_device: if remote_wins { local.device.clone() } else { remote.device.clone() },
                    });
                    if remote_wins {
                        applied.push(*cat);
                    } else {
                        pushed.push(*cat);
                    }
                    kept.clone()
                } else if r_changed {
                    applied.push(*cat);
                    r.clone()
                } else {
                    if l_changed {
                        pushed.push(*cat);
                    }
                    l.clone()
                }
            }
            (Some(l), None) => {
                if l_changed {
                    pushed.push(*cat);
                }
                l.clone()
            }
            (None, Some(r)) => {
                if r_changed {
                    applied.push(*cat);
                }
                r.clone()
            }
            (None, None) => continue,
        };
        categories.insert(id.to_string(), chosen);
    }
    // Only a snapshot that differs from the remote one needs a new revision; otherwise a sync
    // that changed nothing would grow the revision and rewrite the remote file every time.
    let unchanged = categories == remote.categories;
    let snapshot = Snapshot {
        version: 1,
        // An unchanged merge is the remote snapshot: keep its device too, so the caller's
        // "already up to date" check does not see a difference and re-upload it.
        device: if unchanged { remote.device.clone() } else { local.device.clone() },
        revision: if unchanged { remote.revision } else { local.revision.max(remote.revision).saturating_add(1) },
        modified: if unchanged { remote.modified } else { now_secs() },
        categories,
    };
    Merge { snapshot, applied, pushed, conflicts, stale: false }
}

/// Where a snapshot lives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    #[default]
    LocalFolder,
    S3,
    GoogleDrive,
}

impl Provider {
    pub fn id(self) -> &'static str {
        match self {
            Provider::LocalFolder => "localFolder",
            Provider::S3 => "s3",
            Provider::GoogleDrive => "googleDrive",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Provider::LocalFolder => "A folder on this computer or a shared drive",
            Provider::S3 => "S3-compatible storage",
            Provider::GoogleDrive => "Google Drive",
        }
    }
    pub fn parse(id: &str) -> Option<Provider> {
        match id.trim().to_ascii_lowercase().as_str() {
            "localfolder" | "folder" | "local" => Some(Provider::LocalFolder),
            "s3" | "s3compatible" => Some(Provider::S3),
            "googledrive" | "drive" | "google" => Some(Provider::GoogleDrive),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct S3Config {
    /// e.g. `https://s3.eu-west-1.amazonaws.com` or a MinIO endpoint. Empty uses AWS.
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    /// Object key prefix, without leading or trailing slashes.
    pub prefix: String,
    pub access_key: String,
    pub secret_key: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DriveConfig {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    /// A short-lived access token, when the user already has one. Never written by the app.
    #[serde(skip_serializing)]
    pub access_token: String,
    /// A Drive folder id; empty for My Drive's root.
    pub folder_id: String,
    pub file_name: String,
}

impl DriveConfig {
    pub fn file_name(&self) -> &str {
        if self.file_name.trim().is_empty() { "deckcraft-settings.json" } else { self.file_name.trim() }
    }
}

/// The sync settings, saved beside the local settings. Secrets live here: the file is written
/// owner-only, and the values are never logged or put in a command journal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SyncConfig {
    pub enabled: bool,
    pub provider: Provider,
    /// Sync when the app starts and after settings change.
    pub auto: bool,
    pub interval_minutes: u32,
    /// Category ids; empty means [`Category::defaults`].
    pub categories: Vec<String>,
    pub local_folder: String,
    pub s3: S3Config,
    pub drive: DriveConfig,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            enabled: false,
            provider: Provider::LocalFolder,
            auto: false,
            interval_minutes: 30,
            categories: Category::defaults().iter().map(|c| c.id().to_string()).collect(),
            local_folder: String::new(),
            s3: S3Config::default(),
            drive: DriveConfig::default(),
        }
    }
}

impl SyncConfig {
    pub fn sanitized(mut self) -> SyncConfig {
        self.interval_minutes = self.interval_minutes.clamp(1, 24 * 60);
        self.categories.retain(|c| Category::parse(c).is_some());
        self.categories.dedup();
        if self.categories.is_empty() {
            self.categories = Category::defaults().iter().map(|c| c.id().to_string()).collect();
        }
        self.s3.region = clean(&self.s3.region, 64);
        if self.s3.region.is_empty() {
            self.s3.region = "us-east-1".into();
        }
        self.s3.endpoint = clean(&self.s3.endpoint, 512);
        self.s3.bucket = clean(&self.s3.bucket, 255);
        self.s3.prefix = self.s3.prefix.trim_matches('/').to_string();
        self.s3.prefix = clean(&self.s3.prefix, 512);
        self.local_folder = self.local_folder.trim().to_string();
        if self.local_folder.len() > 4096 {
            self.local_folder.truncate(4096);
        }
        self.drive.file_name = clean(self.drive.file_name(), 128);
        self.drive.folder_id = clean(&self.drive.folder_id, 128);
        self
    }

    pub fn selected(&self) -> Vec<Category> {
        let mut out: Vec<Category> = self.categories.iter().filter_map(|c| Category::parse(c)).collect();
        if out.is_empty() {
            out = Category::defaults();
        }
        out
    }

    /// The object key / file name used remotely.
    pub fn object_key(&self) -> String {
        let name = "settings.json";
        if self.provider == Provider::S3 && !self.s3.prefix.is_empty() { format!("{}/{name}", self.s3.prefix) } else { name.to_string() }
    }
}

fn clean(s: &str, max: usize) -> String {
    s.trim().chars().filter(|c| !c.is_control()).take(max).collect()
}

/// What a sync did.
#[derive(Clone, Debug)]
pub struct Report {
    pub provider: Provider,
    pub revision: u64,
    pub uploaded: bool,
    pub downloaded: bool,
    pub applied: Vec<Category>,
    pub pushed: Vec<Category>,
    pub conflicts: Vec<Conflict>,
    pub stale: bool,
    pub bytes: usize,
    /// The merged settings: the caller applies `applied` categories from it and keeps the rest.
    pub snapshot: Snapshot,
}

impl Report {
    pub fn summary(&self) -> String {
        if self.stale {
            return "The remote settings are older than the last sync; nothing was changed.".into();
        }
        let mut parts = vec![];
        if self.downloaded && !self.applied.is_empty() {
            parts.push(format!("{} setting{} updated", self.applied.len(), if self.applied.len() == 1 { "" } else { "s" }));
        }
        if self.uploaded {
            parts.push("uploaded".into());
        }
        if !self.conflicts.is_empty() {
            parts.push(format!("{} conflict{} resolved", self.conflicts.len(), if self.conflicts.len() == 1 { "" } else { "s" }));
        }
        if parts.is_empty() {
            parts.push("already up to date".into());
        }
        format!("Settings sync: {}", parts.join(", "))
    }
}

/// A place a snapshot can be read from and written to.
pub trait Transport {
    fn name(&self) -> &'static str;
    fn provider(&self) -> Provider;
    fn load(&mut self) -> Result<Option<Vec<u8>>>;
    fn save(&mut self, bytes: &[u8]) -> Result<()>;
}

/// A folder: the snapshot is `settings.json` inside it.
pub struct FolderTransport {
    pub dir: std::path::PathBuf,
}

impl Transport for FolderTransport {
    fn name(&self) -> &'static str {
        "folder"
    }
    fn provider(&self) -> Provider {
        Provider::LocalFolder
    }
    fn load(&mut self) -> Result<Option<Vec<u8>>> {
        let path = self.dir.join("settings.json");
        let bytes = crate::io::read_app_file_optional(&path).map_err(EngineError::Other)?;
        if let Some(b) = &bytes
            && b.len() > MAX_SNAPSHOT_BYTES
        {
            return Err(EngineError::Other(format!("{} is too large to be a settings snapshot", path.display())));
        }
        Ok(bytes)
    }
    fn save(&mut self, bytes: &[u8]) -> Result<()> {
        crate::io::write_app_file(&self.dir.join("settings.json"), bytes).map_err(EngineError::Other)
    }
}

/// Read and parse a snapshot; `None` for empty bytes.
pub fn parse(bytes: &[u8]) -> Result<Option<Snapshot>> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let snap: Snapshot =
        serde_json::from_slice(bytes).map_err(|e| EngineError::Other(format!("the remote settings are not a valid snapshot: {e}")))?;
    if snap.version > 1 {
        return Err(EngineError::Other(format!("the remote settings use a newer format (version {})", snap.version)));
    }
    Ok(Some(snap))
}

/// The bytes written remotely: pretty JSON, so a person can inspect or repair it.
pub fn encode(snapshot: &Snapshot) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(snapshot).map_err(|e| EngineError::Other(format!("can't encode the settings snapshot: {e}")))
}

/// Pull, merge and push one snapshot through `transport`.
///
/// `local` is the snapshot built from the settings on this computer; `state` is updated in place
/// (call [`SyncState::sanitized`] after loading it). A remote that is missing is created. The
/// upload is skipped when the merged bytes equal the remote ones.
pub fn sync(transport: &mut dyn Transport, local: &Snapshot, selected: &[Category], state: &mut SyncState) -> Result<Report> {
    let remote_bytes = transport.load()?;
    let remote = remote_bytes.as_deref().map(parse).transpose()?.flatten();
    let (merged, applied, pushed, conflicts, stale) = match &remote {
        Some(r) => {
            let m = merge(local, r, selected, state);
            (m.snapshot, m.applied, m.pushed, m.conflicts, m.stale)
        }
        None => (local.clone(), vec![], vec![], vec![], false),
    };
    let provider = transport.provider();
    if stale {
        return Ok(Report {
            provider,
            revision: merged.revision,
            uploaded: false,
            downloaded: false,
            applied,
            pushed,
            conflicts,
            stale: true,
            bytes: 0,
            snapshot: merged,
        });
    }
    // Compare parsed snapshots, not bytes: a remote file saved with different whitespace is still
    // the same settings and must not be rewritten (or grow a revision) on every sync.
    let upload = remote.as_ref() != Some(&merged);
    let bytes = encode(&merged)?;
    if upload {
        transport.save(&bytes)?;
    }
    state.revision = merged.revision;
    state.last_sync = now_secs();
    state.last_remote_revision = merged.revision;
    state.last_remote_hash = if upload { fingerprint(&bytes) } else { remote_bytes.as_deref().map(fingerprint).unwrap_or_default() };
    state.base = merged.categories.iter().map(|(k, v)| (k.clone(), v.modified)).collect();
    state.base_values = merged.categories.iter().map(|(k, v)| (k.clone(), v.value.clone())).collect();
    Ok(Report {
        provider,
        revision: merged.revision,
        uploaded: upload,
        downloaded: remote.is_some(),
        applied,
        pushed,
        conflicts,
        stale: false,
        bytes: bytes.len(),
        snapshot: merged,
    })
}

/// Load the sync settings from `path`; a missing file is the default (sync off). A damaged file
/// is an error rather than silently forgotten settings, so the UI can tell the user.
pub fn load_config(path: &std::path::Path) -> Result<SyncConfig> {
    let Some(bytes) = crate::io::read_app_file_optional(path).map_err(EngineError::Other)? else {
        return Ok(SyncConfig::default());
    };
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(EngineError::Other(format!("{} is too large to be a settings file", path.display())));
    }
    let config: SyncConfig = serde_json::from_slice(&bytes).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?;
    Ok(config.sanitized())
}

/// Save the sync settings owner-only: the file holds S3 keys and a Drive refresh token.
pub fn save_config(path: &std::path::Path, config: &SyncConfig) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(config).map_err(|e| EngineError::Other(format!("can't encode sync settings: {e}")))?;
    crate::io::write_app_private_file(path, &bytes).map_err(EngineError::Other)
}

/// Load the last-sync state; missing or damaged state is "never synced", which is safe (the next
/// sync merges from scratch rather than trusting a base that may be wrong).
pub fn load_state(path: &std::path::Path) -> SyncState {
    let Ok(Some(bytes)) = crate::io::read_app_file_optional(path) else { return SyncState::default() };
    serde_json::from_slice::<SyncState>(&bytes).map(SyncState::sanitized).unwrap_or_default()
}

pub fn save_state(path: &std::path::Path, state: &SyncState) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| EngineError::Other(format!("can't encode sync state: {e}")))?;
    crate::io::write_app_file(path, &bytes).map_err(EngineError::Other)
}

/// A stable random-looking install id, made without a random-number dependency: time, process
/// and a counter are mixed by FNV-1a. It only has to differ between installs.
pub fn new_device_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    #[cfg(not(target_arch = "wasm32"))]
    let pid = std::process::id();
    #[cfg(target_arch = "wasm32")]
    let pid: u32 = 0;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in nanos.to_le_bytes().iter().chain(pid.to_le_bytes().iter()).chain(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes().iter()) {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The result of running one command (curl, in practice).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandOutput {
    /// The process exit status; non-zero means curl itself failed.
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs an external command. Transports take one so their request construction can be tested
/// without a network: the tests replace curl with a scripted runner.
pub trait Runner {
    fn run(&mut self, program: &str, args: &[String], stdin: Option<&[u8]>) -> Result<CommandOutput>;
}

/// The real runner: spawn the command, write `stdin`, collect both streams.
#[cfg(not(target_arch = "wasm32"))]
pub struct SystemRunner;

#[cfg(not(target_arch = "wasm32"))]
impl Runner for SystemRunner {
    fn run(&mut self, program: &str, args: &[String], stdin: Option<&[u8]>) -> Result<CommandOutput> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new(program)
            .args(args)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if program == "curl" {
                    EngineError::Other(format!("cloud sync needs curl, which could not be started: {e}"))
                } else {
                    EngineError::Other(format!("{program} could not be started: {e}"))
                }
            })?;
        if let Some(bytes) = stdin
            && let Some(mut pipe) = child.stdin.take()
        {
            pipe.write_all(bytes).map_err(|e| EngineError::Other(format!("could not send data to {program}: {e}")))?;
        }
        let out = child.wait_with_output().map_err(|e| EngineError::Other(format!("{program} failed: {e}")))?;
        Ok(CommandOutput { status: out.status.code().unwrap_or(-1), stdout: out.stdout, stderr: out.stderr })
    }
}

/// The browser has no processes: sync reports that it is desktop-only instead of pretending.
#[cfg(target_arch = "wasm32")]
pub struct SystemRunner;

#[cfg(target_arch = "wasm32")]
impl Runner for SystemRunner {
    fn run(&mut self, _program: &str, _args: &[String], _stdin: Option<&[u8]>) -> Result<CommandOutput> {
        Err(EngineError::Other("settings sync is not available in the browser".into()))
    }
}

/// Quote a value for a curl config file (`--config -`). Credentials go through stdin, never the
/// command line, so they do not appear in the process list.
fn curl_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn curl_config(lines: &[(&str, String)]) -> String {
    let mut out = String::new();
    for (key, value) in lines {
        out.push_str(key);
        if !value.is_empty() {
            out.push_str(" = ");
            out.push_str(&curl_quote(value));
        }
        out.push('\n');
    }
    out
}

/// Split curl's `--write-out "\n%{http_code}"` suffix from the response body.
fn split_status(stdout: &[u8]) -> (Vec<u8>, u16) {
    let Some(pos) = stdout.iter().rposition(|b| *b == b'\n') else { return (stdout.to_vec(), 0) };
    let code = std::str::from_utf8(&stdout[pos + 1..]).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    (stdout[..pos].to_vec(), code)
}

fn response_error(what: &str, code: u16, body: &[u8]) -> EngineError {
    let detail = String::from_utf8_lossy(body);
    let detail = detail.trim();
    let mut end = detail.len().min(300);
    while end > 0 && !detail.is_char_boundary(end) {
        end -= 1;
    }
    let detail = &detail[..end];
    EngineError::Other(if detail.is_empty() { format!("{what} failed (HTTP {code})") } else { format!("{what} failed (HTTP {code}): {detail}") })
}

/// Percent-encode an S3 object key, keeping `/` as the path separator.
fn encode_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for b in key.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn temp_body(bytes: &[u8]) -> Result<std::path::PathBuf> {
    use std::io::Write;
    let (path, mut file) = crate::io::create_temp("deckcraft-sync").map_err(EngineError::Other)?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        crate::io::remove_file(&path);
        return Err(EngineError::Other(format!("could not write a temporary sync file: {e}")));
    }
    drop(file);
    Ok(path)
}

/// A snapshot in an S3-compatible bucket, addressed path-style: `endpoint/bucket/key`. Signing is
/// curl's SigV4 support, so no AWS SDK is linked in.
pub struct S3Transport {
    pub config: S3Config,
    pub key: String,
    runner: Box<dyn Runner>,
}

impl S3Transport {
    pub fn new(config: S3Config, key: String) -> Self {
        S3Transport { config, key, runner: Box::new(SystemRunner) }
    }

    pub fn with_runner(config: S3Config, key: String, runner: Box<dyn Runner>) -> Self {
        S3Transport { config, key, runner }
    }

    fn url(&self) -> Result<String> {
        let bucket = self.config.bucket.trim();
        if bucket.is_empty() {
            return Err(EngineError::Other("S3 sync needs a bucket name".into()));
        }
        let key = encode_key(self.key.trim_start_matches('/'));
        if self.config.endpoint.trim().is_empty() {
            let region = if self.config.region.trim().is_empty() { "us-east-1" } else { self.config.region.trim() };
            Ok(format!("https://{bucket}.s3.{region}.amazonaws.com/{key}"))
        } else {
            Ok(format!("{}/{bucket}/{key}", self.config.endpoint.trim().trim_end_matches('/')))
        }
    }

    fn base_config(&self) -> Result<Vec<(&'static str, String)>> {
        if self.config.access_key.trim().is_empty() || self.config.secret_key.is_empty() {
            return Err(EngineError::Other("S3 sync needs an access key and secret key".into()));
        }
        let region = if self.config.region.trim().is_empty() { "us-east-1" } else { self.config.region.trim() };
        Ok(vec![
            ("url", self.url()?),
            ("user", format!("{}:{}", self.config.access_key.trim(), self.config.secret_key)),
            ("aws-sigv4", format!("aws:amz:{region}:s3")),
            ("silent", String::new()),
            ("show-error", String::new()),
            ("write-out", "\n%{http_code}".into()),
        ])
    }

    fn request(&mut self, method: &str, body: Option<&[u8]>) -> Result<(u16, Vec<u8>)> {
        let mut lines = self.base_config()?;
        lines.push(("request", method.to_string()));
        let config = curl_config(&lines);
        let args: Vec<String> = vec!["--config".into(), "-".into()];
        let (out, temp) = match body {
            Some(bytes) => {
                let path = temp_body(bytes)?;
                let args = vec!["--config".into(), "-".into(), "--data-binary".into(), format!("@{}", path.display())];
                (self.runner.run("curl", &args, Some(config.as_bytes())), Some(path))
            }
            None => (self.runner.run("curl", &args, Some(config.as_bytes())), None),
        };
        if let Some(path) = temp {
            crate::io::remove_file(&path);
        }
        let out = out?;
        if out.status != 0 {
            return Err(EngineError::Other(format!("S3 request failed: {}", String::from_utf8_lossy(&out.stderr).trim())));
        }
        let (body, code) = split_status(&out.stdout);
        if code == 0 {
            return Err(EngineError::Other("S3 request returned no HTTP status".into()));
        }
        Ok((code, body))
    }
}

impl Transport for S3Transport {
    fn name(&self) -> &'static str {
        "s3"
    }
    fn provider(&self) -> Provider {
        Provider::S3
    }
    fn load(&mut self) -> Result<Option<Vec<u8>>> {
        let (code, body) = self.request("GET", None)?;
        match code {
            200 => {
                if body.len() > MAX_SNAPSHOT_BYTES {
                    return Err(EngineError::Other("the S3 settings snapshot is too large".into()));
                }
                Ok(Some(body))
            }
            404 => Ok(None),
            _ => Err(response_error("reading the S3 settings", code, &body)),
        }
    }
    fn save(&mut self, bytes: &[u8]) -> Result<()> {
        let (code, body) = self.request("PUT", Some(bytes))?;
        if (200..300).contains(&code) { Ok(()) } else { Err(response_error("writing the S3 settings", code, &body)) }
    }
}

/// A snapshot in Google Drive, found by name in an optional folder. OAuth 2.0 refresh tokens are
/// exchanged through curl; the access token is kept in memory for the sync and never written.
pub struct DriveTransport {
    pub config: DriveConfig,
    pub key: String,
    runner: Box<dyn Runner>,
    token: String,
}

impl DriveTransport {
    pub fn new(config: DriveConfig, key: String) -> Self {
        DriveTransport { config, key, runner: Box::new(SystemRunner), token: String::new() }
    }

    pub fn with_runner(config: DriveConfig, key: String, runner: Box<dyn Runner>) -> Self {
        DriveTransport { config, key, runner, token: String::new() }
    }

    fn run_config(&mut self, lines: &[(&str, String)], args: &[String]) -> Result<(u16, Vec<u8>)> {
        let config = curl_config(lines);
        let out = self.runner.run("curl", args, Some(config.as_bytes()))?;
        if out.status != 0 {
            return Err(EngineError::Other(format!("Google Drive request failed: {}", String::from_utf8_lossy(&out.stderr).trim())));
        }
        let (body, code) = split_status(&out.stdout);
        if code == 0 {
            return Err(EngineError::Other("Google Drive returned no HTTP status".into()));
        }
        Ok((code, body))
    }

    fn token(&mut self) -> Result<String> {
        if !self.token.is_empty() {
            return Ok(self.token.clone());
        }
        if !self.config.access_token.trim().is_empty() {
            self.token = self.config.access_token.trim().to_string();
            return Ok(self.token.clone());
        }
        if self.config.refresh_token.trim().is_empty() || self.config.client_id.trim().is_empty() {
            return Err(EngineError::Other("Google Drive sync needs a client id and refresh token".into()));
        }
        let lines = vec![
            ("url", "https://oauth2.googleapis.com/token".to_string()),
            ("silent", String::new()),
            ("show-error", String::new()),
            ("write-out", "\n%{http_code}".into()),
            ("data-urlencode", format!("client_id={}", self.config.client_id.trim())),
            ("data-urlencode", format!("client_secret={}", self.config.client_secret)),
            ("data-urlencode", format!("refresh_token={}", self.config.refresh_token.trim())),
            ("data-urlencode", "grant_type=refresh_token".into()),
        ];
        let (code, body) = self.run_config(&lines, &["--config".into(), "-".into()])?;
        if !(200..300).contains(&code) {
            return Err(response_error("refreshing the Google Drive token", code, &body));
        }
        let value: Value =
            serde_json::from_slice(&body).map_err(|e| EngineError::Other(format!("Google Drive sent an unexpected token response: {e}")))?;
        let token = value.get("access_token").and_then(Value::as_str).unwrap_or("").to_string();
        if token.is_empty() {
            return Err(EngineError::Other("Google Drive did not return an access token".into()));
        }
        self.token = token.clone();
        Ok(token)
    }

    /// The id of the snapshot file, or `None` when it does not exist yet.
    fn find_file(&mut self) -> Result<Option<String>> {
        let token = self.token()?;
        let mut query = format!("name = '{}' and trashed = false", self.config.file_name().replace('\'', "\\'"));
        if !self.config.folder_id.trim().is_empty() {
            query.push_str(&format!(" and '{}' in parents", self.config.folder_id.trim()));
        }
        let lines = vec![
            ("url", "https://www.googleapis.com/drive/v3/files".to_string()),
            ("header", format!("Authorization: Bearer {token}")),
            ("silent", String::new()),
            ("show-error", String::new()),
            ("write-out", "\n%{http_code}".into()),
            ("get", String::new()),
            ("data-urlencode", format!("q={query}")),
            ("data-urlencode", "fields=files(id,modifiedTime)".into()),
            ("data-urlencode", "pageSize=20".into()),
            ("data-urlencode", "supportsAllDrives=true".into()),
            ("data-urlencode", "includeItemsFromAllDrives=true".into()),
        ];
        let (code, body) = self.run_config(&lines, &["--config".into(), "-".into()])?;
        if !(200..300).contains(&code) {
            return Err(response_error("listing Google Drive settings", code, &body));
        }
        let value: Value =
            serde_json::from_slice(&body).map_err(|e| EngineError::Other(format!("Google Drive sent an unexpected file list: {e}")))?;
        let files = value.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut best: Option<(String, String)> = None;
        for file in files {
            let Some(id) = file.get("id").and_then(Value::as_str) else { continue };
            if !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')) {
                continue;
            }
            let modified = file.get("modifiedTime").and_then(Value::as_str).unwrap_or("").to_string();
            if best.as_ref().is_none_or(|(_, m)| modified > *m) {
                best = Some((id.to_string(), modified));
            }
        }
        Ok(best.map(|(id, _)| id))
    }

    fn download(&mut self, id: &str) -> Result<Option<Vec<u8>>> {
        let token = self.token()?;
        let lines = vec![
            ("url", format!("https://www.googleapis.com/drive/v3/files/{id}?alt=media&supportsAllDrives=true")),
            ("header", format!("Authorization: Bearer {token}")),
            ("silent", String::new()),
            ("show-error", String::new()),
            ("write-out", "\n%{http_code}".into()),
        ];
        let (code, body) = self.run_config(&lines, &["--config".into(), "-".into()])?;
        match code {
            200 => {
                if body.len() > MAX_SNAPSHOT_BYTES {
                    return Err(EngineError::Other("the Google Drive settings snapshot is too large".into()));
                }
                Ok(Some(body))
            }
            404 => Ok(None),
            _ => Err(response_error("reading the Google Drive settings", code, &body)),
        }
    }

    fn upload(&mut self, id: Option<&str>, bytes: &[u8]) -> Result<()> {
        let token = self.token()?;
        let mut metadata = serde_json::Map::new();
        metadata.insert("name".into(), Value::String(self.config.file_name().to_string()));
        if id.is_none() && !self.config.folder_id.trim().is_empty() {
            metadata.insert("parents".into(), Value::Array(vec![Value::String(self.config.folder_id.trim().to_string())]));
        }
        let metadata =
            serde_json::to_vec(&Value::Object(metadata)).map_err(|e| EngineError::Other(format!("can't encode Google Drive metadata: {e}")))?;
        let boundary = "deckcraft-sync-boundary-7f3a";
        let mut body = Vec::with_capacity(metadata.len() + bytes.len() + 256);
        body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes());
        body.extend_from_slice(&metadata);
        body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Type: application/json\r\n\r\n").as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let path = temp_body(&body)?;
        let (url, request) = match id {
            Some(id) => (format!("https://www.googleapis.com/upload/drive/v3/files/{id}?uploadType=multipart&supportsAllDrives=true"), "PATCH"),
            None => ("https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&supportsAllDrives=true".to_string(), "POST"),
        };
        let lines = vec![
            ("url", url),
            ("header", format!("Authorization: Bearer {token}")),
            ("header", format!("Content-Type: multipart/related; boundary={boundary}")),
            ("silent", String::new()),
            ("show-error", String::new()),
            ("write-out", "\n%{http_code}".into()),
            ("request", request.into()),
        ];
        let args = vec!["--config".into(), "-".into(), "--data-binary".into(), format!("@{}", path.display())];
        let result = self.run_config(&lines, &args);
        crate::io::remove_file(&path);
        let (code, body) = result?;
        if (200..300).contains(&code) { Ok(()) } else { Err(response_error("writing the Google Drive settings", code, &body)) }
    }
}

impl Transport for DriveTransport {
    fn name(&self) -> &'static str {
        "googleDrive"
    }
    fn provider(&self) -> Provider {
        Provider::GoogleDrive
    }
    fn load(&mut self) -> Result<Option<Vec<u8>>> {
        let Some(id) = self.find_file()? else { return Ok(None) };
        self.download(&id)
    }
    fn save(&mut self, bytes: &[u8]) -> Result<()> {
        let id = self.find_file()?;
        self.upload(id.as_deref(), bytes)
    }
}

/// The transport for `config`, with the usual runner. Sync is desktop-only; the browser gets the
/// error from [`SystemRunner`] when it tries.
pub fn transport(config: &SyncConfig) -> Result<Box<dyn Transport>> {
    match config.provider {
        Provider::LocalFolder => {
            let folder = config.local_folder.trim();
            if folder.is_empty() {
                return Err(EngineError::Other("choose a folder for settings sync".into()));
            }
            Ok(Box::new(FolderTransport { dir: std::path::PathBuf::from(folder) }))
        }
        Provider::S3 => Ok(Box::new(S3Transport::new(config.s3.clone(), config.object_key()))),
        Provider::GoogleDrive => Ok(Box::new(DriveTransport::new(config.drive.clone(), config.object_key()))),
    }
}

#[cfg(test)]
mod tests;
