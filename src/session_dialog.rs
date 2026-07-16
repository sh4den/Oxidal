use gpui::{
    App, AppContext as _, Context, ParentElement as _, SharedString, Styled as _, Window,
    prelude::FluentBuilder as _,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    h_flex,
    input::{Input, InputState},
    v_flex, Sizable as _, WindowExt as _,
};
use uuid::Uuid;

use crate::app::OxidalApp;
use crate::session::{Session, SessionFolder, SessionKind};

/// Transient dialog-only state for the folder picker: which folder (if any)
/// the session-in-progress is currently assigned to.
struct SelectedFolder(Option<Uuid>);

/// Open the "choose a kind" step of the new-session flow.
pub fn open_new_session_dialog(
    folders: Vec<SessionFolder>,
    window: &mut Window,
    cx: &mut Context<OxidalApp>,
) {
    let weak_app = cx.entity().downgrade();

    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog.title("New Session").child(
            v_flex()
                .gap_2()
                .w(gpui::px(320.))
                .children(SessionKind::ALL.iter().map(|kind| {
                    let kind = *kind;
                    let weak_app = weak_app.clone();
                    let folders = folders.clone();
                    Button::new(SharedString::from(format!("new-session-{}", kind.label())))
                        .w_full()
                        .outline()
                        .icon(kind.icon())
                        .label(kind.label())
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            open_session_details_dialog(
                                kind,
                                None,
                                folders.clone(),
                                weak_app.clone(),
                                window,
                                cx,
                            );
                        })
                })),
        )
    });
}

/// Open the details step pre-filled with an existing session's values.
pub fn open_edit_session_dialog(
    session: Session,
    folders: Vec<SessionFolder>,
    weak_app: gpui::WeakEntity<OxidalApp>,
    window: &mut Window,
    cx: &mut App,
) {
    let kind = session.kind;
    open_session_details_dialog(kind, Some(session), folders, weak_app, window, cx);
}

fn open_session_details_dialog(
    kind: SessionKind,
    existing: Option<Session>,
    folders: Vec<SessionFolder>,
    weak_app: gpui::WeakEntity<OxidalApp>,
    window: &mut Window,
    cx: &mut App,
) {
    let editing_id = existing.as_ref().map(|s| s.id);
    let is_edit = editing_id.is_some();

    let name = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(existing.as_ref().map(|s| s.name.as_str()).unwrap_or(kind.label()))
    });
    let host = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(existing.as_ref().map(|s| s.host.clone()).unwrap_or_default())
            .placeholder(if matches!(kind, SessionKind::Serial) {
                "COM3"
            } else {
                "example.com"
            })
    });
    let port = cx.new(|cx| {
        InputState::new(window, cx).default_value(
            existing
                .as_ref()
                .map(|s| s.port)
                .unwrap_or_else(|| kind.default_port())
                .to_string(),
        )
    });
    let username = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(existing.as_ref().map(|s| s.username.clone()).unwrap_or_default())
            .placeholder("username")
    });
    let password = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(existing.as_ref().map(|s| s.password.clone()).unwrap_or_default())
            .masked(true)
    });
    let baud = cx.new(|cx| {
        InputState::new(window, cx).default_value(
            existing
                .as_ref()
                .map(|s| s.baud_rate)
                .unwrap_or(115_200)
                .to_string(),
        )
    });
    let private_key = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(
                existing
                    .as_ref()
                    .and_then(|s| s.private_key_path.clone())
                    .unwrap_or_default(),
            )
            .placeholder("C:\\Users\\me\\.ssh\\id_ed25519 (optional)")
    });
    let selected_folder =
        cx.new(|_cx| SelectedFolder(existing.as_ref().and_then(|s| s.folder_id)));

    window.open_dialog(cx, move |dialog, _window, cx| {
        let weak_app = weak_app.clone();
        let name = name.clone();
        let host = host.clone();
        let port = port.clone();
        let username = username.clone();
        let password = password.clone();
        let baud = baud.clone();
        let private_key = private_key.clone();
        let selected_folder = selected_folder.clone();
        let current_folder = selected_folder.read(cx).0;

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

        body = body.child(
            v_flex().gap_1().child("Folder").child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .child({
                        let selected_folder = selected_folder.clone();
                        Button::new("folder-none")
                            .xsmall()
                            .when(current_folder.is_none(), |b| b.primary())
                            .when(current_folder.is_some(), |b| b.outline())
                            .label("No Folder")
                            .on_click(move |_, _, cx| {
                                selected_folder.update(cx, |s, cx| {
                                    s.0 = None;
                                    cx.notify();
                                });
                            })
                    })
                    .children(folders.iter().map(|folder| {
                        let folder_id = folder.id;
                        let selected_folder = selected_folder.clone();
                        Button::new(SharedString::from(format!("folder-{folder_id}")))
                            .xsmall()
                            .when(current_folder == Some(folder_id), |b| b.primary())
                            .when(current_folder != Some(folder_id), |b| b.outline())
                            .label(SharedString::from(folder.name.clone()))
                            .on_click(move |_, _, cx| {
                                selected_folder.update(cx, |s, cx| {
                                    s.0 = Some(folder_id);
                                    cx.notify();
                                });
                            })
                    })),
            ),
        );

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
                        if let Some(id) = editing_id {
                            session.id = id;
                        }
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
                        session.folder_id = selected_folder.read(cx).0;

                        let _ = weak_app.update(cx, |app, cx| {
                            if editing_id.is_some() {
                                app.update_session(session, cx);
                            } else {
                                app.add_session(session, cx);
                            }
                        });
                        window.close_dialog(cx);
                    },
                ),
            );

        dialog
            .title(if is_edit {
                format!("Edit {} Session", kind.label())
            } else {
                format!("New {} Session", kind.label())
            })
            .child(body)
            .footer(footer)
    });
}

/// Open a small dialog to create a new session folder.
pub fn open_new_folder_dialog(
    weak_app: gpui::WeakEntity<OxidalApp>,
    window: &mut Window,
    cx: &mut App,
) {
    let name = cx.new(|cx| InputState::new(window, cx).placeholder("Folder name"));

    window.open_dialog(cx, move |dialog, _window, _cx| {
        let weak_app = weak_app.clone();
        let name = name.clone();

        let body = v_flex()
            .gap_1()
            .w(gpui::px(320.))
            .child("Name")
            .child(Input::new(&name));

        let footer = DialogFooter::new()
            .child(Button::new("cancel").label("Cancel").on_click(
                |_, window, cx| {
                    window.close_dialog(cx);
                },
            ))
            .child(
                Button::new("save").primary().label("Save").on_click(
                    move |_, window, cx| {
                        let value = name.read(cx).value().to_string();
                        if !value.trim().is_empty() {
                            let _ = weak_app.update(cx, |app, cx| {
                                app.add_folder(SessionFolder::new(value), cx);
                            });
                        }
                        window.close_dialog(cx);
                    },
                ),
            );

        dialog.title("New Folder").child(body).footer(footer)
    });
}

/// Open a small dialog to rename an existing session folder.
pub fn open_edit_folder_dialog(
    folder: SessionFolder,
    weak_app: gpui::WeakEntity<OxidalApp>,
    window: &mut Window,
    cx: &mut App,
) {
    let folder_id = folder.id;
    let name = cx.new(|cx| InputState::new(window, cx).default_value(folder.name.clone()));

    window.open_dialog(cx, move |dialog, _window, _cx| {
        let weak_app = weak_app.clone();
        let name = name.clone();

        let body = v_flex()
            .gap_1()
            .w(gpui::px(320.))
            .child("Name")
            .child(Input::new(&name));

        let footer = DialogFooter::new()
            .child(Button::new("cancel").label("Cancel").on_click(
                |_, window, cx| {
                    window.close_dialog(cx);
                },
            ))
            .child(
                Button::new("save").primary().label("Save").on_click(
                    move |_, window, cx| {
                        let value = name.read(cx).value().to_string();
                        if !value.trim().is_empty() {
                            let _ = weak_app.update(cx, |app, cx| {
                                app.rename_folder(folder_id, value, cx);
                            });
                        }
                        window.close_dialog(cx);
                    },
                ),
            );

        dialog.title("Rename Folder").child(body).footer(footer)
    });
}
