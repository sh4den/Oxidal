use std::io::Read as _;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    h_flex,
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectItem, SelectState},
    v_flex,
};
use secrecy::{ExposeSecret as _, SecretString};
use serialport::SerialPortType;
use uuid::Uuid;

use crate::app::OxidalApp;
use crate::session::{Session, SessionFolder, SessionKind};

/// Transient dialog-only state for the folder picker: which folder (if any)
/// the session-in-progress is currently assigned to.
struct SelectedFolder(Option<Uuid>);

/// Transient dialog-only state: which session kind tile is selected.
struct SelectedKind(SessionKind);

/// Outcome of the "Test Connection" button, shown as a status line.
#[derive(Clone)]
enum TestState {
    Idle,
    Testing,
    Success(String),
    Failed(String),
}

struct TestStatus(TestState);

/// One entry in the private-key picker: a key discovered in `~/.ssh`, a
/// hand-picked file, or the "no key" placeholder (empty path).
#[derive(Clone, PartialEq)]
struct KeyOption {
    label: SharedString,
    path: SharedString,
}

impl KeyOption {
    fn none() -> Self {
        Self {
            label: "No private key".into(),
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
            v_flex()
                .child(self.label.clone())
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.path.clone()),
                )
                .into_any_element()
        }
    }
}

/// One entry in the serial-port picker: the port name plus a human-readable
/// description of the device behind it, when the platform reports one.
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

/// Open the MobaXterm-style session dialog: kind tiles on top, settings below.
pub fn open_new_session_dialog(
    folders: Vec<SessionFolder>,
    window: &mut Window,
    cx: &mut Context<OxidalApp>,
) {
    let weak_app = cx.entity().downgrade();
    open_session_dialog(None, folders, weak_app, window, cx);
}

/// Open the session dialog pre-filled with an existing session's values.
pub fn open_edit_session_dialog(
    session: Session,
    folders: Vec<SessionFolder>,
    weak_app: gpui::WeakEntity<OxidalApp>,
    window: &mut Window,
    cx: &mut App,
) {
    open_session_dialog(Some(session), folders, weak_app, window, cx);
}

fn open_session_dialog(
    existing: Option<Session>,
    folders: Vec<SessionFolder>,
    weak_app: gpui::WeakEntity<OxidalApp>,
    window: &mut Window,
    cx: &mut App,
) {
    let editing_id = existing.as_ref().map(|s| s.id);
    let is_edit = editing_id.is_some();
    let initial_kind = existing
        .as_ref()
        .map(|s| s.kind)
        .unwrap_or(SessionKind::Ssh);

    let selected_kind = cx.new(|_cx| SelectedKind(initial_kind));
    let test_status = cx.new(|_cx| TestStatus(TestState::Idle));
    let name = cx.new(|cx| {
        InputState::new(window, cx).default_value(
            existing
                .as_ref()
                .map(|s| s.name.as_str())
                .unwrap_or(initial_kind.label()),
        )
    });
    let host = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(
                existing
                    .as_ref()
                    .map(|s| s.host.clone())
                    .unwrap_or_default(),
            )
            .placeholder("example.com")
    });
    let port = cx.new(|cx| {
        InputState::new(window, cx).default_value(
            existing
                .as_ref()
                .map(|s| s.port)
                .unwrap_or_else(|| initial_kind.default_port())
                .to_string(),
        )
    });
    let username = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(
                existing
                    .as_ref()
                    .map(|s| s.username.clone())
                    .unwrap_or_default(),
            )
            .placeholder("username")
    });
    let password = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(
                // The edit dialog needs the plaintext to seed the masked
                // input; this copy lives in the input widget until the
                // dialog closes.
                existing
                    .as_ref()
                    .map(|s| s.password.expose_secret().to_string())
                    .unwrap_or_default(),
            )
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

    let existing_key = existing.as_ref().and_then(|s| s.private_key_path.clone());
    let key_choices = key_options(existing_key.as_deref());
    let key_index = existing_key
        .as_deref()
        .and_then(|path| key_choices.iter().position(|o| o.path.as_ref() == path))
        .unwrap_or(0);
    let private_key = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(key_choices),
            Some(IndexPath::default().row(key_index)),
            window,
            cx,
        )
        .searchable(true)
    });

    let existing_port = existing
        .as_ref()
        .filter(|s| matches!(s.kind, SessionKind::Serial))
        .map(|s| s.host.clone())
        .filter(|h| !h.is_empty());
    let port_choices = port_options(existing_port.as_deref());
    let serial_index = existing_port
        .as_deref()
        .and_then(|p| port_choices.iter().position(|o| o.name.as_ref() == p))
        .map(|i| IndexPath::default().row(i));
    let serial_port = cx.new(|cx| SelectState::new(port_choices, serial_index, window, cx));

    let selected_folder = cx.new(|_cx| SelectedFolder(existing.as_ref().and_then(|s| s.folder_id)));

    window.open_dialog(cx, move |dialog, _window, cx| {
        let weak_app = weak_app.clone();
        let name = name.clone();
        let host = host.clone();
        let port = port.clone();
        let username = username.clone();
        let password = password.clone();
        let baud = baud.clone();
        let private_key = private_key.clone();
        let serial_port = serial_port.clone();
        let selected_folder = selected_folder.clone();
        let selected_kind = selected_kind.clone();
        let test_status = test_status.clone();
        let kind = selected_kind.read(cx).0;
        let current_folder = selected_folder.read(cx).0;
        let test_state = test_status.read(cx).0.clone();
        let testing = matches!(test_state, TestState::Testing);

        let tiles = h_flex()
            .gap_2()
            .pb_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .children(SessionKind::ALL.iter().map(|tile_kind| {
                let tile_kind = *tile_kind;
                let is_selected = tile_kind == kind;
                let selected_kind = selected_kind.clone();
                let test_status = test_status.clone();
                let name = name.clone();
                let port = port.clone();
                div()
                    .id(SharedString::from(format!("kind-{}", tile_kind.label())))
                    .flex_1()
                    .h(gpui::px(64.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .rounded_md()
                    .border_1()
                    .cursor_pointer()
                    .map(|this| {
                        if is_selected {
                            this.border_color(cx.theme().primary)
                                .bg(cx.theme().primary.opacity(0.12))
                                .text_color(cx.theme().primary)
                        } else {
                            this.border_color(cx.theme().border)
                                .text_color(cx.theme().muted_foreground)
                                .hover(|this| this.bg(cx.theme().accent))
                        }
                    })
                    .on_click(move |_, window, cx| {
                        let prev = selected_kind.read(cx).0;
                        if prev == tile_kind {
                            return;
                        }
                        // Carry untouched defaults over to the new kind so
                        // stale values from the previous kind don't linger.
                        if name.read(cx).value().to_string() == prev.label() {
                            name.update(cx, |state, cx| {
                                state.set_value(tile_kind.label(), window, cx);
                            });
                        }
                        if port.read(cx).value().to_string() == prev.default_port().to_string() {
                            port.update(cx, |state, cx| {
                                state.set_value(tile_kind.default_port().to_string(), window, cx);
                            });
                        }
                        test_status.update(cx, |state, cx| {
                            state.0 = TestState::Idle;
                            cx.notify();
                        });
                        selected_kind.update(cx, |state, cx| {
                            state.0 = tile_kind;
                            cx.notify();
                        });
                    })
                    .child(Icon::new(tile_kind.icon()).large())
                    .child(div().text_xs().child(tile_kind.label()))
            }));

        let mut body = v_flex()
            .gap_3()
            .w(gpui::px(400.))
            .child(tiles)
            .child(v_flex().gap_1().child("Name").child(Input::new(&name)));

        let password_field = |cx: &App| {
            v_flex()
                .gap_1()
                .child("Password")
                .child(Input::new(&password))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Saved encrypted in your system credential store"),
                )
        };

        body = match kind {
            SessionKind::Local => body,
            SessionKind::Serial => body
                .child(
                    v_flex().gap_1().child("Serial Port").child(
                        h_flex()
                            .gap_2()
                            .child(
                                Select::new(&serial_port)
                                    .placeholder("Select a port")
                                    .flex_1(),
                            )
                            .child(Button::new("rescan-ports").outline().icon(IconName::Redo2).on_click({
                                let serial_port = serial_port.clone();
                                move |_, window, cx| {
                                    serial_port.update(cx, |state, cx| {
                                        // Keep the configured port selectable even
                                        // if its device is currently unplugged.
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
                                }
                            })),
                    ),
                )
                .child(v_flex().gap_1().child("Baud Rate").child(Input::new(&baud))),
            SessionKind::Ssh => body
                .child(v_flex().gap_1().child("Host").child(Input::new(&host)))
                .child(v_flex().gap_1().child("Port").child(Input::new(&port)))
                .child(
                    v_flex()
                        .gap_1()
                        .child("Username")
                        .child(Input::new(&username)),
                )
                .child(password_field(cx))
                .child(
                    v_flex().gap_1().child("Private Key (optional)").child(
                        h_flex()
                            .gap_2()
                            .child(
                                Select::new(&private_key)
                                    .search_placeholder("Search keys...")
                                    .flex_1(),
                            )
                            .child(
                                Button::new("browse-key")
                                    .outline()
                                    .icon(IconName::FolderOpen)
                                    .on_click({
                                        let private_key = private_key.clone();
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
                                                                SearchableVec::new(key_options(
                                                                    Some(value.as_ref()),
                                                                )),
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
                .child(password_field(cx)),
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

        body = match &test_state {
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

        let mut footer = DialogFooter::new();
        if !matches!(kind, SessionKind::Local) {
            let selected_kind = selected_kind.clone();
            let host = host.clone();
            let port = port.clone();
            let username = username.clone();
            let password = password.clone();
            let private_key = private_key.clone();
            let serial_port = serial_port.clone();
            let baud = baud.clone();
            let test_status = test_status.clone();
            footer = footer.child(
                Button::new("test-connection")
                    .outline()
                    .label(if testing { "Testing..." } else { "Test" })
                    .disabled(testing)
                    .on_click(move |_, _window, cx| {
                        if matches!(test_status.read(cx).0, TestState::Testing) {
                            return;
                        }
                        let kind = selected_kind.read(cx).0;
                        let host_value = if matches!(kind, SessionKind::Serial) {
                            serial_port
                                .read(cx)
                                .selected_value()
                                .map(|v| v.to_string())
                                .unwrap_or_default()
                        } else {
                            host.read(cx).value().to_string()
                        };
                        if host_value.trim().is_empty() {
                            let msg = if matches!(kind, SessionKind::Serial) {
                                "Select a serial port first"
                            } else {
                                "Enter a host first"
                            };
                            test_status.update(cx, |s, cx| {
                                s.0 = TestState::Failed(msg.to_string());
                                cx.notify();
                            });
                            return;
                        }
                        let port_value = port
                            .read(cx)
                            .value()
                            .to_string()
                            .parse()
                            .unwrap_or_else(|_| kind.default_port());
                        let username_value = username.read(cx).value().to_string();
                        let password_value =
                            SecretString::from(password.read(cx).value().to_string());
                        let key_value = private_key
                            .read(cx)
                            .selected_value()
                            .map(|v| v.to_string())
                            .filter(|v| !v.trim().is_empty());
                        let baud_value =
                            baud.read(cx).value().to_string().parse().unwrap_or(115_200);

                        test_status.update(cx, |s, cx| {
                            s.0 = TestState::Testing;
                            cx.notify();
                        });
                        let rx = run_connection_test(
                            kind,
                            host_value,
                            port_value,
                            username_value,
                            password_value,
                            key_value,
                            baud_value,
                        );
                        let test_status = test_status.clone();
                        cx.spawn(async move |cx| {
                            let outcome = match rx.recv().await {
                                Ok(Ok(msg)) => TestState::Success(msg),
                                Ok(Err(err)) => TestState::Failed(err),
                                Err(_) => TestState::Failed("Connection test aborted".to_string()),
                            };
                            let _ = test_status.update(cx, |s, cx| {
                                s.0 = outcome;
                                cx.notify();
                            });
                        })
                        .detach();
                    }),
            );
        }

        let do_save: Rc<dyn Fn(&mut App)> = Rc::new({
            let weak_app = weak_app.clone();
            let name = name.clone();
            let host = host.clone();
            let port = port.clone();
            let username = username.clone();
            let password = password.clone();
            let baud = baud.clone();
            let private_key = private_key.clone();
            let serial_port = serial_port.clone();
            let selected_folder = selected_folder.clone();
            let selected_kind = selected_kind.clone();
            move |cx: &mut App| {
                let kind = selected_kind.read(cx).0;
                let mut session = Session::new(name.read(cx).value().to_string(), kind);
                if let Some(id) = editing_id {
                    session.id = id;
                }
                session.host = if matches!(kind, SessionKind::Serial) {
                    serial_port
                        .read(cx)
                        .selected_value()
                        .map(|v| v.to_string())
                        .unwrap_or_default()
                } else {
                    host.read(cx).value().to_string()
                };
                session.port = port
                    .read(cx)
                    .value()
                    .to_string()
                    .parse()
                    .unwrap_or_else(|_| kind.default_port());
                session.username = username.read(cx).value().to_string();
                session.password = SecretString::from(password.read(cx).value().to_string());
                session.baud_rate = baud.read(cx).value().to_string().parse().unwrap_or(115_200);
                session.private_key_path = private_key
                    .read(cx)
                    .selected_value()
                    .map(|v| v.to_string())
                    .filter(|v| !v.trim().is_empty());
                session.folder_id = selected_folder.read(cx).0;

                let _ = weak_app.update(cx, |app, cx| {
                    if editing_id.is_some() {
                        app.update_session(session, cx);
                    } else {
                        app.add_session(session, cx);
                    }
                });
            }
        });

        footer = footer
            .child(
                Button::new("cancel")
                    .label("Cancel")
                    .on_click(|_, window, cx| {
                        window.close_dialog(cx);
                    }),
            )
            .child(Button::new("save").primary().label("Save").on_click({
                let do_save = do_save.clone();
                move |_, window, cx| {
                    do_save(cx);
                    window.close_dialog(cx);
                }
            }));

        dialog
            .title(if is_edit {
                format!("Edit {} Session", kind.label())
            } else {
                format!("New {} Session", kind.label())
            })
            .child(body)
            .footer(footer)
            // Enter is bound to the dialog's confirm action; without this it
            // would fall through to the default handler and close the dialog
            // without saving.
            .on_ok({
                let do_save = do_save.clone();
                move |_, _window, cx| {
                    do_save(cx);
                    true
                }
            })
    });
}

/// The choices for the private-key picker: "no key", every private key found
/// in the user's `~/.ssh` directory, and `extra` (a previously saved or
/// browsed path) if it isn't already among them.
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

/// Sniff whether `path` looks like a private key russh can load (OpenSSH or
/// PEM format). Content-based so it works regardless of file naming.
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

/// The currently detected serial ports. `extra` (the port stored on the
/// session being edited) is appended if missing so an unplugged device
/// doesn't silently drop the configured port.
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

/// Run a connection test for `kind` on a background thread, reporting the
/// outcome (success message or error text) over the returned channel.
fn run_connection_test(
    kind: SessionKind,
    host: String,
    port: u16,
    username: String,
    password: SecretString,
    private_key_path: Option<String>,
    baud_rate: u32,
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
            SessionKind::Rdp => tcp_check(&host, port),
            SessionKind::Ssh | SessionKind::Sftp => {
                ssh_check(host, port, username, password, private_key_path)
            }
        };
        let _ = tx.send_blocking(result);
    });
    rx
}

/// Plain TCP reachability check (no protocol handshake).
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

/// Full SSH connect + authenticate round-trip on a throwaway tokio runtime,
/// mirroring how the real SSH backends run (dedicated thread + runtime).
fn ssh_check(
    host: String,
    port: u16,
    username: String,
    password: SecretString,
    private_key_path: Option<String>,
) -> Result<String, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let connect =
            crate::ssh_client::connect(host.clone(), port, username, password, private_key_path);
        match tokio::time::timeout(Duration::from_secs(10), connect).await {
            Err(_) => Err(format!("Timed out connecting to {host}:{port}")),
            Ok(Err(e)) => Err(e.to_string()),
            Ok(Ok(handle)) => {
                let _ = handle
                    .disconnect(russh::Disconnect::ByApplication, "", "")
                    .await;
                Ok(format!("Authenticated to {host}:{port}"))
            }
        }
    })
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

        let footer =
            DialogFooter::new()
                .child(
                    Button::new("cancel")
                        .label("Cancel")
                        .on_click(|_, window, cx| {
                            window.close_dialog(cx);
                        }),
                )
                .child(Button::new("save").primary().label("Save").on_click(
                    move |_, window, cx| {
                        let value = name.read(cx).value().to_string();
                        if !value.trim().is_empty() {
                            let _ = weak_app.update(cx, |app, cx| {
                                app.add_folder(SessionFolder::new(value), cx);
                            });
                        }
                        window.close_dialog(cx);
                    },
                ));

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

        let footer =
            DialogFooter::new()
                .child(
                    Button::new("cancel")
                        .label("Cancel")
                        .on_click(|_, window, cx| {
                            window.close_dialog(cx);
                        }),
                )
                .child(Button::new("save").primary().label("Save").on_click(
                    move |_, window, cx| {
                        let value = name.read(cx).value().to_string();
                        if !value.trim().is_empty() {
                            let _ = weak_app.update(cx, |app, cx| {
                                app.rename_folder(folder_id, value, cx);
                            });
                        }
                        window.close_dialog(cx);
                    },
                ));

        dialog.title("Rename Folder").child(body).footer(footer)
    });
}
