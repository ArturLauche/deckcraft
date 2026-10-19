//! Self-hosted TCP relay for live sessions.
//!
//! The relay stores one room's snapshot, journals and presence files and serves them over a
//! newline-delimited JSON protocol. A room needs a token; the token is sent on every request and
//! is never part of the status or the room name. Files are confined to a room directory, names
//! are restricted to a safe alphabet, and every request is size-limited.
//!
//! The client implements the same [`Store`] operations as a shared folder, so the journal,
//! snapshot, recovery and role logic is identical. A relay adds real-time delivery without
//! changing the merge rules; this is still last-writer-wins, not a CRDT.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Store;
use crate::{EngineError, Result};

/// A request larger than this is refused before it is decoded.
const MAX_REQUEST_BYTES: u64 = 48 << 20;
/// The largest file a room may store (matching the collaboration snapshot limit).
const MAX_FILE_BYTES: usize = 32 << 20;
const MAX_ROOMS: usize = 64;
const MAX_NAME: usize = 128;
const MAX_ROOM: usize = 64;
const MAX_TOKEN: usize = 256;

/// A live relay endpoint: `host:port`, a room and its token.
pub struct RelayStore {
    endpoint: String,
    room: String,
    token: String,
}

impl RelayStore {
    pub fn new(endpoint: &str, room: &str, token: &str) -> Result<Self> {
        let endpoint = endpoint.trim().trim_start_matches("tcp://").trim_end_matches('/');
        if endpoint.is_empty() || endpoint.len() > 255 || endpoint.chars().any(|c| c.is_whitespace() || c == '/') {
            return Err(EngineError::Other("the relay address must be `host:port`".into()));
        }
        if !valid_room(room) {
            return Err(EngineError::Other("the relay room must be 1–64 letters, digits, `_` or `-`".into()));
        }
        let token = token.trim();
        if token.is_empty() || token.len() > MAX_TOKEN {
            return Err(EngineError::Other("the relay needs a room token of 1–256 characters".into()));
        }
        Ok(RelayStore { endpoint: endpoint.to_string(), room: room.to_string(), token: token.to_string() })
    }

    /// The endpoint and room, without the token.
    pub fn describe(&self) -> String {
        format!("relay://{}/{}", self.endpoint, self.room)
    }

    fn call(&mut self, op: &str, name: Option<&str>, data: Option<&[u8]>) -> Result<Value> {
        let mut request = json!({"op": op, "room": self.room, "token": self.token});
        if let Some(name) = name {
            request["name"] = json!(name);
        }
        if let Some(data) = data {
            request["data"] = json!(crate::cmd::base64_encode(data));
        }
        let line = serde_json::to_string(&request).map_err(|e| EngineError::Other(format!("can't encode a relay request: {e}")))?;
        if line.len() as u64 > MAX_REQUEST_BYTES {
            return Err(EngineError::Other("the relay request is too large".into()));
        }
        let mut stream = TcpStream::connect(&self.endpoint)
            .map_err(|e| EngineError::Other(format!("can't reach the collaboration relay at {}: {e}", self.endpoint)))?;
        let timeout = Some(Duration::from_secs(5));
        let _ = stream.set_read_timeout(timeout);
        let _ = stream.set_write_timeout(timeout);
        stream
            .write_all(line.as_bytes())
            .and_then(|_| stream.write_all(b"\n"))
            .map_err(|e| EngineError::Other(format!("can't send to the relay: {e}")))?;
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut reader = BufReader::new(stream.take(MAX_REQUEST_BYTES + 1));
        let mut reply = String::new();
        reader.read_line(&mut reply).map_err(|e| EngineError::Other(format!("can't read from the relay: {e}")))?;
        if reply.len() as u64 > MAX_REQUEST_BYTES {
            return Err(EngineError::Other("the relay reply is too large".into()));
        }
        let v: Value = serde_json::from_str(&reply).map_err(|e| EngineError::Other(format!("the relay sent a damaged reply: {e}")))?;
        if !v.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let error = v.get("error").and_then(Value::as_str).unwrap_or("the relay refused the request");
            return Err(EngineError::Other(error.to_string()));
        }
        Ok(v)
    }
}

impl Store for RelayStore {
    fn describe(&self) -> String {
        RelayStore::describe(self)
    }

    fn prepare(&mut self) -> Result<()> {
        // A list is a cheap authentication and reachability check before the session starts.
        self.call("list", None, None).map(|_| ())
    }

    fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
        let v = self.call("read", Some(name), None)?;
        let Some(data) = v.get("data").and_then(Value::as_str) else { return Ok(None) };
        let bytes = crate::cmd::base64_decode(data).ok_or_else(|| EngineError::Other("the relay sent damaged file data".into()))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(EngineError::Other("the relay file is larger than the 32 MiB limit".into()));
        }
        Ok(Some(bytes))
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(EngineError::Other("the collaboration file is larger than the 32 MiB limit".into()));
        }
        self.call("write", Some(name), Some(bytes)).map(|_| ())
    }

    fn append(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(EngineError::Other("the collaboration file is larger than the 32 MiB limit".into()));
        }
        self.call("append", Some(name), Some(bytes)).map(|_| ())
    }

    fn len(&mut self, name: &str) -> Result<Option<u64>> {
        let v = self.call("len", Some(name), None)?;
        Ok(v.get("len").and_then(Value::as_u64))
    }

    fn list(&mut self) -> Result<Vec<String>> {
        let v = self.call("list", None, None)?;
        Ok(v.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default())
    }

    fn remove(&mut self, name: &str) -> Result<()> {
        self.call("remove", Some(name), None).map(|_| ())
    }
}

#[derive(Deserialize)]
struct Request {
    op: String,
    room: String,
    token: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    data: Option<String>,
}

#[derive(Serialize)]
struct Reply {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    len: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    names: Option<Vec<String>>,
}

impl Reply {
    fn error(message: impl Into<String>) -> Reply {
        Reply { ok: false, error: Some(message.into()), data: None, len: None, names: None }
    }
}

/// Serve relay requests from `listener` until `stop` is set. `root` holds one directory per room;
/// `token` is required for every room. Blocks the calling thread.
pub fn serve_until(listener: TcpListener, root: PathBuf, token: String, stop: Arc<AtomicBool>) -> std::result::Result<(), String> {
    if token.trim().is_empty() {
        return Err("a relay token is required".into());
    }
    if let Err(e) = std::fs::create_dir_all(&root) {
        return Err(format!("can't create the relay folder {}: {e}", root.display()));
    }
    listener.set_nonblocking(true).map_err(|e| format!("can't configure the relay listener: {e}"))?;
    let root = Arc::new(root);
    let token = Arc::new(token);
    let lock = Arc::new(Mutex::new(()));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let root = Arc::clone(&root);
                let token = Arc::clone(&token);
                let lock = Arc::clone(&lock);
                if let Err(e) = std::thread::Builder::new().name("deckcraft-relay".into()).spawn(move || {
                    let _ = handle(stream, &root, &token, &lock);
                }) {
                    log::warn!("can't start a relay connection: {e}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("the relay listener failed: {e}")),
        }
    }
    Ok(())
}

/// Serve forever.
pub fn serve(listener: TcpListener, root: PathBuf, token: String) -> std::result::Result<(), String> {
    serve_until(listener, root, token, Arc::new(AtomicBool::new(false)))
}

fn handle(stream: TcpStream, root: &Path, token: &str, lock: &Mutex<()>) -> std::result::Result<(), String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?).take(MAX_REQUEST_BYTES + 1);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    if line.len() as u64 > MAX_REQUEST_BYTES {
        return respond(stream, Reply::error("request too large"));
    }
    let request: Request = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(e) => return respond(stream, Reply::error(format!("damaged relay request: {e}"))),
    };
    if request.token != token {
        return respond(stream, Reply::error("bad room token"));
    }
    if !valid_room(&request.room) {
        return respond(stream, Reply::error("bad room name"));
    }
    if !request.name.is_empty() && !valid_name(&request.name) {
        return respond(stream, Reply::error("bad file name"));
    }
    let room = root.join(&request.room);
    if !room.starts_with(root) {
        return respond(stream, Reply::error("bad room path"));
    }
    if matches!(request.op.as_str(), "write" | "append") && !room.exists() {
        let rooms = std::fs::read_dir(root).map(|entries| entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).count()).unwrap_or(0);
        if rooms >= MAX_ROOMS {
            return respond(stream, Reply::error("the relay has reached its room limit"));
        }
    }
    let _guard = lock.lock().map_err(|_| "the relay lock is poisoned".to_string())?;
    let reply = match request.op.as_str() {
        "list" => list(&room).map(|names| Reply { ok: true, error: None, data: None, len: None, names: Some(names) }),
        "read" => read(&room, &request.name),
        "len" => len(&room, &request.name),
        "write" => write(&room, &request.name, request.data.as_deref()),
        "append" => append(&room, &request.name, request.data.as_deref()),
        "remove" => remove(&room, &request.name),
        _ => Ok(Reply::error("unknown relay operation")),
    };
    respond(stream, reply.unwrap_or_else(Reply::error))
}

fn respond(mut stream: TcpStream, reply: Reply) -> std::result::Result<(), String> {
    let bytes = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;
    let _ = stream.write_all(b"\n");
    Ok(())
}

fn read(room: &Path, name: &str) -> std::result::Result<Reply, String> {
    let path = room.join(name);
    match std::fs::read(&path) {
        Ok(bytes) => {
            if bytes.len() > MAX_FILE_BYTES {
                return Ok(Reply::error("file too large"));
            }
            Ok(Reply { ok: true, error: None, data: Some(crate::cmd::base64_encode(&bytes)), len: None, names: None })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Reply { ok: true, error: None, data: None, len: None, names: None }),
        Err(e) => Ok(Reply::error(format!("can't read {name}: {e}"))),
    }
}

fn len(room: &Path, name: &str) -> std::result::Result<Reply, String> {
    match std::fs::metadata(room.join(name)) {
        Ok(m) if m.is_file() => Ok(Reply { ok: true, error: None, data: None, len: Some(m.len()), names: None }),
        Ok(_) => Ok(Reply { ok: true, error: None, data: None, len: None, names: None }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Reply { ok: true, error: None, data: None, len: None, names: None }),
        Err(e) => Ok(Reply::error(format!("can't stat {name}: {e}"))),
    }
}

fn list(room: &Path) -> std::result::Result<Vec<String>, String> {
    let entries = match std::fs::read_dir(room) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("can't list the room: {e}")),
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str()
            && valid_name(name)
            && !name.starts_with('.')
            && entry.file_type().is_ok_and(|t| t.is_file())
        {
            names.push(name.to_string());
        }
        if names.len() > 4096 {
            break;
        }
    }
    names.sort();
    Ok(names)
}

fn write(room: &Path, name: &str, data: Option<&str>) -> std::result::Result<Reply, String> {
    let bytes = decode(data)?;
    std::fs::create_dir_all(room).map_err(|e| format!("can't create the room: {e}"))?;
    let path = room.join(name);
    let tmp = room.join(format!(".{name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("can't write {name}: {e}"))?;
    #[cfg(windows)]
    {
        // Windows refuses to rename over an existing file; the server lock still serializes
        // requests, so the small remove-then-rename gap is not visible to another request.
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("can't finish writing {name}: {e}")
    })?;
    Ok(Reply { ok: true, error: None, data: None, len: None, names: None })
}

fn append(room: &Path, name: &str, data: Option<&str>) -> std::result::Result<Reply, String> {
    let bytes = decode(data)?;
    std::fs::create_dir_all(room).map_err(|e| format!("can't create the room: {e}"))?;
    let path = room.join(name);
    let current = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if current.saturating_add(bytes.len() as u64) > MAX_FILE_BYTES as u64 {
        return Ok(Reply::error("file too large"));
    }
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("can't append to {name}: {e}"))?;
    f.write_all(&bytes).map_err(|e| format!("can't append to {name}: {e}"))?;
    Ok(Reply { ok: true, error: None, data: None, len: None, names: None })
}

fn remove(room: &Path, name: &str) -> std::result::Result<Reply, String> {
    match std::fs::remove_file(room.join(name)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Ok(Reply::error(format!("can't remove {name}: {e}"))),
    }
    Ok(Reply { ok: true, error: None, data: None, len: None, names: None })
}

fn decode(data: Option<&str>) -> std::result::Result<Vec<u8>, String> {
    let data = data.ok_or_else(|| "missing file data".to_string())?;
    let bytes = crate::cmd::base64_decode(data).ok_or_else(|| "damaged file data".to_string())?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("file too large".into());
    }
    Ok(bytes)
}

fn valid_room(room: &str) -> bool {
    !room.is_empty() && room.len() <= MAX_ROOM && room.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.contains("..")
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}

/// The maximum number of rooms the relay will create; a request beyond it is refused.
pub fn room_limit() -> usize {
    MAX_ROOMS
}
