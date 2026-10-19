//! Shared-folder collaboration tests: real journals, real snapshots, two sessions.

use super::*;
use serde_json::json;

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("deckcraft-collab-{}-{}-{}", name, std::process::id(), now_ms()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn host(dir: &Path) -> Session {
    let mut s = Session::with_new();
    let started = s.execute("collab.start", &json!({"folder": dir.to_string_lossy(), "name": "Host"})).unwrap();
    assert_eq!(started["live"], true);
    s
}

fn guest(dir: &Path, role: &str, name: &str) -> Session {
    let mut s = Session::with_new();
    s.execute("collab.join", &json!({"folder": dir.to_string_lossy(), "role": role, "name": name})).unwrap();
    s
}

fn text_shapes(s: &Session) -> usize {
    s.active().unwrap().current_slide().unwrap().shapes.iter().filter(|shape| shape.text.is_some()).count()
}

#[test]
fn two_peers_share_commands_presence_and_cursors() {
    let dir = temp_dir("share");
    let mut h = host(&dir);
    let mut g = guest(&dir, "editor", "Guest");
    h.collab_tick().unwrap();
    g.collab_tick().unwrap();
    let base = text_shapes(&g);

    h.execute("insert.textBox", &json!({"rect": [40, 40, 400, 80], "text": "Live"})).unwrap();
    g.collab_tick().unwrap();
    assert_eq!(text_shapes(&g), base + 1, "the guest applied the host's command");

    g.execute("insert.textBox", &json!({"rect": [40, 140, 400, 80], "text": "Back"})).unwrap();
    h.collab_tick().unwrap();
    assert_eq!(text_shapes(&h), base + 2, "the host applied the guest's command");

    g.execute("collab.cursor", &json!({"slide": 0, "x": 0.5, "y": 0.25})).unwrap();
    h.collab_tick().unwrap();
    let status = h.collab_status().unwrap();
    let guest = status.peers.iter().find(|p| p.name == "Guest").expect("guest is present");
    assert_eq!(guest.slide, Some(0));
    assert!((guest.x.unwrap_or_default() - 0.5).abs() < 0.001);

    g.execute("collab.leave", &json!({})).unwrap();
    h.collab_tick().unwrap();
    assert!(h.collab_status().unwrap().peers.iter().all(|p| p.name != "Guest"), "leaving removes presence");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_viewer_cannot_edit_but_can_point() {
    let dir = temp_dir("viewer");
    let mut h = host(&dir);
    let mut v = guest(&dir, "viewer", "Viewer");
    h.collab_tick().unwrap();
    let before = text_shapes(&h);
    let refused = v.execute("insert.textBox", &json!({"rect": [40, 40, 400, 80], "text": "No"})).unwrap_err();
    assert!(refused.to_string().contains("can view"), "{refused}");
    let link = v.execute("share.link", &json!({"permission": "editor"})).unwrap_err();
    assert!(link.to_string().contains("can view"), "a viewer cannot mint an editor link: {link}");
    v.execute("collab.cursor", &json!({"slide": 0, "x": 0.1, "y": 0.1})).unwrap();
    h.collab_tick().unwrap();
    assert_eq!(text_shapes(&h), before, "no edit crossed the journal");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undo_publishes_a_snapshot_every_peer_adopts() {
    let dir = temp_dir("undo");
    let mut h = host(&dir);
    h.execute("slide.new", &json!({})).unwrap();
    let mut g = guest(&dir, "editor", "Guest");
    g.collab_tick().unwrap();
    let base = text_shapes(&g);
    h.execute("insert.textBox", &json!({"rect": [40, 40, 400, 80], "text": "Undo me"})).unwrap();
    g.collab_tick().unwrap();
    assert_eq!(text_shapes(&g), base + 1);

    g.execute("slide.next", &json!({})).unwrap();
    let looking_at = g.active().unwrap().selection.slide;
    assert!(looking_at > 0, "the guest moved to another slide");
    h.execute("edit.undo", &json!({})).unwrap();
    g.collab_tick().unwrap();
    assert_eq!(text_shapes(&g), base, "the guest adopted the host's post-undo snapshot");
    assert_eq!(g.active().unwrap().selection.slide, looking_at, "the snapshot keeps the reader's slide");
    assert_eq!(g.collab_status().unwrap().revision, h.collab_status().unwrap().revision);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn joining_needs_a_snapshot_and_a_malformed_journal_line_is_not_fatal() {
    let dir = temp_dir("bad");
    let mut s = Session::with_new();
    let err = s.execute("collab.join", &json!({"folder": dir.to_string_lossy()})).unwrap_err();
    assert!(err.to_string().contains("snapshot"), "{err}");

    let mut h = host(&dir);
    let mut g = guest(&dir, "editor", "Guest");
    std::fs::write(
        dir.join("journal-0000000000000001.jsonl"),
        b"not json\n{\"seq\":1,\"time\":1,\"peer\":2,\"role\":\"editor\",\"id\":\"file.open\",\"params\":{}}\n",
    )
    .unwrap();
    g.collab_tick().unwrap();
    h.collab_tick().unwrap();
    assert!(g.collab_status().unwrap().error.is_some(), "the bad line is reported");
    assert!(h.collab_status().unwrap().live);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_peer_clock_behind_the_snapshot_still_gets_its_command_applied() {
    let dir = temp_dir("skew");
    let mut h = host(&dir);
    let base = text_shapes(&h);
    let entry = json!({
        "seq": 1,
        "time": 1,
        "peer": 2,
        "role": "editor",
        "id": "insert.textBox",
        "params": {"rect": [40, 40, 400, 80], "text": "Skewed clock"}
    });
    std::fs::write(dir.join("journal-0000000000000002.jsonl"), format!("{entry}\n")).unwrap();
    h.collab_tick().unwrap();
    assert_eq!(text_shapes(&h), base + 1, "sequence, not the wall clock, decides whether an entry is new");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_remote_command_requests_and_gets_a_recovery_snapshot() {
    let dir = temp_dir("recovery");
    let mut h = host(&dir);
    let entry = json!({
        "seq": 1,
        "time": now_ms() + 1_000,
        "peer": 2,
        "role": "editor",
        "id": "shape.insert",
        "params": {"preset": "definitelyNotAShape", "rect": [0, 0, 10, 10]}
    });
    std::fs::write(dir.join("journal-0000000000000002.jsonl"), format!("{entry}\n")).unwrap();
    let before = h.collab_status().unwrap().revision;
    h.collab_tick().unwrap();
    assert!(h.collab_status().unwrap().error.is_none(), "the editor answered the recovery request with a snapshot");
    assert!(h.collab_status().unwrap().revision > before, "the recovery snapshot advanced the revision");
    assert!(!std::fs::read_dir(&dir).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("recovery-")), "the request is cleared");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_headless_status_command_catches_up_on_the_journal() {
    let dir = temp_dir("headless");
    let mut h = host(&dir);
    let mut g = guest(&dir, "editor", "Guest");
    let base = text_shapes(&g);
    h.execute("insert.textBox", &json!({"rect": [40, 40, 400, 80], "text": "Headless"})).unwrap();
    // No explicit `collab_tick`: executing a command (here the status query) catches up first.
    let status = g.execute("collab.status", &json!({})).unwrap();
    assert_eq!(status["live"], true);
    assert_eq!(text_shapes(&g), base + 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_live_session_is_tied_to_its_document_and_its_folder() {
    let dir = temp_dir("tied");
    let mut h = host(&dir);
    let switched = h.execute("file.new", &json!({})).unwrap_err();
    assert!(switched.to_string().contains("leave the live session"), "{switched}");

    let mut second = Session::with_new();
    let started = second.execute("collab.start", &json!({"folder": dir.to_string_lossy(), "name": "Second"})).unwrap_err();
    assert!(started.to_string().contains("Join instead"), "{started}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn default_folder_sits_beside_the_deck() {
    assert_eq!(default_dir("/tmp/talk.deckcraft"), PathBuf::from("/tmp/talk.deckcraft.collab"));
    assert_eq!(Role::parse("EDITOR"), Some(Role::Editor));
    assert_eq!(Role::parse("owner"), None);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_relay_carries_commands_presence_and_rejects_a_bad_token() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let root = temp_dir("relay-server");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let server_root = root.clone();
    let server_stop = Arc::clone(&stop);
    let server = std::thread::spawn(move || {
        super::relay::serve_until(listener, server_root, "room secret".into(), server_stop).unwrap();
    });

    let mut h = Session::with_new();
    h.execute("collab.start", &json!({"relay": addr, "room": "deck", "token": "room secret", "name": "Host"})).unwrap();
    let mut g = Session::with_new();
    g.execute("collab.join", &json!({"relay": addr, "room": "deck", "token": "room secret", "name": "Guest"})).unwrap();
    h.collab_tick().unwrap();
    g.collab_tick().unwrap();
    let base = text_shapes(&g);

    h.execute("insert.textBox", &json!({"rect": [40, 40, 400, 80], "text": "Over relay"})).unwrap();
    g.collab_tick().unwrap();
    assert_eq!(text_shapes(&g), base + 1, "the guest applied the host's relayed command");

    g.execute("collab.cursor", &json!({"slide": 0, "x": 0.25, "y": 0.75})).unwrap();
    h.collab_tick().unwrap();
    let status = h.collab_status().unwrap();
    assert!(status.folder.starts_with("relay://"), "{}", status.folder);
    let guest = status.peers.iter().find(|p| p.name == "Guest").expect("guest presence crosses the relay");
    assert_eq!(guest.slide, Some(0));
    assert!((guest.x.unwrap_or_default() - 0.25).abs() < 0.001);

    let mut bad = Session::with_new();
    let err = bad.execute("collab.join", &json!({"relay": addr, "room": "deck", "token": "wrong", "name": "Bad"})).unwrap_err();
    assert!(err.to_string().contains("bad room token"), "{err}");
    let mut other = Session::with_new();
    let err = other.execute("collab.start", &json!({"relay": addr, "room": "deck", "token": "room secret"})).unwrap_err();
    assert!(err.to_string().contains("already has a live session"), "{err}");

    h.execute("collab.leave", &json!({})).unwrap();
    g.execute("collab.leave", &json!({})).unwrap();
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_relay_room_survives_a_server_restart() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let root = temp_dir("relay-restart");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let server = {
        let root = root.clone();
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || super::relay::serve_until(listener, root, "secret".into(), stop).unwrap())
    };
    let mut h = Session::with_new();
    h.execute("collab.start", &json!({"relay": addr, "room": "persist", "token": "secret", "name": "Host"})).unwrap();
    h.execute("insert.textBox", &json!({"rect": [40, 40, 400, 80], "text": "Saved on the relay"})).unwrap();
    h.execute("collab.leave", &json!({})).unwrap();
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();

    let listener = std::net::TcpListener::bind(&addr).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let server = {
        let root = root.clone();
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || super::relay::serve_until(listener, root, "secret".into(), stop).unwrap())
    };
    let mut g = Session::with_new();
    g.execute("collab.join", &json!({"relay": addr, "room": "persist", "token": "secret", "name": "Guest"})).unwrap();
    assert!(text_shapes(&g) >= 1, "the snapshot survived the restart");
    g.execute("collab.leave", &json!({})).unwrap();
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_relay_rejects_bad_tokens_and_path_traversal() {
    use std::io::{BufRead, BufReader, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let root = temp_dir("relay-guard");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let server_root = root.clone();
    let server_stop = Arc::clone(&stop);
    let server = std::thread::spawn(move || super::relay::serve_until(listener, server_root, "secret".into(), server_stop).unwrap());
    let ask = |request: serde_json::Value| -> serde_json::Value {
        let mut stream = std::net::TcpStream::connect(&addr).unwrap();
        stream.write_all(request.to_string().as_bytes()).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let bad = ask(json!({"op": "write", "room": "deck", "token": "wrong", "name": "a", "data": "aGk="}));
    assert_eq!(bad["ok"], false);
    assert_eq!(bad["error"], "bad room token");
    let traversal = ask(json!({"op": "write", "room": "deck", "token": "secret", "name": "../escape", "data": "aGk="}));
    assert_eq!(traversal["ok"], false);
    assert_eq!(traversal["error"], "bad file name");
    assert!(!root.join("escape").exists(), "nothing was written outside the room");
    let ok = ask(json!({"op": "write", "room": "deck", "token": "secret", "name": "note.txt", "data": "aGk="}));
    assert_eq!(ok["ok"], true);
    let read = ask(json!({"op": "read", "room": "deck", "token": "secret", "name": "note.txt"}));
    assert_eq!(read["data"], "aGk=");
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn relay_commands_require_a_room_and_a_token_before_connecting() {
    let mut s = Session::with_new();
    let err = s.execute("collab.start", &json!({"relay": "127.0.0.1:1", "token": "secret"})).unwrap_err();
    assert!(err.to_string().contains("needs `room`"), "{err}");
    let err = s.execute("collab.join", &json!({"relay": "127.0.0.1:1", "room": "deck"})).unwrap_err();
    assert!(err.to_string().contains("needs `token`"), "{err}");
}
