//! File ▸ Share: links, permissions and the People list.
//!
//! Every action goes through `share.*` commands, so the CLI, MCP and the control channel manage
//! the same sidecar. The page reloads `share.info` after a change; a failed action leaves the
//! list as it was and shows the reason.

use deckcraft_engine::collab::Role;
use deckcraft_engine::sharing::{Permission, now_secs};
use egui::{Ui, vec2};
use serde_json::{Value, json};

use crate::SlideApp;

#[derive(Clone, Debug)]
pub struct State {
    /// The last `share.info` result; `None` reloads on the next frame.
    pub info: Option<Value>,
    pub error: Option<String>,
    pub status: String,
    pub permission: Permission,
    /// 0 means never; otherwise hours.
    pub expiry_hours: u32,
    pub name: String,
    /// Live session fields: the journal folder, the name shown to other peers, and the role.
    pub collab_folder: String,
    pub collab_name: String,
    pub collab_role: Role,
}

impl Default for State {
    fn default() -> Self {
        State {
            info: None,
            error: None,
            status: String::new(),
            permission: Permission::Editor,
            expiry_hours: 0,
            name: String::new(),
            collab_folder: String::new(),
            collab_name: String::new(),
            collab_role: Role::Editor,
        }
    }
}

fn expiry_label(hours: u32) -> &'static str {
    match hours {
        0 => "Never expires",
        24 => "Expires in 24 hours",
        168 => "Expires in 7 days",
        720 => "Expires in 30 days",
        _ => "Expires later",
    }
}

fn expiry_text(expires: Option<i64>, revoked: bool, valid: bool) -> String {
    if revoked {
        return "Revoked".into();
    }
    let Some(at) = expires else { return "Never expires".into() };
    let left = at.saturating_sub(now_secs());
    if left <= 0 || !valid {
        return "Expired".into();
    }
    if left < 3600 {
        format!("Expires in {} min", (left / 60).max(1))
    } else if left < 86_400 {
        format!("Expires in {} h", left / 3600)
    } else {
        format!("Expires in {} days", left / 86_400)
    }
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn bool(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn permission_box(ui: &mut Ui, id: &str, permission: &mut Permission) {
    egui::ComboBox::from_id_salt(id).selected_text(permission.label()).width(130.0).show_ui(ui, |ui| {
        for p in Permission::ALL {
            ui.selectable_value(permission, p, p.label());
        }
    });
}

fn expiry_box(ui: &mut Ui, id: &str, hours: &mut u32) {
    egui::ComboBox::from_id_salt(id).selected_text(expiry_label(*hours)).width(170.0).show_ui(ui, |ui| {
        for value in [0u32, 24, 168, 720] {
            ui.selectable_value(hours, value, expiry_label(value));
        }
    });
}

fn run(app: &mut SlideApp, state: &mut State, command: &str, params: Value) -> bool {
    match app.run(command, params) {
        Ok(_) => {
            state.info = None;
            true
        }
        Err(e) => {
            state.status = e.to_string();
            false
        }
    }
}

/// The Share page body. `state` lives in the File tab so the fields survive redraws.
pub fn page(app: &mut SlideApp, ui: &mut Ui, state: &mut State) {
    if state.info.is_none() && state.error.is_none() {
        match app.run("share.info", json!({})) {
            Ok(v) => state.info = Some(v),
            Err(e) => state.error = Some(e.to_string()),
        }
    }
    if let Some(error) = state.error.clone() {
        ui.label(error);
        ui.add_space(8.0);
        if ui.button("Try again").clicked() {
            state.error = None;
            state.info = None;
        }
        return;
    }
    let Some(info) = state.info.clone() else {
        ui.spinner();
        return;
    };

    let access = app.session.active().map(|d| d.access).unwrap_or_default();
    if access != deckcraft_engine::sharing::Access::Full {
        ui.label(egui::RichText::new(format!("This presentation is open for {}.", access.label().to_lowercase())).font(crate::theme::bold(13.0)));
        if ui.button("Save a Copy to edit…").clicked() {
            app.save_as_dialog("deckcraft");
        }
        ui.add_space(12.0);
    }

    ui.label(egui::RichText::new("Share a link").font(crate::theme::bold(14.0)));
    ui.label(
        egui::RichText::new(
            "Anyone with the link gets the access below until it expires or you revoke it. Links live beside the presentation, so a shared folder is enough.",
        )
        .font(crate::theme::font(12.0)),
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Access:");
        permission_box(ui, "share-new-permission", &mut state.permission);
        expiry_box(ui, "share-new-expiry", &mut state.expiry_hours);
        if ui.button("Create link").clicked() {
            let params = json!({"permission": state.permission.id(), "expiresHours": state.expiry_hours});
            if run(app, state, "share.link", params) {
                state.status = "Share link created.".into();
            }
        }
    });
    if !state.status.is_empty() {
        ui.label(egui::RichText::new(&state.status).font(crate::theme::font(12.0)));
    }

    ui.add_space(12.0);
    ui.label(egui::RichText::new("Links").font(crate::theme::bold(14.0)));
    let links = info.get("links").and_then(Value::as_array).cloned().unwrap_or_default();
    if links.is_empty() {
        ui.label(egui::RichText::new("No links yet.").font(crate::theme::font(12.0)));
    }
    for link in &links {
        let token = text(link, "token").to_string();
        let url = text(link, "url").to_string();
        let valid = bool(link, "valid");
        let revoked = bool(link, "revoked");
        let expires = link.get("expires").and_then(Value::as_i64);
        ui.horizontal(|ui| {
            ui.add_sized(vec2(110.0, 20.0), egui::Label::new(text(link, "permissionLabel")));
            let label = text(link, "label");
            ui.add_sized(vec2(140.0, 20.0), egui::Label::new(if label.is_empty() { "Anyone with the link" } else { label }));
            ui.label(egui::RichText::new(expiry_text(expires, revoked, valid)).font(crate::theme::font(12.0)));
            if ui.add_enabled(valid, egui::Button::new("Copy link")).clicked() {
                ui.ctx().copy_text(url.clone());
                state.status = "Link copied to the clipboard.".into();
            }
            if ui.add_enabled(!revoked, egui::Button::new("Revoke")).clicked() && run(app, state, "share.revoke", json!({"token": token})) {
                state.status = "Link revoked.".into();
            }
        });
    }

    ui.add_space(14.0);
    ui.label(egui::RichText::new("People").font(crate::theme::bold(14.0)));
    ui.horizontal(|ui| {
        ui.add_sized(vec2(90.0, 20.0), egui::Label::new("Invite:"));
        ui.add(egui::TextEdit::singleline(&mut state.name).hint_text("Name or email").desired_width(220.0));
        permission_box(ui, "share-invite-permission", &mut state.permission);
        if ui.add_enabled(!state.name.trim().is_empty(), egui::Button::new("Invite")).clicked() {
            let name = state.name.trim().to_string();
            let params = json!({"name": name, "permission": state.permission.id(), "expiresHours": state.expiry_hours});
            if run(app, state, "share.invite", params) {
                state.name.clear();
                state.status = "Invitation created: copy its link from the list.".into();
            }
        }
    });
    let people = info.get("participants").and_then(Value::as_array).cloned().unwrap_or_default();
    if people.is_empty() {
        ui.label(egui::RichText::new("Nobody has been invited yet.").font(crate::theme::font(12.0)));
    }
    for person in &people {
        let name = text(person, "name").to_string();
        let token = text(person, "token").to_string();
        let url = text(person, "url").to_string();
        let valid = bool(person, "valid");
        let revoked = bool(person, "revoked");
        let expires = person.get("expires").and_then(Value::as_i64);
        ui.horizontal(|ui| {
            ui.add_sized(vec2(160.0, 20.0), egui::Label::new(&name));
            ui.add_sized(vec2(110.0, 20.0), egui::Label::new(text(person, "permissionLabel")));
            ui.label(egui::RichText::new(expiry_text(expires, revoked, valid)).font(crate::theme::font(12.0)));
            if ui.add_enabled(valid, egui::Button::new("Copy invite")).clicked() {
                ui.ctx().copy_text(url.clone());
                state.status = "Invitation link copied.".into();
            }
            if ui.add_enabled(!revoked, egui::Button::new("Revoke")).clicked() && run(app, state, "share.revoke", json!({"token": token})) {
                state.status = "Invitation revoked.".into();
            }
            if ui.button("Remove").clicked() && run(app, state, "share.remove", json!({"name": name})) {
                state.status = "Removed from the People list.".into();
            }
        });
    }

    ui.add_space(18.0);
    collab_section(app, ui, state);
}

/// Live collaboration: a journal folder shared by every editor. Starting writes a snapshot; joining
/// adopts it and tails the journals. Presence and cursors come from the same folder, so this works
/// on a network share or a synced folder with no server.
fn collab_section(app: &mut SlideApp, ui: &mut Ui, state: &mut State) {
    ui.separator();
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Live session").font(crate::theme::bold(14.0)));
    ui.label(
        egui::RichText::new(
            "Share a folder with the other editors. Commands, presence and pointers travel through it; the presentation keeps working offline and catches up when the folder is reachable.",
        )
        .font(crate::theme::font(12.0)),
    );
    if state.collab_folder.is_empty()
        && let Some(path) = app.session.active().and_then(|d| d.path.clone())
    {
        state.collab_folder = deckcraft_engine::collab::default_dir(&path).to_string_lossy().to_string();
    }
    if state.collab_name.is_empty() {
        state.collab_name = if app.session.prefs.author.trim().is_empty() { "Presenter".into() } else { app.session.prefs.author.trim().into() };
    }
    let status = app.session.collab_status();
    if let Some(status) = status {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Live:").font(crate::theme::bold(12.5)));
            ui.label(egui::RichText::new(&status.folder).font(crate::theme::font(12.0)));
            ui.label(egui::RichText::new(format!("revision {}", status.revision)).font(crate::theme::font(11.5)));
            if ui.button("Copy folder").clicked() {
                ui.ctx().copy_text(status.folder.clone());
                state.status = "Live folder copied to the clipboard.".into();
            }
            if ui.button("Leave").clicked() && run(app, state, "collab.leave", json!({})) {
                state.status = "Left the live session.".into();
            }
        });
        for peer in &status.peers {
            let color = egui::Color32::from_rgb(peer.color[0], peer.color[1], peer.color[2]);
            ui.horizontal(|ui| {
                ui.colored_label(color, "●");
                let you = if peer.id == status.peer { " (you)" } else { "" };
                ui.label(format!("{}{}", peer.name, you));
                ui.label(egui::RichText::new(peer.role.label()).font(crate::theme::font(11.5)));
                if let (Some(slide), Some(x), Some(y)) = (peer.slide, peer.x, peer.y) {
                    ui.label(
                        egui::RichText::new(format!("slide {}, {:.0}%, {:.0}%", slide + 1, x * 100.0, y * 100.0)).font(crate::theme::font(11.0)),
                    );
                }
            });
        }
        if let Some(error) = &status.error {
            ui.label(egui::RichText::new(error).color(crate::theme::Tokens::get(ui.ctx()).accent));
        }
    } else {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_sized(vec2(90.0, 20.0), egui::Label::new("Folder:"));
            ui.add(egui::TextEdit::singleline(&mut state.collab_folder).hint_text("A shared folder").desired_width(360.0));
        });
        ui.horizontal(|ui| {
            ui.add_sized(vec2(90.0, 20.0), egui::Label::new("Your name:"));
            ui.add(egui::TextEdit::singleline(&mut state.collab_name).desired_width(180.0));
            ui.label("Role:");
            egui::ComboBox::from_id_salt("collab-role").selected_text(state.collab_role.label()).width(130.0).show_ui(ui, |ui| {
                for role in Role::ALL {
                    ui.selectable_value(&mut state.collab_role, role, role.label());
                }
            });
        });
        ui.horizontal(|ui| {
            let params = json!({
                "folder": state.collab_folder.trim(),
                "name": state.collab_name.trim(),
                "role": state.collab_role.id(),
            });
            if ui.add_enabled(!state.collab_folder.trim().is_empty(), egui::Button::new("Start new session")).clicked()
                && run(app, state, "collab.start", params.clone())
            {
                state.status = "Live session started. Share the folder with the other editors.".into();
            }
            if ui.add_enabled(!state.collab_folder.trim().is_empty(), egui::Button::new("Join session")).clicked()
                && run(app, state, "collab.join", params)
            {
                state.status = "Joined the live session.".into();
            }
        });
        ui.label(
            egui::RichText::new(
                "Start on the first copy; Join opens the latest snapshot and applies the journal. A socket relay is the next step for teams that cannot share a folder.",
            )
            .font(crate::theme::font(11.5)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn the_share_page_creates_and_revokes_a_link() {
        let dir = std::env::temp_dir().join(format!("deckcraft-share-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("talk.deckcraft");
        let mut app = SlideApp::new(deckcraft_engine::Session::with_new(), crate::Services::default());
        app.run("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        h.run_steps(3);
        h.state_mut().run("app.backstage", json!({"page": "share"})).unwrap();
        h.run_steps(3);
        assert!(h.query_all_by_label_contains("Share a link").next().is_some(), "the Share page draws its body");
        h.query_all_by_role_and_label(egui::accesskit::Role::Button, "Create link")
            .next()
            .unwrap_or_else(|| panic!("no Create link button"))
            .click_accesskit();
        h.run_steps(3);
        assert!(h.query_all_by_label_contains("Anyone with the link").next().is_some(), "the new link is listed");
        let sidecar = deckcraft_engine::sharing::sidecar_path(&path.to_string_lossy()).unwrap();
        assert!(sidecar.is_file(), "the link is saved beside the deck");
        h.query_all_by_role_and_label(egui::accesskit::Role::Button, "Revoke").next().unwrap_or_else(|| panic!("no Revoke button")).click_accesskit();
        h.run_steps(3);
        assert!(h.query_all_by_label_contains("Revoked").next().is_some(), "the revoked link is marked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_share_page_starts_and_leaves_a_live_session() {
        let dir = std::env::temp_dir().join(format!("deckcraft-collab-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("talk.deckcraft");
        let mut app = SlideApp::new(deckcraft_engine::Session::with_new(), crate::Services::default());
        app.run("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        h.run_steps(3);
        h.state_mut().run("app.backstage", json!({"page": "share"})).unwrap();
        h.run_steps(3);
        let folder = dir.join("live");
        h.state_mut().run("collab.start", json!({"folder": folder.to_string_lossy(), "name": "Host"})).unwrap();
        h.run_steps(3);
        assert!(h.query_all_by_label_contains("Live:").next().is_some(), "the live status is shown");
        assert!(h.query_all_by_label_contains("Host (you)").next().is_some(), "the peer is listed");
        h.query_all_by_role_and_label(egui::accesskit::Role::Button, "Leave").next().unwrap_or_else(|| panic!("no Leave button")).click_accesskit();
        h.run_steps(3);
        assert!(h.state().session.collab.is_none(), "Leave ends the session");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
