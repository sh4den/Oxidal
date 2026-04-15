use gpui::{App, AppContext as _, Context, ParentElement as _, SharedString, Styled as _, Window};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    input::{Input, InputState},
    v_flex, WindowExt as _,
};

use crate::app::OxidalApp;
use crate::session::{Session, SessionKind};

/// Open the "choose a kind" step of the new-session flow.
pub fn open_new_session_dialog(window: &mut Window, cx: &mut Context<OxidalApp>) {
    let weak_app = cx.entity().downgrade();

    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog.title("New Session").child(
            v_flex()
                .gap_2()
                .w(gpui::px(320.))
                .children(SessionKind::ALL.iter().map(|kind| {
                    let kind = *kind;
                    let weak_app = weak_app.clone();
                    Button::new(SharedString::from(format!("new-session-{}", kind.label())))
                        .w_full()
                        .outline()
                        .icon(kind.icon())
                        .label(kind.label())
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            open_session_details_dialog(kind, weak_app.clone(), window, cx);
                        })
                })),
        )
    });
}

fn open_session_details_dialog(
    kind: SessionKind,
    weak_app: gpui::WeakEntity<OxidalApp>,
    window: &mut Window,
    cx: &mut App,
) {
    let name = cx.new(|cx| InputState::new(window, cx).default_value(kind.label()));
    let host = cx.new(|cx| {
        InputState::new(window, cx).placeholder(if matches!(kind, SessionKind::Serial) {
            "COM3"
        } else {
            "example.com"
        })
    });
    let port = cx.new(|cx| InputState::new(window, cx).default_value(kind.default_port().to_string()));
    let username = cx.new(|cx| InputState::new(window, cx).placeholder("username"));
    let password = cx.new(|cx| InputState::new(window, cx).masked(true));
    let baud = cx.new(|cx| InputState::new(window, cx).default_value("115200"));
    let private_key = cx.new(|cx| {
        InputState::new(window, cx).placeholder("C:\\Users\\me\\.ssh\\id_ed25519 (optional)")
    });

    window.open_dialog(cx, move |dialog, _window, _cx| {
        let weak_app = weak_app.clone();
        let name = name.clone();
        let host = host.clone();
        let port = port.clone();
        let username = username.clone();
        let password = password.clone();
        let baud = baud.clone();
        let private_key = private_key.clone();

        let mut body = v_flex()
            .gap_3()
            .w(gpui::px(360.))
            .child(v_flex().gap_1().child("Name").child(Input::new(&name)));

        body = match kind {
            SessionKind::Local => body,
            SessionKind::Serial => body
                .child(
                    v_flex()
                        .gap_1()
                        .child("Serial Port")
                        .child(Input::new(&host)),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child("Baud Rate")
                        .child(Input::new(&baud)),
                ),
            SessionKind::Ssh => body
                .child(v_flex().gap_1().child("Host").child(Input::new(&host)))
                .child(v_flex().gap_1().child("Port").child(Input::new(&port)))
                .child(
                    v_flex()
                        .gap_1()
                        .child("Username")
                        .child(Input::new(&username)),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child("Password")
                        .child(Input::new(&password)),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child("Private Key File (optional)")
                        .child(Input::new(&private_key)),
                ),
            SessionKind::Sftp | SessionKind::Rdp => body
                .child(v_flex().gap_1().child("Host").child(Input::new(&host)))
                .child(v_flex().gap_1().child("Port").child(Input::new(&port)))
                .child(
                    v_flex()
                        .gap_1()
                        .child("Username")
                        .child(Input::new(&username)),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child("Password")
                        .child(Input::new(&password)),
                ),
        };

        let footer = DialogFooter::new()
            .child(Button::new("cancel").label("Cancel").on_click(
                |_, window, cx| {
                    window.close_dialog(cx);
                },
            ))
            .child(
                Button::new("save").primary().label("Save").on_click(
                    move |_, window, cx| {
                        let mut session = Session::new(name.read(cx).value().to_string(), kind);
                        session.host = host.read(cx).value().to_string();
                        session.port = port
                            .read(cx)
                            .value()
                            .to_string()
                            .parse()
                            .unwrap_or_else(|_| kind.default_port());
                        session.username = username.read(cx).value().to_string();
                        session.password = password.read(cx).value().to_string();
                        session.baud_rate = baud
                            .read(cx)
                            .value()
                            .to_string()
                            .parse()
                            .unwrap_or(115_200);
                        let key_path = private_key.read(cx).value().to_string();
                        session.private_key_path = if key_path.trim().is_empty() {
                            None
                        } else {
                            Some(key_path)
                        };

                        let _ = weak_app.update(cx, |app, cx| {
                            app.add_session(session, cx);
                        });
                        window.close_dialog(cx);
                    },
                ),
            );

        dialog
            .title(format!("New {} Session", kind.label()))
            .child(body)
            .footer(footer)
    });
}
