use std::io::Read as _;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IconNamed as _, IndexPath, Root,
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{SearchableVec, Select, SelectItem, SelectState},
    tab::{Tab, TabBar},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, PathPromptOptions,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    TitlebarOptions, WeakEntity, Window, WindowBounds, WindowHandle, WindowOptions, div,
    prelude::FluentBuilder as _, px, size,
};
use secrecy::{ExposeSecret as _, SecretString};
use serialport::SerialPortType;
use uuid::Uuid;

use crate::app::OxidalApp;
use crate::proxy::ProxyConfig;
use crate::session::{
    FtpSecurity, ItemColor, ItemIcon, Session, SessionFolder, SessionKind, default_proxy_port,
};
use crate::ssh_client::SshCredentials;

fn scrollable_body(
    id: &'static str,
    body: impl IntoElement,
    scroll: &ScrollHandle,
    max_height: gpui_kit::Pixels,
    cx: &App,
) -> impl IntoElement {
    let remaining = scroll.max_offset().y + scroll.offset().y;
    let background = cx.theme().background;
    let border = cx.theme().border;
    let popover = cx.theme().popover;
    let muted_foreground = cx.theme().muted_foreground;

    div()
        .relative()
        .w_full()
        .child(
            div()
                .id(id)
                .w_full()
                .max_h(max_height)
                .overflow_y_scroll()
                .track_scroll(scroll)
                .child(body),
        )
        .when(remaining > gpui_kit::px(4.), |this| {
            this.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(gpui_kit::px(48.))
                    .pb_1()
                    .flex()
                    .items_end()
                    .justify_center()
                    .bg(gpui_kit::linear_gradient(
                        180.,
                        gpui_kit::linear_color_stop(background.opacity(0.), 0.),
                        gpui_kit::linear_color_stop(background, 0.7),
                    ))
                    .child(
                        h_flex()
                            .h(gpui_kit::px(22.))
                            .px_2()
                            .gap_1()
                            .items_center()
                            .rounded_full()
                            .bg(popover)
                            .border_1()
                            .border_color(border)
                            .text_color(muted_foreground)
                            .shadow_sm()
                            .child(Icon::new(IconName::ChevronDown).xsmall())
                            .child(div().text_xs().child("Scroll down"))
                            .child(Icon::new(IconName::ChevronDown).xsmall()),
                    ),
            )
        })
}

const ICON_RESULTS_MAX: usize = 120;

struct SelectedIcon {
    icon: Option<ItemIcon>,
    query: Entity<InputState>,
}

impl SelectedIcon {
    fn new(icon: Option<ItemIcon>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search all icons"));
        cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        })
        .detach();
        Self { icon, query }
    }

    fn select(&mut self, icon: Option<ItemIcon>, cx: &mut Context<Self>) {
        self.icon = icon;
        cx.notify();
    }
}

struct SelectedColor(ItemColor);

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Auth,
    Proxy,
    Monitoring,
    Label,
}

impl SettingsTab {
    fn label(self) -> &'static str {
        match self {
            SettingsTab::Auth => "Auth",
            SettingsTab::Proxy => "Proxy",
            SettingsTab::Monitoring => "Monitoring",
            SettingsTab::Label => "Label",
        }
    }

    fn available(kind: SessionKind) -> &'static [SettingsTab] {
        match kind {
            SessionKind::Ssh => &[
                SettingsTab::Auth,
                SettingsTab::Proxy,
                SettingsTab::Monitoring,
                SettingsTab::Label,
            ],
            SessionKind::Sftp | SessionKind::Ftp => {
                &[SettingsTab::Auth, SettingsTab::Proxy, SettingsTab::Label]
            }
            SessionKind::Telnet => &[SettingsTab::Proxy, SettingsTab::Label],
            SessionKind::Rdp => &[SettingsTab::Auth, SettingsTab::Label],
            SessionKind::Serial | SessionKind::Local => &[SettingsTab::Label],
        }
    }
}

fn cleartext_warning(message: &'static str, cx: &App) -> impl IntoElement {
    h_flex()
        .gap_2()
        .items_start()
        .p_2()
        .rounded_md()
        .bg(cx.theme().muted)
        .child(
            Icon::new(IconName::TriangleAlert)
                .xsmall()
                .text_color(cx.theme().warning),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(message),
        )
}

impl Render for SelectedIcon {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.icon;
        let border = cx.theme().border;
        let primary = cx.theme().primary;
        let accent = cx.theme().accent;
        let muted = cx.theme().muted_foreground;
        let query = self.query.read(cx).value().trim().to_string();
        let searching = !query.is_empty();

        let (icons, total) = if searching {
            let mut matches = ItemIcon::search(&query);
            let shown: Vec<ItemIcon> = matches.by_ref().take(ICON_RESULTS_MAX).collect();
            let total = shown.len() + matches.count();
            (shown, total)
        } else {
            let mut shown: Vec<ItemIcon> = ItemIcon::suggested().collect();
            if let Some(icon) = current
                && !icon.is_os()
                && !shown.contains(&icon)
            {
                shown.insert(0, icon);
            }
            let total = shown.len();
            (shown, total)
        };
        let os_logos: Vec<ItemIcon> = if searching {
            ItemIcon::search_os(&query).collect()
        } else {
            ItemIcon::os_logos().collect()
        };

        let swatch = move |selected: bool| {
            div()
                .h(gpui_kit::px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .border_1()
                .cursor_pointer()
                .map(move |this| {
                    if selected {
                        this.border_color(primary).bg(primary.opacity(0.12))
                    } else {
                        this.border_color(border).hover(|this| this.bg(accent))
                    }
                })
        };

        let auto = swatch(current.is_none())
            .id("icon-default")
            .px_2()
            .child(
                div()
                    .text_xs()
                    .when(current.is_some(), |this| this.text_color(muted))
                    .child("Auto"),
            )
            .on_click(cx.listener(|this, _, _, cx| this.select(None, cx)));

        let tile = |icon: ItemIcon, cx: &mut Context<Self>| {
            let label = icon.label();
            swatch(current == Some(icon))
                .id(SharedString::new_static(icon.name()))
                .w(gpui_kit::px(28.))
                .child(Icon::empty().path(icon.path()).small())
                .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
                .on_click(cx.listener(move |this, _, _, cx| this.select(Some(icon), cx)))
        };

        let grid = h_flex()
            .flex_wrap()
            .gap_1()
            .when(!searching, |this| this.child(auto))
            .children(icons.into_iter().map(|icon| tile(icon, cx)));

        let os_section = (!os_logos.is_empty()).then(|| {
            v_flex()
                .gap_1()
                .pt_2()
                .child(div().text_xs().text_color(muted).child("Operating systems"))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1()
                        .children(os_logos.into_iter().map(|icon| tile(icon, cx))),
                )
        });
        let no_os_match = os_section.is_none();

        let note = if searching && total == 0 && no_os_match {
            Some(format!("No icon matches \"{query}\""))
        } else if total > ICON_RESULTS_MAX {
            Some(format!(
                "Showing {ICON_RESULTS_MAX} of {total} icons. Keep typing to narrow it down."
            ))
        } else {
            None
        };

        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child("Icon")
                    .when_some(current, |this, icon| {
                        this.child(div().text_xs().text_color(muted).child(icon.label()))
                    }),
            )
            .child(
                Input::new(&self.query)
                    .small()
                    .prefix(Icon::new(IconName::Search).xsmall().text_color(muted))
                    .cleanable(true),
            )
            .child(
                div()
                    .id("icon-results")
                    .max_h(gpui_kit::px(196.))
                    .overflow_y_scroll()
                    .child(grid)
                    .children(os_section),
            )
            .when_some(note, |this, note| {
                this.child(div().text_xs().text_color(muted).child(note))
            })
    }
}

fn color_picker(state: &Entity<SelectedColor>, cx: &App) -> impl IntoElement {
    let current = state.read(cx).0;
    let border = cx.theme().border;
    let primary = cx.theme().primary;
    let foreground = cx.theme().foreground;

    v_flex()
        .gap_1()
        .child("Color")
        .child(
            h_flex()
                .flex_wrap()
                .gap_1()
                .children(ItemColor::ALL.iter().map(|color| {
                    let color = *color;
                    let selected = current == color;
                    let state = state.clone();
                    div()
                        .id(SharedString::from(format!("color-{color:?}")))
                        .size(gpui_kit::px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .border_1()
                        .cursor_pointer()
                        .map(|this| {
                            if selected {
                                this.border_color(primary).bg(primary.opacity(0.12))
                            } else {
                                this.border_color(border)
                            }
                        })
                        .child(
                            div()
                                .size(gpui_kit::px(14.))
                                .rounded_full()
                                .bg(color.hsla().unwrap_or(foreground)),
                        )
                        .on_click(move |_, _, cx| {
                            state.update(cx, |s, cx| {
                                s.0 = color;
                                cx.notify();
                            });
                        })
                })),
        )
}

#[derive(Clone)]
enum TestState {
    Idle,
    Testing,
    Success(String),
    Failed(String),
}

#[derive(Clone, PartialEq)]
struct KeyOption {
    label: SharedString,
    detail: SharedString,
    path: SharedString,
}

impl KeyOption {
    fn none() -> Self {
        Self {
            label: "No private key".into(),
            detail: "".into(),
            path: "".into(),
        }
    }

    fn from_path(path: &Path) -> Self {
        Self {
            label: path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string())
                .into(),
            detail: path
                .parent()
                .map(|parent| parent.display().to_string())
                .unwrap_or_default()
                .into(),
            path: path.display().to_string().into(),
        }
    }
}

impl SelectItem for KeyOption {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.path
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.label.to_lowercase().contains(&query) || self.path.to_lowercase().contains(&query)
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.path.is_empty() {
            self.label.clone().into_any_element()
        } else {
            h_flex()
                .w_full()
                .gap_2()
                .justify_between()
                .child(div().flex_none().child(self.label.clone()))
                .child(
                    div()
                        .min_w_0()
                        .text_xs()
                        .text_ellipsis_start()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.detail.clone()),
                )
                .into_any_element()
        }
    }
}

#[derive(Clone, PartialEq)]
struct PortOption {
    name: SharedString,
    detail: SharedString,
}

impl SelectItem for PortOption {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.name
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .gap_2()
            .child(self.name.clone())
            .when(!self.detail.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.detail.clone()),
                )
            })
    }
}

pub fn open_session_window(
    existing: Option<Session>,
    folders: Vec<SessionFolder>,
    weak_app: WeakEntity<OxidalApp>,
    cx: &mut App,
) -> Option<WindowHandle<Root>> {
    let bounds = Bounds::centered(None, size(px(680.), px(700.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Session".into()),
                ..Default::default()
            }),
            is_minimizable: false,
            app_id: Some(crate::APP_ID.to_string()),
            window_min_size: Some(size(px(600.), px(480.))),
            ..Default::default()
        },
        |window, cx| {
            crate::settings::apply_window_appearance(window, cx);
            let view = cx.new(|cx| SessionWindow::new(existing, folders, weak_app, window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        },
    )
    .ok()
}

pub struct SessionWindow {
    weak_app: WeakEntity<OxidalApp>,
    _app_observer: Option<Subscription>,
    folders: Vec<SessionFolder>,
    editing_id: Option<Uuid>,
    kind: SessionKind,
    tab: SettingsTab,
    folder_id: Option<Uuid>,
    monitoring: bool,
    legacy_algorithms: bool,
    ftp_security: FtpSecurity,
    ftp_active_mode: bool,
    test_state: TestState,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    passphrase: Entity<InputState>,
    baud: Entity<InputState>,
    proxy_host: Entity<InputState>,
    proxy_port: Entity<InputState>,
    proxy_username: Entity<InputState>,
    proxy_password: Entity<InputState>,
    private_key: Entity<SelectState<SearchableVec<KeyOption>>>,
    serial_port: Entity<SelectState<Vec<PortOption>>>,
    selected_icon: Entity<SelectedIcon>,
    selected_color: Entity<SelectedColor>,
    focus_handle: FocusHandle,
    body_scroll: ScrollHandle,
}

fn text_field(
    value: String,
    placeholder: &'static str,
    masked: bool,
    window: &mut Window,
    cx: &mut Context<SessionWindow>,
) -> Entity<InputState> {
    let input = cx.new(|cx| {
        let mut state = InputState::new(window, cx).default_value(value);
        if !placeholder.is_empty() {
            state = state.placeholder(placeholder);
        }
        if masked {
            state = state.masked(true);
        }
        state
    });
    cx.subscribe_in(
        &input,
        window,
        |this: &mut SessionWindow, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.save(window, cx);
            }
        },
    )
    .detach();
    input
}

impl SessionWindow {
    fn new(
        existing: Option<Session>,
        folders: Vec<SessionFolder>,
        weak_app: WeakEntity<OxidalApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let app_observer = weak_app.upgrade().map(|app| {
            cx.observe(&app, |this, app, cx| {
                let folders = app.read(cx).folders();
                if this.folders.as_slice() != folders {
                    this.folders = folders.to_vec();
                    cx.notify();
                }
            })
        });
        let session = existing.as_ref();
        let editing_id = session.map(|s| s.id);
        let kind = session.map(|s| s.kind).unwrap_or(SessionKind::Ssh);

        window.set_window_title(&window_title(editing_id.is_some(), kind));

        let existing_key = session.and_then(|s| s.private_key_path.clone());
        let key_choices = key_options_without_scan(existing_key.as_deref());
        let key_index = existing_key
            .as_deref()
            .and_then(|path| key_choices.iter().position(|o| o.path.as_ref() == path))
            .unwrap_or(0);
        let key_searchable = key_choices.len() > 8;
        let private_key = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(key_choices),
                Some(IndexPath::default().row(key_index)),
                window,
                cx,
            )
            .searchable(key_searchable)
        });

        let existing_port = session
            .filter(|s| matches!(s.kind, SessionKind::Serial))
            .map(|s| s.host.clone())
            .filter(|h| !h.is_empty());
        let port_choices = extra_port(Vec::new(), existing_port.as_deref());
        let serial_index = existing_port
            .as_deref()
            .and_then(|p| port_choices.iter().position(|o| o.name.as_ref() == p))
            .map(|i| IndexPath::default().row(i));
        let serial_port = cx.new(|cx| SelectState::new(port_choices, serial_index, window, cx));

        cx.spawn_in(window, async move |this, cx| {
            let (keys, ports) = cx
                .background_spawn(async move {
                    (
                        key_options(existing_key.as_deref()),
                        port_options(existing_port.as_deref()),
                    )
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.apply_scan(keys, ports, window, cx);
            });
        })
        .detach();

        Self {
            _app_observer: app_observer,
            folders,
            editing_id,
            kind,
            tab: SettingsTab::available(kind)[0],
            folder_id: session.and_then(|s| s.folder_id),
            monitoring: session.is_none_or(|s| s.monitoring),
            legacy_algorithms: session.is_some_and(|s| s.legacy_algorithms),
            ftp_security: session.map(|s| s.ftp_security).unwrap_or_default(),
            ftp_active_mode: session.is_some_and(|s| s.ftp_active_mode),
            test_state: TestState::Idle,
            name: text_field(
                session.map(|s| s.name.clone()).unwrap_or_default(),
                "host (user)",
                false,
                window,
                cx,
            ),
            host: text_field(
                session.map(|s| s.host.clone()).unwrap_or_default(),
                "example.com",
                false,
                window,
                cx,
            ),
            port: text_field(
                session
                    .map(|s| s.port)
                    .unwrap_or_else(|| kind.default_port())
                    .to_string(),
                "",
                false,
                window,
                cx,
            ),
            username: text_field(
                session.map(|s| s.username.clone()).unwrap_or_default(),
                "username",
                false,
                window,
                cx,
            ),
            password: text_field(
                session
                    .map(|s| s.password.expose_secret().to_string())
                    .unwrap_or_default(),
                "",
                true,
                window,
                cx,
            ),
            passphrase: text_field(
                session
                    .map(|s| s.key_passphrase.expose_secret().to_string())
                    .unwrap_or_default(),
                "",
                true,
                window,
                cx,
            ),
            baud: text_field(
                session.map(|s| s.baud_rate).unwrap_or(115_200).to_string(),
                "",
                false,
                window,
                cx,
            ),
            proxy_host: text_field(
                session.map(|s| s.proxy_host.clone()).unwrap_or_default(),
                "proxy.example.com",
                false,
                window,
                cx,
            ),
            proxy_port: text_field(
                session
                    .map(|s| s.proxy_port)
                    .unwrap_or_else(default_proxy_port)
                    .to_string(),
                "",
                false,
                window,
                cx,
            ),
            proxy_username: text_field(
                session
                    .map(|s| s.proxy_username.clone())
                    .unwrap_or_default(),
                "username",
                false,
                window,
                cx,
            ),
            proxy_password: text_field(
                session
                    .map(|s| s.proxy_password.expose_secret().to_string())
                    .unwrap_or_default(),
                "",
                true,
                window,
                cx,
            ),
            private_key,
            serial_port,
            selected_icon: cx.new(|cx| SelectedIcon::new(session.and_then(|s| s.icon), window, cx)),
            selected_color: cx
                .new(|_cx| SelectedColor(session.map(|s| s.color).unwrap_or(ItemColor::Default))),
            focus_handle: cx.focus_handle(),
            body_scroll: ScrollHandle::new(),
            weak_app,
        }
    }

    fn apply_scan(
        &mut self,
        keys: Vec<KeyOption>,
        ports: Vec<PortOption>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_key = self.private_key.read(cx).selected_value().cloned();
        let key_index = selected_key
            .and_then(|path| keys.iter().position(|o| o.path == path))
            .unwrap_or(0);
        let key_searchable = keys.len() > 8;
        self.private_key = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(keys),
                Some(IndexPath::default().row(key_index)),
                window,
                cx,
            )
            .searchable(key_searchable)
        });
        self.serial_port.update(cx, |state, cx| {
            let selected = state.selected_value().cloned();
            state.set_items(ports, window, cx);
            if let Some(value) = selected {
                state.set_selected_value(&value, window, cx);
            }
        });
        cx.notify();
    }

    fn set_kind(&mut self, kind: SessionKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind == kind {
            return;
        }
        let prev_port = self.default_port_for(self.kind);
        self.kind = kind;
        window.set_window_title(&window_title(self.editing_id.is_some(), kind));
        if self.port.read(cx).value() == prev_port.to_string() {
            let port = self.default_port_for(kind);
            self.port.update(cx, |state, cx| {
                state.set_value(port.to_string(), window, cx)
            });
        }
        self.test_state = TestState::Idle;
        let tabs = SettingsTab::available(kind);
        if !tabs.contains(&self.tab) {
            self.tab = tabs[0];
        }
        cx.notify();
    }

    fn default_port_for(&self, kind: SessionKind) -> u16 {
        match (kind, self.ftp_security) {
            (SessionKind::Ftp, FtpSecurity::Implicit) => FtpSecurity::IMPLICIT_PORT,
            _ => kind.default_port(),
        }
    }

    fn set_ftp_security(
        &mut self,
        security: FtpSecurity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prev_port = self.default_port_for(self.kind);
        self.ftp_security = security;
        if self.port.read(cx).value() == prev_port.to_string() {
            let port = self.default_port_for(self.kind);
            self.port.update(cx, |state, cx| {
                state.set_value(port.to_string(), window, cx)
            });
        }
        self.test_state = TestState::Idle;
        cx.notify();
    }

    fn ftp_target(&self, cx: &App) -> crate::sftp::FtpTarget {
        crate::sftp::FtpTarget {
            host: self.host.read(cx).value().trim().to_string(),
            port: self
                .port
                .read(cx)
                .value()
                .to_string()
                .parse()
                .unwrap_or_else(|_| self.default_port_for(SessionKind::Ftp)),
            username: self.username.read(cx).value().trim().to_string(),
            password: SecretString::from(self.password.read(cx).value().to_string()),
            security: self.ftp_security,
            active_mode: self.ftp_active_mode,
            proxy: proxy_config(
                SessionKind::Ftp,
                self.proxy_host.read(cx).value().trim(),
                self.proxy_port.read(cx).value().trim(),
                self.proxy_username.read(cx).value().trim(),
                &self.proxy_password.read(cx).value(),
            ),
        }
    }

    fn run_test(&mut self, cx: &mut Context<Self>) {
        if matches!(self.test_state, TestState::Testing) {
            return;
        }
        let kind = self.kind;
        let host_value = if matches!(kind, SessionKind::Serial) {
            self.serial_port
                .read(cx)
                .selected_value()
                .map(|v| v.to_string())
                .unwrap_or_default()
        } else {
            self.host.read(cx).value().to_string()
        };
        if host_value.trim().is_empty() {
            let msg = if matches!(kind, SessionKind::Serial) {
                "Select a serial port first"
            } else {
                "Enter a host first"
            };
            self.test_state = TestState::Failed(msg.to_string());
            cx.notify();
            return;
        }
        let port_value = self
            .port
            .read(cx)
            .value()
            .to_string()
            .parse()
            .unwrap_or_else(|_| kind.default_port());
        let credentials = SshCredentials::new(
            self.username.read(cx).value().to_string(),
            SecretString::from(self.password.read(cx).value().to_string()),
            self.private_key
                .read(cx)
                .selected_value()
                .map(|v| v.to_string())
                .filter(|v| !v.trim().is_empty()),
            SecretString::from(self.passphrase.read(cx).value().to_string()),
        )
        .with_legacy_algorithms(self.legacy_algorithms);
        let proxy = proxy_config(
            kind,
            self.proxy_host.read(cx).value().trim(),
            self.proxy_port.read(cx).value().trim(),
            self.proxy_username.read(cx).value().trim(),
            &self.proxy_password.read(cx).value(),
        );
        let baud_value = self
            .baud
            .read(cx)
            .value()
            .to_string()
            .parse()
            .unwrap_or(115_200);

        let ftp = matches!(kind, SessionKind::Ftp).then(|| self.ftp_target(cx));

        self.test_state = TestState::Testing;
        cx.notify();
        let rx = run_connection_test(
            kind,
            host_value,
            port_value,
            credentials,
            proxy,
            baud_value,
            ftp,
        );
        cx.spawn(async move |this, cx| {
            let outcome = match rx.recv().await {
                Ok(Ok(msg)) => TestState::Success(msg),
                Ok(Err(err)) => TestState::Failed(err),
                Err(_) => TestState::Failed("Connection test aborted".to_string()),
            };
            let _ = this.update(cx, |view, cx| {
                view.test_state = outcome;
                cx.notify();
            });
        })
        .detach();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.kind;
        let label = self.name.read(cx).value().trim().to_string();
        let mut session = Session::new(label, kind);
        if let Some(id) = self.editing_id {
            session.id = id;
        }
        session.host = if matches!(kind, SessionKind::Serial) {
            self.serial_port
                .read(cx)
                .selected_value()
                .map(|v| v.to_string())
                .unwrap_or_default()
        } else {
            self.host.read(cx).value().to_string()
        };
        session.port = self
            .port
            .read(cx)
            .value()
            .to_string()
            .parse()
            .unwrap_or_else(|_| kind.default_port());
        session.username = self.username.read(cx).value().to_string();
        session.password = SecretString::from(self.password.read(cx).value().to_string());
        session.baud_rate = self
            .baud
            .read(cx)
            .value()
            .to_string()
            .parse()
            .unwrap_or(115_200);
        session.private_key_path = self
            .private_key
            .read(cx)
            .selected_value()
            .map(|v| v.to_string())
            .filter(|v| !v.trim().is_empty());
        session.key_passphrase = match session.private_key_path {
            Some(_) => SecretString::from(self.passphrase.read(cx).value().to_string()),
            None => SecretString::default(),
        };
        if let Some(proxy) = proxy_config(
            kind,
            self.proxy_host.read(cx).value().trim(),
            self.proxy_port.read(cx).value().trim(),
            self.proxy_username.read(cx).value().trim(),
            &self.proxy_password.read(cx).value(),
        ) {
            session.proxy_host = proxy.host;
            session.proxy_port = proxy.port;
            session.proxy_username = proxy.username;
            session.proxy_password = proxy.password;
        }
        session.folder_id = self.folder_id;
        session.icon = self.selected_icon.read(cx).icon;
        session.color = self.selected_color.read(cx).0;
        session.monitoring = self.monitoring;
        session.legacy_algorithms = self.legacy_algorithms;
        session.ftp_security = self.ftp_security;
        session.ftp_active_mode = self.ftp_active_mode;
        if session.name.is_empty() {
            session.name = session.default_name();
        }

        let editing = self.editing_id.is_some();
        let _ = self.weak_app.update(cx, |app, cx| {
            if editing {
                app.update_session(session, cx);
            } else {
                app.add_session(session, cx);
            }
        });
        window.remove_window();
    }

    fn render_tiles(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        h_flex()
            .gap_2()
            .pb_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .children(SessionKind::ALL.iter().map(|tile_kind| {
                let tile_kind = *tile_kind;
                let is_selected = tile_kind == kind;
                let available = tile_kind.is_available();
                div()
                    .id(SharedString::from(format!("kind-{}", tile_kind.label())))
                    .flex_1()
                    .h(px(64.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .rounded_md()
                    .border_1()
                    .map(|this| {
                        if is_selected {
                            this.border_color(cx.theme().primary)
                                .bg(cx.theme().primary.opacity(0.12))
                                .text_color(cx.theme().primary)
                        } else if available {
                            this.border_color(cx.theme().border)
                                .text_color(cx.theme().muted_foreground)
                                .hover(|this| this.bg(cx.theme().accent))
                        } else {
                            this.border_color(cx.theme().border)
                                .text_color(cx.theme().muted_foreground)
                        }
                    })
                    .map(|this| {
                        if available {
                            this.cursor_pointer().on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.set_kind(tile_kind, window, cx);
                                },
                            ))
                        } else {
                            this.opacity(0.4)
                                .cursor_not_allowed()
                                .tooltip(|window, cx| {
                                    Tooltip::new("Not available yet").build(window, cx)
                                })
                        }
                    })
                    .child(Icon::new(tile_kind.icon()).large())
                    .child(div().text_xs().child(tile_kind.label()))
            }))
    }

    fn render_host_row(&self) -> impl IntoElement {
        h_flex()
            .gap_2()
            .items_start()
            .child(
                v_flex()
                    .flex_1()
                    .gap_1()
                    .child("Host")
                    .child(Input::new(&self.host)),
            )
            .when(!matches!(self.kind, SessionKind::Telnet), |this| {
                this.child(
                    v_flex()
                        .w(px(180.))
                        .gap_1()
                        .child("Username")
                        .child(Input::new(&self.username)),
                )
            })
            .child(
                v_flex()
                    .w(px(110.))
                    .gap_1()
                    .child("Port")
                    .child(Input::new(&self.port)),
            )
    }

    fn render_serial(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .items_start()
            .child(
                v_flex().flex_1().gap_1().child("Serial Port").child(
                    h_flex()
                        .gap_2()
                        .child(
                            Select::new(&self.serial_port)
                                .placeholder("Select a port")
                                .flex_1(),
                        )
                        .child(
                            Button::new("rescan-ports")
                                .outline()
                                .icon(IconName::Redo2)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.serial_port.update(cx, |state, cx| {
                                        let selected = state.selected_value().cloned();
                                        state.set_items(
                                            port_options(selected.as_deref()),
                                            window,
                                            cx,
                                        );
                                        if let Some(value) = selected {
                                            state.set_selected_value(&value, window, cx);
                                        }
                                    });
                                })),
                        ),
                ),
            )
            .child(
                v_flex()
                    .w(px(110.))
                    .gap_1()
                    .child("Baud Rate")
                    .child(Input::new(&self.baud)),
            )
    }

    fn render_auth(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let key_selected = self
            .private_key
            .read(cx)
            .selected_value()
            .is_some_and(|value| !value.trim().is_empty());
        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child("Password")
                    .child(Input::new(&self.password))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if matches!(self.kind, SessionKind::Ftp) {
                                "Saved encrypted in your system credential store. Leave the \
                                 username empty to log in anonymously"
                            } else {
                                "Saved encrypted in your system credential store"
                            }),
                    ),
            )
            .when(matches!(self.kind, SessionKind::Ftp), |this| {
                this.child(self.render_ftp_options(cx))
            })
            .when(
                matches!(self.kind, SessionKind::Ssh | SessionKind::Sftp),
                |this| {
                    this.child(
                        v_flex().gap_1().child("Private Key (optional)").child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Select::new(&self.private_key)
                                        .search_placeholder("Search keys...")
                                        .menu_max_h(px(220.))
                                        .flex_1(),
                                )
                                .child(
                                    Button::new("browse-key")
                                        .outline()
                                        .icon(IconName::FolderOpen)
                                        .on_click({
                                            let private_key = self.private_key.clone();
                                            move |_, window, cx| {
                                                let rx = cx.prompt_for_paths(PathPromptOptions {
                                                    files: true,
                                                    directories: false,
                                                    multiple: false,
                                                    prompt: None,
                                                });
                                                let private_key = private_key.clone();
                                                window
                                                    .spawn(cx, async move |cx| {
                                                        let Ok(Ok(Some(paths))) = rx.await else {
                                                            return;
                                                        };
                                                        let Some(path) = paths.into_iter().next()
                                                        else {
                                                            return;
                                                        };
                                                        let value = SharedString::from(
                                                            path.display().to_string(),
                                                        );
                                                        let _ = private_key.update_in(
                                                            cx,
                                                            |state, window, cx| {
                                                                state.set_items(
                                                                    SearchableVec::new(
                                                                        key_options(Some(
                                                                            value.as_ref(),
                                                                        )),
                                                                    ),
                                                                    window,
                                                                    cx,
                                                                );
                                                                state.set_selected_value(
                                                                    &value, window, cx,
                                                                );
                                                            },
                                                        );
                                                    })
                                                    .detach();
                                            }
                                        }),
                                ),
                        ),
                    )
                    .when(key_selected, |this| {
                        this.child(
                            v_flex()
                                .gap_1()
                                .child("Key Passphrase")
                                .child(Input::new(&self.passphrase))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            "Only needed if the key is encrypted. Saved \
                                             encrypted in your system credential store",
                                        ),
                                ),
                        )
                    })
                    .child(self.render_legacy_algorithms(cx))
                },
            )
    }

    fn render_legacy_algorithms(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.legacy_algorithms;
        let toggle = |id: &'static str, label: &'static str, value: bool| {
            Button::new(id)
                .xsmall()
                .label(label)
                .when(enabled == value, |b| b.primary())
                .when(enabled != value, |b| b.outline())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.legacy_algorithms = value;
                    this.test_state = TestState::Idle;
                    cx.notify();
                }))
        };

        v_flex()
            .gap_1()
            .child("Legacy Algorithms")
            .child(
                h_flex()
                    .gap_2()
                    .child(toggle("legacy-algorithms-off", "Off", false))
                    .child(toggle("legacy-algorithms-on", "On", true)),
            )
            .child(if enabled {
                cleartext_warning(
                    "Also offers SHA-1 key exchange, CBC ciphers and SHA-1 MACs. They're only \
                     used when the server supports nothing newer, but they're weak: keep this \
                     for old routers, switches and NAS that need it.",
                    cx,
                )
                .into_any_element()
            } else {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Turn on for old devices that fail with \"no matching key exchange \
                         method\"",
                    )
                    .into_any_element()
            })
    }

    fn render_proxy(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child("SOCKS5 Proxy Host")
                            .child(Input::new(&self.proxy_host))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Leave empty to connect directly"),
                            ),
                    )
                    .child(
                        v_flex()
                            .w(px(110.))
                            .gap_1()
                            .child("Port")
                            .child(Input::new(&self.proxy_port)),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child("Username (optional)")
                            .child(Input::new(&self.proxy_username)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child("Password (optional)")
                            .child(Input::new(&self.proxy_password))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Saved encrypted in your system credential store"),
                            ),
                    ),
            )
    }

    fn render_ftp_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let security = self.ftp_security;
        let active = self.ftp_active_mode;
        let security_button = |id: &'static str, label: &'static str, value: FtpSecurity| {
            Button::new(id)
                .xsmall()
                .label(label)
                .when(security == value, |b| b.primary())
                .when(security != value, |b| b.outline())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_ftp_security(value, window, cx);
                }))
        };
        let mode_button = |id: &'static str, label: &'static str, value: bool| {
            Button::new(id)
                .xsmall()
                .label(label)
                .when(active == value, |b| b.primary())
                .when(active != value, |b| b.outline())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ftp_active_mode = value;
                    this.test_state = TestState::Idle;
                    cx.notify();
                }))
        };
        let note = |text: &'static str| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(text)
        };

        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child("Encryption")
                    .child(
                        h_flex()
                            .gap_2()
                            .child(security_button(
                                "ftp-explicit",
                                "Explicit TLS",
                                FtpSecurity::Explicit,
                            ))
                            .child(security_button(
                                "ftp-implicit",
                                "Implicit TLS",
                                FtpSecurity::Implicit,
                            ))
                            .child(security_button("ftp-plain", "None", FtpSecurity::None)),
                    )
                    .child(note(match security {
                        FtpSecurity::Explicit => {
                            "Connects in plain text and upgrades with AUTH TLS before logging \
                             in. The connection is refused if the server can't do TLS"
                        }
                        FtpSecurity::Implicit => "TLS from the first byte, usually on port 990",
                        FtpSecurity::None => "No encryption at all",
                    })),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child("Transfer Mode")
                    .child(
                        h_flex()
                            .gap_2()
                            .child(mode_button("ftp-passive", "Passive", false))
                            .child(mode_button("ftp-active", "Active", true)),
                    )
                    .child(note(if active {
                        "The server connects back to this computer for each transfer. Rarely \
                         works behind a router or firewall, and never through a proxy"
                    } else {
                        "This computer opens the data connections. Works behind most routers \
                         and firewalls"
                    })),
            )
    }

    fn render_monitoring(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.monitoring;
        v_flex()
            .gap_1()
            .child("Resource Monitoring")
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("monitoring-on")
                            .xsmall()
                            .label("On")
                            .when(enabled, |b| b.primary())
                            .when(!enabled, |b| b.outline())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.monitoring = true;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("monitoring-off")
                            .xsmall()
                            .label("Off")
                            .when(!enabled, |b| b.primary())
                            .when(enabled, |b| b.outline())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.monitoring = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Runs a second channel that samples CPU, memory, network and disk once a \
                         second for the whole session. Turn it off on hosts where the extra \
                         commands are unwelcome or audited.",
                    ),
            )
    }

    fn render_label_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let folders = &self.folders;
        let current_folder = self.folder_id;
        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child("Name")
                    .child(Input::new(&self.name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Shown in the sessions list. Leave empty to use host (user)"),
                    ),
            )
            .child(self.selected_icon.clone())
            .child(color_picker(&self.selected_color, cx))
            .child(
                v_flex().gap_1().child("Folder").child(
                    h_flex()
                        .flex_wrap()
                        .gap_1()
                        .child(
                            Button::new("folder-none")
                                .xsmall()
                                .when(current_folder.is_none(), |b| b.primary())
                                .when(current_folder.is_some(), |b| b.outline())
                                .label("No Folder")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.folder_id = None;
                                    cx.notify();
                                })),
                        )
                        .children(folders.iter().map(|folder| {
                            let folder_id = folder.id;
                            Button::new(SharedString::from(format!("folder-{folder_id}")))
                                .xsmall()
                                .when(current_folder == Some(folder_id), |b| b.primary())
                                .when(current_folder != Some(folder_id), |b| b.outline())
                                .label(SharedString::from(folder.name.clone()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.folder_id = Some(folder_id);
                                    cx.notify();
                                }))
                        })),
                ),
            )
    }
}

impl Render for SessionWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        let tabs = SettingsTab::available(kind);
        let active_index = tabs.iter().position(|tab| *tab == self.tab).unwrap_or(0);
        let testing = matches!(self.test_state, TestState::Testing);

        let mut body = v_flex().gap_3().w_full().child(self.render_tiles(cx));
        body = match kind {
            SessionKind::Local => body,
            SessionKind::Serial => body.child(self.render_serial(cx)),
            SessionKind::Ssh
            | SessionKind::Sftp
            | SessionKind::Ftp
            | SessionKind::Telnet
            | SessionKind::Rdp => body
                .child(self.render_host_row())
                .when(matches!(kind, SessionKind::Telnet), |this| {
                    this.child(cleartext_warning(
                        "Telnet carries everything you type, passwords included, in the clear. \
                         Anyone between you and the host can read it.",
                        cx,
                    ))
                })
                .when(
                    matches!(kind, SessionKind::Ftp)
                        && matches!(self.ftp_security, FtpSecurity::None),
                    |this| {
                        this.child(cleartext_warning(
                            "Plain FTP sends the password and every file in the clear. Anyone \
                             between you and the server can read them.",
                            cx,
                        ))
                    },
                ),
        };

        let tab_bar = TabBar::new("session-settings-tabs")
            .underline()
            .selected_index(active_index)
            .on_click(cx.listener(|this, index: &usize, _, cx| {
                if let Some(tab) = SettingsTab::available(this.kind).get(*index).copied() {
                    this.tab = tab;
                    cx.notify();
                }
            }))
            .children(tabs.iter().map(|tab| Tab::new().label(tab.label())));

        let tab_content = match tabs[active_index] {
            SettingsTab::Auth => self.render_auth(cx).into_any_element(),
            SettingsTab::Proxy => self.render_proxy(cx).into_any_element(),
            SettingsTab::Monitoring => self.render_monitoring(cx).into_any_element(),
            SettingsTab::Label => self.render_label_tab(cx).into_any_element(),
        };

        body = body
            .child(tab_bar)
            .child(div().w_full().min_h(px(240.)).child(tab_content));

        body = match &self.test_state {
            TestState::Idle => body,
            TestState::Testing => body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Testing connection..."),
            ),
            TestState::Success(msg) => body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().success)
                    .child(SharedString::from(msg.clone())),
            ),
            TestState::Failed(msg) => body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(SharedString::from(msg.clone())),
            ),
        };

        let footer = h_flex()
            .p_3()
            .gap_2()
            .justify_end()
            .border_t_1()
            .border_color(cx.theme().border)
            .when(!matches!(kind, SessionKind::Local), |this| {
                this.child(
                    Button::new("test-connection")
                        .outline()
                        .label(if testing { "Testing..." } else { "Test" })
                        .disabled(testing)
                        .on_click(cx.listener(|this, _, _, cx| this.run_test(cx))),
                )
            })
            .child(
                Button::new("cancel")
                    .label("Cancel")
                    .on_click(|_, window, _| window.remove_window()),
            )
            .child(
                Button::new("save")
                    .primary()
                    .label("Save")
                    .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
            );

        let body_max = window.viewport_size().height - px(85.);

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            .child(div().flex_1().min_h_0().p_4().child(scrollable_body(
                "session-form",
                body,
                &self.body_scroll,
                body_max,
                cx,
            )))
            .child(footer)
    }
}

fn window_title(editing: bool, kind: SessionKind) -> String {
    format!(
        "{} {} Session",
        if editing { "Edit" } else { "New" },
        kind.label()
    )
}

fn key_options_without_scan(extra: Option<&str>) -> Vec<KeyOption> {
    let mut options = vec![KeyOption::none()];
    if let Some(extra) = extra.filter(|p| !p.trim().is_empty()) {
        options.push(KeyOption::from_path(Path::new(extra)));
    }
    options
}

fn key_options(extra: Option<&str>) -> Vec<KeyOption> {
    let mut options = vec![KeyOption::none()];
    if let Some(ssh_dir) = dirs::home_dir().map(|home| home.join(".ssh"))
        && let Ok(entries) = std::fs::read_dir(ssh_dir)
    {
        let mut found: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_private_key(path))
            .collect();
        found.sort();
        options.extend(found.iter().map(|path| KeyOption::from_path(path)));
    }
    if let Some(extra) = extra.filter(|p| !p.trim().is_empty())
        && !options.iter().any(|o| o.path.as_ref() == extra)
    {
        options.push(KeyOption::from_path(Path::new(extra)));
    }
    options
}

fn is_private_key(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    if matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("pub") | Some("ppk")
    ) {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 48];
    let Ok(n) = file.read(&mut head) else {
        return false;
    };
    let head = String::from_utf8_lossy(&head[..n]);
    head.starts_with("-----BEGIN") && head.contains("PRIVATE KEY")
}

fn port_options(extra: Option<&str>) -> Vec<PortOption> {
    let mut options: Vec<PortOption> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|port| PortOption {
            name: SharedString::from(port.port_name),
            detail: SharedString::from(match port.port_type {
                SerialPortType::UsbPort(usb) => usb
                    .product
                    .or(usb.manufacturer)
                    .unwrap_or_else(|| "USB serial device".to_string()),
                SerialPortType::BluetoothPort => "Bluetooth".to_string(),
                SerialPortType::PciPort => "PCI".to_string(),
                SerialPortType::Unknown => String::new(),
            }),
        })
        .collect();
    options.sort_by(|a, b| a.name.cmp(&b.name));
    extra_port(options, extra)
}

fn extra_port(mut options: Vec<PortOption>, extra: Option<&str>) -> Vec<PortOption> {
    if let Some(extra) = extra.filter(|p| !p.trim().is_empty())
        && !options.iter().any(|o| o.name.as_ref() == extra)
    {
        options.push(PortOption {
            name: extra.to_string().into(),
            detail: "Not detected".into(),
        });
    }
    options
}

fn proxy_config(
    kind: SessionKind,
    host: &str,
    port: &str,
    username: &str,
    password: &str,
) -> Option<ProxyConfig> {
    if host.is_empty()
        || !matches!(
            kind,
            SessionKind::Ssh | SessionKind::Sftp | SessionKind::Ftp | SessionKind::Telnet
        )
    {
        return None;
    }
    Some(ProxyConfig {
        host: host.to_string(),
        port: port.parse().unwrap_or_else(|_| default_proxy_port()),
        username: username.to_string(),
        password: SecretString::from(password),
    })
}

fn run_connection_test(
    kind: SessionKind,
    host: String,
    port: u16,
    credentials: SshCredentials,
    proxy: Option<ProxyConfig>,
    baud_rate: u32,
    ftp: Option<crate::sftp::FtpTarget>,
) -> async_channel::Receiver<Result<String, String>> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match kind {
            SessionKind::Local => Ok("Local shell needs no connection".to_string()),
            SessionKind::Serial => serialport::new(host.as_str(), baud_rate)
                .timeout(Duration::from_millis(1500))
                .open()
                .map(|_| format!("Opened {host} at {baud_rate} baud"))
                .map_err(|e| format!("Could not open {host}: {e}")),
            SessionKind::Telnet => stream_check(host, port, proxy),
            SessionKind::Rdp => tcp_check(&host, port),
            SessionKind::Ssh | SessionKind::Sftp => ssh_check(host, port, credentials, proxy),
            SessionKind::Ftp => match ftp {
                Some(target) => crate::sftp::check_ftp(target),
                None => Err("Missing FTP settings".to_string()),
            },
        };
        let _ = tx.send_blocking(result);
    });
    rx
}

fn stream_check(host: String, port: u16, proxy: Option<ProxyConfig>) -> Result<String, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        crate::proxy::open_stream(&host, port, proxy.as_ref(), Duration::from_secs(5))
            .await
            .map_err(|e| e.to_string())?;
        Ok(match proxy {
            Some(proxy) => format!(
                "{host}:{port} is reachable through {}:{}",
                proxy.host, proxy.port
            ),
            None => format!("{host}:{port} is reachable"),
        })
    })
}

fn tcp_check(host: &str, port: u16) -> Result<String, String> {
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("Could not resolve {host}: {e}"))?
        .collect();
    let mut last_err = format!("Could not resolve {host}");
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            Ok(_) => return Ok(format!("{host}:{port} is reachable")),
            Err(e) => last_err = format!("{host}:{port} unreachable: {e}"),
        }
    }
    Err(last_err)
}

fn ssh_check(
    host: String,
    port: u16,
    credentials: SshCredentials,
    proxy: Option<ProxyConfig>,
) -> Result<String, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let connect = crate::ssh_client::connect(host.clone(), port, credentials, proxy);
        match connect.await {
            Err(e) => Err(e.to_string()),
            Ok(handle) => {
                let _ = handle
                    .disconnect(russh::Disconnect::ByApplication, "", "")
                    .await;
                Ok(format!("Authenticated to {host}:{port}"))
            }
        }
    })
}

pub fn open_new_folder_dialog(weak_app: gpui_kit::WeakEntity<OxidalApp>, cx: &mut App) {
    open_folder_window(None, weak_app, cx);
}

pub fn open_edit_folder_dialog(
    folder: SessionFolder,
    weak_app: gpui_kit::WeakEntity<OxidalApp>,
    cx: &mut App,
) {
    open_folder_window(Some(folder), weak_app, cx);
}

fn open_folder_window(
    existing: Option<SessionFolder>,
    weak_app: gpui_kit::WeakEntity<OxidalApp>,
    cx: &mut App,
) {
    let title = if existing.is_some() {
        "Edit Folder"
    } else {
        "New Folder"
    };
    crate::dialog_window::open_with(title, px(420.), cx, move |window, cx| {
        let folder_id = existing.as_ref().map(|folder| folder.id);
        let name = cx.new(|cx| {
            let state = InputState::new(window, cx).placeholder("Folder name");
            match &existing {
                Some(folder) => state.default_value(folder.name.clone()),
                None => state,
            }
        });
        crate::forms::confirm_on_enter(&name, window, cx);
        name.read(cx).focus_handle(cx).focus(window, cx);
        let icon = existing.as_ref().and_then(|folder| folder.icon);
        let selected_icon = cx.new(|cx| SelectedIcon::new(icon, window, cx));
        let color = existing
            .as_ref()
            .map_or(ItemColor::Default, |folder| folder.color);
        let selected_color = cx.new(|_cx| SelectedColor(color));

        let do_save: Rc<dyn Fn(&mut App)> = {
            let name = name.clone();
            let selected_icon = selected_icon.clone();
            let selected_color = selected_color.clone();
            Rc::new(move |cx: &mut App| {
                let value = name.read(cx).value().trim().to_string();
                if value.is_empty() {
                    return;
                }
                let mut folder = SessionFolder::new(value);
                folder.icon = selected_icon.read(cx).icon;
                folder.color = selected_color.read(cx).0;
                let _ = weak_app.update(cx, |app, cx| match folder_id {
                    Some(id) => {
                        folder.id = id;
                        app.update_folder(folder, cx);
                    }
                    None => app.add_folder(folder, cx),
                });
            })
        };

        move |dialog, _window, cx| {
            dialog
                .child(v_flex().gap_1().child("Name").child(Input::new(&name)))
                .child(selected_icon.clone())
                .child(color_picker(&selected_color, cx))
                .button(
                    Button::new("cancel")
                        .label("Cancel")
                        .on_click(|_, window, _| window.remove_window()),
                )
                .button(Button::new("save").primary().label("Save").on_click({
                    let do_save = do_save.clone();
                    move |_, window, cx| {
                        do_save(cx);
                        window.remove_window();
                    }
                }))
                .on_ok({
                    let do_save = do_save.clone();
                    move |_, cx| do_save(cx)
                })
        }
    });
}
