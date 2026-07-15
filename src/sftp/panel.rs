use std::path::PathBuf;

use gpui::{
    div, prelude::FluentBuilder as _, px, AnyElement, AppContext as _, ClickEvent, Context,
    InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    input::{Input, InputState},
    menu::{ContextMenuExt as _, PopupMenuItem},
    progress::Progress,
    h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _,
};

use super::{format_modified, format_size, join_remote, parent_remote, SftpEntry, SftpEvent};

struct TransferState {
    label: String,
    transferred: u64,
    total: Option<u64>,
}

impl TransferState {
    fn percent(&self) -> f32 {
        match self.total {
            Some(total) if total > 0 => (self.transferred as f32 / total as f32 * 100.0).min(100.0),
            _ => 0.0,
        }
    }
}

/// A MobaXterm-style remote file browser backed by its own SFTP connection.
pub struct SftpPanel {
    client: super::SftpClient,
    current_path: String,
    entries: Vec<SftpEntry>,
    selected: Option<String>,
    loading: bool,
    error: Option<String>,
    closed: Option<String>,
    transfer: Option<TransferState>,
}

impl SftpPanel {
    pub fn new(
        host: String,
        port: u16,
        username: String,
        password: String,
        private_key_path: Option<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let client = super::spawn(host, port, username, password, private_key_path, ".".to_string());

        let events = client.events.clone();
        cx.spawn(async move |this, cx| loop {
            match events.recv().await {
                Ok(SftpEvent::Listing { path, entries }) => {
                    if this
                        .update(cx, |panel, cx| {
                            panel.current_path = path;
                            panel.entries = entries;
                            panel.loading = false;
                            panel.error = None;
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(SftpEvent::Error(message)) => {
                    if this
                        .update(cx, |panel, cx| {
                            panel.loading = false;
                            panel.error = Some(message);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(SftpEvent::TransferStarted { label, total }) => {
                    if this
                        .update(cx, |panel, cx| {
                            panel.transfer = Some(TransferState {
                                label,
                                transferred: 0,
                                total,
                            });
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(SftpEvent::TransferProgress { transferred }) => {
                    if this
                        .update(cx, |panel, cx| {
                            if let Some(transfer) = panel.transfer.as_mut() {
                                transfer.transferred = transferred;
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(SftpEvent::TransferFinished { error }) => {
                    if this
                        .update(cx, |panel, cx| {
                            panel.transfer = None;
                            if let Some(err) = error {
                                panel.error = Some(err);
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(SftpEvent::Closed(message)) => {
                    let _ = this.update(cx, |panel, cx| {
                        panel.closed = Some(message.unwrap_or_else(|| "Connection closed".to_string()));
                        cx.notify();
                    });
                    break;
                }
                Err(_) => break,
            }
        })
        .detach();

        Self {
            client,
            current_path: "/".to_string(),
            entries: Vec::new(),
            selected: None,
            loading: true,
            error: None,
            closed: None,
            transfer: None,
        }
    }

    fn navigate(&mut self, path: String, cx: &mut Context<Self>) {
        self.loading = true;
        self.selected = None;
        self.client.list(path);
        cx.notify();
    }

    fn go_up(&mut self, cx: &mut Context<Self>) {
        if self.current_path == "/" {
            return;
        }
        let parent = parent_remote(&self.current_path);
        self.navigate(parent, cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.navigate(self.current_path.clone(), cx);
    }

    fn open_entry(&mut self, entry: &SftpEntry, cx: &mut Context<Self>) {
        if entry.is_dir {
            self.navigate(entry.path.clone(), cx);
        } else {
            self.selected = Some(entry.path.clone());
            cx.notify();
        }
    }

    fn new_folder_dialog(&self, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.client.clone();
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("New folder name"));

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let client = client.clone();
            let name = name.clone();

            dialog
                .title("New Folder")
                .child(v_flex().gap_2().w(px(320.)).child(Input::new(&name)))
                .footer(
                    DialogFooter::new()
                        .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| {
                            window.close_dialog(cx);
                        }))
                        .child(Button::new("create").primary().label("Create").on_click(
                            move |_, window, cx| {
                                let value = name.read(cx).value().to_string();
                                if !value.trim().is_empty() {
                                    client.create_dir(value);
                                }
                                window.close_dialog(cx);
                            },
                        )),
                )
        });
    }

    fn rename_dialog(&self, entry: SftpEntry, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.client.clone();
        let parent = parent_remote(&entry.path);
        let name = cx.new(|cx| InputState::new(window, cx).default_value(entry.name.clone()));

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let client = client.clone();
            let parent = parent.clone();
            let old_path = entry.path.clone();
            let name = name.clone();

            dialog
                .title(format!("Rename \"{}\"", entry.name))
                .child(v_flex().gap_2().w(px(320.)).child(Input::new(&name)))
                .footer(
                    DialogFooter::new()
                        .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| {
                            window.close_dialog(cx);
                        }))
                        .child(Button::new("rename").primary().label("Rename").on_click(
                            move |_, window, cx| {
                                let value = name.read(cx).value().to_string();
                                if !value.trim().is_empty() {
                                    client.rename(old_path.clone(), join_remote(&parent, &value));
                                }
                                window.close_dialog(cx);
                            },
                        )),
                )
        });
    }

    fn delete_dialog(&self, entry: SftpEntry, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.client.clone();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let client = client.clone();
            let entry = entry.clone();

            dialog
                .title("Delete")
                .child(format!(
                    "Delete {} \"{}\"? This cannot be undone.",
                    if entry.is_dir { "folder" } else { "file" },
                    entry.name
                ))
                .footer(
                    DialogFooter::new()
                        .child(Button::new("cancel").label("Cancel").on_click(|_, window, cx| {
                            window.close_dialog(cx);
                        }))
                        .child(Button::new("delete").danger().label("Delete").on_click(
                            move |_, window, cx| {
                                if entry.is_dir {
                                    client.remove_dir(entry.path.clone());
                                } else {
                                    client.remove_file(entry.path.clone());
                                }
                                window.close_dialog(cx);
                            },
                        )),
                )
        });
    }

    fn upload(&self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(SharedString::from("Upload")),
        });
        let client = self.client.clone();
        let remote_dir = self.current_path.clone();
        cx.spawn(async move |_this, _cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                for local in paths {
                    let name = local
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    client.upload(local, join_remote(&remote_dir, &name));
                }
            }
        })
        .detach();
    }

    fn download(&self, entry: SftpEntry, cx: &mut Context<Self>) {
        let start_dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&start_dir, Some(&entry.name));
        let client = self.client.clone();
        cx.spawn(async move |_this, _cx| {
            if let Ok(Ok(Some(local))) = rx.await {
                client.download(entry.path.clone(), local);
            }
        })
        .detach();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("sftp-up")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowUp)
                    .tooltip("Up a directory")
                    .disabled(self.current_path == "/")
                    .on_click(cx.listener(|panel, _, _, cx| panel.go_up(cx))),
            )
            .child(
                Button::new("sftp-refresh")
                    .ghost()
                    .xsmall()
                    .label("Refresh")
                    .on_click(cx.listener(|panel, _, _, cx| panel.refresh(cx))),
            )
            .child(
                Button::new("sftp-new-folder")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Folder)
                    .label("New Folder")
                    .on_click(cx.listener(|panel, _, window, cx| {
                        panel.new_folder_dialog(window, cx);
                    })),
            )
            .child(
                Button::new("sftp-upload")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowUp)
                    .label("Upload")
                    .on_click(cx.listener(|panel, _, _, cx| panel.upload(cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(self.current_path.clone())),
            )
            .into_any_element()
    }

    fn render_row(&self, entry: &SftpEntry, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let selected = self.selected.as_deref() == Some(entry.path.as_str());
        let row_entry = entry.clone();
        let row_entry_click = entry.clone();
        let is_dir = entry.is_dir;

        h_flex()
            .id(SharedString::from(format!("sftp-entry-{}", entry.path)))
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .cursor_pointer()
            .when(selected, |this| {
                this.bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
            })
            .on_click(cx.listener(move |panel, event: &ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    panel.open_entry(&row_entry_click, cx);
                } else {
                    panel.selected = Some(row_entry_click.path.clone());
                    cx.notify();
                }
            }))
            .child(Icon::new(if entry.is_dir {
                IconName::Folder
            } else {
                IconName::File
            }).small())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_sm()
                    .child(SharedString::from(entry.name.clone())),
            )
            .child(
                div()
                    .w(px(72.))
                    .flex_none()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if entry.is_dir {
                        String::new()
                    } else {
                        format_size(entry.size)
                    }),
            )
            .child(
                div()
                    .w(px(120.))
                    .flex_none()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(entry.modified.map(format_modified).unwrap_or_default()),
            )
            .context_menu(move |menu, window, _cx| {
                let entry = row_entry.clone();
                let mut menu = menu;
                if !is_dir {
                    let entry = entry.clone();
                    menu = menu.item(
                        PopupMenuItem::new("Download").on_click(window.listener_for(
                            &view,
                            move |panel, _, _, cx| panel.download(entry.clone(), cx),
                        )),
                    );
                    menu = menu.separator();
                }
                menu = menu.item(
                    PopupMenuItem::new("Rename").on_click(window.listener_for(&view, {
                        let entry = entry.clone();
                        move |panel, _, window, cx| panel.rename_dialog(entry.clone(), window, cx)
                    })),
                );
                menu.item(
                    PopupMenuItem::new("Delete").on_click(window.listener_for(&view, {
                        move |panel, _, window, cx| panel.delete_dialog(entry.clone(), window, cx)
                    })),
                )
            })
            .into_any_element()
    }
}

impl Render for SftpPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(closed) = &self.closed {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .p_4()
                .child(Icon::new(IconName::TriangleAlert).with_size(px(28.)))
                .child(
                    div()
                        .text_sm()
                        .text_center()
                        .text_color(cx.theme().muted_foreground)
                        .child(SharedString::from(closed.clone())),
                )
                .into_any_element();
        }

        let view = cx.entity();
        let rows = self
            .entries
            .iter()
            .map(|entry| self.render_row(entry, cx))
            .collect::<Vec<_>>();

        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .child(self.render_toolbar(cx))
            .when_some(self.error.clone(), |this, message| {
                this.child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .gap_1()
                        .bg(cx.theme().danger.opacity(0.1))
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(Icon::new(IconName::TriangleAlert).xsmall())
                        .child(SharedString::from(message)),
                )
            })
            .child(
                div()
                    .id("sftp-entries")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex().children(rows).when(self.entries.is_empty(), |this| {
                            this.child(
                                div()
                                    .p_4()
                                    .text_sm()
                                    .text_center()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if self.loading {
                                        "Loading..."
                                    } else {
                                        "Empty directory"
                                    }),
                            )
                        }),
                    )
                    .context_menu(move |menu, window, _cx| {
                        menu.item(
                            PopupMenuItem::new("New Folder").on_click(window.listener_for(
                                &view,
                                |panel, _, window, cx| panel.new_folder_dialog(window, cx),
                            )),
                        )
                        .item(PopupMenuItem::new("Upload").on_click(window.listener_for(
                            &view,
                            |panel, _, _, cx| panel.upload(cx),
                        )))
                        .separator()
                        .item(PopupMenuItem::new("Refresh").on_click(window.listener_for(
                            &view,
                            |panel, _, _, cx| panel.refresh(cx),
                        )))
                    }),
            )
            .when_some(self.transfer.as_ref(), |this, transfer| {
                this.child(
                    v_flex()
                        .gap_1()
                        .px_2()
                        .py_1()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .child(
                            h_flex()
                                .justify_between()
                                .text_xs()
                                .child(SharedString::from(transfer.label.clone()))
                                .child(format!("{:.0}%", transfer.percent())),
                        )
                        .child(Progress::new("sftp-transfer").value(transfer.percent()).small()),
                )
            })
            .into_any_element()
    }
}
