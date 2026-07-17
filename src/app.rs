use gpui::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Root, Sizable as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    resizable::{h_resizable, resizable_panel},
    tab::{Tab, TabBar},
    v_flex,
};
use std::collections::HashSet;

use uuid::Uuid;

use crate::session::{self, Session, SessionFolder, SessionKind};
use crate::session_dialog;
use crate::settings_view::SettingsView;
use crate::sftp::SftpPanel;
use crate::terminal::{self, TerminalView};

const TERM_ROWS: usize = 32;
const TERM_COLS: usize = 110;

enum TabContent {
    Terminal(Entity<TerminalView>),
    /// An SSH session pairs a terminal with a MobaXterm-style SFTP file
    /// browser docked on the left, each over its own connection.
    SshSession {
        sftp: Entity<SftpPanel>,
        terminal: Entity<TerminalView>,
    },
    Sftp(Entity<SftpPanel>),
    Settings(Entity<SettingsView>),
    Message(SharedString),
}

struct OpenTab {
    session_id: Option<Uuid>,
    title: SharedString,
    icon: IconName,
    content: TabContent,
}

/// Which content the left sidebar panel currently shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarMode {
    Sessions,
    Explorer,
}

/// Root application view: title bar, sessions sidebar, tabbed workspace and status bar.
pub struct OxidalApp {
    sessions: Vec<Session>,
    folders: Vec<SessionFolder>,
    collapsed_folders: HashSet<Uuid>,
    selected_session: Option<Uuid>,
    tabs: Vec<OpenTab>,
    active_tab: Option<usize>,
    sidebar_mode: SidebarMode,
    sidebar_collapsed: bool,
}

impl OxidalApp {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            sessions: session::load_sessions(),
            folders: session::load_folders(),
            collapsed_folders: HashSet::new(),
            selected_session: None,
            tabs: Vec::new(),
            active_tab: None,
            sidebar_mode: SidebarMode::Sessions,
            sidebar_collapsed: false,
        }
    }

    fn set_sidebar_mode(&mut self, mode: SidebarMode, cx: &mut Context<Self>) {
        self.sidebar_mode = mode;
        self.sidebar_collapsed = false;
        cx.notify();
    }

    fn toggle_sidebar_collapsed(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    pub fn add_session(&mut self, new_session: Session, cx: &mut Context<Self>) {
        crate::credentials::store_password(new_session.id, &new_session.password);
        self.sessions.push(new_session);
        session::save_sessions(&self.sessions);
        cx.notify();
    }

    pub fn update_session(&mut self, updated: Session, cx: &mut Context<Self>) {
        if let Some(existing) = self.sessions.iter_mut().find(|s| s.id == updated.id) {
            let mut updated = updated;
            // Not editable in the session dialog; keep the stored value.
            updated.show_hidden_files = existing.show_hidden_files;
            crate::credentials::store_password(updated.id, &updated.password);
            *existing = updated;
            session::save_sessions(&self.sessions);
            cx.notify();
        }
    }

    /// Persist the explorer's "show hidden files" toggle for a session.
    fn set_session_show_hidden(&mut self, id: Uuid, value: bool) {
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
            session.show_hidden_files = value;
            session::save_sessions(&self.sessions);
        }
    }

    fn delete_session(&mut self, id: Uuid, cx: &mut Context<Self>) {
        crate::credentials::delete_password(id);
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

    pub fn rename_folder(&mut self, id: Uuid, name: String, cx: &mut Context<Self>) {
        if let Some(folder) = self.folders.iter_mut().find(|f| f.id == id) {
            folder.name = name;
            session::save_folders(&self.folders);
            cx.notify();
        }
    }

    fn delete_folder(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.folders.retain(|f| f.id != id);
        session::save_folders(&self.folders);
        // Sessions inside the deleted folder move back to the root level
        // rather than being deleted along with it.
        for session in self.sessions.iter_mut() {
            if session.folder_id == Some(id) {
                session.folder_id = None;
            }
        }
        session::save_sessions(&self.sessions);
        self.collapsed_folders.remove(&id);
        cx.notify();
    }

    fn toggle_folder_collapsed(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if !self.collapsed_folders.remove(&id) {
            self.collapsed_folders.insert(id);
        }
        cx.notify();
    }

    fn open_settings_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            icon: IconName::Settings,
            content: TabContent::Settings(view),
        });
        self.active_tab = Some(self.tabs.len() - 1);
        cx.notify();
    }

    fn connect_session(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.sessions.iter().find(|s| s.id == id).cloned() else {
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
                    target.username.clone(),
                    target.password.clone(),
                    target.private_key_path.clone(),
                    TERM_ROWS as u16,
                    TERM_COLS as u16,
                );
                let terminal = cx.new(|cx| {
                    TerminalView::new(backend, TERM_ROWS, TERM_COLS, Some(stats), window, cx)
                });
                let weak_app = cx.entity().downgrade();
                let sftp = cx.new(|cx| {
                    SftpPanel::new(
                        target.host.clone(),
                        target.port,
                        target.username.clone(),
                        target.password.clone(),
                        target.private_key_path.clone(),
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
                    SftpPanel::new(
                        target.host.clone(),
                        target.port,
                        target.username.clone(),
                        target.password.clone(),
                        target.private_key_path.clone(),
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

        self.tabs.push(OpenTab {
            session_id: Some(id),
            title: SharedString::from(target.name.clone()),
            icon: target.kind.icon(),
            content,
        });
        self.active_tab = Some(self.tabs.len() - 1);
        cx.notify();
    }

    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        self.active_tab = match self.active_tab {
            Some(_active) if self.tabs.is_empty() => None,
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

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(IconName::SquareTerminal).small())
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Oxidal")),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new("new-session")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .label("Session")
                            .on_click(cx.listener(|view, _, window, cx| {
                                let folders = view.folders.clone();
                                session_dialog::open_new_session_dialog(folders, window, cx);
                            })),
                    )
                    .child(
                        Button::new("settings")
                            .ghost()
                            .small()
                            .icon(IconName::Settings)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.open_settings_tab(window, cx);
                            })),
                    ),
            )
            .pr_2()
    }

    fn render_session_row(
        &self,
        item: &Session,
        indent: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = item.id;
        let selected = self.selected_session == Some(id);
        let group_name = SharedString::from(format!("session-{id}"));
        let folders = self.folders.clone();
        let session = item.clone();

        h_flex()
            .id(SharedString::from(format!("session-{id}")))
            .group(group_name.clone())
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .mx_1()
            .when(indent, |this| this.pl_6())
            .rounded_md()
            .cursor_pointer()
            .when(selected, |this| {
                this.bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
            })
            .on_click(
                cx.listener(move |view, event: &gpui::ClickEvent, window, cx| {
                    if event.click_count() >= 2 {
                        view.connect_session(id, window, cx);
                    } else {
                        view.selected_session = Some(id);
                        cx.notify();
                    }
                }),
            )
            .child(Icon::new(item.kind.icon()).small())
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().child(SharedString::from(item.name.clone())))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(SharedString::from(item.detail())),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .invisible()
                    .group_hover(group_name, |this| this.visible())
                    .child(
                        Button::new(SharedString::from(format!("edit-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Settings2)
                            .tooltip("Edit")
                            .on_click(cx.listener(move |_view, _, window, cx| {
                                let weak_app = cx.weak_entity();
                                session_dialog::open_edit_session_dialog(
                                    session.clone(),
                                    folders.clone(),
                                    weak_app,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Delete)
                            .tooltip("Delete")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.delete_session(id, cx);
                            })),
                    ),
            )
    }

    fn render_sidebar_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sessions_active = !self.sidebar_collapsed && self.sidebar_mode == SidebarMode::Sessions;
        let explorer_active = !self.sidebar_collapsed && self.sidebar_mode == SidebarMode::Explorer;
        let has_open_session = !self.tabs.is_empty();

        v_flex()
            .w(px(72.))
            .flex_none()
            .h_full()
            .items_center()
            .py_4()
            .gap_2()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                Button::new("sidebar-sessions")
                    .large()
                    .icon(IconName::SquareTerminal)
                    .tooltip("Sessions")
                    .when(sessions_active, |b| b.primary())
                    .when(!sessions_active, |b| b.ghost())
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.set_sidebar_mode(SidebarMode::Sessions, cx);
                    })),
            )
            .when(has_open_session, |this| {
                this.child(
                    Button::new("sidebar-explorer")
                        .large()
                        .icon(IconName::Folder)
                        .tooltip("File Explorer")
                        .when(explorer_active, |b| b.primary())
                        .when(!explorer_active, |b| b.ghost())
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.set_sidebar_mode(SidebarMode::Explorer, cx);
                        })),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("sidebar-collapse")
                    .ghost()
                    .large()
                    .icon(if self.sidebar_collapsed {
                        IconName::PanelLeftOpen
                    } else {
                        IconName::PanelLeftClose
                    })
                    .tooltip(if self.sidebar_collapsed {
                        "Show Sidebar"
                    } else {
                        "Hide Sidebar"
                    })
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.toggle_sidebar_collapsed(cx);
                    })),
            )
    }

    fn render_explorer_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sftp = self
            .active_tab
            .and_then(|index| self.tabs.get(index))
            .and_then(|tab| match &tab.content {
                TabContent::SshSession { sftp, .. } => Some(sftp.clone()),
                TabContent::Sftp(sftp) => Some(sftp.clone()),
                _ => None,
            });

        let content = match sftp {
            Some(sftp) => sftp.into_any_element(),
            None => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .p_4()
                .child(Icon::new(IconName::Folder).with_size(px(32.)))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .text_center()
                        .child("Connect to an SSH or SFTP session to browse its files"),
                )
                .into_any_element(),
        };

        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("File Explorer"),
            )
            .child(div().flex_1().min_h_0().overflow_hidden().child(content))
    }

    fn render_sessions_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<gpui::AnyElement> = Vec::new();

        for folder in self.folders.clone() {
            let folder_id = folder.id;
            let collapsed = self.collapsed_folders.contains(&folder_id);
            let group_name = SharedString::from(format!("folder-{folder_id}"));

            rows.push(
                h_flex()
                    .id(SharedString::from(format!("folder-{folder_id}")))
                    .group(group_name.clone())
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .mx_1()
                    .rounded_md()
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.toggle_folder_collapsed(folder_id, cx);
                    }))
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .xsmall(),
                    )
                    .child(Icon::new(IconName::Folder).small())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(SharedString::from(folder.name.clone())),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .invisible()
                            .group_hover(group_name, |this| this.visible())
                            .child({
                                let folder = folder.clone();
                                Button::new(SharedString::from(format!("edit-folder-{folder_id}")))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Settings2)
                                    .tooltip("Rename")
                                    .on_click(cx.listener(move |_view, _, window, cx| {
                                        let weak_app = cx.weak_entity();
                                        session_dialog::open_edit_folder_dialog(
                                            folder.clone(),
                                            weak_app,
                                            window,
                                            cx,
                                        );
                                    }))
                            })
                            .child(
                                Button::new(SharedString::from(format!(
                                    "delete-folder-{folder_id}"
                                )))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Delete)
                                .tooltip("Delete Folder")
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.delete_folder(folder_id, cx);
                                    },
                                )),
                            ),
                    )
                    .into_any_element(),
            );

            if !collapsed {
                for item in self
                    .sessions
                    .iter()
                    .filter(|s| s.folder_id == Some(folder_id))
                {
                    rows.push(self.render_session_row(item, true, cx).into_any_element());
                }
            }
        }

        for item in self.sessions.iter().filter(|s| s.folder_id.is_none()) {
            rows.push(self.render_session_row(item, false, cx).into_any_element());
        }

        v_flex()
            .w(px(280.))
            .flex_none()
            .h_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Sessions"),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("new-folder")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Folder)
                                    .tooltip("New Folder")
                                    .on_click(cx.listener(|_view, _, window, cx| {
                                        let weak_app = cx.weak_entity();
                                        session_dialog::open_new_folder_dialog(
                                            weak_app, window, cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new("add")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Plus)
                                    .tooltip("New Session")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        let folders = view.folders.clone();
                                        session_dialog::open_new_session_dialog(
                                            folders, window, cx,
                                        );
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .id("sessions-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .py_1()
                    .children(rows),
            )
    }

    fn render_workspace(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.tabs.is_empty() {
            return self.render_welcome(cx).into_any_element();
        }

        let active_index = self.active_tab.unwrap_or(0);
        let tab_bar = TabBar::new("open-tabs")
            .selected_index(active_index)
            .on_click(cx.listener(|view, index: &usize, _, cx| {
                view.active_tab = Some(*index);
                cx.notify();
            }))
            .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                Tab::new()
                    .prefix(Icon::new(tab.icon.clone()).xsmall())
                    .pl_3()
                    .label(tab.title.clone())
                    .suffix(
                        Button::new(SharedString::from(format!("close-tab-{index}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.close_tab(index, cx);
                            })),
                    )
            }))
            .suffix(
                Button::new("new-tab-from-selection")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip("Connect selected session")
                    .on_click(cx.listener(|view, _, window, cx| {
                        if let Some(id) = view.selected_session {
                            view.connect_session(id, window, cx);
                        }
                    })),
            );

        let content = self.tabs.get(active_index).map(|tab| match &tab.content {
            TabContent::Terminal(view) => view.clone().into_any_element(),
            TabContent::SshSession { terminal, .. } => terminal.clone().into_any_element(),
            TabContent::Sftp(view) => view.clone().into_any_element(),
            TabContent::Settings(view) => view.clone().into_any_element(),
            TabContent::Message(msg) => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Icon::new(IconName::TriangleAlert).with_size(px(32.)))
                .child(
                    div()
                        .text_sm()
                        .max_w(px(420.))
                        .text_center()
                        .child(msg.clone()),
                )
                .into_any_element(),
        });

        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(tab_bar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .children(content),
            )
            .into_any_element()
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self
            .selected_session
            .and_then(|id| self.sessions.iter().find(|s| s.id == id));

        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(Icon::new(IconName::SquareTerminal).with_size(px(48.)))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Oxidal Terminal"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(match selected {
                        Some(s) => SharedString::from(format!("Ready to connect: {}", s.detail())),
                        None => {
                            SharedString::from("Select a session on the left, or add a new one")
                        }
                    }),
            )
            .when_some(selected.map(|s| s.id), |this, id| {
                this.child(
                    Button::new("connect")
                        .primary()
                        .icon(IconName::SquareTerminal)
                        .label("Connect")
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.connect_session(id, window, cx);
                        })),
                )
            })
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .items_center()
            .justify_between()
            .h(px(24.))
            .px_3()
            .bg(cx.theme().sidebar)
            .border_t_1()
            .border_color(cx.theme().border)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(format!(
                "{} sessions · {} open",
                self.sessions.len(),
                self.tabs.len()
            ))
            .child(div().child("Oxidal 0.3.0"))
    }
}

impl Render for OxidalApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            // No background here: gpui-component's Root already paints
            // `tokens.background` across the window, and repainting it would
            // stack a second alpha layer in glass mode.
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .child({
                let explorer_open =
                    !self.sidebar_collapsed && self.sidebar_mode == SidebarMode::Explorer;
                let mut content = h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar_rail(cx));
                if explorer_open {
                    // The explorer sits behind a drag handle so the file
                    // list can be widened MobaXterm-style.
                    content = content.child(
                        div().flex_1().min_w_0().h_full().child(
                            h_resizable("explorer-split")
                                .child(
                                    resizable_panel()
                                        .size(px(380.))
                                        .size_range(px(300.)..px(800.))
                                        .child(self.render_explorer_panel(cx).into_any_element()),
                                )
                                .child(self.render_workspace(cx).into_any_element()),
                        ),
                    );
                } else {
                    if !self.sidebar_collapsed {
                        content = content.child(self.render_sessions_panel(cx).into_any_element());
                    }
                    content = content.child(self.render_workspace(cx).into_any_element());
                }
                content
            })
            .child(self.render_status_bar(cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}
