use gpui::{
    div, prelude::FluentBuilder as _, px, AppContext as _, Context, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    tab::{Tab, TabBar},
    v_flex, ActiveTheme as _, Icon, IconName, Root, Sizable as _, TitleBar,
};
use uuid::Uuid;

use crate::session::{self, Session, SessionKind};
use crate::session_dialog;
use crate::settings_view::SettingsView;
use crate::terminal::{self, TerminalView};

const TERM_ROWS: usize = 32;
const TERM_COLS: usize = 110;

enum TabContent {
    Terminal(Entity<TerminalView>),
    Settings(Entity<SettingsView>),
    Message(SharedString),
}

struct OpenTab {
    session_id: Option<Uuid>,
    title: SharedString,
    icon: IconName,
    content: TabContent,
}

/// Root application view: title bar, sessions sidebar, tabbed workspace and status bar.
pub struct OxidalApp {
    sessions: Vec<Session>,
    selected_session: Option<Uuid>,
    tabs: Vec<OpenTab>,
    active_tab: Option<usize>,
}

impl OxidalApp {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            sessions: session::load_sessions(),
            selected_session: None,
            tabs: Vec::new(),
            active_tab: None,
        }
    }

    pub fn add_session(&mut self, new_session: Session, cx: &mut Context<Self>) {
        self.sessions.push(new_session);
        session::save_sessions(&self.sessions);
        cx.notify();
    }

    fn delete_session(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.sessions.retain(|s| s.id != id);
        session::save_sessions(&self.sessions);
        if self.selected_session == Some(id) {
            self.selected_session = None;
        }
        let tab_count_before = self.tabs.len();
        self.tabs.retain(|t| t.session_id != Some(id));
        if self.tabs.len() != tab_count_before {
            self.active_tab = if self.tabs.is_empty() { None } else { Some(0) };
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
            SessionKind::Local => match terminal::local::spawn(TERM_ROWS as u16, TERM_COLS as u16) {
                Ok(backend) => TabContent::Terminal(
                    cx.new(|cx| TerminalView::new(backend, TERM_ROWS, TERM_COLS, window, cx)),
                ),
                Err(err) => TabContent::Message(
                    format!("Failed to start local shell: {err}").into(),
                ),
            },
            SessionKind::Ssh => {
                let backend = terminal::ssh::spawn(
                    target.host.clone(),
                    target.port,
                    target.username.clone(),
                    target.password.clone(),
                    target.private_key_path.clone(),
                    TERM_ROWS as u16,
                    TERM_COLS as u16,
                );
                TabContent::Terminal(
                    cx.new(|cx| TerminalView::new(backend, TERM_ROWS, TERM_COLS, window, cx)),
                )
            }
            SessionKind::Serial => match terminal::serial::spawn(target.host.clone(), target.baud_rate) {
                Ok(backend) => TabContent::Terminal(
                    cx.new(|cx| TerminalView::new(backend, TERM_ROWS, TERM_COLS, window, cx)),
                ),
                Err(err) => TabContent::Message(format!("Failed to open serial port: {err}").into()),
            },
            SessionKind::Sftp => TabContent::Message(
                "SFTP file browsing isn't implemented yet — only terminal sessions work so far.".into(),
            ),
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
            Some(active) if active == index => {
                Some(index.min(self.tabs.len().saturating_sub(1)))
            }
            other => other,
        };
        if self.tabs.is_empty() {
            self.active_tab = None;
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
                    .child(
                        Button::new("new-session")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .label("Session")
                            .on_click(cx.listener(|_, _, window, cx| {
                                session_dialog::open_new_session_dialog(window, cx);
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
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.sessions.iter().map(|item| {
            let id = item.id;
            let selected = self.selected_session == Some(id);
            let supported = item.kind.is_supported();

            h_flex()
                .id(SharedString::from(format!("session-{id}")))
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .mx_1()
                .rounded_md()
                .cursor_pointer()
                .when(selected, |this| {
                    this.bg(cx.theme().sidebar_accent)
                        .text_color(cx.theme().sidebar_accent_foreground)
                })
                .on_click(cx.listener(move |view, event: &gpui::ClickEvent, window, cx| {
                    if event.click_count() >= 2 {
                        view.connect_session(id, window, cx);
                    } else {
                        view.selected_session = Some(id);
                        cx.notify();
                    }
                }))
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
                    div()
                        .text_xs()
                        .when(!supported, |this| this.text_color(cx.theme().muted_foreground))
                        .child(item.kind.label()),
                )
                .child(
                    Button::new(SharedString::from(format!("connect-{id}")))
                        .ghost()
                        .xsmall()
                        .icon(IconName::SquareTerminal)
                        .tooltip("Connect")
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.connect_session(id, window, cx);
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
                )
        });

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
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Sessions"))
                    .child(
                        Button::new("add")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|_, _, window, cx| {
                                session_dialog::open_new_session_dialog(window, cx);
                            })),
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
            TabContent::Settings(view) => view.clone().into_any_element(),
            TabContent::Message(msg) => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Icon::new(IconName::TriangleAlert).with_size(px(32.)))
                .child(div().text_sm().max_w(px(420.)).text_center().child(msg.clone()))
                .into_any_element(),
        });

        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(cx.theme().background)
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
            .bg(cx.theme().background)
            .items_center()
            .justify_center()
            .gap_3()
            .child(Icon::new(IconName::SquareTerminal).with_size(px(48.)))
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Oxidal Terminal"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(match selected {
                        Some(s) => SharedString::from(format!("Ready to connect: {}", s.detail())),
                        None => SharedString::from("Select a session on the left, or add a new one"),
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
            .child(div().child("Oxidal 0.1.0"))
    }
}

impl Render for OxidalApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(self.render_workspace(cx)),
            )
            .child(self.render_status_bar(cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}
