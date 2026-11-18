use crate::settings::AppSettings;
use gpui_kit::assets::IconName as Lucide;

use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window,
    div, prelude::FluentBuilder as _, px,
};

use gpui_kit::component::button::Button;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, button::ButtonVariants as _,
    resizable::ResizableState, tooltip::Tooltip, v_flex,
};

use crate::app::{OxidalApp, SidebarMode};

pub(in crate::app) struct SidebarPane {
    app: WeakEntity<OxidalApp>,
}

impl SidebarPane {
    fn new(
        app: &Entity<OxidalApp>,
        sidebar_state: &Entity<ResizableState>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(app, |_, _, cx| cx.notify()).detach();
        cx.observe(sidebar_state, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<AppSettings>(|_, cx| cx.notify())
            .detach();
        Self {
            app: app.downgrade(),
        }
    }
}

impl Render for SidebarPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.app
            .update(cx, |app, cx| {
                if app.effective_sidebar_mode() == SidebarMode::Explorer {
                    app.render_explorer_panel(cx).into_any_element()
                } else {
                    app.render_sessions_panel(cx).into_any_element()
                }
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl OxidalApp {
    pub(super) fn sidebar_pane(&mut self, cx: &mut Context<Self>) -> Entity<SidebarPane> {
        if let Some(pane) = &self.sidebar_pane {
            return pane.clone();
        }
        let app = cx.entity();
        let sidebar_state = self.sidebar_state.clone();
        let pane = cx.new(|cx| SidebarPane::new(&app, &sidebar_state, cx));
        self.sidebar_pane = Some(pane.clone());
        pane
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

    pub(super) fn sidebar_width(&self, cx: &gpui_kit::App) -> gpui_kit::Pixels {
        self.sidebar_state
            .read(cx)
            .sizes()
            .first()
            .copied()
            .filter(|width| f32::from(*width) > 0.)
            .unwrap_or_else(|| px(cx.global::<AppSettings>().sidebar_width))
    }

    fn effective_sidebar_mode(&self) -> SidebarMode {
        match self.sidebar_mode {
            SidebarMode::Explorer if self.active_sftp_panel().is_none() => SidebarMode::Sessions,
            mode => mode,
        }
    }

    fn render_rail_item(
        &self,
        id: &'static str,
        icon: impl Into<Icon>,
        tooltip: &'static str,
        active: bool,
        mode: SidebarMode,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (fg, bg) = if active {
            (theme.primary, theme.primary.opacity(0.14))
        } else {
            (theme.muted_foreground, gpui_kit::transparent_black())
        };
        let hover_bg = if active {
            theme.primary.opacity(0.2)
        } else {
            theme.sidebar_accent
        };
        let hover_fg = if active {
            theme.primary
        } else {
            theme.sidebar_accent_foreground
        };

        div()
            .relative()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(10.))
                    .bottom(px(10.))
                    .w(px(3.))
                    .rounded_r_sm()
                    .when(active, |this| this.bg(theme.primary)),
            )
            .child(
                div()
                    .id(id)
                    .size(px(44.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_lg()
                    .cursor_pointer()
                    .bg(bg)
                    .text_color(fg)
                    .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
                    .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
                    .child(icon.into().with_size(px(22.)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.set_sidebar_mode(mode, cx);
                    })),
            )
    }

    pub(super) fn render_sidebar_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(self.render_rail_item(
                "sidebar-sessions",
                Icon::new(Lucide::List),
                "Sessions",
                sessions_active,
                SidebarMode::Sessions,
                cx,
            ))
            .when(has_explorer, |this| {
                this.child(self.render_rail_item(
                    "sidebar-explorer",
                    IconName::FolderOpen,
                    "File Explorer",
                    explorer_active,
                    SidebarMode::Explorer,
                    cx,
                ))
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
}
