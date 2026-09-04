use crate::session::{self, Session, SessionFolder, SessionKind};
use crate::session_dialog;
use crate::settings::{self, AppSettings};
use crate::settings_view::SettingsView;
use crate::sftp::{SftpPanel, SftpWorkspace};
use crate::terminal::{self, TerminalView};
use gpui::{
    AppContext as _, Context, Div, ElementId, Entity, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, WindowHandle, div,
    prelude::FluentBuilder as _, px,
};

use gpui_component::button::Button;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, IconNamed as _, Root, Sizable as _, TitleBar, WindowExt as _,
    button::ButtonVariants as _,
    dialog::DialogFooter,
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    resizable::{ResizableState, h_resizable, resizable_panel},
    scroll::ScrollableElement as _,
    tab::{Tab, TabBar},
    tooltip::Tooltip,
    v_flex,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use uuid::Uuid;

const TERM_ROWS: usize = 32;
const TERM_COLS: usize = 110;

const FOLDER_GUIDE_INDENT: gpui::Pixels = px(18.);

const CLIPBOARD_MODIFIER: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl"
};

const WORD_MOVE_MODIFIER: &str = if cfg!(target_os = "macos") {
    "⌥"
} else {
    "Ctrl"
};

// Row chrome around a label: margins, padding, icon, gaps and the hover buttons.
const ROW_CHROME_WIDTH: f32 = 132.;
const APPROX_CHAR_WIDTH: f32 = 7.;

fn label_capacity(sidebar_width: gpui::Pixels) -> usize {
    let text_width = f32::from(sidebar_width) - ROW_CHROME_WIDTH;
    ((text_width / APPROX_CHAR_WIDTH).floor() as usize).max(6)
}

fn truncating_label(
    id: impl Into<ElementId>,
    text: SharedString,
    capacity: usize,
) -> Stateful<Div> {
    div()
        .id(id)
        .truncate()
        .when(text.chars().count() > capacity, {
            let full = text.clone();
            move |this| this.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
        })
        .child(text)
}

enum TabContent {
    Terminal(Entity<TerminalView>),
    SshSession {
        sftp: Entity<SftpPanel>,
        terminal: Entity<TerminalView>,
    },
    Sftp(Entity<SftpWorkspace>),
    Settings(Entity<SettingsView>),
    Message(SharedString),
}

struct OpenTab {
    session_id: Option<Uuid>,
    title: SharedString,
    icon: SharedString,
    icon_color: Option<Hsla>,
    content: TabContent,
}

#[derive(Clone)]
struct DragPreview {
    icon: SharedString,
    color: Option<Hsla>,
    label: SharedString,
}

#[derive(Clone)]
struct SessionDrag {
    id: Uuid,
    preview: DragPreview,
}

#[derive(Clone)]
struct FolderDrag {
    id: Uuid,
    preview: DragPreview,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().primary)
            .bg(cx.theme().popover)
            .shadow_md()
            .text_xs()
            .child(
                Icon::empty()
                    .path(self.icon.clone())
                    .xsmall()
                    .when_some(self.color, |this, color| this.text_color(color)),
            )
            .child(self.label.clone())
    }
}

#[derive(Clone, Copy)]
enum DropTarget {
    Before(Uuid),
    Into(Option<Uuid>),
}

fn reorder_sessions(sessions: &mut Vec<Session>, id: Uuid, target: DropTarget) -> bool {
    let Some(index) = sessions.iter().position(|s| s.id == id) else {
        return false;
    };
    let mut session = sessions.remove(index);
    match target {
        DropTarget::Before(target_id) => {
            let Some(at) = sessions.iter().position(|s| s.id == target_id) else {
                sessions.insert(index, session);
                return false;
            };
            session.folder_id = sessions[at].folder_id;
            sessions.insert(at, session);
        }
        DropTarget::Into(folder_id) => {
            session.folder_id = folder_id;
            sessions.push(session);
        }
    }
    true
}

fn reorder_folders(folders: &mut Vec<SessionFolder>, id: Uuid, before: Option<Uuid>) -> bool {
    let Some(index) = folders.iter().position(|f| f.id == id) else {
        return false;
    };
    let folder = folders.remove(index);
    let at = match before {
        None => folders.len(),
        Some(target) => match folders.iter().position(|f| f.id == target) {
            Some(at) => at,
            None => {
                folders.insert(index, folder);
                return false;
            }
        },
    };
    folders.insert(at, folder);
    true
}

fn matches_filter(session: &Session, query: &str) -> bool {
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarMode {
    Sessions,
    Explorer,
}

enum UpdateState {
    Idle,
    Available(crate::update::AvailableUpdate),
    Downloading(crate::update::AvailableUpdate),
    Ready(PathBuf),
}

pub struct OxidalApp {
    sessions: Vec<Session>,
    folders: Vec<SessionFolder>,
    collapsed_folders: HashSet<Uuid>,
    selected_session: Option<Uuid>,
    tabs: Vec<OpenTab>,
    active_tab: Option<usize>,
    sidebar_mode: SidebarMode,
    sidebar_collapsed: bool,
    sidebar_state: Entity<ResizableState>,
    filter: Entity<InputState>,
    update_state: UpdateState,
    session_windows: HashMap<Option<Uuid>, WindowHandle<Root>>,
}

impl OxidalApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let window_handle = window.window_handle();
        cx.spawn(async move |_this, cx| {
            let requests = crate::host_keys::requests();
            while let Ok(request) = requests.recv().await {
                let opened = cx.update_window(window_handle, |_, window, cx| {
                    window.activate_window();
                    crate::host_keys::open_prompt(request, window, cx);
                });
                if opened.is_err() {
                    break;
                }
            }
        })
        .detach();

        let updates = crate::update::check();
        cx.spawn(async move |this, cx| {
            if let Ok(found) = updates.recv().await {
                let _ = this.update(cx, |app, cx| {
                    app.update_state = UpdateState::Available(found);
                    cx.notify();
                });
            }
        })
        .detach();

        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter sessions"));
        cx.subscribe(&filter, |_: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();

        Self {
            sessions: session::load_sessions(),
            folders: session::load_folders(),
            collapsed_folders: HashSet::new(),
            selected_session: None,
            tabs: Vec::new(),
            active_tab: None,
            sidebar_mode: SidebarMode::Sessions,
            sidebar_collapsed: false,
            sidebar_state: cx.new(|_| ResizableState::default()),
            filter,
            update_state: UpdateState::Idle,
            session_windows: HashMap::new(),
        }
    }

    pub fn folders(&self) -> &[SessionFolder] {
        &self.folders
    }

    fn open_session_window(&mut self, existing: Option<Session>, cx: &mut Context<Self>) {
        let key = existing.as_ref().map(|s| s.id);
        if let Some(&handle) = self.session_windows.get(&key)
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let weak_app = cx.entity().downgrade();
        if let Some(handle) = session_dialog::open_session_window(existing, weak_app, cx) {
            self.session_windows.insert(key, handle);
        }
    }

    fn start_update_download(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Available(found) = &self.update_state else {
            return;
        };
        let found = found.clone();
        let download = crate::update::download(found.clone());
        self.update_state = UpdateState::Downloading(found);
        cx.notify();
        cx.spawn(async move |this, cx| {
            if let Ok(result) = download.recv().await {
                let _ = this.update(cx, |app, cx| {
                    let previous = std::mem::replace(&mut app.update_state, UpdateState::Idle);
                    app.update_state = match (result, previous) {
                        (Ok(path), _) => UpdateState::Ready(path),
                        (Err(_), UpdateState::Downloading(found)) => UpdateState::Available(found),
                        (Err(_), other) => other,
                    };
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn restart_to_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let UpdateState::Ready(path) = &self.update_state else {
            return;
        };
        match crate::update::apply_and_restart(path) {
            Ok(()) => cx.quit(),
            Err(e) => {
                window.push_notification(Notification::error(format!("Update failed: {e}")), cx);
            }
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
        crate::credentials::store_key_passphrase(new_session.id, &new_session.key_passphrase);
        crate::credentials::store_proxy_password(new_session.id, &new_session.proxy_password);
        self.sessions.push(new_session);
        session::save_sessions(&self.sessions);
        cx.notify();
    }

    pub fn update_session(&mut self, updated: Session, cx: &mut Context<Self>) {
        if let Some(existing) = self.sessions.iter_mut().find(|s| s.id == updated.id) {
            let mut updated = updated;
            updated.show_hidden_files = existing.show_hidden_files;
            crate::credentials::store_password(updated.id, &updated.password);
            crate::credentials::store_key_passphrase(updated.id, &updated.key_passphrase);
            crate::credentials::store_proxy_password(updated.id, &updated.proxy_password);
            *existing = updated;
            session::save_sessions(&self.sessions);
            cx.notify();
        }
    }

    fn set_session_show_hidden(&mut self, id: Uuid, value: bool) {
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
            session.show_hidden_files = value;
            session::save_sessions(&self.sessions);
        }
    }

    fn delete_session(&mut self, id: Uuid, cx: &mut Context<Self>) {
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

    fn delete_folder(&mut self, id: Uuid, cx: &mut Context<Self>) {
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

    fn toggle_folder_collapsed(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if !self.collapsed_folders.remove(&id) {
            self.collapsed_folders.insert(id);
        }
        cx.notify();
    }

    fn move_session(&mut self, id: Uuid, target: DropTarget, cx: &mut Context<Self>) {
        if !reorder_sessions(&mut self.sessions, id, target) {
            return;
        }
        if let DropTarget::Into(Some(folder_id)) = target {
            self.collapsed_folders.remove(&folder_id);
        }
        session::save_sessions(&self.sessions);
        cx.notify();
    }

    fn move_folder(&mut self, id: Uuid, before: Option<Uuid>, cx: &mut Context<Self>) {
        if !reorder_folders(&mut self.folders, id, before) {
            return;
        }
        session::save_folders(&self.folders);
        cx.notify();
    }

    fn filter_query(&self, cx: &gpui::App) -> String {
        self.filter.read(cx).value().trim().to_lowercase()
    }

    fn is_open(&self, id: Uuid) -> bool {
        self.tabs.iter().any(|tab| tab.session_id == Some(id))
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
            icon: IconName::Settings.path(),
            icon_color: None,
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

    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
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

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let update_button = match &self.update_state {
            UpdateState::Idle => None,
            UpdateState::Available(found) => Some(
                Button::new("update")
                    .primary()
                    .small()
                    .icon(IconName::ArrowDown)
                    .label("Download update")
                    .tooltip(format!("Version {}", found.version))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.start_update_download(cx);
                    })),
            ),
            UpdateState::Downloading(_) => Some(
                Button::new("update")
                    .ghost()
                    .small()
                    .icon(IconName::Loader)
                    .label("Downloading update"),
            ),
            UpdateState::Ready(_) => Some(
                Button::new("update")
                    .primary()
                    .small()
                    .icon(IconName::Redo2)
                    .label("Restart to update")
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.restart_to_update(window, cx);
                    })),
            ),
        };

        let this = cx.entity();

        TitleBar::new()
            .child(
                Button::new("application-menu")
                    .icon(IconName::Menu)
                    .ghost()
                    .small()
                    .dropdown_menu(move |menu, _window, _cx| {
                        let this = this.clone();
                        menu.item(
                            PopupMenuItem::new("Settings")
                                .icon(IconName::Settings)
                                .on_click(move |_, window, cx| {
                                    this.update(cx, |view, cx| {
                                        view.open_settings_tab(window, cx);
                                    });
                                }),
                        )
                        .item(PopupMenuItem::new("About").icon(IconName::Info).on_click(
                            |_, window, cx| {
                                open_about_dialog(window, cx);
                            },
                        ))
                        .separator()
                        .item(
                            PopupMenuItem::new("Exit")
                                .icon(IconName::WindowClose)
                                .on_click(|_, _, cx| {
                                    cx.quit();
                                }),
                        )
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .pr_2()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when_some(update_button, |this, button| this.child(button)),
            )
    }

    fn render_session_row(&self, item: &Session, cx: &mut Context<Self>) -> impl IntoElement {
        let id = item.id;
        let selected = self.selected_session == Some(id);
        let open = self.is_open(id);
        let group_name = SharedString::from(format!("session-{id}"));
        let name = SharedString::from(item.name.clone());
        let icon = item.display_icon();
        let color = item.color.hsla();
        let accent = color.unwrap_or(cx.theme().primary);
        let detail = SharedString::from(match item.kind {
            SessionKind::Local => item.detail(),
            kind => format!("{} · {}", kind.label(), item.detail()),
        });
        let capacity = self.label_capacity(cx);
        let hover_bg = cx.theme().sidebar_accent.opacity(0.5);

        h_flex()
            .id(SharedString::from(format!("session-{id}")))
            .group(group_name.clone())
            .relative()
            .items_center()
            .gap_2()
            .px_2()
            .pt(px(2.))
            .pb_1()
            .mx_1()
            .rounded_md()
            .border_t_2()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .when(selected, |this| {
                this.bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(6.))
                            .bottom(px(6.))
                            .w(px(3.))
                            .rounded_full()
                            .bg(accent),
                    )
            })
            .when(!selected, |this| {
                this.hover(move |style| style.bg(hover_bg))
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
            .on_drag(
                SessionDrag {
                    id,
                    preview: DragPreview {
                        icon: icon.clone(),
                        color,
                        label: name.clone(),
                    },
                },
                |drag, _, _, cx| cx.new(|_| drag.preview.clone()),
            )
            .can_drop(move |drag, _, _| {
                drag.downcast_ref::<SessionDrag>()
                    .is_some_and(|drag| drag.id != id)
            })
            .drag_over::<SessionDrag>(|style, _, _, cx| style.border_color(cx.theme().primary))
            .on_drop(cx.listener(move |view, drag: &SessionDrag, _, cx| {
                view.move_session(drag.id, DropTarget::Before(id), cx);
            }))
            .child(
                div()
                    .flex_none()
                    .size(px(24.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(match color {
                        Some(color) => color.opacity(0.18),
                        None => cx.theme().sidebar_foreground.opacity(0.1),
                    })
                    .child(
                        Icon::empty()
                            .path(icon)
                            .small()
                            .when_some(color, |this, color| this.text_color(color)),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .min_w_0()
                            .child(
                                truncating_label(
                                    SharedString::from(format!("name-{id}")),
                                    name.clone(),
                                    capacity,
                                )
                                .min_w_0()
                                .text_sm(),
                            )
                            .when(open, |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .size(px(6.))
                                        .rounded_full()
                                        .bg(cx.theme().success),
                                )
                            }),
                    )
                    .child(
                        truncating_label(
                            SharedString::from(format!("detail-{id}")),
                            detail,
                            capacity,
                        )
                        .text_xs()
                        .text_color(cx.theme().muted_foreground),
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
                            .on_click(cx.listener(move |view, _, _window, cx| {
                                let Some(session) =
                                    view.sessions.iter().find(|s| s.id == id).cloned()
                                else {
                                    return;
                                };
                                view.open_session_window(Some(session), cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Delete)
                            .tooltip("Delete")
                            .on_click(cx.listener(move |_view, _, window, cx| {
                                let weak_app = cx.weak_entity();
                                let name = name.clone();
                                window.open_dialog(cx, move |dialog, _window, _cx| {
                                    let weak_app = weak_app.clone();
                                    dialog
                                        .title("Delete Session")
                                        .child(div().w(px(360.)).child(format!(
                                            "Delete \"{name}\"? This also removes its saved \
                                             password and closes any open tabs for it."
                                        )))
                                        .footer(
                                            DialogFooter::new()
                                                .child(
                                                    Button::new("cancel").label("Cancel").on_click(
                                                        |_, window, cx| {
                                                            window.close_dialog(cx);
                                                        },
                                                    ),
                                                )
                                                .child(
                                                    Button::new("delete")
                                                        .danger()
                                                        .label("Delete")
                                                        .on_click(move |_, window, cx| {
                                                            let _ =
                                                                weak_app.update(cx, |app, cx| {
                                                                    app.delete_session(id, cx);
                                                                });
                                                            window.close_dialog(cx);
                                                        }),
                                                ),
                                        )
                                });
                            })),
                    ),
            )
    }

    fn sidebar_width(&self, cx: &gpui::App) -> gpui::Pixels {
        self.sidebar_state
            .read(cx)
            .sizes()
            .first()
            .copied()
            .filter(|width| f32::from(*width) > 0.)
            .unwrap_or_else(|| px(cx.global::<AppSettings>().sidebar_width))
    }

    fn label_capacity(&self, cx: &gpui::App) -> usize {
        label_capacity(self.sidebar_width(cx))
    }

    fn active_sftp_panel(&self) -> Option<Entity<SftpPanel>> {
        self.active_tab
            .and_then(|index| self.tabs.get(index))
            .and_then(|tab| match &tab.content {
                TabContent::SshSession { sftp, .. } => Some(sftp.clone()),
                _ => None,
            })
    }

    fn effective_sidebar_mode(&self) -> SidebarMode {
        match self.sidebar_mode {
            SidebarMode::Explorer if self.active_sftp_panel().is_none() => SidebarMode::Sessions,
            mode => mode,
        }
    }

    fn render_sidebar_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.effective_sidebar_mode();
        let sessions_active = !self.sidebar_collapsed && mode == SidebarMode::Sessions;
        let explorer_active = !self.sidebar_collapsed && mode == SidebarMode::Explorer;
        let has_explorer = self.active_sftp_panel().is_some();

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
                    .ghost()
                    .large()
                    .icon(
                        Icon::new(IconName::SquareTerminal)
                            .when(sessions_active, |this| this.text_color(cx.theme().primary)),
                    )
                    .tooltip("Sessions")
                    .when(sessions_active, |b| b.bg(cx.theme().primary.opacity(0.12)))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.set_sidebar_mode(SidebarMode::Sessions, cx);
                    })),
            )
            .when(has_explorer, |this| {
                this.child(
                    Button::new("sidebar-explorer")
                        .ghost()
                        .large()
                        .icon(
                            Icon::new(IconName::Folder)
                                .when(explorer_active, |this| this.text_color(cx.theme().primary)),
                        )
                        .tooltip("File Explorer")
                        .when(explorer_active, |b| b.bg(cx.theme().primary.opacity(0.12)))
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
        let content = match self.active_sftp_panel() {
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

    fn render_folder_row(
        &self,
        folder: &SessionFolder,
        count: usize,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let folder_id = folder.id;
        let group_name = SharedString::from(format!("folder-{folder_id}"));
        let capacity = self.label_capacity(cx);
        let hover_bg = cx.theme().sidebar_accent.opacity(0.5);
        let preview = DragPreview {
            icon: folder.display_icon(),
            color: folder.color.hsla(),
            label: SharedString::from(folder.name.clone()),
        };

        h_flex()
            .id(SharedString::from(format!("folder-{folder_id}")))
            .group(group_name.clone())
            .items_center()
            .gap_1()
            .px_2()
            .pt(px(2.))
            .pb_1()
            .mx_1()
            .rounded_md()
            .border_t_2()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .on_drag(
                FolderDrag {
                    id: folder_id,
                    preview,
                },
                |drag, _, _, cx| cx.new(|_| drag.preview.clone()),
            )
            .can_drop(move |drag, _, _| {
                drag.downcast_ref::<FolderDrag>()
                    .is_none_or(|drag| drag.id != folder_id)
            })
            .drag_over::<SessionDrag>(|style, _, _, cx| style.bg(cx.theme().primary.opacity(0.12)))
            .on_drop(cx.listener(move |view, drag: &SessionDrag, _, cx| {
                view.move_session(drag.id, DropTarget::Into(Some(folder_id)), cx);
            }))
            .drag_over::<FolderDrag>(|style, _, _, cx| style.border_color(cx.theme().primary))
            .on_drop(cx.listener(move |view, drag: &FolderDrag, _, cx| {
                view.move_folder(drag.id, Some(folder_id), cx);
            }))
            .on_click(cx.listener(move |view, _, _, cx| {
                view.toggle_folder_collapsed(folder_id, cx);
            }))
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall()
                .text_color(cx.theme().muted_foreground),
            )
            .child(
                Icon::empty()
                    .path(folder.display_icon())
                    .small()
                    .when_some(folder.color.hsla(), |this, color| this.text_color(color)),
            )
            .child(
                truncating_label(
                    SharedString::from(format!("folder-name-{folder_id}")),
                    SharedString::from(folder.name.clone()),
                    capacity,
                )
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD),
            )
            .child(
                div()
                    .flex_none()
                    .px_1p5()
                    .rounded_full()
                    .text_xs()
                    .bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(count.to_string())),
            )
            .child(
                h_flex()
                    .gap_1()
                    .invisible()
                    .group_hover(group_name, |this| this.visible())
                    .child(
                        Button::new(SharedString::from(format!("edit-folder-{folder_id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Settings2)
                            .tooltip("Edit")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                let Some(folder) =
                                    view.folders.iter().find(|f| f.id == folder_id).cloned()
                                else {
                                    return;
                                };
                                let weak_app = cx.weak_entity();
                                session_dialog::open_edit_folder_dialog(
                                    folder, weak_app, window, cx,
                                );
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-folder-{folder_id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Delete)
                            .tooltip("Delete Folder")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.delete_folder(folder_id, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_sessions_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.filter_query(cx);
        let filtering = !query.is_empty();
        let muted = cx.theme().muted_foreground;
        let mut rows: Vec<gpui::AnyElement> = Vec::new();

        for folder in &self.folders {
            let folder_id = folder.id;
            let children: Vec<&Session> = self
                .sessions
                .iter()
                .filter(|s| s.folder_id == Some(folder_id) && matches_filter(s, &query))
                .collect();
            if filtering && children.is_empty() {
                continue;
            }
            let collapsed = !filtering && self.collapsed_folders.contains(&folder_id);
            rows.push(self.render_folder_row(folder, children.len(), collapsed, cx));

            if !collapsed && !children.is_empty() {
                let nested: Vec<gpui::AnyElement> = children
                    .into_iter()
                    .map(|item| self.render_session_row(item, cx).into_any_element())
                    .collect();
                rows.push(
                    v_flex()
                        .ml(FOLDER_GUIDE_INDENT)
                        .pl_1()
                        .border_l_1()
                        .border_color(cx.theme().sidebar_border)
                        .children(nested)
                        .into_any_element(),
                );
            }
        }

        let loose: Vec<&Session> = self
            .sessions
            .iter()
            .filter(|s| s.folder_id.is_none() && matches_filter(s, &query))
            .collect();
        if !loose.is_empty() {
            if !rows.is_empty() {
                let first = loose[0].id;
                rows.push(
                    h_flex()
                        .id("ungrouped-sessions")
                        .items_center()
                        .gap_2()
                        .mx_1()
                        .mt_2()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .drag_over::<SessionDrag>(|style, _, _, cx| {
                            style.bg(cx.theme().primary.opacity(0.12))
                        })
                        .on_drop(cx.listener(move |view, drag: &SessionDrag, _, cx| {
                            view.move_session(drag.id, DropTarget::Before(first), cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(muted)
                                .child("Other"),
                        )
                        .child(div().flex_1().h(px(1.)).bg(cx.theme().sidebar_border))
                        .into_any_element(),
                );
            }
            rows.extend(
                loose
                    .iter()
                    .map(|item| self.render_session_row(item, cx).into_any_element()),
            );
        }

        if rows.is_empty() {
            rows.push(
                div()
                    .px_3()
                    .py_4()
                    .text_xs()
                    .text_color(muted)
                    .child(if filtering {
                        "No sessions match"
                    } else {
                        "No sessions yet. Press + to add one."
                    })
                    .into_any_element(),
            );
        }

        rows.push(
            div()
                .id("sessions-drop-tail")
                .flex_1()
                .min_h(px(32.))
                .mx_1()
                .rounded_md()
                .drag_over::<SessionDrag>(|style, _, _, cx| {
                    style.bg(cx.theme().primary.opacity(0.12))
                })
                .on_drop(cx.listener(|view, drag: &SessionDrag, _, cx| {
                    view.move_session(drag.id, DropTarget::Into(None), cx);
                }))
                .drag_over::<FolderDrag>(|style, _, _, cx| {
                    style.bg(cx.theme().primary.opacity(0.12))
                })
                .on_drop(cx.listener(|view, drag: &FolderDrag, _, cx| {
                    view.move_folder(drag.id, None, cx);
                }))
                .into_any_element(),
        );

        v_flex()
            .size_full()
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
                                    .on_click(cx.listener(|view, _, _window, cx| {
                                        view.open_session_window(None, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div().px_2().pb_2().child(
                    Input::new(&self.filter)
                        .small()
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(muted))
                        .cleanable(true),
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
                let group_name = SharedString::from(format!("tab-{index}"));
                Tab::new()
                    .group(group_name.clone())
                    .prefix(
                        Icon::empty()
                            .path(tab.icon.clone())
                            .xsmall()
                            .when_some(tab.icon_color, |this, color| this.text_color(color)),
                    )
                    .pl_3()
                    .pr_2()
                    .label(tab.title.clone())
                    .suffix(
                        Button::new(SharedString::from(format!("close-tab-{index}")))
                            .ghost()
                            .xsmall()
                            .rounded(px(10.))
                            .icon(Icon::new(IconName::Close).with_size(px(11.)))
                            .when(index != active_index, |this| this.invisible())
                            .group_hover(group_name, |this| this.visible())
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
            .id("welcome")
            .flex_1()
            .min_w_0()
            .h_full()
            .px_6()
            .py_4()
            .child(
                v_flex()
                    .my_auto()
                    .w_full()
                    .items_center()
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
                                Some(s) => {
                                    SharedString::from(format!("Ready to connect: {}", s.detail()))
                                }
                                None => SharedString::from(
                                    "Select a session on the left, or add a new one",
                                ),
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
                    .child(render_shortcuts(cx)),
            )
            .overflow_y_scrollbar()
    }
}

fn render_shortcuts(cx: &gpui::App) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;

    let section = |title: &'static str| {
        div()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(muted)
            .child(title)
    };

    v_flex()
        .mt_4()
        .p_4()
        .gap_3()
        .max_w_full()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted.opacity(0.35))
        .child(
            h_flex()
                .flex_wrap()
                .items_start()
                .gap_x_10()
                .gap_y_4()
                .child(
                    v_flex()
                        .min_w_0()
                        .gap_2()
                        .child(section("CLIPBOARD"))
                        .child(shortcut(
                            &[CLIPBOARD_MODIFIER, "C"],
                            if cfg!(target_os = "macos") {
                                "Copy selection"
                            } else {
                                "Copy selection, else interrupt"
                            },
                            cx,
                        ))
                        .child(shortcut(&[CLIPBOARD_MODIFIER, "V"], "Paste", cx))
                        .child(shortcut(&[CLIPBOARD_MODIFIER, "X"], "Cut selection", cx))
                        .child(shortcut(&["Right click"], "Paste", cx)),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .gap_2()
                        .child(section("NAVIGATION"))
                        .child(shortcut(&[WORD_MOVE_MODIFIER, "←/→"], "Move by word", cx))
                        .child(shortcut(&["Shift", "PgUp/PgDn"], "Scroll history", cx))
                        .child(shortcut(&["Drag"], "Select text", cx))
                        .child(shortcut(&["Double click"], "Open a session", cx)),
                ),
        )
}

fn shortcut(keys: &[&str], label: &'static str, cx: &gpui::App) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;

    let mut chips = h_flex().w(px(150.)).flex_none().items_center().gap_1();
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            chips = chips.child(div().text_xs().text_color(muted).child("+"));
        }
        chips = chips.child(
            div()
                .px_2()
                .py_0p5()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .text_xs()
                .child(SharedString::from(key.to_string())),
        );
    }

    h_flex().items_center().gap_3().child(chips).child(
        div()
            .min_w_0()
            .text_xs()
            .text_ellipsis()
            .text_color(muted)
            .child(label),
    )
}

impl Render for OxidalApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .child({
                let sidebar = if self.sidebar_collapsed {
                    None
                } else if self.effective_sidebar_mode() == SidebarMode::Explorer {
                    Some(self.render_explorer_panel(cx).into_any_element())
                } else {
                    Some(self.render_sessions_panel(cx).into_any_element())
                };

                let content = h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar_rail(cx));
                match sidebar {
                    // Both modes share one split, so a width set on either side
                    // is the width the other opens at.
                    Some(sidebar) => content.child(
                        div().flex_1().min_w_0().h_full().child(
                            h_resizable("sidebar-split")
                                .with_state(&self.sidebar_state)
                                .child(
                                    resizable_panel()
                                        .size(px(cx.global::<AppSettings>().sidebar_width))
                                        .size_range(
                                            px(settings::SIDEBAR_MIN_WIDTH)
                                                ..px(settings::SIDEBAR_MAX_WIDTH),
                                        )
                                        .child(sidebar),
                                )
                                .child(self.render_workspace(cx).into_any_element())
                                // Fires once on mouse up, so this is a write per
                                // drag rather than per frame.
                                .on_resize(|state, _, cx| {
                                    let Some(width) = state.read(cx).sizes().first().copied()
                                    else {
                                        return;
                                    };
                                    let width = f32::from(width).clamp(
                                        settings::SIDEBAR_MIN_WIDTH,
                                        settings::SIDEBAR_MAX_WIDTH,
                                    );
                                    if cx.global::<AppSettings>().sidebar_width != width {
                                        cx.global_mut::<AppSettings>().sidebar_width = width;
                                        settings::save_settings(cx.global::<AppSettings>());
                                    }
                                }),
                        ),
                    ),
                    None => content.child(self.render_workspace(cx).into_any_element()),
                }
            })
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

fn open_about_dialog(window: &mut Window, cx: &mut gpui::App) {
    window.open_dialog(cx, |dialog, _window, cx| {
        let muted = cx.theme().muted_foreground;
        dialog
            .title("About")
            .child(
                v_flex()
                    .w(px(380.))
                    .gap_3()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::SquareTerminal).small())
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Oxidal"))
                            .child(
                                div()
                                    .text_color(muted)
                                    .child(concat!("v", env!("CARGO_PKG_VERSION"))),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .bg(cx.theme().muted)
                                    .text_xs()
                                    .child("Community Edition"),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("Cross-platform SSH, SFTP and serial terminal client."),
                    )
                    .child(
                        h_flex()
                            .id("about-repo")
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(cx.theme().primary)
                            .child(Icon::new(IconName::Github).xsmall())
                            .child(div().underline().child("github.com/sh4den/Oxidal"))
                            .on_click(|_, _, _| {
                                let _ = open::that_detached("https://github.com/sh4den/Oxidal");
                            }),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Core maintainers"),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(Icon::new(IconName::User).xsmall())
                                    .child("𝑺𝒉𝒂𝒅𝒆𝒏")
                                    .child(
                                        div()
                                            .id("about-maintainer")
                                            .cursor_pointer()
                                            .underline()
                                            .text_color(cx.theme().primary)
                                            .child("@sh4den")
                                            .on_click(|_, _, _| {
                                                let _ = open::that_detached(
                                                    "https://github.com/sh4den",
                                                );
                                            }),
                                    ),
                            ),
                    ),
            )
            .footer(
                DialogFooter::new().child(Button::new("close-about").label("Close").on_click(
                    |_, window, cx| {
                        window.close_dialog(cx);
                    },
                )),
            )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str, folder_id: Option<Uuid>) -> Session {
        let mut session = Session::new(name, SessionKind::Ssh);
        session.folder_id = folder_id;
        session
    }

    fn names(sessions: &[Session]) -> Vec<&str> {
        sessions.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn dropping_before_a_row_reorders_and_adopts_its_folder() {
        let folder = Uuid::new_v4();
        let mut sessions = vec![
            session("a", None),
            session("b", None),
            session("c", Some(folder)),
        ];
        let (a, c) = (sessions[0].id, sessions[2].id);

        assert!(reorder_sessions(&mut sessions, a, DropTarget::Before(c)));
        assert_eq!(names(&sessions), vec!["b", "a", "c"]);
        assert_eq!(sessions[1].folder_id, Some(folder));
    }

    #[test]
    fn dropping_into_a_folder_appends_and_ungrouping_clears_the_folder() {
        let folder = Uuid::new_v4();
        let mut sessions = vec![
            session("a", None),
            session("b", Some(folder)),
            session("c", Some(folder)),
        ];
        let (a, b) = (sessions[0].id, sessions[1].id);

        assert!(reorder_sessions(
            &mut sessions,
            a,
            DropTarget::Into(Some(folder))
        ));
        assert_eq!(names(&sessions), vec!["b", "c", "a"]);
        assert_eq!(sessions[2].folder_id, Some(folder));

        assert!(reorder_sessions(&mut sessions, b, DropTarget::Into(None)));
        assert_eq!(names(&sessions), vec!["c", "a", "b"]);
        assert_eq!(sessions[2].folder_id, None);
    }

    #[test]
    fn unknown_targets_leave_the_list_untouched() {
        let mut sessions = vec![session("a", None), session("b", None)];
        let b = sessions[1].id;

        assert!(!reorder_sessions(
            &mut sessions,
            b,
            DropTarget::Before(Uuid::new_v4())
        ));
        assert_eq!(names(&sessions), vec!["a", "b"]);
        assert!(!reorder_sessions(
            &mut sessions,
            Uuid::new_v4(),
            DropTarget::Into(None)
        ));
        assert_eq!(names(&sessions), vec!["a", "b"]);
    }

    #[test]
    fn folders_reorder_before_a_target_or_to_the_end() {
        let mut folders = vec![
            SessionFolder::new("a"),
            SessionFolder::new("b"),
            SessionFolder::new("c"),
        ];
        let (a, c) = (folders[0].id, folders[2].id);
        let folder_names =
            |folders: &[SessionFolder]| folders.iter().map(|f| f.name.clone()).collect::<Vec<_>>();

        assert!(reorder_folders(&mut folders, c, Some(a)));
        assert_eq!(folder_names(&folders), ["c", "a", "b"]);
        assert!(reorder_folders(&mut folders, c, None));
        assert_eq!(folder_names(&folders), ["a", "b", "c"]);
        assert!(!reorder_folders(&mut folders, a, Some(Uuid::new_v4())));
        assert_eq!(folder_names(&folders), ["a", "b", "c"]);
    }

    #[test]
    fn filter_matches_name_host_user_and_kind() {
        let mut s = session("prod-web", None);
        s.host = "10.0.1.12".into();
        s.username = "deploy".into();

        for query in ["prod", "0.1.1", "deploy", "ssh"] {
            assert!(matches_filter(&s, query), "{query}");
        }
        assert!(!matches_filter(&s, "sftp"));
        assert!(matches_filter(&s, ""));
    }
}
