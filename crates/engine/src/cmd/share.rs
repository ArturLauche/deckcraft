//! File ▸ Share: capability links with a permission and expiry, and the People list.
//!
//! The links live beside the presentation (`<name>.share.json`), so sharing works offline and in
//! a folder the team already shares. Opening a link is `share.open`, which checks the token and
//! gives the document the link's access; [`crate::Session`] enforces that for every command.

use serde_json::{Value, json};

use super::*;
use crate::sharing::{self, Permission};
use crate::{Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "share.info",
            "Sharing",
            ["File"],
            None,
            "{} → {path, access, links: [{token, url, permission, created, expires, revoked, valid, label}], participants: [{name, token, url, permission, invited, expires, revoked, valid}]}",
            has_doc,
            info
        ),
        cmd!(
            noundo "share.link",
            "Create Share Link",
            ["File"],
            None,
            "{permission?: viewer|commenter|editor, expiresHours?: number, label?} → {token, url, permission, expires}",
            has_doc,
            link
        ),
        cmd!(noundo "share.revoke", "Revoke Share Link", ["File"], None, "{token} → {revoked}", has_doc, revoke),
        cmd!(
            noundo "share.invite",
            "Invite People",
            ["File"],
            None,
            "{name, permission?: viewer|commenter|editor, expiresHours?: number} → {name, token, url, permission, expires}",
            has_doc,
            invite
        ),
        cmd!(noundo "share.remove", "Remove Person", ["File"], None, "{name} → {removed}", has_doc, remove),
        cmd!(
            noundo "share.open",
            "Open Shared Link",
            ["File"],
            None,
            "{path, token} → {document, slides, permission} (opens with the link's access: view, comment or edit)",
            always,
            open
        ),
        cmd!(query "share.parseLink", "Read Share Link", [], None, "{url} → {path, token}", always, parse),
    ]
}

/// The active presentation's path and title; a share lives beside the saved file.
fn deck(s: &Session) -> Result<(String, String)> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let path = d.path.clone().ok_or_else(|| EngineError::Other("save the presentation before sharing it".into()))?;
    Ok((path, d.title()))
}

fn permission(p: &Value) -> Result<Permission> {
    match p.get("permission") {
        None | Some(Value::Null) => Ok(Permission::Editor),
        Some(Value::String(id)) => {
            Permission::parse(id).ok_or_else(|| bad("share", format!("`permission` must be viewer, commenter or editor: {id}")))
        }
        Some(_) => Err(bad("share", "`permission` must be viewer, commenter or editor")),
    }
}

/// `expiresHours`: absent or 0 means never; anything else is 1–8760 hours.
fn expires_hours(p: &Value) -> Result<Option<u32>> {
    match p.get("expiresHours") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| bad("share", "`expiresHours` must be a whole number"))?;
            if n == 0 {
                Ok(None)
            } else if n > 24 * 365 {
                Err(bad("share", "`expiresHours` cannot be more than a year"))
            } else {
                Ok(Some(n as u32))
            }
        }
    }
}

fn link_json(path: &str, link: &sharing::ShareLink, now: i64) -> Value {
    json!({
        "token": link.token,
        "url": sharing::link_url(path, &link.token),
        "permission": link.permission.id(),
        "permissionLabel": link.permission.label(),
        "created": link.created,
        "expires": link.expires,
        "revoked": link.revoked,
        "valid": link.valid(now),
        "label": link.label,
    })
}

fn participant_json(path: &str, p: &sharing::Participant, now: i64) -> Value {
    json!({
        "name": p.name,
        "token": p.token,
        "url": sharing::link_url(path, &p.token),
        "permission": p.permission.id(),
        "permissionLabel": p.permission.label(),
        "invited": p.invited,
        "expires": p.expires,
        "revoked": p.revoked,
        "valid": p.valid(now),
    })
}

fn info(s: &mut Session, _p: &Value) -> Result<Value> {
    let (path, title) = deck(s)?;
    let share = sharing::load(&path, &title)?;
    let now = sharing::now_secs();
    let access = s.active().map(|d| d.access).unwrap_or_default();
    Ok(json!({
        "path": path,
        "access": access.id(),
        "links": share.links.iter().map(|l| link_json(&path, l, now)).collect::<Vec<_>>(),
        "participants": share.participants.iter().map(|p| participant_json(&path, p, now)).collect::<Vec<_>>(),
    }))
}

fn link(s: &mut Session, p: &Value) -> Result<Value> {
    let (path, title) = deck(s)?;
    let permission = permission(p)?;
    let hours = expires_hours(p)?;
    let label = str_param(p, "label").unwrap_or("");
    let mut share = sharing::load(&path, &title)?;
    let link = share.create_link(permission, hours, label);
    sharing::save(&path, &share)?;
    Ok(json!({
        "token": link.token,
        "url": sharing::link_url(&path, &link.token),
        "permission": link.permission.id(),
        "expires": link.expires,
    }))
}

fn revoke(s: &mut Session, p: &Value) -> Result<Value> {
    let (path, title) = deck(s)?;
    let token = str_param(p, "token").ok_or_else(|| bad("share.revoke", "missing `token`"))?;
    let mut share = sharing::load(&path, &title)?;
    if !share.revoke(token) {
        return Err(EngineError::Other("no active link or invitation has that token".into()));
    }
    sharing::save(&path, &share)?;
    Ok(json!({"revoked": true}))
}

fn invite(s: &mut Session, p: &Value) -> Result<Value> {
    let (path, title) = deck(s)?;
    let name = str_param(p, "name").ok_or_else(|| bad("share.invite", "missing `name`"))?;
    let permission = permission(p)?;
    let hours = expires_hours(p)?;
    let mut share = sharing::load(&path, &title)?;
    let person = share.invite(name, permission, hours)?;
    sharing::save(&path, &share)?;
    Ok(json!({
        "name": person.name,
        "token": person.token,
        "url": sharing::link_url(&path, &person.token),
        "permission": person.permission.id(),
        "expires": person.expires,
    }))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let (path, title) = deck(s)?;
    let name = str_param(p, "name").ok_or_else(|| bad("share.remove", "missing `name`"))?;
    let mut share = sharing::load(&path, &title)?;
    if !share.remove_participant(name) {
        return Err(EngineError::Other(format!("no invitation for {name}")));
    }
    sharing::save(&path, &share)?;
    Ok(json!({"removed": true}))
}

#[cfg(not(target_arch = "wasm32"))]
fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_param(p, "path").ok_or_else(|| bad("share.open", "missing `path`"))?;
    let token = str_param(p, "token").ok_or_else(|| bad("share.open", "missing `token`"))?;
    let share = sharing::load(path, "")?;
    let permission = share.find(token).ok_or_else(|| EngineError::Other("the share link is unknown, revoked or expired".into()))?;
    let loaded = file::read_presentation_file(path)?;
    let result = file::adopt_loaded(s, loaded)?;
    s.set_access(permission.access())?;
    Ok(json!({
        "document": result.get("document").cloned().unwrap_or(Value::Null),
        "slides": result.get("slides").cloned().unwrap_or(Value::Null),
        "permission": permission.id(),
        "access": permission.access().id(),
    }))
}

#[cfg(target_arch = "wasm32")]
fn open(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(EngineError::Other("share.open needs the desktop app: the browser has no path to open".into()))
}

fn parse(_s: &mut Session, p: &Value) -> Result<Value> {
    let url = str_param(p, "url").ok_or_else(|| bad("share.parseLink", "missing `url`"))?;
    let target = sharing::parse_link(url).ok_or_else(|| EngineError::Other("not a DeckCraft share link".into()))?;
    Ok(json!({"path": target.path, "token": target.token}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sharing::{Access, Share};

    fn saved_session(name: &str) -> (Session, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("deckcraft-share-cmd-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("talk.deckcraft");
        let mut s = Session::with_new();
        s.execute("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
        (s, path)
    }

    fn link(s: &mut Session, permission: &str) -> Value {
        s.execute("share.link", &json!({"permission": permission, "label": "press"})).unwrap()
    }

    #[test]
    fn a_viewer_can_navigate_but_not_edit_or_comment() {
        let (mut s, path) = saved_session("viewer");
        let link = link(&mut s, "viewer");
        let token = link["token"].as_str().unwrap().to_string();
        assert!(link["url"].as_str().unwrap().starts_with("deckcraft://share?"));
        s.execute("share.open", &json!({"path": path.to_string_lossy(), "token": token})).unwrap();
        assert_eq!(s.active().unwrap().access, Access::View);
        assert!(s.execute("shape.insert", &json!({"preset": "rect", "rect": [10.0, 10.0, 100.0, 100.0]})).is_err());
        assert!(s.execute("comment.add", &json!({"text": "Hi"})).is_err());
        assert!(s.execute("slide.go", &json!({"index": 0})).is_ok());
        assert!(s.execute("file.save", &json!({})).is_err(), "a viewer cannot overwrite the file");
        assert!(s.execute("share.info", &json!({})).is_err(), "a viewer cannot read other capability tokens");
        assert!(s.execute("share.link", &json!({"permission": "editor"})).is_err(), "a viewer cannot mint an editor link");
        assert!(s.execute("file.saveAs", &json!({"path": path.to_string_lossy()})).is_err(), "nor save over it");
        let dotted = path.with_file_name(".").join(path.file_name().unwrap_or_default());
        assert!(s.execute("file.saveAs", &json!({"path": dotted.to_string_lossy()})).is_err(), "nor reach it through a dot path");
        let pointer =
            crate::PointerEvent { kind: crate::PointerKind::Down, x: 20.0, y: 20.0, mods: crate::Mods::default(), tol: 4.0, pressure: None };
        assert!(s.pointer(pointer).is_err(), "a viewer cannot edit through a pointer gesture");
        let copy = path.with_file_name("copy.deckcraft");
        s.execute("file.saveAs", &json!({"path": copy.to_string_lossy()})).unwrap();
        assert_eq!(s.active().unwrap().access, Access::Full, "a saved copy is the viewer's own");
        assert!(s.execute("shape.insert", &json!({"preset": "rect", "rect": [10.0, 10.0, 100.0, 100.0]})).is_ok());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_commenter_can_comment_but_not_edit() {
        let (mut s, path) = saved_session("commenter");
        let link = link(&mut s, "commenter");
        let token = link["token"].as_str().unwrap().to_string();
        s.execute("share.open", &json!({"path": path.to_string_lossy(), "token": token})).unwrap();
        assert_eq!(s.active().unwrap().access, Access::Comment);
        assert!(s.execute("comment.add", &json!({"text": "A note"})).is_ok());
        assert!(s.execute("shape.insert", &json!({"preset": "rect", "rect": [10.0, 10.0, 100.0, 100.0]})).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_editor_link_keeps_full_access() {
        let (mut s, path) = saved_session("editor");
        let link = link(&mut s, "editor");
        let token = link["token"].as_str().unwrap().to_string();
        s.execute("share.open", &json!({"path": path.to_string_lossy(), "token": token})).unwrap();
        assert_eq!(s.active().unwrap().access, Access::Full);
        assert!(s.execute("shape.insert", &json!({"preset": "rect", "rect": [10.0, 10.0, 100.0, 100.0]})).is_ok());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn revoked_expired_and_unknown_links_do_not_open() {
        let (mut s, path) = saved_session("invalid");
        let link = link(&mut s, "viewer");
        let token = link["token"].as_str().unwrap().to_string();
        let docs = s.documents().len();
        assert!(s.execute("share.open", &json!({"path": path.to_string_lossy(), "token": "nope"})).is_err());
        assert_eq!(s.documents().len(), docs);

        let mut share = sharing::load(&path.to_string_lossy(), "Talk").unwrap();
        share.links[0].expires = Some(1);
        sharing::save(&path.to_string_lossy(), &share).unwrap();
        assert!(s.execute("share.open", &json!({"path": path.to_string_lossy(), "token": token})).is_err());

        let mut share = sharing::load(&path.to_string_lossy(), "Talk").unwrap();
        share.links[0].expires = None;
        share.links[0].revoked = true;
        sharing::save(&path.to_string_lossy(), &share).unwrap();
        assert!(s.execute("share.open", &json!({"path": path.to_string_lossy(), "token": token})).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn info_lists_links_and_invitations_and_revoke_removes_them() {
        let (mut s, path) = saved_session("info");
        let link = link(&mut s, "commenter");
        let token = link["token"].as_str().unwrap().to_string();
        s.execute("share.invite", &json!({"name": "Ada", "permission": "viewer"})).unwrap();
        let info = s.execute("share.info", &json!({})).unwrap();
        assert_eq!(info["links"].as_array().unwrap().len(), 1);
        assert_eq!(info["participants"].as_array().unwrap()[0]["name"], "Ada");
        assert_eq!(info["links"][0]["permission"], "commenter");
        s.execute("share.revoke", &json!({"token": token})).unwrap();
        let info = s.execute("share.info", &json!({})).unwrap();
        assert_eq!(info["links"][0]["valid"], false);
        assert!(s.execute("share.revoke", &json!({"token": token})).is_err(), "already revoked");
        s.execute("share.remove", &json!({"name": "Ada"})).unwrap();
        assert_eq!(s.execute("share.info", &json!({})).unwrap()["participants"].as_array().unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_share_requires_a_saved_presentation_and_bad_parameters_are_rejected() {
        let mut s = Session::with_new();
        assert!(s.execute("share.link", &json!({})).is_err(), "unsaved");
        let (mut s, path) = saved_session("params");
        assert!(s.execute("share.link", &json!({"permission": "owner"})).is_err());
        assert!(s.execute("share.link", &json!({"expiresHours": 24 * 366})).is_err());
        assert!(s.execute("share.invite", &json!({"name": ""})).is_err());
        assert!(s.execute("share.revoke", &json!({"token": "nope"})).is_err());
        assert!(s.execute("share.parseLink", &json!({"url": "https://example.test"})).is_err());
        let parsed = s.execute("share.parseLink", &json!({"url": "deckcraft://share?file=/tmp/a.deckcraft&token=abc"})).unwrap();
        assert_eq!(parsed["path"], "/tmp/a.deckcraft");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_share_file_with_a_newer_version_is_refused() {
        let (s, path) = saved_session("version");
        let mut share = Share::new("Talk");
        share.version = 2;
        let sidecar = sharing::sidecar_path(&path.to_string_lossy()).unwrap();
        std::fs::write(&sidecar, serde_json::to_vec(&share).unwrap()).unwrap();
        assert!(sharing::load(&path.to_string_lossy(), "Talk").is_err());
        assert!(s.active().is_some());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
