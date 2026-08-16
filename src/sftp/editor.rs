use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, WindowBounds, WindowOptions, actions,
    div, prelude::FluentBuilder as _, px, size,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Root, Rope, Sizable as _, TitleBar,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    h_flex,
    input::{Input, InputEvent, InputState, RopeExt as _},
    text::{TextView, TextViewState},
    v_flex,
};
use sha2::{Digest as _, Sha256};

use crate::settings::AppSettings;

use super::{SftpClient, display_name, format_size};

actions!(editor, [SaveFile]);

enum SaveStatus {
    Saved,
    Failed(String),
}

pub struct EditorWindow {
    client: SftpClient,
    remote: String,
    name: SharedString,
    language_label: Option<SharedString>,
    editor: Entity<InputState>,
    original_len: usize,
    original_digest: [u8; 32],
    modified: bool,
    is_markdown: bool,
    preview_doc: Option<Entity<TextViewState>>,
    saving: bool,
    status: Option<SaveStatus>,
    handle: AnyWindowHandle,
    close_after_save: bool,
}

fn digest_rope(rope: &Rope) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for chunk in rope.chunks() {
        hasher.update(chunk);
    }
    hasher.finalize().into()
}

pub fn open(
    client: SftpClient,
    remote: String,
    name: String,
    text: String,
    cx: &mut App,
) -> anyhow::Result<()> {
    let bounds = Bounds::centered(None, size(px(920.), px(720.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitleBar::title_bar_options()),
        app_id: Some(crate::APP_ID.to_string()),
        window_min_size: Some(size(px(480.), px(360.))),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        crate::settings::apply_appearance(window, cx);
        let view = cx.new(|cx| EditorWindow::new(client, remote, name, text, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    Ok(())
}

impl EditorWindow {
    fn new(
        client: SftpClient,
        remote: String,
        name: String,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let extension = match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => Some(ext.to_lowercase()),
            _ => None,
        };
        let language = extension.clone().unwrap_or_else(|| "text".to_string());
        let is_markdown = matches!(language.as_str(), "md" | "markdown");
        let original_len = text.len();
        let original_digest = Sha256::digest(&text).into();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(language)
                .default_value(text)
        });
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&editor, |editor_window, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                editor_window.refresh_modified(cx);
            }
        })
        .detach();

        let name = display_name(&name);
        window.set_window_title(&format!("{name} — Oxidal"));

        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let Some(this) = this.upgrade() else {
                return true;
            };
            this.update(cx, |editor, cx| editor.request_close(window, cx))
        });

        Self {
            client,
            remote,
            name: SharedString::from(name),
            language_label: extension.map(|ext| SharedString::from(ext.to_uppercase())),
            editor,
            original_len,
            original_digest,
            modified: false,
            is_markdown,
            preview_doc: None,
            saving: false,
            status: None,
            handle: window.window_handle(),
            close_after_save: false,
        }
    }

    fn refresh_modified(&mut self, cx: &mut Context<Self>) {
        let modified = {
            let rope = self.editor.read(cx).text();
            rope.len() != self.original_len || digest_rope(rope) != self.original_digest
        };
        if modified != self.modified {
            self.modified = modified;
            cx.notify();
        }
    }

    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.modified {
            return true;
        }
        self.confirm_close_dialog(window, cx);
        false
    }

    fn confirm_close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        let name = self.name.clone();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view = view.clone();

            dialog
                .w(px(400.))
                .title("Unsaved changes")
                .child(format!(
                    "\"{name}\" has changes that were not uploaded to the server. Save them \
                     before closing?"
                ))
                .footer(
                    DialogFooter::new()
                        .child(Button::new("close-cancel").label("Cancel").on_click(
                            |_, window, cx| {
                                window.close_dialog(cx);
                            },
                        ))
                        .child(Button::new("close-discard").danger().label("Discard").on_click(
                            |_, window, cx| {
                                window.close_dialog(cx);
                                window.remove_window();
                            },
                        ))
                        .child(
                            Button::new("close-save")
                                .primary()
                                .label("Save and close")
                                .on_click({
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        view.update(cx, |editor, cx| {
                                            editor.close_after_save = true;
                                            editor.save(cx);
                                        });
                                    }
                                }),
                        ),
                )
        });
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let text = self.editor.read(cx).text().to_string();
        let digest: [u8; 32] = Sha256::digest(&text).into();
        if text.len() == self.original_len && digest == self.original_digest {
            self.close_after_save = false;
            return;
        }
        self.saving = true;
        self.status = None;
        let len = text.len();
        let handle = self.handle;
        let ack = self.client.write_file(self.remote.clone(), text.into_bytes());
        cx.spawn(async move |this, cx| {
            let result = ack.recv().await;
            let mut close_window = false;
            let _ = this.update(cx, |editor, cx| {
                editor.saving = false;
                editor.status = Some(match result {
                    Ok(None) => {
                        editor.original_len = len;
                        editor.original_digest = digest;
                        close_window = editor.close_after_save;
                        SaveStatus::Saved
                    }
                    Ok(Some(err)) => SaveStatus::Failed(err),
                    Err(_) => SaveStatus::Failed("The connection is closed".to_string()),
                });
                editor.close_after_save = false;
                editor.refresh_modified(cx);
                cx.notify();
            });
            if close_window {
                let _ = cx.update_window(handle, |_, window, _| window.remove_window());
            }
        })
        .detach();
        cx.notify();
    }
}

impl Render for EditorWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (font_family, font_size) = {
            let settings = cx.global::<AppSettings>();
            (settings.font_family.clone(), settings.font_size)
        };
        let (position, lines, bytes) = {
            let state = self.editor.read(cx);
            (
                state.cursor_position(),
                state.text().lines_len(),
                state.text().len(),
            )
        };
        let modified = self.modified;
        let doc_label = SharedString::from(format!(
            "Ln {}, Col {} · {} lines · {}",
            position.line + 1,
            position.character + 1,
            lines,
            format_size(bytes as u64),
        ));

        let status: Option<(Icon, SharedString, gpui::Hsla)> = if self.saving {
            Some((
                Icon::new(IconName::Loader),
                "Saving...".into(),
                cx.theme().muted_foreground,
            ))
        } else {
            match &self.status {
                Some(SaveStatus::Failed(err)) => Some((
                    Icon::new(IconName::TriangleAlert),
                    SharedString::from(err.clone()),
                    cx.theme().danger,
                )),
                _ if modified => Some((
                    Icon::new(IconName::Info),
                    "Unsaved changes".into(),
                    cx.theme().muted_foreground,
                )),
                Some(SaveStatus::Saved) => Some((
                    Icon::new(IconName::CircleCheck),
                    "Saved".into(),
                    cx.theme().success,
                )),
                _ => None,
            }
        };

        let save_shortcut = if cfg!(target_os = "macos") {
            "Upload to the server (⌘S)"
        } else {
            "Upload to the server (Ctrl+S)"
        };

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .key_context("EditorWindow")
            .on_action(cx.listener(|editor, _: &SaveFile, _, cx| editor.save(cx)))
            .child(
                TitleBar::new().child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::File)
                                .small()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(self.name.clone()),
                        )
                        .when(modified, |this| {
                            this.child(
                                div()
                                    .size_1p5()
                                    .rounded_full()
                                    .bg(cx.theme().warning),
                            )
                        }),
                ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1p5()
                    .bg(cx.theme().sidebar)
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Icon::new(IconName::Globe)
                            .xsmall()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(SharedString::from(display_name(&self.remote))),
                    )
                    .when_some(self.language_label.clone(), |this, label| {
                        this.child(
                            div()
                                .flex_none()
                                .px_1p5()
                                .py_0p5()
                                .rounded_sm()
                                .bg(cx.theme().muted.opacity(0.5))
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(label),
                        )
                    }),
            )
            .child(
                div().flex_1().min_h_0().w_full().map(|this| {
                    if let Some(doc) = &self.preview_doc {
                        this.child(
                            div()
                                .id("editor-md-preview")
                                .size_full()
                                .overflow_y_scroll()
                                .px_6()
                                .py_4()
                                .child(TextView::new(doc).selectable(true)),
                        )
                    } else {
                        this.child(
                            div()
                                .size_full()
                                .font_family(font_family)
                                .text_size(px(font_size))
                                .child(Input::new(&self.editor).appearance(false).h_full()),
                        )
                    }
                }),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1p5()
                    .bg(cx.theme().sidebar)
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .when_some(status, |this, (icon, text, color)| {
                        this.child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .min_w_0()
                                .text_color(color)
                                .child(icon.xsmall())
                                .child(div().text_xs().truncate().child(text)),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(doc_label),
                    )
                    .when(self.is_markdown, |this| {
                        this.child(
                            Button::new("editor-preview")
                                .ghost()
                                .xsmall()
                                .icon(if self.preview_doc.is_some() {
                                    IconName::EyeOff
                                } else {
                                    IconName::Eye
                                })
                                .label(if self.preview_doc.is_some() {
                                    "Edit"
                                } else {
                                    "Preview"
                                })
                                .on_click(cx.listener(|editor, _, _, cx| {
                                    editor.preview_doc = match editor.preview_doc.take() {
                                        Some(_) => None,
                                        None => {
                                            let text =
                                                editor.editor.read(cx).text().to_string();
                                            Some(cx.new(|cx| {
                                                TextViewState::markdown(&text, cx)
                                            }))
                                        }
                                    };
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new("editor-save")
                            .primary()
                            .xsmall()
                            .icon(IconName::ArrowUp)
                            .label("Save")
                            .tooltip(save_shortcut)
                            .disabled(self.saving || !modified)
                            .on_click(cx.listener(|editor, _, _, cx| editor.save(cx))),
                    ),
            )
    }
}
