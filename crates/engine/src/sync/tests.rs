use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use serde_json::{Value, json};

use super::*;

fn cv(modified: i64, value: Value) -> CategoryValue {
    CategoryValue { modified, value }
}

fn snap(device: &str, revision: u64, cats: &[(&str, i64, Value)]) -> Snapshot {
    let mut categories = BTreeMap::new();
    for (id, modified, value) in cats {
        categories.insert((*id).to_string(), cv(*modified, value.clone()));
    }
    Snapshot { version: 1, device: device.into(), revision, modified: 1_000, categories }
}

fn values(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect()
}

fn all() -> Vec<Category> {
    Category::ALL.to_vec()
}

fn temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("deckcraft-sync-test-{tag}-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn categories_round_trip_and_defaults_avoid_private_data() {
    for cat in Category::ALL {
        assert_eq!(Category::parse(cat.id()), Some(cat));
        assert!(!cat.label().is_empty());
    }
    assert_eq!(Category::parse(" APPEARANCE "), Some(Category::Appearance));
    assert_eq!(Category::parse("nope"), None);
    let defaults = Category::defaults();
    assert!(!defaults.contains(&Category::Recent));
    assert!(!defaults.contains(&Category::Dictionary));
    assert!(defaults.contains(&Category::Appearance));
}

#[test]
fn snapshot_keeps_the_modified_time_of_an_unchanged_value() {
    let previous = snap("dev", 3, &[("appearance", 42, json!({"dark": true}))]);
    let same = snapshot("dev", &[Category::Appearance], &values(&[("appearance", json!({"dark": true}))]), Some(&previous));
    assert_eq!(same.categories["appearance"].modified, 42);
    assert_eq!(same.revision, 3);

    let changed = snapshot("dev", &[Category::Appearance], &values(&[("appearance", json!({"dark": false}))]), Some(&previous));
    assert_ne!(changed.categories["appearance"].modified, 42);
}

#[test]
fn snapshot_omits_unselected_and_missing_categories() {
    let values = values(&[("appearance", json!({"dark": true})), ("recent", json!(["a"]))]);
    let snap = snapshot("dev", &[Category::Appearance], &values, None);
    assert!(snap.categories.contains_key("appearance"));
    assert!(!snap.categories.contains_key("recent"));
    let empty = snapshot("dev", &[Category::Layout], &values, None);
    assert!(empty.categories.is_empty());
}

#[test]
fn merge_takes_a_remote_value_changed_since_the_base() {
    let local = snap("a", 2, &[("appearance", 10, json!({"dark": false}))]);
    let remote = snap("b", 5, &[("appearance", 20, json!({"dark": true}))]);
    let mut state = SyncState::default();
    state.base.insert("appearance".into(), 10);
    let merged = merge(&local, &remote, &all(), &state);
    assert!(!merged.stale);
    assert_eq!(merged.applied, vec![Category::Appearance]);
    assert!(merged.pushed.is_empty());
    assert!(merged.conflicts.is_empty());
    assert_eq!(merged.snapshot.categories["appearance"].value, json!({"dark": true}));
    // The merged snapshot equals the remote one, so its revision does not need to grow.
    assert_eq!(merged.snapshot.revision, 5);
}

#[test]
fn merge_pushes_a_local_value_changed_since_the_base() {
    let local = snap("a", 2, &[("appearance", 30, json!({"dark": true}))]);
    let remote = snap("b", 5, &[("appearance", 10, json!({"dark": false}))]);
    let mut state = SyncState::default();
    state.base.insert("appearance".into(), 10);
    let merged = merge(&local, &remote, &all(), &state);
    assert_eq!(merged.pushed, vec![Category::Appearance]);
    assert!(merged.applied.is_empty());
    assert_eq!(merged.snapshot.categories["appearance"].value, json!({"dark": true}));
}

#[test]
fn merge_reports_a_conflict_and_uses_the_newest_then_the_device_id() {
    let local = snap("a", 2, &[("appearance", 30, json!({"dark": true}))]);
    let remote = snap("b", 5, &[("appearance", 20, json!({"dark": false}))]);
    let mut state = SyncState::default();
    state.base.insert("appearance".into(), 10);
    let merged = merge(&local, &remote, &all(), &state);
    assert_eq!(merged.conflicts.len(), 1);
    assert_eq!(merged.conflicts[0].kept, json!({"dark": true}));
    assert_eq!(merged.conflicts[0].lost, json!({"dark": false}));
    assert_eq!(merged.conflicts[0].kept_device, "a");

    // Equal times: the greater device id wins, so every install agrees.
    let local = snap("a", 2, &[("appearance", 20, json!({"dark": true}))]);
    let remote = snap("b", 5, &[("appearance", 20, json!({"dark": false}))]);
    let merged = merge(&local, &remote, &all(), &state);
    assert_eq!(merged.applied, vec![Category::Appearance]);
    assert_eq!(merged.conflicts[0].kept_device, "b");
}

#[test]
fn merge_keeps_remote_categories_the_user_did_not_select() {
    let local = snap("a", 1, &[("appearance", 10, json!({"dark": true}))]);
    let remote =
        snap("b", 4, &[("appearance", 10, json!({"dark": true})), ("dictionary", 20, json!(["deckcraft"])), ("recent", 20, json!(["secret.pptx"]))]);
    let merged = merge(&local, &remote, &[Category::Appearance], &SyncState::default());
    assert!(merged.snapshot.categories.contains_key("dictionary"));
    assert!(merged.snapshot.categories.contains_key("recent"));
}

#[test]
fn merge_with_nothing_to_do_does_not_grow_the_revision() {
    let local = snap("a", 7, &[("appearance", 10, json!({"dark": true}))]);
    let remote = snap("b", 9, &[("appearance", 10, json!({"dark": true}))]);
    let mut state = SyncState::default();
    state.base.insert("appearance".into(), 10);
    let merged = merge(&local, &remote, &all(), &state);
    assert_eq!(merged.snapshot.revision, 9);
    assert_eq!(merged.snapshot.modified, remote.modified);
    assert_eq!(merged.snapshot.categories, remote.categories);
}

#[test]
fn merge_refuses_a_stale_or_newer_format() {
    let local = snap("a", 2, &[("appearance", 10, json!({"dark": true}))]);
    let remote = snap("b", 1, &[("appearance", 20, json!({"dark": false}))]);
    let state = SyncState { last_remote_revision: 3, ..SyncState::default() };
    let merged = merge(&local, &remote, &all(), &state);
    assert!(merged.stale);
    assert_eq!(merged.snapshot, local);

    let mut future = remote.clone();
    future.version = 2;
    let merged = merge(&local, &future, &all(), &SyncState::default());
    assert!(merged.stale);
}

#[test]
fn snapshot_json_round_trips_and_refuses_newer_versions() {
    let s = snap("dev", 3, &[("appearance", 10, json!({"dark": true}))]);
    let bytes = encode(&s).unwrap();
    assert_eq!(parse(&bytes).unwrap(), Some(s));
    assert_eq!(parse(b"  \n").unwrap(), None);
    let mut bad = bytes;
    bad.extend_from_slice(b"junk");
    assert!(parse(&bad).is_err());
    let future = br#"{"version":9,"device":"x","revision":0,"modified":0,"categories":{}}"#;
    assert!(parse(future).is_err());
}

#[test]
fn folder_sync_uploads_once_then_applies_remote_changes() {
    let dir = temp_dir("folder");
    let mut transport = FolderTransport { dir: dir.clone() };
    let mut state = SyncState::default();
    let selected = Category::defaults();
    let local = snapshot("dev-a", &selected, &values(&[("appearance", json!({"dark": true}))]), None);
    let report = sync(&mut transport, &local, &selected, &mut state).unwrap();
    assert!(report.uploaded);
    assert_eq!(report.provider, Provider::LocalFolder);
    assert_eq!(state.revision, local.revision);

    let remote = parse(&crate::io::read_app_file(&dir.join("settings.json")).unwrap()).unwrap().unwrap();
    let again = snapshot("dev-a", &selected, &values(&[("appearance", json!({"dark": true}))]), Some(&remote));
    let report = sync(&mut transport, &again, &selected, &mut state).unwrap();
    assert!(!report.uploaded, "an unchanged sync must not rewrite the remote");
    assert_eq!(report.summary(), "Settings sync: already up to date");

    // Another install changes appearance remotely; the local value is still at the base.
    let mut remote = remote;
    remote.device = "dev-b".into();
    remote.revision += 1;
    remote.modified += 1;
    remote.categories.insert("appearance".into(), cv(remote.modified, json!({"dark": false})));
    crate::io::write_app_file(&dir.join("settings.json"), &encode(&remote).unwrap()).unwrap();
    let report = sync(&mut transport, &again, &selected, &mut state).unwrap();
    assert!(report.downloaded);
    assert_eq!(report.applied, vec![Category::Appearance]);
    assert!(!report.uploaded);
    assert_eq!(state.base["appearance"], remote.modified);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn folder_sync_refuses_a_stale_remote_and_survives_a_damaged_one() {
    let dir = temp_dir("stale");
    let mut transport = FolderTransport { dir: dir.clone() };
    let selected = Category::defaults();
    let local = snapshot("dev-a", &selected, &values(&[("appearance", json!({"dark": true}))]), None);
    crate::io::write_app_file(&dir.join("settings.json"), b"{not json").unwrap();
    assert!(sync(&mut transport, &local, &selected, &mut SyncState::default()).is_err());

    let remote = snap("dev-b", 9, &[("appearance", 10, json!({"dark": false}))]);
    crate::io::write_app_file(&dir.join("settings.json"), &encode(&remote).unwrap()).unwrap();
    let mut state = SyncState { last_remote_revision: 10, ..SyncState::default() };
    let report = sync(&mut transport, &local, &selected, &mut state).unwrap();
    assert!(report.stale);
    assert!(!report.uploaded);
    assert_eq!(report.summary(), "The remote settings are older than the last sync; nothing was changed.");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn config_is_sanitized_and_saved_owner_only() {
    let dir = temp_dir("config");
    let path = dir.join("sync.json");
    assert_eq!(load_config(&path).unwrap(), SyncConfig::default());
    let config = SyncConfig {
        enabled: true,
        provider: Provider::S3,
        interval_minutes: 0,
        categories: vec!["appearance".into(), "nonsense".into(), "appearance".into()],
        local_folder: "  /tmp/deckcraft sync  ".into(),
        s3: S3Config { bucket: " bucket ".into(), prefix: "/deck/settings/".into(), ..S3Config::default() },
        ..SyncConfig::default()
    }
    .sanitized();
    assert_eq!(config.interval_minutes, 1);
    assert_eq!(config.categories, vec!["appearance".to_string()]);
    assert_eq!(config.local_folder, "/tmp/deckcraft sync");
    assert_eq!(config.s3.region, "us-east-1");
    assert_eq!(config.s3.bucket, "bucket");
    assert_eq!(config.s3.prefix, "deck/settings");
    assert_eq!(config.object_key(), "deck/settings/settings.json");
    save_config(&path, &config).unwrap();
    assert_eq!(load_config(&path).unwrap(), config);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the config holds credentials");
    }

    // A damaged config is an error, not silently replaced.
    crate::io::write_app_file(&path, b"{").unwrap();
    assert!(load_config(&path).is_err());

    let state_path = dir.join("state.json");
    assert_eq!(load_state(&state_path), SyncState::default());
    crate::io::write_app_file(&state_path, b"{").unwrap();
    assert_eq!(load_state(&state_path), SyncState::default());
    let state = SyncState { device: "dev".into(), revision: 4, ..SyncState::default() };
    save_state(&state_path, &state).unwrap();
    assert_eq!(load_state(&state_path), state);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn device_ids_are_stable_safe_and_distinct() {
    let a = new_device_id();
    let b = new_device_id();
    assert_ne!(a, b);
    assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    assert!(a.len() <= 64);
}

#[test]
fn s3_keys_are_encoded_but_keep_their_slashes() {
    assert_eq!(encode_key("deck/settings.json"), "deck/settings.json");
    assert_eq!(encode_key("a b/ü.json"), "a%20b/%C3%BC.json");
}

#[test]
fn curl_config_quotes_values_and_keeps_booleans_bare() {
    let text = curl_config(&[("silent", String::new()), ("url", "https://example.test/a\"b".into()), ("user", "key:se\\cret".into())]);
    assert_eq!(text, "silent\nurl = \"https://example.test/a\\\"b\"\nuser = \"key:se\\\\cret\"\n");
}

type CallLog = std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<String>, Option<Vec<u8>>, Option<Vec<u8>>)>>>;

struct FakeRunner {
    calls: CallLog,
    replies: VecDeque<CommandOutput>,
}

impl FakeRunner {
    fn new(replies: Vec<CommandOutput>) -> Self {
        FakeRunner { calls: std::rc::Rc::new(std::cell::RefCell::new(vec![])), replies: replies.into() }
    }

    /// A handle to the calls made, valid after the runner has been boxed into a transport.
    fn log(&self) -> CallLog {
        self.calls.clone()
    }
}

fn http(code: u16, body: &[u8]) -> CommandOutput {
    let mut stdout = body.to_vec();
    stdout.extend_from_slice(format!("\n{code}").as_bytes());
    CommandOutput { status: 0, stdout, stderr: vec![] }
}

impl Runner for FakeRunner {
    fn run(&mut self, program: &str, args: &[String], stdin: Option<&[u8]>) -> Result<CommandOutput> {
        // `--data-binary @file`: read the file now, because the transport removes it after the call.
        let body = args.iter().find_map(|a| a.strip_prefix('@')).and_then(|p| std::fs::read(p).ok());
        self.calls.borrow_mut().push((program.to_string(), args.to_vec(), stdin.map(<[u8]>::to_vec), body));
        Ok(self.replies.pop_front().unwrap_or_default())
    }
}

fn s3_config() -> S3Config {
    S3Config {
        endpoint: "https://minio.example.test".into(),
        region: "eu-west-1".into(),
        bucket: "deck".into(),
        prefix: "settings".into(),
        access_key: "AKIA-TEST".into(),
        secret_key: "top secret/+==".into(),
    }
}

#[test]
fn s3_transport_signs_requests_without_putting_secrets_in_arguments() {
    let body = encode(&snap("dev", 1, &[("appearance", 10, json!({"dark": true}))])).unwrap();
    let fake = FakeRunner::new(vec![http(200, &body), http(404, b""), http(200, b"")]);
    let log = fake.log();
    let mut transport = S3Transport::with_runner(s3_config(), "settings/settings.json".into(), Box::new(fake));
    assert_eq!(transport.load().unwrap(), Some(body.clone()));
    assert_eq!(transport.load().unwrap(), None);
    transport.save(&body).unwrap();

    let calls = log.borrow();
    assert_eq!(calls.len(), 3);
    let (program, args, stdin, _) = &calls[0];
    assert_eq!(program, "curl");
    assert!(!args.iter().any(|a| a.contains("top secret")), "the secret must not be in argv: {args:?}");
    let config = String::from_utf8(stdin.clone().unwrap()).unwrap();
    assert!(config.contains("url = \"https://minio.example.test/deck/settings/settings.json\""));
    assert!(config.contains("aws-sigv4 = \"aws:amz:eu-west-1:s3\""));
    assert!(config.contains("top secret/+=="));
    assert!(config.contains("request = \"GET\""));

    let upload = &calls[2];
    let upload_config = String::from_utf8(upload.2.clone().unwrap()).unwrap();
    assert!(upload_config.contains("request = \"PUT\""));
    let path = upload.1.iter().find_map(|a| a.strip_prefix('@')).unwrap();
    assert_eq!(upload.3.as_deref(), Some(body.as_slice()));
    assert!(!std::path::Path::new(path).exists(), "the temporary body is removed");
}

#[test]
fn s3_transport_reports_missing_credentials_and_http_errors() {
    let fake = FakeRunner::new(vec![http(403, b"AccessDenied")]);
    let mut bad = s3_config();
    bad.secret_key.clear();
    let mut transport = S3Transport::with_runner(bad, "settings.json".into(), Box::new(fake));
    assert!(transport.load().unwrap_err().to_string().contains("secret key"));

    let fake = FakeRunner::new(vec![http(403, b"AccessDenied")]);
    let mut transport = S3Transport::with_runner(s3_config(), "settings.json".into(), Box::new(fake));
    let error = transport.load().unwrap_err().to_string();
    assert!(error.contains("403") && error.contains("AccessDenied"), "{error}");
}

#[test]
fn drive_transport_downloads_and_uploads_the_snapshot() {
    let body = encode(&snap("dev", 2, &[("appearance", 10, json!({"dark": true}))])).unwrap();
    let replies = vec![
        http(200, br#"{"files":[{"id":"file_1","modifiedTime":"2025-01-01T00:00:00Z"}]}"#),
        http(200, &body),
        http(200, br#"{"files":[{"id":"file_1","modifiedTime":"2025-01-01T00:00:00Z"}]}"#),
        http(200, br#"{"id":"file_1"}"#),
    ];
    let fake = FakeRunner::new(replies);
    let log = fake.log();
    let mut config = DriveConfig {
        client_id: "client".into(),
        client_secret: "secret".into(),
        refresh_token: "refresh".into(),
        access_token: "access-token".into(),
        folder_id: "folder_1".into(),
        file_name: "deckcraft-settings.json".into(),
    };
    let mut transport = DriveTransport::with_runner(config.clone(), "settings.json".into(), Box::new(fake));
    assert_eq!(transport.load().unwrap(), Some(body.clone()));
    transport.save(&body).unwrap();

    let calls = log.borrow();
    assert_eq!(calls.len(), 4);
    let search = String::from_utf8(calls[0].2.clone().unwrap()).unwrap();
    assert!(search.contains("Authorization: Bearer access-token"));
    assert!(search.contains("name = 'deckcraft-settings.json'"));
    assert!(search.contains("'folder_1' in parents"));
    let upload = &calls[3];
    assert_eq!(upload.0, "curl");
    let upload_config = String::from_utf8(upload.2.clone().unwrap()).unwrap();
    assert!(upload_config.contains("request = \"PATCH\""));
    assert!(upload_config.contains("multipart/related"));
    let multipart = upload.3.clone().unwrap();
    assert!(multipart.windows(body.len()).any(|w| w == body.as_slice()));
    let multipart = String::from_utf8_lossy(&multipart).to_string();
    assert!(multipart.contains("\"name\":\"deckcraft-settings.json\""));
    assert!(!multipart.contains("\"parents\""), "an update keeps the file in its folder");

    // Creating a new file uses POST and no file id.
    config.access_token = "access-token".into();
    let fake = FakeRunner::new(vec![http(200, br#"{"files":[]}"#), http(200, br#"{"id":"new"}"#)]);
    let log = fake.log();
    let mut transport = DriveTransport::with_runner(config, "settings.json".into(), Box::new(fake));
    transport.save(&body).unwrap();
    let calls = log.borrow();
    let create_config = String::from_utf8(calls[1].2.clone().unwrap()).unwrap();
    assert!(create_config.contains("request = \"POST\""));
    assert!(!create_config.contains("files/new"), "{create_config}");
    let created = String::from_utf8_lossy(calls[1].3.as_deref().unwrap()).to_string();
    assert!(created.contains("\"parents\":[\"folder_1\"]"), "a new file goes in the chosen folder");
}

#[test]
fn drive_transport_refreshes_an_expired_token() {
    let body = encode(&snap("dev", 2, &[("appearance", 10, json!({"dark": true}))])).unwrap();
    let fake = FakeRunner::new(vec![http(200, br#"{"access_token":"fresh-token","expires_in":3600}"#), http(200, br#"{"files":[]}"#)]);
    let config = DriveConfig {
        client_id: "client".into(),
        client_secret: "secret".into(),
        refresh_token: "refresh".into(),
        folder_id: String::new(),
        ..DriveConfig::default()
    };
    let log = fake.log();
    let mut transport = DriveTransport::with_runner(config, "settings.json".into(), Box::new(fake));
    assert_eq!(transport.load().unwrap(), None);
    let calls = log.borrow();
    let refresh = String::from_utf8(calls[0].2.clone().unwrap()).unwrap();
    assert!(refresh.contains("refresh_token=refresh"));
    assert!(refresh.contains("client_secret=secret"));
    assert!(refresh.contains("grant_type=refresh_token"));
    assert!(!calls[0].1.iter().any(|a| a.contains("secret")), "the client secret stays off argv");
    assert!(String::from_utf8(calls[1].2.clone().unwrap()).unwrap().contains("Bearer fresh-token"));
    let _ = body;
}

#[test]
fn drive_transport_reports_a_bad_token_response() {
    let fake = FakeRunner::new(vec![http(400, br#"{"error":"invalid_grant"}"#)]);
    let config = DriveConfig { client_id: "c".into(), refresh_token: "r".into(), ..DriveConfig::default() };
    let mut transport = DriveTransport::with_runner(config, "settings.json".into(), Box::new(fake));
    let error = transport.load().unwrap_err().to_string();
    assert!(error.contains("400") && error.contains("invalid_grant"), "{error}");
}

#[test]
fn transport_for_config_rejects_an_empty_folder() {
    let config = SyncConfig { provider: Provider::LocalFolder, ..SyncConfig::default() };
    assert!(transport(&config).is_err());
    let config = SyncConfig { provider: Provider::LocalFolder, local_folder: "/tmp/x".into(), ..SyncConfig::default() };
    assert_eq!(transport(&config).unwrap().provider(), Provider::LocalFolder);
}
