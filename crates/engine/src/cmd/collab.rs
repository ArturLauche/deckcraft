//! Live collaboration through a shared folder: start a journal beside the deck, join one, publish
//! the pointer and report who is here. The transport is [`crate::collab`]; the commands are the
//! same ones the Share page and the CLI use.

use serde_json::{Value, json};

use super::*;
use crate::collab::{self, Role, Store};
use crate::{Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            noundo "collab.start",
            "Start Live Session",
            ["File", "Share"],
            None,
            "{folder?, relay?: host:port, room?, token?, role?: editor|commenter|viewer, name?} → {live, folder, peer, name, role, revision, peers}",
            has_doc,
            start
        ),
        cmd!(
            noundo "collab.join",
            "Join Live Session",
            ["File", "Share"],
            None,
            "{folder?, relay?: host:port, room?, token?, role?: editor|commenter|viewer, name?} → {live, folder, peer, name, role, revision, peers}",
            has_doc,
            join
        ),
        cmd!(noundo "collab.leave", "Leave Live Session", ["File", "Share"], None, "{} → {left}", always, leave),
        cmd!(query "collab.status", "Live Session", [], None, "{} → {live: false} or {live: true, folder, peer, name, role, revision, peers, error}", always, status),
        cmd!(
            noundo "collab.cursor",
            "Share Pointer",
            [],
            None,
            "{slide, x, y} — x and y are normalized to the slide rectangle",
            has_doc,
            cursor
        ),
    ]
}

fn role(p: &Value) -> Result<Role> {
    match p.get("role") {
        None | Some(Value::Null) => Ok(Role::Editor),
        Some(Value::String(id)) => Role::parse(id).ok_or_else(|| bad("collab", format!("`role` must be editor, commenter or viewer: {id}"))),
        Some(_) => Err(bad("collab", "`role` must be editor, commenter or viewer")),
    }
}

fn name(s: &Session, p: &Value) -> String {
    p.get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| if s.prefs.author.trim().is_empty() { "Presenter" } else { s.prefs.author.trim() })
        .chars()
        .take(48)
        .collect()
}

fn full_access(s: &Session) -> Result<()> {
    if s.active().is_some_and(|d| d.access != crate::sharing::Access::Full) {
        return Err(bad("collab", "a shared copy cannot start or join a live session; save a copy first"));
    }
    Ok(())
}

/// `relay`, `room` and `token`, all required together. Returns `None` for a folder session.
fn relay_params(p: &Value) -> Result<Option<(String, String, String)>> {
    let Some(relay) = p.get("relay").and_then(Value::as_str).map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    let room = p
        .get("room")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| bad("collab", "a relay session needs `room`"))?;
    let token = p
        .get("token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| bad("collab", "a relay session needs `token`"))?;
    Ok(Some((relay.to_string(), room.to_string(), token.to_string())))
}

fn start(s: &mut Session, p: &Value) -> Result<Value> {
    full_access(s)?;
    if s.collab.is_some() {
        return Err(bad("collab", "this presentation is already in a live session; leave it first"));
    }
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let c = if let Some((relay, room, token)) = relay_params(p)? {
        let mut store: Box<dyn Store> = Box::new(collab::relay::RelayStore::new(&relay, &room, &token)?);
        if store.read(collab::SNAPSHOT_FILE)?.is_some() {
            return Err(bad("collab", "that relay room already has a live session; use Join instead of Start"));
        }
        collab::Collab::start_store(store, role(p)?, name(s, p), Some(&doc))?
    } else {
        let dir = match p.get("folder").and_then(Value::as_str).map(str::trim).filter(|f| !f.is_empty()) {
            Some(folder) => std::path::PathBuf::from(folder),
            None => {
                let path = s
                    .active()
                    .and_then(|d| d.path.clone())
                    .ok_or_else(|| EngineError::Other("save the presentation before starting a live session, or choose a folder".into()))?;
                collab::default_dir(&path)
            }
        };
        if collab::snapshot_exists(&dir) {
            return Err(bad("collab", "that folder already has a live session; use Join instead of Start"));
        }
        collab::Collab::start(dir, role(p)?, name(s, p), Some(&doc))?
    };
    let out = serde_json::to_value(c.status()).map_err(|e| EngineError::Other(e.to_string()))?;
    s.collab = Some(c);
    Ok(out)
}

fn join(s: &mut Session, p: &Value) -> Result<Value> {
    full_access(s)?;
    if s.collab.is_some() {
        return Err(bad("collab", "this presentation is already in a live session; leave it first"));
    }
    let (c, doc) = if let Some((relay, room, token)) = relay_params(p)? {
        let store: Box<dyn Store> = Box::new(collab::relay::RelayStore::new(&relay, &room, &token)?);
        collab::Collab::join_store(store, role(p)?, name(s, p))?
    } else {
        let folder = p
            .get("folder")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .ok_or_else(|| bad("collab.join", "missing `folder`, or pass `relay`, `room` and `token`"))?;
        collab::Collab::join(std::path::PathBuf::from(folder), role(p)?, name(s, p))?
    };
    s.adopt_remote_document(doc)?;
    let out = serde_json::to_value(c.status()).map_err(|e| EngineError::Other(e.to_string()))?;
    s.collab = Some(c);
    Ok(out)
}

fn leave(s: &mut Session, _p: &Value) -> Result<Value> {
    let had = s.collab.is_some();
    s.collab_leave();
    Ok(json!({"left": had}))
}

fn status(s: &mut Session, _p: &Value) -> Result<Value> {
    match s.collab_status() {
        Some(status) => serde_json::to_value(status).map_err(|e| EngineError::Other(e.to_string())),
        None => Ok(json!({"live": false})),
    }
}

fn cursor(s: &mut Session, p: &Value) -> Result<Value> {
    let slide = p.get("slide").and_then(Value::as_u64).ok_or_else(|| bad("collab.cursor", "missing `slide`"))? as usize;
    let x = p.get("x").and_then(Value::as_f64).ok_or_else(|| bad("collab.cursor", "missing `x`"))? as f32;
    let y = p.get("y").and_then(Value::as_f64).ok_or_else(|| bad("collab.cursor", "missing `y`"))? as f32;
    if !x.is_finite() || !y.is_finite() {
        return Err(bad("collab.cursor", "`x` and `y` must be finite"));
    }
    s.collab_cursor(slide, x, y);
    Ok(json!({"slide": slide, "x": x.clamp(0.0, 1.0), "y": y.clamp(0.0, 1.0)}))
}
