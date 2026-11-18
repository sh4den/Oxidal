use crate::session::SessionKind;
use crate::settings_view::SettingsView;
use crate::sftp::{SftpPanel, SftpWorkspace};
use crate::terminal::{self, TerminalView};

use gpui_kit::{AppContext as _, Context, Entity, SharedString, Window};

use gpui_kit::component::{IconName, IconNamed as _};
use uuid::Uuid;

use super::{OpenTab, OxidalApp, SidebarMode, TabContent};

const TERM_ROWS: usize = 32;
const TERM_COLS: usize = 110;

fn move_index<T>(items: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from == to || from >= items.len() || to >= items.len() {
        return false;
    }
    let item = items.remove(from);
    items.insert(to, item);
    true
}

fn shifted_index(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}

impl OxidalApp {
    pub(super) fn open_settings_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|t| matches!(t.content, TabContent::Settings(_)))
        {
            self.active_tab = Some(index);
            cx.notify();
            return;
        }

        let view = cx.new(|cx| SettingsView::new(window, cx));
        self.tabs.push(OpenTab {
            session_id: None,
            title: SharedString::from("Settings"),
            icon: IconName::Settings.path(),
            icon_color: None,
            content: TabContent::Settings(view),
        });
        self.active_tab = Some(self.tabs.len() - 1);
        cx.notify();
    }

    pub(super) fn connect_session(
        &mut self,
        id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.session_with_secrets(id) else {
            return;
        };

        let content = match target.kind {
            SessionKind::Local => {
                match terminal::local::spawn(TERM_ROWS as u16, TERM_COLS as u16) {
                    Ok(backend) => TabContent::Terminal(cx.new(|cx| {
                        TerminalView::new(backend, TERM_ROWS, TERM_COLS, None, window, cx)
                    })),
                    Err(err) => {
                        TabContent::Message(format!("Failed to start local shell: {err}").into())
                    }
                }
            }
            SessionKind::Ssh => {
                let (backend, stats) = terminal::ssh::spawn(
                    target.host.clone(),
                    target.port,
                    target.credentials(),
                    target.proxy(),
                    TERM_ROWS as u16,
                    TERM_COLS as u16,
                    target.monitoring,
                );
                let terminal = cx
                    .new(|cx| TerminalView::new(backend, TERM_ROWS, TERM_COLS, stats, window, cx));
                let weak_app = cx.entity().downgrade();
                let sftp = cx.new(|cx| {
                    SftpPanel::new(
                        target.host.clone(),
                        target.port,
                        target.credentials(),
                        target.proxy(),
                        target.show_hidden_files,
                        move |value, cx| {
                            let _ = weak_app
                                .update(cx, |app, _| app.set_session_show_hidden(id, value));
                        },
                        window,
                        cx,
                    )
                });
                TabContent::SshSession { sftp, terminal }
            }
            SessionKind::Telnet => {
                let backend = terminal::telnet::spawn(
                    target.host.clone(),
                    target.port,
                    target.proxy(),
                    TERM_ROWS as u16,
                    TERM_COLS as u16,
                );
                let terminal =
                    cx.new(|cx| TerminalView::new(backend, TERM_ROWS, TERM_COLS, None, window, cx));
                TabContent::Terminal(terminal)
            }
            SessionKind::Serial => {
                match terminal::serial::spawn(target.host.clone(), target.baud_rate) {
                    Ok(backend) => TabContent::Terminal(cx.new(|cx| {
                        TerminalView::new(backend, TERM_ROWS, TERM_COLS, None, window, cx)
                    })),
                    Err(err) => {
                        TabContent::Message(format!("Failed to open serial port: {err}").into())
                    }
                }
            }
            SessionKind::Sftp => {
                let weak_app = cx.entity().downgrade();
                TabContent::Sftp(cx.new(|cx| {
                    SftpWorkspace::new(
                        target.host.clone(),
                        target.port,
                        target.credentials(),
                        target.proxy(),
                        target.show_hidden_files,
                        move |value, cx| {
                            let _ = weak_app
                                .update(cx, |app, _| app.set_session_show_hidden(id, value));
                        },
                        window,
                        cx,
                    )
                }))
            }
            SessionKind::Ftp => {
                let weak_app = cx.entity().downgrade();
                TabContent::Sftp(cx.new(|cx| {
                    SftpWorkspace::ftp(
                        target.ftp_target(),
                        target.show_hidden_files,
                        move |value, cx| {
                            let _ = weak_app
                                .update(cx, |app, _| app.set_session_show_hidden(id, value));
                        },
                        window,
                        cx,
                    )
                }))
            }
            SessionKind::Rdp => TabContent::Message(
                "RDP isn't implemented yet — only terminal sessions work so far.".into(),
            ),
        };

        let has_explorer = matches!(content, TabContent::SshSession { .. });
        self.tabs.push(OpenTab {
            session_id: Some(id),
            title: SharedString::from(target.name.clone()),
            icon: target.display_icon(),
            icon_color: target.color.hsla(),
            content,
        });
        self.active_tab = Some(self.tabs.len() - 1);
        if has_explorer {
            self.sidebar_mode = SidebarMode::Explorer;
            self.sidebar_collapsed = false;
        }
        cx.notify();
    }

    pub(super) fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        self.active_tab = match self.active_tab {
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(index.min(self.tabs.len().saturating_sub(1))),
            other => other,
        };
        if self.tabs.is_empty() {
            self.active_tab = None;
            if self.sidebar_mode == SidebarMode::Explorer {
                self.sidebar_mode = SidebarMode::Sessions;
            }
        }
        cx.notify();
    }

    pub(super) fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if !move_index(&mut self.tabs, from, to) {
            return;
        }
        self.active_tab = self
            .active_tab
            .map(|active| shifted_index(active, from, to));
        cx.notify();
    }

    pub(super) fn active_sftp_panel(&self) -> Option<Entity<SftpPanel>> {
        self.active_tab
            .and_then(|index| self.tabs.get(index))
            .and_then(|tab| match &tab.content {
                TabContent::SshSession { sftp, .. } => Some(sftp.clone()),
                _ => None,
            })
    }
}

#[cfg(test)]
#[path = "../../tests/app/tabs.rs"]
mod tests;
