//! Share links and access levels for a presentation.
//!
//! A link is a capability: a random token with a permission, an optional expiry and a revocation
//! flag. The links and invitations for a presentation live beside it in `<name>.share.json`; the
//! presentation itself is unchanged. Opening a link checks the token and opens the file with the
//! link's [`Access`], which the engine enforces for every command.
//!
//! This is the offline part of collaboration: links can be handed out, expire, be revoked and be
//! audited without a server. Real-time co-editing needs a relay (the shared-folder journal is the
//! next step); nothing here pretends a file was uploaded anywhere.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{EngineError, Result};

/// A snapshot larger than this is not a share file.
const MAX_SHARE_BYTES: usize = 1 << 20;

/// What a share lets the holder do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    Viewer,
    Commenter,
    #[default]
    Editor,
}

impl Permission {
    pub const ALL: [Permission; 3] = [Permission::Viewer, Permission::Commenter, Permission::Editor];

    pub fn id(self) -> &'static str {
        match self {
            Permission::Viewer => "viewer",
            Permission::Commenter => "commenter",
            Permission::Editor => "editor",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Permission::Viewer => "Can view",
            Permission::Commenter => "Can comment",
            Permission::Editor => "Can edit",
        }
    }

    pub fn parse(id: &str) -> Option<Permission> {
        match id.trim().to_ascii_lowercase().as_str() {
            "view" | "viewer" | "read" | "readonly" => Some(Permission::Viewer),
            "comment" | "commenter" | "review" => Some(Permission::Commenter),
            "edit" | "editor" | "write" => Some(Permission::Editor),
            _ => None,
        }
    }

    /// The session access this permission gives.
    pub fn access(self) -> Access {
        match self {
            Permission::Viewer => Access::View,
            Permission::Commenter => Access::Comment,
            Permission::Editor => Access::Full,
        }
    }
}

/// What the open presentation may be used for. [`Access::Full`] is the ordinary editor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Access {
    #[default]
    Full,
    Comment,
    View,
}

impl Access {
    pub fn id(self) -> &'static str {
        match self {
            Access::Full => "full",
            Access::Comment => "comment",
            Access::View => "view",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Access::Full => "Editing",
            Access::Comment => "Commenting only",
            Access::View => "View only",
        }
    }
    /// Whether a command that changes the presentation (or its saved copy) is allowed.
    pub fn allows_edit(self) -> bool {
        self == Access::Full
    }
    pub fn allows_comment(self) -> bool {
        self != Access::View
    }
}

/// A share link (or an invitation: the label names the person).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShareLink {
    pub token: String,
    pub permission: Permission,
    /// Unix seconds.
    pub created: i64,
    /// Unix seconds; `None` never expires.
    pub expires: Option<i64>,
    pub revoked: bool,
    /// Empty for an anonymous link, a person's name for an invitation.
    pub label: String,
}

impl Default for ShareLink {
    fn default() -> Self {
        ShareLink { token: String::new(), permission: Permission::Editor, created: 0, expires: None, revoked: false, label: String::new() }
    }
}

impl ShareLink {
    pub fn valid(&self, now: i64) -> bool {
        !self.revoked && self.expires.is_none_or(|at| at > now) && !self.token.is_empty()
    }
}

/// A named participant: an invitation with its own token, kept apart from anonymous links so the
/// People list stays readable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Participant {
    pub name: String,
    pub token: String,
    pub permission: Permission,
    pub invited: i64,
    pub expires: Option<i64>,
    pub revoked: bool,
}

impl Default for Participant {
    fn default() -> Self {
        Participant { name: String::new(), token: String::new(), permission: Permission::Editor, invited: 0, expires: None, revoked: false }
    }
}

impl Participant {
    pub fn valid(&self, now: i64) -> bool {
        !self.revoked && self.expires.is_none_or(|at| at > now) && !self.token.is_empty()
    }
}

/// Everything shared about one presentation, stored in `<name>.share.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Share {
    pub version: u32,
    /// The presentation's title when the share was last saved (for the People page).
    pub deck: String,
    pub revision: u64,
    pub modified: i64,
    pub links: Vec<ShareLink>,
    pub participants: Vec<Participant>,
}

impl Default for Share {
    fn default() -> Self {
        Share { version: 1, deck: String::new(), revision: 0, modified: 0, links: vec![], participants: vec![] }
    }
}

pub fn now_secs() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs().min(i64::MAX as u64) as i64).unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

/// A fresh capability token. Two 64-bit SipHash outputs from OS-seeded hashers, mixed with time,
/// process and a counter. Not a password hash, but not guessable from another token either.
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    #[cfg(not(target_arch = "wasm32"))]
    let pid = std::process::id();
    #[cfg(target_arch = "wasm32")]
    let pid: u32 = 0;
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut out = String::with_capacity(32);
    for salt in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(nanos);
        h.write_u32(pid);
        h.write_u64(counter);
        h.write_u64(salt);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

fn clean(value: &str, max: usize) -> String {
    value.trim().chars().filter(|c| !c.is_control()).take(max).collect()
}

fn token_ok(token: &str) -> bool {
    !token.is_empty() && token.len() <= 64 && token.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

impl Share {
    pub fn new(deck: &str) -> Share {
        Share { deck: clean(deck, 256), modified: now_secs(), ..Share::default() }
    }

    pub fn sanitized(mut self) -> Share {
        if self.version > 1 {
            self.version = 1;
        }
        self.deck = clean(&self.deck, 256);
        self.links.truncate(128);
        self.participants.truncate(128);
        let mut seen = BTreeSet::new();
        self.links.retain(|l| token_ok(&l.token) && seen.insert(l.token.clone()));
        self.links.iter_mut().for_each(|l| l.label = clean(&l.label, 64));
        self.participants.retain(|p| token_ok(&p.token) && seen.insert(p.token.clone()));
        self.participants.iter_mut().for_each(|p| p.name = clean(&p.name, 64));
        self
    }

    fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.modified = now_secs();
    }

    /// Create an anonymous link. `expires_hours` is clamped to a day or a year.
    pub fn create_link(&mut self, permission: Permission, expires_hours: Option<u32>, label: &str) -> ShareLink {
        self.create_link_at(permission, expires_hours, label, now_secs())
    }

    pub fn create_link_at(&mut self, permission: Permission, expires_hours: Option<u32>, label: &str, now: i64) -> ShareLink {
        let link = ShareLink {
            token: new_token(),
            permission,
            created: now,
            expires: expires_hours.map(|h| now.saturating_add(i64::from(h.clamp(1, 24 * 365)) * 3600)),
            revoked: false,
            label: clean(label, 64),
        };
        self.links.push(link.clone());
        self.touch();
        link
    }

    /// Invite a person. A second invitation for the same name updates the permission and expiry
    /// instead of creating a second entry with the same name.
    pub fn invite(&mut self, name: &str, permission: Permission, expires_hours: Option<u32>) -> Result<Participant> {
        let name = clean(name, 64);
        if name.is_empty() {
            return Err(EngineError::Other("an invitation needs a name".into()));
        }
        let now = now_secs();
        let expires = expires_hours.map(|h| now.saturating_add(i64::from(h.clamp(1, 24 * 365)) * 3600));
        if let Some(p) = self.participants.iter_mut().find(|p| p.name.eq_ignore_ascii_case(&name)) {
            p.permission = permission;
            p.expires = expires;
            p.revoked = false;
            p.invited = now;
            let p = p.clone();
            self.touch();
            return Ok(p);
        }
        let participant = Participant { name, token: new_token(), permission, invited: now, expires, revoked: false };
        self.participants.push(participant.clone());
        self.touch();
        Ok(participant)
    }

    /// Revoke a link or invitation by token. Returns whether one was found.
    pub fn revoke(&mut self, token: &str) -> bool {
        let mut found = false;
        for link in &mut self.links {
            if link.token == token && !link.revoked {
                link.revoked = true;
                found = true;
            }
        }
        for p in &mut self.participants {
            if p.token == token && !p.revoked {
                p.revoked = true;
                found = true;
            }
        }
        if found {
            self.touch();
        }
        found
    }

    /// Remove an invitation from the People list (its link stops working too).
    pub fn remove_participant(&mut self, name: &str) -> bool {
        let before = self.participants.len();
        self.participants.retain(|p| !p.name.eq_ignore_ascii_case(name));
        let found = self.participants.len() != before;
        if found {
            self.touch();
        }
        found
    }

    /// The permission a token currently gives, or `None` when it is unknown, revoked or expired.
    pub fn find(&self, token: &str) -> Option<Permission> {
        self.find_at(token, now_secs())
    }

    pub fn find_at(&self, token: &str, now: i64) -> Option<Permission> {
        if !token_ok(token) {
            return None;
        }
        if let Some(link) = self.links.iter().find(|l| l.token == token && l.valid(now)) {
            return Some(link.permission);
        }
        self.participants.iter().find(|p| p.token == token && p.valid(now)).map(|p| p.permission)
    }

    /// Every valid link and invitation, for the Share page.
    pub fn active_links(&self) -> Vec<&ShareLink> {
        let now = now_secs();
        self.links.iter().filter(|l| l.valid(now)).collect()
    }

    pub fn active_participants(&self) -> Vec<&Participant> {
        let now = now_secs();
        self.participants.iter().filter(|p| p.valid(now)).collect()
    }
}

/// The sidecar for the presentation at `deck_path`: `talk.deckcraft` → `talk.share.json`.
pub fn sidecar_path(deck_path: &str) -> Option<PathBuf> {
    let path = std::path::Path::new(deck_path);
    let stem = path.file_stem()?.to_string_lossy();
    if stem.is_empty() {
        return None;
    }
    Some(path.with_file_name(format!("{stem}.share.json")))
}

/// Load the share file for `deck_path`; a missing one is an empty share.
pub fn load(deck_path: &str, title: &str) -> Result<Share> {
    let path = sidecar_path(deck_path).ok_or_else(|| EngineError::Other("save the presentation before sharing it".into()))?;
    let path = path.to_string_lossy().to_string();
    let Some(bytes) = crate::io::read_file_optional(&path).map_err(EngineError::Other)? else {
        return Ok(Share::new(title));
    };
    if bytes.len() > MAX_SHARE_BYTES {
        return Err(EngineError::Other(format!("{path} is too large to be a share file")));
    }
    let share: Share = serde_json::from_slice(&bytes).map_err(|e| EngineError::Other(format!("{path} is not a valid share file: {e}")))?;
    if share.version > 1 {
        return Err(EngineError::Other(format!("{path} uses a newer share format (version {})", share.version)));
    }
    Ok(share.sanitized())
}

/// Write the share file beside the presentation, atomically.
pub fn save(deck_path: &str, share: &Share) -> Result<()> {
    let path = sidecar_path(deck_path).ok_or_else(|| EngineError::Other("save the presentation before sharing it".into()))?;
    let bytes = serde_json::to_vec_pretty(&share.clone().sanitized()).map_err(|e| EngineError::Other(format!("can't encode the share file: {e}")))?;
    crate::io::write_file(&path.to_string_lossy(), &bytes).map_err(EngineError::Other)
}

/// A link as a URL a person can paste into a message.
pub fn link_url(deck_path: &str, token: &str) -> String {
    format!("deckcraft://share?file={}&token={}", encode_component(deck_path), encode_component(token))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareTarget {
    pub path: String,
    pub token: String,
}

/// Parse a `deckcraft://share?file=…&token=…` link.
pub fn parse_link(url: &str) -> Option<ShareTarget> {
    let query = url.trim().strip_prefix("deckcraft://share?")?;
    let mut path = None;
    let mut token = None;
    for part in query.split('&') {
        let (key, value) = part.split_once('=')?;
        match key {
            "file" | "path" => path = decode_component(value),
            "token" => token = decode_component(value),
            _ => {}
        }
    }
    let (path, token) = (path?, token?);
    if path.is_empty() || !token_ok(&token) {
        return None;
    }
    Some(ShareTarget { path, token })
}

fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hi = hex(*bytes.get(i + 1)?)?;
                let lo = hex(*bytes.get(i + 2)?)?;
                out.push(hi << 4 | lo);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_deck(name: &str) -> (PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("deckcraft-share-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let deck = dir.join("talk.deckcraft");
        std::fs::write(&deck, b"deck").unwrap();
        (dir, deck.to_string_lossy().to_string())
    }

    #[test]
    fn permissions_round_trip() {
        for p in Permission::ALL {
            assert_eq!(Permission::parse(p.id()), Some(p));
            assert!(!p.label().is_empty());
            assert_eq!(p.access().allows_edit(), p == Permission::Editor);
        }
        assert_eq!(Permission::parse("comment"), Some(Permission::Commenter));
        assert_eq!(Permission::parse("nonsense"), None);
    }

    #[test]
    fn a_link_expires_and_can_be_revoked() {
        let mut share = Share::new("Talk");
        let link = share.create_link_at(Permission::Viewer, Some(1), "press", 1_000);
        assert_eq!(share.find_at(&link.token, 1_001), Some(Permission::Viewer));
        assert_eq!(share.find_at(&link.token, 1_000 + 3600), None, "expiry is exclusive");
        assert!(share.revoke(&link.token));
        assert_eq!(share.find_at(&link.token, 1_001), None);
        assert!(!share.revoke("unknown"));
    }

    #[test]
    fn invitations_are_unique_by_name_and_removable() {
        let mut share = Share::new("Talk");
        let first = share.invite("Ada", Permission::Commenter, None).unwrap();
        let second = share.invite("ada", Permission::Viewer, None).unwrap();
        assert_eq!(first.token, second.token, "re-inviting updates the same person");
        assert_eq!(share.participants.len(), 1);
        assert_eq!(share.find(&first.token), Some(Permission::Viewer));
        assert!(share.remove_participant("ADA"));
        assert_eq!(share.find(&first.token), None);
        assert!(share.invite("", Permission::Viewer, None).is_err());
    }

    #[test]
    fn hostile_tokens_and_duplicates_are_dropped() {
        let mut share = Share::new("Talk");
        share.links.push(ShareLink { token: "bad token".into(), ..Default::default() });
        share.links.push(ShareLink { token: "good".into(), ..Default::default() });
        share.links.push(ShareLink { token: "good".into(), ..Default::default() });
        share.participants.push(Participant { name: "x".repeat(200), token: "p".into(), ..Default::default() });
        let share = share.sanitized();
        assert_eq!(share.links.len(), 1);
        assert_eq!(share.links[0].token, "good");
        assert_eq!(share.participants[0].name.chars().count(), 64);
    }

    #[test]
    fn sidecar_round_trips_beside_the_deck() {
        let (dir, deck) = temp_deck("sidecar");
        assert_eq!(sidecar_path(&deck).unwrap().file_name().unwrap(), "talk.share.json");
        let mut share = load(&deck, "Talk").unwrap();
        assert_eq!(share.links.len(), 0);
        let link = share.create_link(Permission::Editor, Some(24), "team");
        share.invite("Ada", Permission::Commenter, None).unwrap();
        save(&deck, &share).unwrap();
        let again = load(&deck, "Talk").unwrap();
        assert_eq!(again.find(&link.token), Some(Permission::Editor));
        assert_eq!(again.participants[0].name, "Ada");
        assert_eq!(again.revision, share.revision);

        std::fs::write(sidecar_path(&deck).unwrap(), b"{").unwrap();
        assert!(load(&deck, "Talk").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn links_parse_and_keep_windows_paths_and_unicode() {
        let url = link_url("C:\\Talks\\Café & co.deckcraft", "abc123");
        let target = parse_link(&url).unwrap();
        assert_eq!(target.path, "C:\\Talks\\Café & co.deckcraft");
        assert_eq!(target.token, "abc123");
        assert!(parse_link("https://example.test").is_none());
        assert!(parse_link("deckcraft://share?file=x&token=bad token").is_none());
    }

    #[test]
    fn tokens_are_distinct_and_safe() {
        let a = new_token();
        let b = new_token();
        assert_ne!(a, b);
        assert!(token_ok(&a) && token_ok(&b));
    }
}
