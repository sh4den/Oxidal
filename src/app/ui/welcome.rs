use crate::session::{Session, SessionKind};

use gpui_kit::{
    Context, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    px,
};

use gpui_kit::component::button::Button;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, button::ButtonVariants as _, h_flex,
    scroll::ScrollableElement as _, v_flex,
};

use crate::app::OxidalApp;

const WELCOME_SESSIONS: usize = 6;

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

impl OxidalApp {
    pub(super) fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self
            .selected_session
            .and_then(|id| self.sessions.iter().find(|s| s.id == id));
        let muted = cx.theme().muted_foreground;
        let in_sidebar_order = self
            .folders
            .iter()
            .map(|folder| Some(folder.id))
            .chain([None])
            .flat_map(|folder_id| {
                self.sessions
                    .iter()
                    .filter(move |s| s.folder_id == folder_id)
            });
        let cards: Vec<gpui_kit::AnyElement> = in_sidebar_order
            .take(WELCOME_SESSIONS)
            .map(|session| self.render_welcome_card(session, cx).into_any_element())
            .collect();
        let hidden = self.sessions.len().saturating_sub(WELCOME_SESSIONS);

        let actions = h_flex()
            .gap_2()
            .justify_center()
            .when_some(selected, |this, session| {
                let id = session.id;
                this.child(
                    Button::new("connect")
                        .primary()
                        .icon(IconName::SquareTerminal)
                        .label(SharedString::from(format!("Connect to {}", session.name)))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.connect_session(id, window, cx);
                        })),
                )
            })
            .child(
                Button::new("welcome-new-session")
                    .when(selected.is_none(), |b| b.primary())
                    .when(selected.is_some(), |b| b.outline())
                    .icon(IconName::Plus)
                    .label("New Session")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.open_session_window(None, cx);
                    })),
            );

        let sessions = (!cards.is_empty()).then(|| {
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(welcome_section("SESSIONS", cx))
                        .child(div().flex_1().h(px(1.)).bg(cx.theme().border)),
                )
                .child(div().grid().grid_cols(2).gap_2().children(cards))
                .when(hidden > 0, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(SharedString::from(format!("{hidden} more in the sidebar"))),
                    )
                })
        });

        v_flex()
            .id("welcome")
            .flex_1()
            .min_w_0()
            .h_full()
            .px_6()
            .py_8()
            .child(
                v_flex()
                    .my_auto()
                    .mx_auto()
                    .w_full()
                    .max_w(px(640.))
                    .items_center()
                    .gap_8()
                    .child(
                        v_flex()
                            .items_center()
                            .gap_4()
                            .child(welcome_logo(cx))
                            .child(
                                v_flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_2xl()
                                            .font_weight(FontWeight::BOLD)
                                            .child("Oxidal"),
                                    )
                                    .child(div().text_sm().text_color(muted).text_center().child(
                                        "SSH, SFTP, Telnet, serial and local shells in one window",
                                    )),
                            )
                            .child(actions),
                    )
                    .children(sessions)
                    .child(render_shortcuts(cx)),
            )
            .overflow_y_scrollbar()
    }

    fn render_welcome_card(&self, session: &Session, cx: &mut Context<Self>) -> impl IntoElement {
        let id = session.id;
        let theme = cx.theme();
        let color = session.color.hsla();
        let selected = self.selected_session == Some(id);
        let hover_border = color.unwrap_or(theme.primary).opacity(0.6);

        h_flex()
            .id(SharedString::from(format!("welcome-session-{id}")))
            .min_w_0()
            .items_center()
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if selected {
                theme.primary
            } else {
                theme.border
            })
            .bg(theme.muted.opacity(0.35))
            .cursor_pointer()
            .hover(move |style| style.border_color(hover_border))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.connect_session(id, window, cx);
            }))
            .child(
                div()
                    .flex_none()
                    .size(px(32.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(match color {
                        Some(color) => color.opacity(0.18),
                        None => theme.foreground.opacity(0.08),
                    })
                    .child(
                        Icon::empty()
                            .path(session.display_icon())
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
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(SharedString::from(session.name.clone())),
                            )
                            .when(self.is_open(id), |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .size(px(6.))
                                        .rounded_full()
                                        .bg(theme.success),
                                )
                            }),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(SharedString::from(match session.kind {
                                SessionKind::Local => session.detail(),
                                kind => format!("{} · {}", kind.label(), session.detail()),
                            })),
                    ),
            )
    }
}

fn welcome_logo(cx: &gpui_kit::App) -> impl IntoElement {
    let theme = cx.theme();
    let bg = theme.background;
    let dark = bg.l < 0.5;
    let glow = |alpha: f32| {
        if dark {
            Hsla {
                l: (bg.l + 0.55).min(1.),
                a: alpha,
                ..bg
            }
        } else {
            Hsla {
                l: (bg.l - 0.7).max(0.),
                a: alpha * 0.6,
                ..bg
            }
        }
    };
    let layer = |alpha: f32, y: f32, blur: f32, spread: f32| gpui_kit::BoxShadow {
        color: glow(alpha),
        offset: gpui_kit::point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    };
    let radius = px(22.);

    div()
        .relative()
        .size(px(96.))
        .rounded(radius)
        .shadow(if dark {
            vec![layer(0.22, 0., 16., 1.), layer(0.28, 0., 80., 16.)]
        } else {
            vec![layer(0.35, 2., 6., 0.), layer(0.5, 12., 32., 0.)]
        })
        .child(
            gpui_kit::img(crate::assets::LOGO)
                .size_full()
                .rounded(radius),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(radius)
                .border_1()
                .border_color(theme.foreground.opacity(if dark { 0.14 } else { 0.08 })),
        )
}

fn welcome_section(title: &'static str, cx: &gpui_kit::App) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(title)
}

fn render_shortcuts(cx: &gpui_kit::App) -> impl IntoElement {
    let section = |title: &'static str| welcome_section(title, cx);

    v_flex()
        .w_full()
        .p_4()
        .gap_3()
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

fn shortcut(keys: &[&str], label: &'static str, cx: &gpui_kit::App) -> impl IntoElement {
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
