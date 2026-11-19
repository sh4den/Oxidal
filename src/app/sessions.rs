use crate::session::{self, Session, SessionFolder};
use crate::session_dialog;
use secrecy::{ExposeSecret as _, SecretString};

use gpui_kit::Context;

use uuid::Uuid;

use super::drag::{DropTarget, reorder_folders, reorder_sessions};
use super::{OxidalApp, SidebarMode};

fn store_changed_secrets(previous: Option<&Session>, session: &Session) {
    let changed = |old: Option<&SecretString>, new: &SecretString| match old {
        Some(old) => old.expose_secret() != new.expose_secret(),
        None => !new.expose_secret().is_empty(),
    };
    if changed(previous.map(|p| &p.password), &session.password) {
        crate::credentials::store_password(session.id, &session.password);
    }
    if changed(previous.map(|p| &p.key_passphrase), &session.key_passphrase) {
        crate::credentials::store_key_passphrase(session.id, &session.key_passphrase);
    }
    if changed(previous.map(|p| &p.proxy_password), &session.proxy_password) {
        crate::credentials::store_proxy_password(session.id, &session.proxy_password);
    }
}

pub(super) fn matches_filter(session: &Session, query: &str) -> bool {
    query.is_empty()
        || [
            session.name.as_str(),
            session.host.as_str(),
            session.username.as_str(),
            session.kind.label(),
        ]
        .iter()
        .any(|field| field.to_lowercase().contains(query))
}

impl OxidalApp {
    pub fn folders(&self) -> &[SessionFolder] {
        &self.folders
    }

    pub(super) fn open_session_window(
        &mut self,
        existing: Option<Session>,
        cx: &mut Context<Self>,
    ) {
        let key = existing.as_ref().map(|s| s.id);
        if let Some(&handle) = self.session_windows.get(&key)
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let weak_app = cx.entity().downgrade();
        let folders = self.folders.clone();
        if let Some(handle) = session_dialog::open_session_window(existing, folders, weak_app, cx) {
            self.session_windows.insert(key, handle);
        }
    }

    pub fn add_session(&mut self, new_session: Session, cx: &mut Context<Self>) {
        store_changed_secrets(None, &new_session);
        self.sessions.push(new_session);
        session::save_sessions(&self.sessions);
        cx.notify();
    }

    pub fn update_session(&mut self, updated: Session, cx: &mut Context<Self>) {
        if let Some(existing) = self.sessions.iter_mut().find(|s| s.id == updated.id) {
            let mut updated = updated;
            updated.show_hidden_files = existing.show_hidden_files;
            store_changed_secrets(existing.secrets_loaded().then_some(&*existing), &updated);
            *existing = updated;
            session::save_sessions(&self.sessions);
            cx.notify();
        }
    }

    pub(super) fn set_session_show_hidden(&mut self, id: Uuid, value: bool) {
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
            session.show_hidden_files = value;
            session::save_sessions(&self.sessions);
        }
    }

    pub(super) fn delete_session(&mut self, id: Uuid, cx: &mut Context<Self>) {
        crate::credentials::delete_password(id);
        if let Some(handle) = self.session_windows.remove(&Some(id)) {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        self.sessions.retain(|s| s.id != id);
        session::save_sessions(&self.sessions);
        if self.selected_session == Some(id) {
            self.selected_session = None;
        }
        let tab_count_before = self.tabs.len();
        self.tabs.retain(|t| t.session_id != Some(id));
        if self.tabs.len() != tab_count_before {
            self.active_tab = if self.tabs.is_empty() { None } else { Some(0) };
            if self.tabs.is_empty() && self.sidebar_mode == SidebarMode::Explorer {
                self.sidebar_mode = SidebarMode::Sessions;
            }
        }
        cx.notify();
    }

    pub fn add_folder(&mut self, folder: SessionFolder, cx: &mut Context<Self>) {
        self.folders.push(folder);
        session::save_folders(&self.folders);
        cx.notify();
    }

    pub fn update_folder(&mut self, updated: SessionFolder, cx: &mut Context<Self>) {
        if let Some(folder) = self.folders.iter_mut().find(|f| f.id == updated.id) {
            *folder = updated;
            session::save_folders(&self.folders);
            cx.notify();
        }
    }

    pub(super) fn delete_folder(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.folders.retain(|f| f.id != id);
        session::save_folders(&self.folders);
        for session in self.sessions.iter_mut() {
            if session.folder_id == Some(id) {
                session.folder_id = None;
            }
        }
        session::save_sessions(&self.sessions);
        self.collapsed_folders.remove(&id);
        cx.notify();
    }

    pub(super) fn toggle_folder_collapsed(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if !self.collapsed_folders.remove(&id) {
            self.collapsed_folders.insert(id);
        }
        cx.notify();
    }

    pub(super) fn move_session(&mut self, id: Uuid, target: DropTarget, cx: &mut Context<Self>) {
        if !reorder_sessions(&mut self.sessions, id, target) {
            return;
        }
        if let DropTarget::Into(Some(folder_id)) = target {
            self.collapsed_folders.remove(&folder_id);
        }
        session::save_sessions(&self.sessions);
        cx.notify();
    }

    pub(super) fn move_folder(&mut self, id: Uuid, before: Option<Uuid>, cx: &mut Context<Self>) {
        if !reorder_folders(&mut self.folders, id, before) {
            return;
        }
        session::save_folders(&self.folders);
        cx.notify();
    }

    pub(super) fn filter_query(&self, cx: &gpui_kit::App) -> String {
        self.filter.read(cx).value().trim().to_lowercase()
    }

    pub(super) fn is_open(&self, id: Uuid) -> bool {
        self.tabs.iter().any(|tab| tab.session_id == Some(id))
    }

    pub(super) fn session_with_secrets(&mut self, id: Uuid) -> Option<Session> {
        let session = self.sessions.iter_mut().find(|s| s.id == id)?;
        session.ensure_secrets();
        Some(session.clone())
    }
}

#[cfg(test)]
#[path = "../../tests/app/sessions.rs"]
mod tests;
