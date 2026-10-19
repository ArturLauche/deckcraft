//! Settings sync in the editor: the Preferences section, the background worker and applying a
//! merged snapshot to the local settings.
//!
//! The local settings files stay the primary copy. The worker only reads the local snapshot and
//! talks to the remote; when it finishes, the main thread applies the categories the merge chose
//! from the remote and saves the last-sync state. A failed sync changes nothing locally.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use deckcraft_engine::sync::{self, Category, CategoryValue, Conflict, Provider, Report, Snapshot, SyncConfig, SyncState};
use deckcraft_engine::{Prefs, spell};
use egui::{Ui, vec2};
use serde_json::{Value, json};

use crate::{RECENT_MAX, SlideApp, UiState, ViewMode};

pub const CONFIG_FILE: &str = "sync.json";
pub const STATE_FILE: &str = "sync-state.json";

/// What the worker last reported.
#[derive(Clone, Debug)]
pub enum State {
    Running,
    Done(Box<Outcome>),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub report: Report,
    pub state: SyncState,
}

/// A running sync (the window and Preferences show its progress).
pub struct Job {
    state: Arc<Mutex<State>>,
    pub started: f64,
}

impl Job {
    pub fn state(&self) -> State {
        match self.state.lock() {
            Ok(s) => s.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.state(), State::Running)
    }
}

/// The sync settings and last result, loaded from the settings folder when the editor starts.
pub struct SyncUi {
    pub config: SyncConfig,
    pub state: SyncState,
    /// The folder the settings were loaded from; `None` until a desktop settings folder is known.
    pub loaded_for: Option<PathBuf>,
    pub status: String,
    pub conflicts: Vec<Conflict>,
    pub job: Option<Job>,
    /// egui time of the last attempt, for the automatic interval.
    pub last_attempt: f64,
    /// The config when Preferences opened, so Cancel restores it without putting credentials in
    /// the dialog's serializable parameters.
    pub editing_before: Option<SyncConfig>,
}

impl Default for SyncUi {
    fn default() -> Self {
        SyncUi {
            config: SyncConfig::default(),
            state: SyncState::default(),
            loaded_for: None,
            status: String::new(),
            conflicts: vec![],
            job: None,
            last_attempt: 0.0,
            editing_before: None,
        }
    }
}

impl SyncUi {
    /// Read `sync.json` and `sync-state.json` from `dir` once. A missing folder (the browser) is
    /// reported, not treated as "never sync": the user asked for a desktop-only feature.
    pub fn ensure_loaded(&mut self, dir: Option<&Path>) {
        let Some(dir) = dir else {
            if self.loaded_for.is_none() && self.status.is_empty() {
                self.status = "Settings sync needs the desktop app.".into();
            }
            return;
        };
        if self.loaded_for.as_deref() == Some(dir) {
            return;
        }
        self.loaded_for = Some(dir.to_path_buf());
        match sync::load_config(&dir.join(CONFIG_FILE)) {
            Ok(config) => self.config = config,
            Err(e) => {
                self.config = SyncConfig::default();
                self.status = format!("The sync settings could not be read: {e}");
            }
        }
        self.state = sync::load_state(&dir.join(STATE_FILE));
        if self.state.device.is_empty() {
            self.state.device = sync::new_device_id();
        }
    }

    /// Save the config (owner-only: it holds credentials) and the last-sync state.
    pub fn persist(&self) {
        let Some(dir) = self.loaded_for.as_deref() else { return };
        if let Err(e) = sync::save_config(&dir.join(CONFIG_FILE), &self.config) {
            log::warn!("sync settings: {e}");
        }
        if let Err(e) = sync::save_state(&dir.join(STATE_FILE), &self.state) {
            log::warn!("sync state: {e}");
        }
    }

    fn previous(&self) -> Option<Snapshot> {
        if self.state.base_values.is_empty() {
            return None;
        }
        let categories = self
            .state
            .base_values
            .iter()
            .map(|(id, value)| (id.clone(), CategoryValue { modified: self.state.base.get(id).copied().unwrap_or(0), value: value.clone() }))
            .collect();
        Some(Snapshot { version: 1, device: self.state.device.clone(), revision: self.state.revision, modified: self.state.last_sync, categories })
    }

    /// The local settings as a snapshot, keeping each category's modified time when it has not
    /// changed since the last sync (so the merge can tell which side changed).
    pub fn local_snapshot(&self, app: &SlideApp) -> Snapshot {
        let values = local_values(app);
        let previous = self.previous();
        sync::snapshot(&self.state.device, &self.config.selected(), &values, previous.as_ref())
    }

    pub fn start(&mut self, dir: PathBuf, local: Snapshot, now: f64) {
        if self.job.as_ref().is_some_and(Job::running) {
            return;
        }
        let config = self.config.clone();
        let selected = config.selected();
        let passphrase = if config.encrypt { Some(config.passphrase.clone()).filter(|p| !p.trim().is_empty()) } else { None };
        let mut state = self.state.clone();
        let state_path = dir.join(STATE_FILE);
        let shared = Arc::new(Mutex::new(State::Running));
        let worker = shared.clone();
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::thread::spawn(move || {
                let result = (|| -> deckcraft_engine::Result<(Report, SyncState)> {
                    let mut transport = sync::transport(&config)?;
                    let report = sync::sync_with(transport.as_mut(), &local, &selected, &mut state, passphrase.as_deref())?;
                    sync::save_state(&state_path, &state)?;
                    Ok((report, state))
                })();
                let next = match result {
                    Ok((report, state)) => State::Done(Box::new(Outcome { report, state })),
                    Err(e) => State::Failed(e.to_string()),
                };
                if let Ok(mut s) = worker.lock() {
                    *s = next;
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (local, selected, state, state_path);
            if let Ok(mut s) = worker.lock() {
                *s = State::Failed("Settings sync needs the desktop app.".into());
            }
        }
        self.job = Some(Job { state: shared, started: now });
        self.last_attempt = now;
        self.status = "Syncing settings…".into();
    }

    /// The finished result, once. The caller applies it and keeps the state.
    pub fn take_finished(&mut self) -> Option<State> {
        let state = self.job.as_ref()?.state();
        match state {
            State::Done(_) | State::Failed(_) => {
                self.job = None;
                Some(state)
            }
            State::Running => None,
        }
    }
}

/// The settings categories as JSON, exactly as they are sent.
pub fn local_values(app: &SlideApp) -> BTreeMap<String, Value> {
    let ui = &app.ui;
    let mut values = BTreeMap::new();
    values.insert("appearance".into(), json!({"dark": ui.brightness == crate::theme::Brightness::Dark, "uiScale": ui.ui_scale}));
    values.insert(
        "layout".into(),
        json!({
            "tab": ui.tab.clone(),
            "view": ui.view,
            "notes": ui.notes,
            "notesHeight": ui.notes_height,
            "thumbsWidth": ui.thumbs_width,
            "pane": ui.pane.clone(),
            "paneWidth": ui.pane_width,
            "ruler": ui.ruler,
            "gridlines": ui.gridlines,
            "guides": ui.guides,
            "zoom": ui.zoom,
            "sorterZoom": ui.sorter_zoom,
            "handoutPerPage": ui.handout_per_page,
        }),
    );
    values.insert("editing".into(), serde_json::to_value(&app.session.prefs).unwrap_or(Value::Null));
    values.insert("recent".into(), json!(ui.recent.clone()));
    values.insert("favourites".into(), json!(ui.favourite_templates.clone()));
    values.insert("quickAccess".into(), json!(ui.qat.clone()));
    values.insert("dictionary".into(), json!(spell::user_words()));
    values
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 4096 && !s.contains('\0')).map(str::to_string).collect())
        .unwrap_or_default()
}

fn short(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).filter(|s| s.len() <= 64 && !s.contains('\0')).map(str::to_string)
}

/// Apply the categories the merge took from the remote. Returns true when the appearance changed
/// (the caller reinstalls the theme).
pub fn apply(ui: &mut UiState, prefs: &mut Prefs, snapshot: &Snapshot, applied: &[Category]) -> bool {
    let mut appearance = false;
    for category in applied {
        let Some(value) = snapshot.categories.get(category.id()).map(|c| &c.value) else { continue };
        match category {
            Category::Appearance => {
                if let Some(dark) = value.get("dark").and_then(Value::as_bool) {
                    let next = if dark { crate::theme::Brightness::Dark } else { crate::theme::Brightness::Light };
                    if ui.brightness != next {
                        ui.brightness = next;
                        appearance = true;
                    }
                }
                if let Some(scale) = value.get("uiScale").and_then(Value::as_f64) {
                    ui.ui_scale = (scale as f32).clamp(0.5, 3.0);
                }
            }
            Category::Layout => {
                if let Some(tab) = short(value, "tab") {
                    ui.tab = tab;
                }
                if let Some(view) = value.get("view").and_then(|v| serde_json::from_value::<ViewMode>(v.clone()).ok()) {
                    ui.view = view;
                }
                if let Some(v) = value.get("notes").and_then(Value::as_bool) {
                    ui.notes = v;
                }
                if let Some(v) = value.get("notesHeight").and_then(Value::as_f64) {
                    ui.notes_height = v as f32;
                }
                if let Some(v) = value.get("thumbsWidth").and_then(Value::as_f64) {
                    ui.thumbs_width = v as f32;
                }
                if value.get("pane").is_some_and(Value::is_null) {
                    ui.pane = None;
                } else if let Some(pane) = short(value, "pane") {
                    ui.pane = Some(pane);
                }
                if let Some(v) = value.get("paneWidth").and_then(Value::as_f64) {
                    ui.pane_width = v as f32;
                }
                for (key, field) in [("ruler", &mut ui.ruler), ("gridlines", &mut ui.gridlines), ("guides", &mut ui.guides)] {
                    if let Some(v) = value.get(key).and_then(Value::as_bool) {
                        *field = v;
                    }
                }
                if value.get("zoom").is_some_and(Value::is_null) {
                    ui.zoom = None;
                } else if let Some(v) = value.get("zoom").and_then(Value::as_f64) {
                    ui.zoom = Some((v as f32).clamp(*crate::ZOOM_RANGE.start(), *crate::ZOOM_RANGE.end()));
                }
                if let Some(v) = value.get("sorterZoom").and_then(Value::as_f64) {
                    ui.sorter_zoom = (v as f32).clamp(0.5, 3.0);
                }
                if let Some(v) = value.get("handoutPerPage").and_then(Value::as_u64)
                    && matches!(v, 1 | 2 | 3 | 4 | 6 | 9)
                {
                    ui.handout_per_page = v as u8;
                }
            }
            Category::Editing => {
                if let Ok(next) = serde_json::from_value::<Prefs>(value.clone()) {
                    *prefs = next.sanitized();
                }
            }
            Category::Recent => {
                ui.recent = strings(value);
                ui.recent.truncate(RECENT_MAX);
            }
            Category::Favourites => {
                ui.favourite_templates = strings(value);
                ui.favourite_templates.retain(|s| s.len() <= 64);
                ui.favourite_templates.truncate(64);
            }
            Category::QuickAccess => {
                if let Ok(items) = serde_json::from_value::<Vec<crate::qat::QatItem>>(value.clone()) {
                    ui.qat = items.into_iter().filter(|i| !i.id.is_empty() && i.id.len() <= 64).take(64).collect();
                }
            }
            Category::Dictionary => {
                for word in strings(value) {
                    let word = word.trim();
                    if !word.is_empty() && word.chars().count() <= 64 {
                        let _ = spell::add_user_word(word);
                    }
                }
            }
        }
    }
    *ui = ui.clone().sanitized();
    *prefs = prefs.clone().sanitized();
    appearance
}

fn ago(seconds: i64) -> String {
    if seconds <= 0 {
        return "never".into();
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs().min(i64::MAX as u64) as i64).unwrap_or(0);
    let age = now.saturating_sub(seconds);
    match age {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", age / 60),
        3600..=86_399 => format!("{} h ago", age / 3600),
        _ => format!("{} days ago", age / 86_400),
    }
}

fn text_row(ui: &mut Ui, label: &str, value: &mut String, password: bool) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(150.0, 20.0), egui::Label::new(label));
        let edit = egui::TextEdit::singleline(value).desired_width(220.0);
        ui.add(if password { edit.password(true) } else { edit });
    });
}

fn compact(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "?".into());
    if text.chars().count() > 60 { text.chars().take(60).collect::<String>() + "…" } else { text }
}

/// Preferences ▸ Settings sync. Called from the Preferences dialog body.
pub fn preferences(app: &mut SlideApp, ui: &mut Ui) {
    app.sync.ensure_loaded(app.settings_dir.as_deref());
    ui.add_space(8.0);
    ui.separator();
    ui.label(egui::RichText::new("Settings sync").font(crate::theme::bold(13.0)));
    if app.settings_dir.is_none() {
        ui.label("Settings sync needs the desktop app: the browser has no settings folder.");
        return;
    }

    let mut config = app.sync.config.clone();
    ui.checkbox(&mut config.enabled, "Sync settings between my computers");
    if config.enabled {
        ui.horizontal(|ui| {
            ui.label("Where:");
            egui::ComboBox::from_id_salt("sync-provider").selected_text(config.provider.label()).show_ui(ui, |ui| {
                for provider in [Provider::LocalFolder, Provider::S3, Provider::GoogleDrive] {
                    ui.selectable_value(&mut config.provider, provider, provider.label());
                }
            });
        });
        match config.provider {
            Provider::LocalFolder => {
                text_row(ui, "Folder", &mut config.local_folder, false);
                ui.label(egui::RichText::new("A shared drive or a folder your computers can reach.").font(crate::theme::font(11.5)));
            }
            Provider::S3 => {
                text_row(ui, "Endpoint", &mut config.s3.endpoint, false);
                text_row(ui, "Region", &mut config.s3.region, false);
                text_row(ui, "Bucket", &mut config.s3.bucket, false);
                text_row(ui, "Prefix", &mut config.s3.prefix, false);
                text_row(ui, "Access key", &mut config.s3.access_key, false);
                text_row(ui, "Secret key", &mut config.s3.secret_key, true);
                ui.label(
                    egui::RichText::new("AWS S3 and compatible services (MinIO, Cloudflare R2…). The endpoint may be empty for AWS.")
                        .font(crate::theme::font(11.5)),
                );
            }
            Provider::GoogleDrive => {
                text_row(ui, "Client id", &mut config.drive.client_id, false);
                text_row(ui, "Client secret", &mut config.drive.client_secret, true);
                text_row(ui, "Refresh token", &mut config.drive.refresh_token, true);
                text_row(ui, "Folder id", &mut config.drive.folder_id, false);
                text_row(ui, "File name", &mut config.drive.file_name, false);
                ui.label(
                    egui::RichText::new("Use an OAuth client of your own; the refresh token is saved owner-only.").font(crate::theme::font(11.5)),
                );
            }
        }
        ui.add_space(4.0);
        ui.checkbox(&mut config.encrypt, "Encrypt remote settings");
        if config.encrypt {
            text_row(ui, "Passphrase", &mut config.passphrase, true);
            ui.label(
                egui::RichText::new(
                    "AES-256-GCM with an Argon2id key. The passphrase is kept in the owner-only sync settings so automatic sync can run.",
                )
                .font(crate::theme::font(11.5)),
            );
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Categories").font(crate::theme::bold(12.0)));
        let selected = config.selected();
        egui::Grid::new("sync-categories").num_columns(2).show(ui, |ui| {
            for category in Category::ALL {
                let mut on = selected.contains(&category);
                if ui.checkbox(&mut on, category.label()).changed() {
                    config.categories.retain(|c| Category::parse(c) != Some(category));
                    if on {
                        config.categories.push(category.id().into());
                    }
                }
                ui.end_row();
            }
        });
        if config.categories.is_empty() {
            ui.label(egui::RichText::new("No category is selected; the defaults will be used.").font(crate::theme::font(11.5)));
        }
        ui.add_space(4.0);
        ui.checkbox(&mut config.auto, "Sync automatically");
        if config.auto {
            ui.add(egui::Slider::new(&mut config.interval_minutes, 5..=1440).text("minutes between syncs"));
        }
        ui.label(
            egui::RichText::new("Local settings are the primary copy. A sync merges the selected categories; conflicts keep the newer value.")
                .font(crate::theme::font(11.5)),
        );
    }
    app.sync.config = config.sanitized();

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let running = app.sync.job.as_ref().is_some_and(Job::running);
        let ready = !app.sync.config.encrypt || !app.sync.config.passphrase.trim().is_empty();
        if ui.add_enabled(!running && app.sync.config.enabled && ready, egui::Button::new("Sync now")).clicked() {
            app.sync_now();
        }
        if running {
            ui.spinner();
            ui.label("Syncing…");
        }
        ui.label(egui::RichText::new(format!("Last sync: {}", ago(app.sync.state.last_sync))).font(crate::theme::font(11.5)));
    });
    if !app.sync.status.is_empty() {
        ui.label(app.sync.status.clone());
    }
    if !app.sync.conflicts.is_empty() {
        egui::CollapsingHeader::new(format!("{} conflict{}", app.sync.conflicts.len(), if app.sync.conflicts.len() == 1 { "" } else { "s" }))
            .id_salt("sync-conflicts")
            .show(ui, |ui| {
                for conflict in &app.sync.conflicts {
                    ui.label(format!(
                        "{}: kept {} from {}, discarded {}",
                        conflict.category.label(),
                        compact(&conflict.kept),
                        conflict.kept_device,
                        compact(&conflict.lost)
                    ));
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckcraft_engine::Session;

    fn app() -> SlideApp {
        SlideApp::new(Session::with_new(), crate::Services::default())
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("deckcraft-syncui-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn local_values_round_trip_through_apply() {
        let mut source = app();
        source.ui.brightness = crate::theme::Brightness::Dark;
        source.ui.ui_scale = 1.25;
        source.ui.tab = "Insert".into();
        source.ui.notes = false;
        source.ui.zoom = Some(1.5);
        source.ui.recent = vec!["/tmp/talk.deckcraft".into()];
        source.ui.favourite_templates = vec!["Orchard".into()];
        source.ui.qat = vec![crate::qat::QatItem::new("file.save", "Save", crate::icons::Icon::Save)];
        source.session.prefs.author = "Ann".into();
        source.session.prefs.grid_spacing = 9.0;

        let sync = SyncUi {
            config: SyncConfig { categories: Category::ALL.iter().map(|c| c.id().to_string()).collect(), ..Default::default() },
            state: SyncState { device: "dev".into(), ..Default::default() },
            ..Default::default()
        };
        let local = sync.local_snapshot(&source);
        let mut target = app();
        let mut prefs = Prefs::default();
        let changed = apply(&mut target.ui, &mut prefs, &local, &Category::ALL);
        assert!(changed);
        assert_eq!(target.ui.brightness, crate::theme::Brightness::Dark);
        assert_eq!(target.ui.ui_scale, 1.25);
        assert_eq!(target.ui.tab, "Insert");
        assert!(!target.ui.notes);
        assert_eq!(target.ui.zoom, Some(1.5));
        assert_eq!(target.ui.recent, vec!["/tmp/talk.deckcraft".to_string()]);
        assert_eq!(target.ui.favourite_templates, vec!["Orchard".to_string()]);
        assert_eq!(target.ui.qat[0].id, "file.save");
        assert_eq!(prefs.author, "Ann");
        assert_eq!(prefs.grid_spacing, 9.0);
    }

    #[test]
    fn apply_ignores_categories_that_were_not_applied() {
        let source = app();
        let sync = SyncUi { state: SyncState { device: "dev".into(), ..Default::default() }, ..Default::default() };
        let local = sync.local_snapshot(&source);
        let mut target = app();
        target.ui.tab = "Home".into();
        let mut prefs = Prefs::default();
        apply(&mut target.ui, &mut prefs, &local, &[]);
        assert_eq!(target.ui.tab, "Home");
    }

    #[test]
    fn hostile_values_are_sanitized_when_applied() {
        let mut snapshot = snap(&[("layout", json!({"tab": "x".repeat(100), "pane": "y".repeat(100), "zoom": 1e30, "handoutPerPage": 7}))]);
        snapshot.categories.insert("recent".into(), CategoryValue { modified: 1, value: json!(["", "a".repeat(5000), "/ok"]) });
        snapshot.categories.insert("quickAccess".into(), CategoryValue { modified: 1, value: json!([{"id": ""}, {"id": "x".repeat(100)}]) });
        let mut ui = UiState::default();
        let mut prefs = Prefs::default();
        apply(&mut ui, &mut prefs, &snapshot, &[Category::Layout, Category::Recent, Category::QuickAccess]);
        assert_eq!(ui.tab, "Home");
        assert_eq!(ui.pane, None);
        assert_eq!(ui.zoom, Some(*crate::ZOOM_RANGE.end()));
        assert_eq!(ui.handout_per_page, 6);
        assert_eq!(ui.recent, vec!["/ok".to_string()]);
        assert!(ui.qat.is_empty());
    }

    fn snap(cats: &[(&str, Value)]) -> Snapshot {
        Snapshot {
            version: 1,
            device: "dev".into(),
            revision: 1,
            modified: 1,
            categories: cats.iter().map(|(id, v)| ((*id).to_string(), CategoryValue { modified: 1, value: v.clone() })).collect(),
        }
    }

    #[test]
    fn config_and_state_are_loaded_and_saved() {
        let d = dir("persist");
        let mut sync = SyncUi::default();
        sync.ensure_loaded(Some(&d));
        assert_eq!(sync.config, SyncConfig::default());
        assert!(!sync.state.device.is_empty());
        sync.config.enabled = true;
        sync.config.provider = Provider::S3;
        sync.config.s3.bucket = "deck".into();
        sync.state.last_sync = 42;
        sync.persist();
        let mut again = SyncUi::default();
        again.ensure_loaded(Some(&d));
        assert!(again.config.enabled);
        assert_eq!(again.config.provider, Provider::S3);
        assert_eq!(again.config.s3.bucket, "deck");
        assert_eq!(again.state.last_sync, 42);
        assert_eq!(again.state.device, sync.state.device);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_damaged_config_is_reported_not_used() {
        let d = dir("damaged");
        std::fs::write(d.join(CONFIG_FILE), b"{").unwrap();
        let mut sync = SyncUi::default();
        sync.ensure_loaded(Some(&d));
        assert!(!sync.config.enabled);
        assert!(sync.status.contains("could not be read"), "{}", sync.status);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn preferences_show_sync_and_cancel_restores_the_config() {
        use egui_kittest::kittest::Queryable;
        let d = dir("preferences");
        let mut app = app();
        app.settings_dir = Some(d.clone());
        app.sync.ensure_loaded(Some(&d));
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        harness.run_steps(2);
        harness.state_mut().run("app.preferences", json!({})).unwrap();
        harness.run_steps(3);
        assert!(harness.query_by_label_contains("Settings sync").is_some(), "Preferences lists settings sync");
        let enable = harness
            .query_all_by_role_and_label(egui::accesskit::Role::CheckBox, "Sync settings between my computers")
            .next()
            .unwrap_or_else(|| panic!("no sync checkbox"));
        enable.click_accesskit();
        harness.run_steps(2);
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "Done")
            .next()
            .unwrap_or_else(|| panic!("no Done button"))
            .click_accesskit();
        harness.run_steps(2);
        assert!(harness.state().sync.config.enabled, "Done keeps the sync setting");
        assert!(d.join(CONFIG_FILE).is_file(), "Done saves sync.json");

        harness.state_mut().run("app.preferences", json!({})).unwrap();
        harness.run_steps(3);
        let enable = harness
            .query_all_by_role_and_label(egui::accesskit::Role::CheckBox, "Sync settings between my computers")
            .next()
            .unwrap_or_else(|| panic!("no sync checkbox"));
        enable.click_accesskit();
        harness.run_steps(2);
        assert!(!harness.state().sync.config.enabled);
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
            .next()
            .unwrap_or_else(|| panic!("no Cancel button"))
            .click_accesskit();
        harness.run_steps(2);
        assert!(harness.state().sync.config.enabled, "Cancel restores the sync setting");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_folder_sync_runs_on_the_worker_and_applies_the_result() {
        let d = dir("worker");
        let remote = d.join("remote");
        std::fs::create_dir_all(&remote).unwrap();
        let mut app = app();
        app.settings_dir = Some(d.clone());
        app.sync.ensure_loaded(Some(&d));
        app.sync.config.enabled = true;
        app.sync.config.provider = Provider::LocalFolder;
        app.sync.config.local_folder = remote.to_string_lossy().to_string();
        app.sync.config.categories = vec!["appearance".into()];
        app.ui.brightness = crate::theme::Brightness::Dark;
        app.sync_now();
        assert!(app.sync.job.is_some());
        for _ in 0..200 {
            app.sync_tick(1.0);
            if app.sync.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.sync.job.is_none(), "the worker did not finish");
        assert!(app.sync.status.contains("uploaded") || app.sync.status.contains("up to date"), "{}", app.sync.status);
        assert!(remote.join("settings.json").is_file());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_worker_encrypts_the_remote_when_asked() {
        let d = dir("encrypted-worker");
        let remote = d.join("remote");
        std::fs::create_dir_all(&remote).unwrap();
        let mut app = app();
        app.settings_dir = Some(d.clone());
        app.sync.ensure_loaded(Some(&d));
        app.sync.config.enabled = true;
        app.sync.config.provider = Provider::LocalFolder;
        app.sync.config.local_folder = remote.to_string_lossy().to_string();
        app.sync.config.categories = vec!["appearance".into()];
        app.sync.config.encrypt = true;
        app.sync.config.passphrase = "a test passphrase".into();
        app.sync_now();
        for _ in 0..400 {
            app.sync_tick(1.0);
            if app.sync.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.sync.job.is_none(), "the worker did not finish: {}", app.sync.status);
        let bytes = std::fs::read(remote.join("settings.json")).unwrap();
        assert!(sync::is_encrypted(&bytes));
        assert!(sync::parse_with(&bytes, Some("a test passphrase")).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&d);
    }
}
