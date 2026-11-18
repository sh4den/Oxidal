use crate::session::{self, Session, SessionFolder};
use crate::settings_view::SettingsView;
use crate::sftp::{SftpPanel, SftpWorkspace};
use crate::terminal::TerminalView;

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, Hsla, ScrollHandle, SharedString, Window,
    WindowHandle,
};

use gpui_kit::component::{
    Root,
    input::{InputEvent, InputState},
    resizable::ResizableState,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use uuid::Uuid;

mod drag;
mod sessions;
mod tabs;
mod ui;
mod updater;

use drag::DragSource;
use ui::SidebarPane;

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
    dragging: Option<DragSource>,
    tab_scroll: ScrollHandle,
    sidebar_pane: Option<Entity<SidebarPane>>,
}

impl OxidalApp {
    fn forward_prompts<R: 'static>(
        requests: async_channel::Receiver<R>,
        open: fn(R, &mut Window, &mut gpui_kit::App),
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Ok(request) = requests.recv().await {
                let active = cx.update(|cx| cx.active_window());
                let target = this
                    .read_with(cx, |app, _| {
                        active.filter(|active| {
                            app.session_windows
                                .values()
                                .any(|handle| AnyWindowHandle::from(*handle) == *active)
                        })
                    })
                    .ok()
                    .flatten()
                    .unwrap_or(window_handle);
                let opened = cx.update_window(target, |_, window, cx| {
                    window.activate_window();
                    open(request, window, cx);
                });
                if opened.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let window_handle = window.window_handle();
        Self::forward_prompts(
            crate::host_keys::requests(),
            crate::host_keys::open_prompt,
            window_handle,
            cx,
        );
        Self::forward_prompts(
            crate::cert_trust::requests(),
            crate::cert_trust::open_prompt,
            window_handle,
            cx,
        );

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
            dragging: None,
            tab_scroll: ScrollHandle::new(),
            sidebar_pane: None,
        }
    }
}
