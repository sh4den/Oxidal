use std::path::PathBuf;

use gpui::{
    AppContext as _, Context, Entity, FontWeight, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::Button,
    h_flex,
    resizable::{h_resizable, resizable_panel},
    v_flex,
};

use super::panel::PanelEvent;
use super::{FileClient, FileDrag, PanelSide, SftpEntry, SftpPanel, home_dir, join_remote};

pub struct SftpWorkspace {
    local: Entity<SftpPanel>,
    remote: Entity<SftpPanel>,
    client: super::SftpClient,
    label: SharedString,
}

impl SftpWorkspace {
    pub fn new(
        host: String,
        port: u16,
        credentials: crate::ssh_client::SshCredentials,
        show_hidden: bool,
        on_show_hidden_changed: impl Fn(bool, &mut gpui::App) + Clone + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let label = if credentials.username.is_empty() {
            SharedString::from(format!("{host}:{port}"))
        } else {
            SharedString::from(format!("{}@{host}:{port}", credentials.username))
        };

        let client = super::spawn(host, port, credentials, ".".to_string());

        let remote = cx.new({
            let client = client.clone();
            let on_show_hidden_changed = on_show_hidden_changed.clone();
            |cx| {
                SftpPanel::from_client(
                    FileClient::Remote(client),
                    show_hidden,
                    on_show_hidden_changed,
                    window,
                    cx,
                )
            }
        });
        let local = cx.new(|cx| {
            SftpPanel::local(home_dir(), show_hidden, on_show_hidden_changed, window, cx)
        });

        for panel in [&local, &remote] {
            cx.subscribe(panel, |workspace, _, event: &PanelEvent, cx| {
                workspace.handle_panel_event(event, cx);
            })
            .detach();
        }

        Self {
            local,
            remote,
            client,
            label,
        }
    }

    fn handle_panel_event(&mut self, event: &PanelEvent, cx: &mut Context<Self>) {
        match event {
            PanelEvent::TransferRequested { drag, dest_dir } => {
                self.transfer(drag, dest_dir, cx);
            }
            PanelEvent::TransferFinished => {
                self.local.update(cx, |panel, cx| panel.refresh_listing(cx));
            }
            PanelEvent::SelectionChanged => cx.notify(),
        }
    }

    fn transfer(&mut self, drag: &FileDrag, dest_dir: &str, cx: &mut Context<Self>) {
        match drag.side {
            PanelSide::Local => {
                self.send_to_remote(&drag.entry_path, &drag.name, drag.is_dir, dest_dir)
            }
            PanelSide::Remote => {
                self.send_to_local(&drag.entry_path, &drag.name, drag.is_dir, dest_dir)
            }
        }
        cx.notify();
    }

    fn send_to_remote(&self, local_path: &str, name: &str, is_dir: bool, remote_dir: &str) {
        let destination = join_remote(remote_dir, name);
        let source = PathBuf::from(local_path);
        if is_dir {
            self.client.upload_dir(source, destination);
        } else {
            self.client.upload(source, destination);
        }
    }

    fn send_to_local(&self, remote_path: &str, name: &str, is_dir: bool, local_dir: &str) {
        let destination = PathBuf::from(local_dir).join(super::safe_local_name(name));
        if is_dir {
            self.client
                .download_dir(remote_path.to_string(), destination);
        } else {
            self.client.download(remote_path.to_string(), destination);
        }
    }

    fn upload_selection(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = selected_of(&self.local, cx) else {
            return;
        };
        let remote_dir = self.remote.read(cx).current_path().to_string();
        self.send_to_remote(&entry.path, &entry.name, entry.is_dir, &remote_dir);
        cx.notify();
    }

    fn download_selection(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = selected_of(&self.remote, cx) else {
            return;
        };
        let local_dir = self.local.read(cx).current_path().to_string();
        self.send_to_local(&entry.path, &entry.name, entry.is_dir, &local_dir);
        cx.notify();
    }

    fn side_badge(
        &self,
        icon: IconName,
        title: &'static str,
        detail: SharedString,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .min_w_0()
            .items_center()
            .gap_2()
            .child(
                Icon::new(icon)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(cx.theme().muted_foreground)
                            .child(detail),
                    ),
            )
    }
}

fn selected_of(panel: &Entity<SftpPanel>, cx: &Context<SftpWorkspace>) -> Option<SftpEntry> {
    panel.read(cx).selected_entry().cloned()
}

impl Render for SftpWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let local_selection = selected_of(&self.local, cx);
        let remote_selection = selected_of(&self.remote, cx);
        let local_path = SharedString::from(self.local.read(cx).current_path().to_string());

        let upload_tooltip = match &local_selection {
            Some(entry) => format!("Send \"{}\" to the remote folder", entry.name),
            None => "Select something on the left first".to_string(),
        };
        let download_tooltip = match &remote_selection {
            Some(entry) => format!("Bring \"{}\" to the local folder", entry.name),
            None => "Select something on the right first".to_string(),
        };

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .items_center()
                    .gap_4()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().sidebar)
                    .child(div().flex_1().min_w_0().child(self.side_badge(
                        IconName::HardDrive,
                        "This computer",
                        local_path,
                        cx,
                    )))
                    .child(
                        h_flex()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("sftp-send")
                                    .outline()
                                    .small()
                                    .icon(IconName::ArrowRight)
                                    .tooltip(upload_tooltip)
                                    .disabled(local_selection.is_none())
                                    .on_click(cx.listener(|workspace, _, _, cx| {
                                        workspace.upload_selection(cx)
                                    })),
                            )
                            .child(
                                Button::new("sftp-fetch")
                                    .outline()
                                    .small()
                                    .icon(IconName::ArrowLeft)
                                    .tooltip(download_tooltip)
                                    .disabled(remote_selection.is_none())
                                    .on_click(cx.listener(|workspace, _, _, cx| {
                                        workspace.download_selection(cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .justify_end()
                            .child(self.side_badge(
                                IconName::Globe,
                                "Remote server",
                                self.label.clone(),
                                cx,
                            )),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("sftp-dual-pane")
                        .child(
                            resizable_panel()
                                .size(px(520.))
                                .size_range(px(280.)..px(1200.))
                                .child(
                                    div()
                                        .size_full()
                                        .border_r_1()
                                        .border_color(cx.theme().border)
                                        .child(self.local.clone()),
                                ),
                        )
                        .child(
                            div()
                                .size_full()
                                .child(self.remote.clone())
                                .into_any_element(),
                        ),
                ),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().sidebar)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(IconName::Info).xsmall())
                    .child("Drag a file or folder onto the other side to transfer it"),
            )
    }
}
