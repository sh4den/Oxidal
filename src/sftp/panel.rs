use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::path::PathBuf;

use gpui::{
    Anchor, AnyElement, AppContext as _, ClickEvent, Context, DragMoveEvent, Empty, EntityId,
    FontWeight, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseUpEvent,
    ParentElement as _, PathPromptOptions, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
    relative,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    progress::Progress,
    scroll::Scrollbar,
    skeleton::Skeleton,
    v_flex,
};

use super::{
    SftpEntry, SftpEvent, format_kind, format_modified, format_permissions, format_size,
    join_remote, parent_remote, safe_local_name,
};

const HEADER_HEIGHT: f32 = 26.;
const ICON_COL_WIDTH: f32 = 16.;
const ROW_PADDING_X: f32 = 8.;
const COLUMN_GAP: f32 = 8.;
const NAME_MIN_WIDTH: f32 = 120.;
const COLUMN_MIN_WIDTH: f32 = 48.;

#[derive(Clone, Copy, PartialEq)]
enum ListColumn {
    Name,
    Size,
    Kind,
    Modified,
    Accessed,
    Access,
    Owner,
    Group,
}

impl ListColumn {
    const COUNT: usize = 8;
    const ALL: [ListColumn; Self::COUNT] = [
        ListColumn::Name,
        ListColumn::Size,
        ListColumn::Kind,
        ListColumn::Modified,
        ListColumn::Accessed,
        ListColumn::Access,
        ListColumn::Owner,
        ListColumn::Group,
    ];

    fn label(self) -> &'static str {
        match self {
            ListColumn::Name => "Name",
            ListColumn::Size => "Size",
            ListColumn::Kind => "Kind",
            ListColumn::Modified => "Modified",
            ListColumn::Accessed => "Accessed",
            ListColumn::Access => "Access",
            ListColumn::Owner => "Owner",
            ListColumn::Group => "Group",
        }
    }

    fn default_width(self) -> f32 {
        match self {
            // The real Name default is filling the viewport; this is only
            // the fallback lower bound.
            ListColumn::Name => NAME_MIN_WIDTH,
            ListColumn::Size => 72.,
            ListColumn::Kind => 64.,
            ListColumn::Modified => 110.,
            ListColumn::Accessed => 110.,
            ListColumn::Access => 72.,
            ListColumn::Owner => 72.,
            ListColumn::Group => 72.,
        }
    }

    fn right_align(self) -> bool {
        matches!(self, ListColumn::Size)
    }
}

struct ColumnResize {
    column: ListColumn,
    start_width: f32,
    start_x: Pixels,
}

#[derive(Clone)]
struct ColumnDrag(EntityId);

impl Render for ColumnDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

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

pub struct SftpPanel {
    client: super::SftpClient,
    current_path: String,
    entries: Vec<SftpEntry>,
    selected: Option<String>,
    loading: bool,
    error: Option<String>,
    closed: Option<String>,
    transfer: Option<TransferState>,
    show_hidden: bool,
    on_show_hidden_changed: Box<dyn Fn(bool, &mut gpui::App)>,
    path_input: gpui::Entity<InputState>,
    synced_path: String,
    list_scroll: ScrollHandle,
    h_scroll: ScrollHandle,
    column_widths: [Option<f32>; ListColumn::COUNT],
    hidden_columns: [bool; ListColumn::COUNT],
    resizing: Option<ColumnResize>,
}

impl SftpPanel {
    pub fn new(
        host: String,
        port: u16,
        username: String,
        password: secrecy::SecretString,
        private_key_path: Option<String>,
        show_hidden: bool,
        on_show_hidden_changed: impl Fn(bool, &mut gpui::App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let client = super::spawn(
            host,
            port,
            username,
            password,
            private_key_path,
            ".".to_string(),
        );

        let path_input = cx.new(|cx| InputState::new(window, cx).placeholder("/"));
        cx.subscribe(
            &path_input,
            |panel: &mut Self, input, event: &InputEvent, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let path = input.read(cx).value().trim().to_string();
                    if !path.is_empty() {
                        panel.navigate(path, cx);
                    }
                }
            },
        )
        .detach();

        let events = client.events.clone();
        cx.spawn(async move |this, cx| {
            loop {
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
                            panel.closed =
                                Some(message.unwrap_or_else(|| "Connection closed".to_string()));
                            cx.notify();
                        });
                        break;
                    }
                    Err(_) => break,
                }
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
            show_hidden,
            on_show_hidden_changed: Box::new(on_show_hidden_changed),
            path_input,
            synced_path: String::new(),
            list_scroll: ScrollHandle::new(),
            h_scroll: ScrollHandle::new(),
            column_widths: [None; ListColumn::COUNT],
            hidden_columns: [false; ListColumn::COUNT],
            resizing: None,
        }
    }

    fn is_hidden(&self, column: ListColumn) -> bool {
        self.hidden_columns[column as usize]
    }

    fn visible_columns(&self) -> impl Iterator<Item = ListColumn> + '_ {
        ListColumn::ALL
            .into_iter()
            .filter(|column| !self.is_hidden(*column))
    }

    /// Width of the name column; unset means fill the visible list area so
    /// the other columns start exactly past its right edge.
    fn resolved_name_width(&self) -> f32 {
        if let Some(width) = self.column_widths[ListColumn::Name as usize] {
            return width;
        }
        let viewport = f32::from(self.h_scroll.bounds().size.width);
        if viewport > 0. {
            (viewport - (2. * ROW_PADDING_X + ICON_COL_WIDTH + COLUMN_GAP)).max(NAME_MIN_WIDTH)
        } else {
            280.
        }
    }

    fn column_width(&self, column: ListColumn) -> f32 {
        if column == ListColumn::Name {
            return self.resolved_name_width();
        }
        self.column_widths[column as usize].unwrap_or(column.default_width())
    }

    fn set_column_width(&mut self, column: ListColumn, width: f32) {
        let min = if column == ListColumn::Name {
            NAME_MIN_WIDTH
        } else {
            COLUMN_MIN_WIDTH
        };
        self.column_widths[column as usize] = Some(width.max(min));
    }

    fn reset_column_width(&mut self, column: ListColumn) {
        self.column_widths[column as usize] = None;
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
            self.open_file(entry, cx);
            cx.notify();
        }
    }

    fn open_file(&mut self, entry: &SftpEntry, cx: &mut Context<Self>) {
        let mut hasher = DefaultHasher::new();
        entry.path.hash(&mut hasher);
        let dir = std::env::temp_dir()
            .join("Oxidal")
            .join(format!("{:016x}", hasher.finish()));
        if let Err(err) = std::fs::create_dir_all(&dir) {
            self.error = Some(format!("Couldn't prepare temp folder: {err}"));
            cx.notify();
            return;
        }
        self.client
            .download_and_open(entry.path.clone(), dir.join(safe_local_name(&entry.name)));
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
                        .child(
                            Button::new("cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
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
                        .child(
                            Button::new("cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
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
                        .child(
                            Button::new("cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
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
        let suggested = safe_local_name(&entry.name);
        let rx = cx.prompt_for_new_path(&start_dir, Some(&suggested));
        let client = self.client.clone();
        cx.spawn(async move |_this, _cx| {
            if let Ok(Ok(Some(local))) = rx.await {
                client.download(entry.path.clone(), local);
            }
        })
        .detach();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let show_hidden = self.show_hidden;
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
                    .icon(IconName::Redo2)
                    .tooltip("Refresh")
                    .on_click(cx.listener(|panel, _, _, cx| panel.refresh(cx))),
            )
            .child(
                Button::new("sftp-new-folder")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Folder)
                    .tooltip("New folder")
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
            .child(div().flex_1())
            .child(
                Button::new("sftp-more")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Ellipsis)
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, _cx| {
                        menu.item(
                            PopupMenuItem::new("Show hidden files")
                                .checked(show_hidden)
                                .on_click(window.listener_for(&view, |panel, _, _, cx| {
                                    panel.show_hidden = !panel.show_hidden;
                                    (panel.on_show_hidden_changed)(panel.show_hidden, cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    fn header_cell(&self, column: ListColumn, cx: &mut Context<Self>) -> AnyElement {
        div()
            .relative()
            .w(px(self.column_width(column)))
            .flex_none()
            .h_full()
            .flex()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .when(column.right_align(), |this| this.text_right())
                    .child(column.label()),
            )
            .child(
                div()
                    .id(("sftp-col-resize", column as usize))
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(-5.))
                    .w(px(7.))
                    .cursor_col_resize()
                    .occlude()
                    .flex()
                    .justify_center()
                    .child(div().w(px(1.)).h_full().bg(cx.theme().border))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |panel, event: &MouseDownEvent, _, cx| {
                            if event.click_count >= 2 {
                                panel.reset_column_width(column);
                                panel.resizing = None;
                            } else {
                                panel.resizing = Some(ColumnResize {
                                    column,
                                    start_width: panel.column_width(column),
                                    start_x: event.position.x,
                                });
                            }
                            cx.notify();
                        }),
                    )
                    .on_drag(ColumnDrag(cx.entity_id()), |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    }),
            )
            .into_any_element()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut header = h_flex()
            .items_center()
            .gap_2()
            .px_2()
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().muted_foreground)
            .whitespace_nowrap()
            .child(div().w(px(ICON_COL_WIDTH)).flex_none());
        for column in self.visible_columns() {
            header = header.child(self.header_cell(column, cx));
        }
        header.into_any_element()
    }

    fn render_row(&self, entry: &SftpEntry, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let selected = self.selected.as_deref() == Some(entry.path.as_str());
        let row_entry = entry.clone();
        let row_entry_click = entry.clone();
        let is_dir = entry.is_dir;

        let mut row = h_flex()
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
            .when(!selected, |this| {
                this.hover(|this| this.bg(cx.theme().accent))
            })
            .on_click(cx.listener(move |panel, event: &ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    panel.open_entry(&row_entry_click, cx);
                } else {
                    panel.selected = Some(row_entry_click.path.clone());
                    cx.notify();
                }
            }))
            .child(div().w(px(ICON_COL_WIDTH)).flex_none().child(if entry.is_dir {
                Icon::new(IconName::Folder)
                    .small()
                    .text_color(cx.theme().warning)
            } else {
                Icon::new(IconName::File)
                    .small()
                    .text_color(cx.theme().muted_foreground)
            }))
            .child(
                div()
                    .w(px(self.resolved_name_width()))
                    .flex_none()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_sm()
                    .text_ellipsis_middle()
                    .when(entry.is_dir, |this| this.font_weight(FontWeight::MEDIUM))
                    .child(SharedString::from(entry.name.clone())),
            );

        for column in self.visible_columns() {
            let value = match column {
                ListColumn::Name => continue,
                ListColumn::Size => {
                    if entry.is_dir {
                        String::new()
                    } else {
                        format_size(entry.size)
                    }
                }
                ListColumn::Kind => format_kind(entry),
                ListColumn::Modified => entry.modified.map(format_modified).unwrap_or_default(),
                ListColumn::Accessed => entry.accessed.map(format_modified).unwrap_or_default(),
                ListColumn::Access => format_permissions(entry.is_dir, entry.permissions),
                ListColumn::Owner => entry.owner.clone().unwrap_or_default(),
                ListColumn::Group => entry.group.clone().unwrap_or_default(),
            };
            row = row.child(
                div()
                    .w(px(self.column_width(column)))
                    .flex_none()
                    .text_xs()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_color(cx.theme().muted_foreground)
                    .when(column.right_align(), |this| this.text_right())
                    .child(value),
            );
        }

        row.context_menu(move |menu, window, _cx| {
                let entry = row_entry.clone();
                let mut menu = menu;
                if !is_dir {
                    let open_entry = entry.clone();
                    menu = menu.item(PopupMenuItem::new("Open").on_click(
                        window.listener_for(&view, move |panel, _, _, cx| {
                            panel.open_file(&open_entry, cx)
                        }),
                    ));
                    let entry = entry.clone();
                    menu = menu.item(PopupMenuItem::new("Download").on_click(
                        window.listener_for(&view, move |panel, _, _, cx| {
                            panel.download(entry.clone(), cx)
                        }),
                    ));
                    menu = menu.separator();
                }
                menu = menu.item(PopupMenuItem::new("Rename").on_click(window.listener_for(
                    &view,
                    {
                        let entry = entry.clone();
                        move |panel, _, window, cx| panel.rename_dialog(entry.clone(), window, cx)
                    },
                )));
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        if self.synced_path != self.current_path {
            self.synced_path = self.current_path.clone();
            let value = self.current_path.clone();
            self.path_input
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }

        let view = cx.entity();
        let columns_view = cx.entity();
        let hidden_columns = self.hidden_columns;
        let visible: Vec<&SftpEntry> = self
            .entries
            .iter()
            .filter(|entry| self.show_hidden || !entry.name.starts_with('.'))
            .collect();
        let no_rows = visible.is_empty();
        let rows = visible
            .into_iter()
            .map(|entry| self.render_row(entry, cx))
            .collect::<Vec<_>>();
        let name_width = self.resolved_name_width();
        // Explicit strip width: a flex child would otherwise be shrunk to the
        // scroll container's width, leaving nothing to scroll horizontally.
        let content_width = 2. * ROW_PADDING_X
            + ICON_COL_WIDTH
            + self.visible_columns().count() as f32 * COLUMN_GAP
            + self
                .visible_columns()
                .map(|column| self.column_width(column))
                .sum::<f32>();

        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .w_full()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Input::new(&self.path_input).small()),
            )
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
                    .id("sftp-list-area")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .when(self.resizing.is_some(), |this| this.cursor_col_resize())
                    .on_drag_move(cx.listener(
                        |panel, event: &DragMoveEvent<ColumnDrag>, _, cx| {
                            let ColumnDrag(entity_id) = event.drag(cx);
                            if *entity_id != cx.entity_id() {
                                return;
                            }
                            if let Some(resize) = &panel.resizing {
                                let width = resize.start_width
                                    + f32::from(event.event.position.x - resize.start_x);
                                panel.set_column_width(resize.column, width);
                                cx.notify();
                            }
                        },
                    ))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|panel, _: &MouseUpEvent, _, cx| {
                            if panel.resizing.take().is_some() {
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|panel, _: &MouseUpEvent, _, cx| {
                            if panel.resizing.take().is_some() {
                                cx.notify();
                            }
                        }),
                    )
                    .child(
                        div()
                            .id("sftp-hscroll")
                            .size_full()
                            .overflow_x_scroll()
                            .track_scroll(&self.h_scroll)
                            .child(
                                v_flex()
                                    .h_full()
                                    .w(px(content_width))
                                    .min_w(relative(1.))
                                    .flex_none()
                                    .child(self.render_header(cx))
                                    .child(
                                        div()
                                            .id("sftp-entries")
                                            .flex_1()
                                            .min_h_0()
                                            .overflow_y_scroll()
                                            .track_scroll(&self.list_scroll)
                                            .child(
                                                v_flex().min_w(relative(1.)).children(rows).when(
                                                    no_rows,
                                                    |this| {
                                                        if self.loading {
                                                            const BAR_WIDTHS: [f32; 8] = [
                                                                0.45, 0.7, 0.55, 0.8, 0.35, 0.65,
                                                                0.5, 0.75,
                                                            ];
                                                            this.children((0..8).map(|ix| {
                                                                h_flex()
                                                                    .items_center()
                                                                    .gap_2()
                                                                    .px_2()
                                                                    .py_1()
                                                                    .child(
                                                                        Skeleton::new()
                                                                            .size_4()
                                                                            .rounded_full()
                                                                            .flex_none(),
                                                                    )
                                                                    .child(
                                                                        Skeleton::new()
                                                                            .secondary()
                                                                            .h_3()
                                                                            .w(px(name_width
                                                                                * BAR_WIDTHS[ix]))
                                                                            .rounded_md(),
                                                                    )
                                                            }))
                                                        } else {
                                                            this.child(
                                                                div()
                                                                    .p_4()
                                                                    .text_sm()
                                                                    .text_center()
                                                                    .text_color(
                                                                        cx.theme().muted_foreground,
                                                                    )
                                                                    .child(
                                                                        if self.entries.is_empty() {
                                                                            "Empty directory"
                                                                        } else {
                                                                            "Only hidden files here"
                                                                        },
                                                                    ),
                                                            )
                                                        }
                                                    },
                                                ),
                                            )
                                            .context_menu(move |menu, window, _cx| {
                                                menu.item(PopupMenuItem::new("New Folder").on_click(
                                                    window.listener_for(
                                                        &view,
                                                        |panel, _, window, cx| {
                                                            panel.new_folder_dialog(window, cx)
                                                        },
                                                    ),
                                                ))
                                                .item(PopupMenuItem::new("Upload").on_click(
                                                    window.listener_for(&view, |panel, _, _, cx| {
                                                        panel.upload(cx)
                                                    }),
                                                ))
                                                .separator()
                                                .item(PopupMenuItem::new("Refresh").on_click(
                                                    window.listener_for(&view, |panel, _, _, cx| {
                                                        panel.refresh(cx)
                                                    }),
                                                ))
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(HEADER_HEIGHT))
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .child(Scrollbar::vertical(&self.list_scroll)),
                    )
                    .child(Scrollbar::horizontal(&self.h_scroll))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .h(px(HEADER_HEIGHT))
                            .px_1()
                            .flex()
                            .items_center()
                            .bg(cx.theme().sidebar)
                            .border_b_1()
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .child(
                                Button::new("sftp-columns")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::EllipsisVertical)
                                    .tooltip("Show or hide columns")
                                    .dropdown_menu_with_anchor(Anchor::TopRight, {
                                        move |menu, window, _cx| {
                                            let mut menu = menu;
                                            for column in ListColumn::ALL.into_iter().skip(1) {
                                                let columns_view = columns_view.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(column.label())
                                                        .checked(
                                                            !hidden_columns[column as usize],
                                                        )
                                                        .on_click(window.listener_for(
                                                            &columns_view,
                                                            move |panel, _, _, cx| {
                                                                let index = column as usize;
                                                                panel.hidden_columns[index] =
                                                                    !panel.hidden_columns[index];
                                                                cx.notify();
                                                            },
                                                        )),
                                                );
                                            }
                                            menu
                                        }
                                    }),
                            ),
                    ),
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
                        .child(
                            Progress::new("sftp-transfer")
                                .value(transfer.percent())
                                .small(),
                        ),
                )
            })
            .into_any_element()
    }
}
