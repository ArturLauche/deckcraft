//! Shared-folder live collaboration: every peer appends commands to its own journal beside the
//! presentation, and every peer tails the others' journals. A shared drive, a Syncthing folder or
//! a network share is the transport, so there is no server to run and collaboration keeps working
//! offline.
//!
//! The journal is ordered by `(time, peer, seq)`. Commands are applied through the normal command
//! engine, so a remote edit behaves exactly like a local one. Undo, redo and repeat publish a new
//! snapshot instead of an inverse command: every peer then adopts the same document and starts a
//! fresh history. Presence and cursors are small per-peer JSON files. A peer that falls behind
//! requests a snapshot; an editor answers with one, and the others reload it.
//!
//! This is deliberately a journal protocol, not a CRDT: two edits to the same text at the same
//! instant are ordered by their timestamps, and the later command wins. The same journal also runs
//! over a self-hosted TCP relay (`collab::relay`), which stores the room's files and serves them
//! to peers in real time; the folder transport remains the offline default.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use deckcraft_model::Presentation;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{EngineError, Result, Session};

#[cfg(not(target_arch = "wasm32"))]
pub mod relay;

/// The browser has no TCP sockets. The relay exists so the command registry and shared code
/// compile; every operation explains that a relay needs the desktop app.
#[cfg(target_arch = "wasm32")]
pub mod relay {
    use super::Store;
    use crate::{EngineError, Result};

    pub struct RelayStore;

    impl RelayStore {
        pub fn new(_endpoint: &str, _room: &str, _token: &str) -> Result<Self> {
            Err(EngineError::Other("live collaboration over a relay needs the desktop app".into()))
        }
    }

    impl Store for RelayStore {
        fn describe(&self) -> String {
            "relay (unavailable in the browser)".into()
        }
        fn prepare(&mut self) -> Result<()> {
            Err(EngineError::Other("live collaboration over a relay needs the desktop app".into()))
        }
        fn read(&mut self, _name: &str) -> Result<Option<Vec<u8>>> {
            Err(EngineError::Other("live collaboration over a relay needs the desktop app".into()))
        }
        fn write(&mut self, _name: &str, _bytes: &[u8]) -> Result<()> {
            Err(EngineError::Other("live collaboration over a relay needs the desktop app".into()))
        }
        fn append(&mut self, _name: &str, _bytes: &[u8]) -> Result<()> {
            Err(EngineError::Other("live collaboration over a relay needs the desktop app".into()))
        }
        fn len(&mut self, _name: &str) -> Result<Option<u64>> {
            Err(EngineError::Other("live collaboration over a relay needs the desktop app".into()))
        }
        fn list(&mut self) -> Result<Vec<String>> {
            Err(EngineError::Other("live collaboration over a relay needs the desktop app".into()))
        }
        fn remove(&mut self, _name: &str) -> Result<()> {
            Err(EngineError::Other("live collaboration over a relay needs the desktop app".into()))
        }
    }
}

/// Folder beside the presentation, e.g. `talk.deckcraft.collab/`.
pub const DIR_SUFFIX: &str = ".collab";
pub const SNAPSHOT_FILE: &str = "snapshot.json";
const PEER_PREFIX: &str = "peer-";
const JOURNAL_PREFIX: &str = "journal-";
const RECOVERY_PREFIX: &str = "recovery-";
const STALE_MS: i64 = 15_000;
const HEARTBEAT_MS: i64 = 1_000;
const MAX_SNAPSHOT_BYTES: usize = 32 << 20;
const MAX_JOURNAL_BYTES: usize = 32 << 20;
const MAX_PEERS: usize = 64;

/// Where a live session's snapshot, journals and presence files live. The folder transport is the
/// offline default; the relay transport speaks the same file operations over TCP.
pub trait Store: Send {
    /// A short location for the Share page and status bar.
    fn describe(&self) -> String;
    /// Create the room/folder if needed. Called once when a session starts.
    fn prepare(&mut self) -> Result<()>;
    fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>>;
    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<()>;
    fn append(&mut self, name: &str, bytes: &[u8]) -> Result<()>;
    fn len(&mut self, name: &str) -> Result<Option<u64>>;
    fn list(&mut self) -> Result<Vec<String>>;
    fn remove(&mut self, name: &str) -> Result<()>;
}

/// A shared folder, network share or Syncthing folder.
pub struct FolderStore {
    dir: PathBuf,
}

impl FolderStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        FolderStore { dir: dir.into() }
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }
}

impl Store for FolderStore {
    fn describe(&self) -> String {
        self.dir.to_string_lossy().to_string()
    }

    fn prepare(&mut self) -> Result<()> {
        crate::io::create_dir_all(&self.dir).map_err(EngineError::Other)
    }

    fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
        crate::io::read_file_optional(&self.dir.join(name).to_string_lossy()).map_err(EngineError::Other)
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        crate::io::write_file(&self.dir.join(name).to_string_lossy(), bytes).map_err(EngineError::Other)
    }

    fn append(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        crate::io::append_file(&self.dir.join(name), bytes).map_err(EngineError::Other)
    }

    fn len(&mut self, name: &str) -> Result<Option<u64>> {
        Ok(crate::io::file_len(&self.dir.join(name).to_string_lossy()))
    }

    fn list(&mut self) -> Result<Vec<String>> {
        let paths = crate::io::read_dir(&self.dir).map_err(EngineError::Other)?;
        let mut names: Vec<String> = paths.iter().filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_string)).collect();
        names.sort();
        Ok(names)
    }

    fn remove(&mut self, name: &str) -> Result<()> {
        crate::io::remove_file(&self.dir.join(name));
        Ok(())
    }
}

/// What a peer may do. The owner of a shared folder can hand a viewer or commenter link to it;
/// the role is checked here before a local command runs and again when a journal entry is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Editor,
    Commenter,
    Viewer,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Editor, Role::Commenter, Role::Viewer];

    pub fn id(self) -> &'static str {
        match self {
            Role::Editor => "editor",
            Role::Commenter => "commenter",
            Role::Viewer => "viewer",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Role::Editor => "Can edit",
            Role::Commenter => "Can comment",
            Role::Viewer => "Can view",
        }
    }

    pub fn parse(id: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.id().eq_ignore_ascii_case(id.trim()))
    }

    /// Whether this role may run `id` locally. It mirrors the share-link rule: viewers navigate,
    /// present, export and read; commenters add comments; editors do everything.
    pub fn allows(self, id: &str, spec: &crate::cmd::CommandSpec) -> bool {
        if self == Role::Editor {
            return true;
        }
        // A viewer or commenter must not mint, revoke or invite links (which could escalate their
        // access), but may read the sharing state and parse a link they already have.
        if id.starts_with("share.") {
            return matches!(id, "share.info" | "share.parseLink" | "share.open");
        }
        let mutating = spec.undoable
            || matches!(
                id,
                "edit.undo"
                    | "edit.redo"
                    | "edit.repeat"
                    | "file.save"
                    | "file.saveAs"
                    | "file.saveTemplate"
                    | "file.revert"
                    | "text.compose"
                    | "text.commit"
            );
        if !mutating {
            return true;
        }
        self == Role::Commenter && id.starts_with("comment.")
    }
}

/// One participant, written to `peer-<id>.json` and refreshed while the session runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub id: u64,
    pub name: String,
    pub color: [u8; 3],
    pub role: Role,
    pub last_seen: i64,
    #[serde(default)]
    pub slide: Option<usize>,
    #[serde(default)]
    pub x: Option<f32>,
    #[serde(default)]
    pub y: Option<f32>,
}

/// A command in a peer's append-only journal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub seq: u64,
    pub time: i64,
    pub peer: u64,
    pub role: Role,
    pub id: String,
    pub params: Value,
}

/// The latest full document. Undo/redo writes one so every peer converges on the same state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    version: u32,
    revision: u64,
    saved_at: i64,
    /// Highest sequence applied from each peer when this snapshot was written. Unlike a wall
    /// clock, this survives clock skew: an entry is skipped only when its own peer's sequence is
    /// already in the snapshot.
    #[serde(default)]
    seen: BTreeMap<u64, u64>,
    doc: Presentation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Recovery {
    peer: u64,
    time: i64,
    revision: u64,
}

/// What the Share page and the status bar show.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub live: bool,
    pub folder: String,
    pub peer: u64,
    pub name: String,
    pub role: Role,
    pub revision: u64,
    pub peers: Vec<Peer>,
    pub error: Option<String>,
}

/// A live shared-folder session owned by one [`Session`].
pub struct Collab {
    store: Box<dyn Store>,
    pub peer: u64,
    pub name: String,
    pub role: Role,
    pub color: [u8; 3],
    pub seq: u64,
    pub revision: u64,
    pub error: Option<String>,
    snapshot_at: i64,
    offsets: HashMap<String, u64>,
    seen: BTreeMap<u64, u64>,
    peers: BTreeMap<u64, Peer>,
    last_heartbeat: i64,
    cursor: Option<(usize, f32, f32)>,
    last_cursor_sent: i64,
}

impl Collab {
    /// Start a session in a folder. `doc` writes the first snapshot when the folder has none. A
    /// folder that already has a snapshot keeps it; [`Self::join`] is the path that also returns it.
    pub fn start(dir: PathBuf, role: Role, name: String, doc: Option<&Presentation>) -> Result<Collab> {
        Self::start_store(Box::new(FolderStore::new(dir)), role, name, doc)
    }

    /// Start a session on any [`Store`].
    pub fn start_store(mut store: Box<dyn Store>, role: Role, name: String, doc: Option<&Presentation>) -> Result<Collab> {
        store.prepare()?;
        let existing = read_snapshot(store.as_mut())?;
        let peer = new_peer_id();
        let mut c = Collab {
            store,
            peer,
            name: clean_name(&name),
            role,
            color: peer_color(peer),
            seq: 0,
            revision: 0,
            error: None,
            snapshot_at: 0,
            offsets: HashMap::new(),
            seen: BTreeMap::new(),
            peers: BTreeMap::new(),
            last_heartbeat: 0,
            cursor: None,
            last_cursor_sent: 0,
        };
        if let Some(snap) = existing {
            c.revision = snap.revision;
            c.snapshot_at = snap.saved_at;
            c.seen = snap.seen;
            // A new host joins the existing journal; it does not replay it into a document it
            // already has.
            c.remember_offsets();
        } else if let Some(doc) = doc {
            c.write_snapshot(doc)?;
        }
        c.write_peer();
        Ok(c)
    }

    /// Join a folder that has a snapshot. The caller adopts the returned document before polling.
    pub fn join(dir: PathBuf, role: Role, name: String) -> Result<(Collab, Presentation)> {
        Self::join_store(Box::new(FolderStore::new(dir)), role, name)
    }

    /// Join any [`Store`] that has a snapshot.
    pub fn join_store(mut store: Box<dyn Store>, role: Role, name: String) -> Result<(Collab, Presentation)> {
        store.prepare()?;
        let snap =
            read_snapshot(store.as_mut())?.ok_or_else(|| EngineError::Other(format!("{} has no collaboration snapshot yet", store.describe())))?;
        let mut c = Collab::start_store(store, role, name, None)?;
        c.revision = snap.revision;
        c.snapshot_at = snap.saved_at;
        c.seen = snap.seen;
        c.offsets.clear();
        Ok((c, snap.doc))
    }

    /// Publish a full snapshot. Called for undo/redo and when answering a recovery request.
    pub fn write_snapshot(&mut self, doc: &Presentation) -> Result<()> {
        self.revision = self.revision.saturating_add(1);
        self.seen.insert(self.peer, self.seq);
        let now = now_ms();
        let snap = Snapshot { version: 1, revision: self.revision, saved_at: now, seen: self.seen.clone(), doc: doc.clone() };
        let bytes = serde_json::to_vec(&snap).map_err(|e| EngineError::Other(format!("can't encode the collaboration snapshot: {e}")))?;
        self.store.write(SNAPSHOT_FILE, &bytes)?;
        self.snapshot_at = now;
        self.remember_offsets();
        self.clear_recovery();
        self.error = None;
        Ok(())
    }

    /// Append a command to this peer's journal. `text.*` and comments are journaled even though
    /// some of them are not undoable commands.
    pub fn append(&mut self, id: &str, params: &Value) {
        self.seq = self.seq.saturating_add(1);
        self.seen.insert(self.peer, self.seq);
        self.revision = self.revision.saturating_add(1);
        let entry = Entry { seq: self.seq, time: now_ms(), peer: self.peer, role: self.role, id: id.to_string(), params: params.clone() };
        let line = match serde_json::to_vec(&entry) {
            Ok(mut bytes) => {
                bytes.push(b'\n');
                bytes
            }
            Err(e) => {
                self.error = Some(format!("can't encode a collaboration command: {e}"));
                return;
            }
        };
        let name = self.journal_name(self.peer);
        if let Err(e) = self.store.append(&name, &line) {
            self.error = Some(e.to_string());
        }
    }

    /// Refresh presence, adopt a newer snapshot, and apply commands other peers appended.
    pub fn tick(&mut self, session: &mut Session) -> Result<()> {
        let now = now_ms();
        if now.saturating_sub(self.last_heartbeat) >= HEARTBEAT_MS {
            self.write_peer();
        }
        self.adopt_newer_snapshot(session)?;
        self.read_peers(now);
        self.apply_journals(session)?;
        // Answer after applying, so the snapshot includes every command this peer could apply.
        self.answer_recovery(session);
        Ok(())
    }

    /// Tell the others where this peer's pointer is. Throttled to 100 ms and only written when it
    /// actually moved; a slide change is immediate.
    pub fn set_cursor(&mut self, slide: usize, x: f32, y: f32) {
        let x = x.clamp(0.0, 1.0);
        let y = y.clamp(0.0, 1.0);
        let now = now_ms();
        let same = self.cursor.is_some_and(|(s, ox, oy)| s == slide && (ox - x).abs() < 0.001 && (oy - y).abs() < 0.001);
        if same && now.saturating_sub(self.last_cursor_sent) < 100 {
            return;
        }
        self.cursor = Some((slide, x, y));
        self.last_cursor_sent = now;
        self.write_peer();
    }

    pub fn clear_cursor(&mut self) {
        if self.cursor.take().is_some() {
            self.write_peer();
        }
    }

    /// Leave: publish the final document (so a later joiner sees the latest state) and remove the
    /// presence file. The journals stay as the history.
    pub fn leave(&mut self, session: &mut Session) {
        // Apply anything already in the journals before the final snapshot, so leaving does not
        // publish a document that is behind the peers it just read.
        let _ = self.tick(session);
        if self.role == Role::Editor
            && let Some(doc) = session.active().map(|d| (*d.doc).clone())
        {
            let _ = self.write_snapshot(&doc);
        }
        let _ = self.store.remove(&self.peer_name(self.peer));
    }

    pub fn status(&self) -> Status {
        let mut peers: Vec<Peer> = self.peers.values().cloned().collect();
        if let Some(me) = peers.iter_mut().find(|p| p.id == self.peer) {
            me.name = self.name.clone();
            me.role = self.role;
        }
        peers.sort_by_key(|p| (p.name.to_lowercase(), p.id));
        Status {
            live: true,
            folder: self.store.describe(),
            peer: self.peer,
            name: self.name.clone(),
            role: self.role,
            revision: self.revision,
            peers,
            error: self.error.clone(),
        }
    }

    fn journal_name(&self, peer: u64) -> String {
        format!("{JOURNAL_PREFIX}{peer:016x}.jsonl")
    }

    fn peer_name(&self, peer: u64) -> String {
        format!("{PEER_PREFIX}{peer:016x}.json")
    }

    fn write_peer(&mut self) {
        self.last_heartbeat = now_ms();
        let peer = Peer {
            id: self.peer,
            name: self.name.clone(),
            color: self.color,
            role: self.role,
            last_seen: self.last_heartbeat,
            slide: self.cursor.map(|c| c.0),
            x: self.cursor.map(|c| c.1),
            y: self.cursor.map(|c| c.2),
        };
        match serde_json::to_vec(&peer) {
            Ok(bytes) => {
                if let Err(e) = self.store.write(&self.peer_name(self.peer), &bytes) {
                    self.error = Some(e.to_string());
                }
                self.peers.insert(self.peer, peer);
            }
            Err(e) => self.error = Some(format!("can't encode presence: {e}")),
        }
    }

    fn read_peers(&mut self, now: i64) {
        let names = match self.store.list() {
            Ok(names) => names,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let mut seen = BTreeMap::new();
        for name in names {
            if !name.starts_with(PEER_PREFIX) || !name.ends_with(".json") {
                continue;
            }
            let Ok(Some(bytes)) = self.store.read(&name) else { continue };
            if bytes.len() > 64 * 1024 {
                continue;
            }
            let Ok(peer) = serde_json::from_slice::<Peer>(&bytes) else { continue };
            if peer.id == self.peer {
                continue;
            }
            if now.saturating_sub(peer.last_seen) <= STALE_MS && seen.len() < MAX_PEERS {
                seen.insert(peer.id, peer);
            }
        }
        if let Some(me) = self.peers.get(&self.peer).cloned() {
            seen.insert(self.peer, me);
        }
        self.peers = seen;
    }

    fn adopt_newer_snapshot(&mut self, session: &mut Session) -> Result<()> {
        let Some(snap) = read_snapshot(self.store.as_mut())? else { return Ok(()) };
        if snap.saved_at <= self.snapshot_at && snap.revision <= self.revision {
            return Ok(());
        }
        session.adopt_remote_document(snap.doc)?;
        self.revision = snap.revision;
        self.snapshot_at = snap.saved_at;
        self.seen = snap.seen;
        // Keep the journal offsets: entries after the snapshot's `seen` sequences still apply.
        self.error = None;
        Ok(())
    }

    fn answer_recovery(&mut self, session: &Session) {
        if self.role != Role::Editor {
            return;
        }
        let Ok(names) = self.store.list() else { return };
        let now = now_ms();
        let mut asked = false;
        for name in &names {
            if !name.starts_with(RECOVERY_PREFIX) || !name.ends_with(".json") {
                continue;
            }
            let Ok(Some(bytes)) = self.store.read(name) else { continue };
            if bytes.len() > 64 * 1024 {
                continue;
            }
            let Ok(r) = serde_json::from_slice::<Recovery>(&bytes) else { continue };
            if now.saturating_sub(r.time) < 120_000 {
                // The requester is behind or diverged; a snapshot from any editor moves it
                // forward. The request is cleared by the snapshot write below.
                asked = true;
            }
        }
        if asked && let Some(doc) = session.active().map(|d| (*d.doc).clone()) {
            let _ = self.write_snapshot(&doc);
        }
    }

    fn apply_journals(&mut self, session: &mut Session) -> Result<()> {
        let names = match self.store.list() {
            Ok(names) => names,
            Err(e) => {
                self.error = Some(e.to_string());
                return Ok(());
            }
        };
        let mut entries = Vec::new();
        for name in names {
            if !name.starts_with(JOURNAL_PREFIX) || !name.ends_with(".jsonl") {
                continue;
            }
            let key = name.clone();
            let Ok(Some(bytes)) = self.store.read(&name) else { continue };
            if bytes.len() > MAX_JOURNAL_BYTES {
                self.error = Some(format!("{name} is larger than the 32 MiB collaboration limit"));
                continue;
            }
            let start = self.offsets.get(&key).copied().unwrap_or(0) as usize;
            if start > bytes.len() {
                self.offsets.insert(key.clone(), 0);
                continue;
            }
            let end = bytes.iter().rposition(|b| *b == b'\n').map_or(start, |i| i + 1);
            if end <= start {
                continue;
            }
            for line in bytes[start..end].split(|b| *b == b'\n') {
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_slice::<Entry>(line) {
                    Ok(e) => entries.push(e),
                    Err(e) => self.error = Some(format!("{name}: a journal line is not a command: {e}")),
                }
            }
            self.offsets.insert(key, end as u64);
        }
        entries.sort_by_key(|e| (e.time, e.peer, e.seq));
        for entry in entries {
            if entry.peer == self.peer || entry.seq <= self.seen.get(&entry.peer).copied().unwrap_or(0) {
                continue;
            }
            if !entry_allowed(&entry) {
                self.error = Some(format!("ignored “{}” from a {} peer", entry.id, entry.role.id()));
                self.seen.insert(entry.peer, entry.seq);
                continue;
            }
            match session.execute_remote(&entry.id, &entry.params) {
                Ok(_) => {
                    self.revision = self.revision.saturating_add(1);
                    self.seen.insert(entry.peer, entry.seq);
                }
                Err(e) => {
                    self.error = Some(format!("{} could not apply “{}”: {e}", entry.peer, entry.id));
                    // The recovery snapshot supersedes this entry; mark it seen so it is not
                    // retried forever, and let the editor publish the converged document.
                    self.seen.insert(entry.peer, entry.seq);
                    self.write_recovery();
                    break;
                }
            }
        }
        Ok(())
    }

    fn write_recovery(&mut self) {
        let name = format!("{RECOVERY_PREFIX}{:016x}.json", self.peer);
        let r = Recovery { peer: self.peer, time: now_ms(), revision: self.revision };
        if let Ok(bytes) = serde_json::to_vec(&r)
            && let Err(e) = self.store.write(&name, &bytes)
        {
            self.error = Some(e.to_string());
        }
    }

    fn clear_recovery(&mut self) {
        let Ok(names) = self.store.list() else { return };
        for name in names {
            if name.starts_with(RECOVERY_PREFIX) && name.ends_with(".json") {
                let _ = self.store.remove(&name);
            }
        }
    }

    fn remember_offsets(&mut self) {
        self.offsets.clear();
        let Ok(names) = self.store.list() else { return };
        for name in names {
            if name.starts_with(JOURNAL_PREFIX)
                && name.ends_with(".jsonl")
                && let Ok(Some(len)) = self.store.len(&name)
            {
                self.offsets.insert(name, len);
            }
        }
    }
}

fn read_snapshot(store: &mut dyn Store) -> Result<Option<Snapshot>> {
    let Some(bytes) = store.read(SNAPSHOT_FILE)? else {
        return Ok(None);
    };
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(EngineError::Other("the collaboration snapshot is larger than the 32 MiB limit".into()));
    }
    let snap: Snapshot = serde_json::from_slice(&bytes).map_err(|e| EngineError::Other(format!("the collaboration snapshot is damaged: {e}")))?;
    if snap.version > 1 {
        return Err(EngineError::Other("the collaboration snapshot uses a newer format".into()));
    }
    Ok(Some(snap))
}

fn entry_allowed(entry: &Entry) -> bool {
    let Some(spec) = crate::cmd::find_command(&entry.id) else { return false };
    if entry.id.starts_with("file.")
        || entry.id.starts_with("share.")
        || entry.id.starts_with("app.")
        || entry.id.starts_with("show.")
        || entry.id.starts_with("view.")
        || entry.id.starts_with("collab.")
        || matches!(entry.id.as_str(), "edit.undo" | "edit.redo" | "edit.repeat")
    {
        return false;
    }
    if !(spec.undoable || entry.id.starts_with("text.") || entry.id.starts_with("comment.")) {
        return false;
    }
    match entry.role {
        Role::Editor => true,
        Role::Commenter => entry.id.starts_with("comment."),
        Role::Viewer => false,
    }
}

fn clean_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() { "Guest".to_string() } else { name.chars().take(48).collect() }
}

fn peer_color(peer: u64) -> [u8; 3] {
    const COLORS: [[u8; 3]; 8] = [
        [0xE0, 0x5A, 0x4A],
        [0x2F, 0x9E, 0x8F],
        [0x4A, 0x7D, 0xD0],
        [0xD0, 0x8A, 0x2F],
        [0x8A, 0x5A, 0xC8],
        [0xC8, 0x4A, 0x8A],
        [0x5A, 0xA8, 0x3A],
        [0x3A, 0x8A, 0xC8],
    ];
    COLORS[(peer as usize) % COLORS.len()]
}

fn new_peer_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    nanos ^ ((std::process::id() as u64) << 24) ^ NEXT.fetch_add(1, Ordering::Relaxed) | 1
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis().min(i64::MAX as u128) as i64)
}

/// Whether a folder already has a snapshot (so a new host should Join rather than Start).
pub fn snapshot_exists(dir: &Path) -> bool {
    matches!(crate::io::read_file_optional(&dir.join(SNAPSHOT_FILE).to_string_lossy()), Ok(Some(bytes)) if !bytes.is_empty())
}

/// The default folder for a presentation path: `talk.deckcraft.collab`.
pub fn default_dir(deck_path: &str) -> PathBuf {
    PathBuf::from(format!("{deck_path}{DIR_SUFFIX}"))
}

/// A short, stable label for a peer's cursor.
pub fn cursor_label(peer: &Peer) -> String {
    if peer.name.is_empty() { format!("Peer {}", peer.id & 0xffff) } else { peer.name.clone() }
}

#[cfg(test)]
mod tests;
