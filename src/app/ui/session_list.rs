use crate::session::{Session, SessionFolder, SessionKind};
use crate::session_dialog;
use gpui_kit::assets::IconName as Lucide;

use gpui_kit::{
    Context, Div, ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};

use gpui_kit::component::button::Button;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, button::ButtonVariants as _, h_flex,
    input::Input, tooltip::Tooltip, v_flex,
};

use crate::app::OxidalApp;
use crate::app::drag::{
    DragPreview, DragSource, DropTarget, FolderDrag, PreviewKind, SessionDrag, start_drag,
};
use crate::app::sessions::matches_filter;

const FOLDER_GUIDE_INDENT: gpui_kit::Pixels = px(18.);
// Row chrome around a label: margins, padding, icon, gaps and the hover buttons.
const ROW_CHROME_WIDTH: f32 = 132.;
const APPROX_CHAR_WIDTH: f32 = 7.;

fn label_capacity(sidebar_width: gpui_kit::Pixels) -> usize {
    let text_width = f32::from(sidebar_width) - ROW_CHROME_WIDTH;
    ((text_width / APPROX_CHAR_WIDTH).floor() as usize).max(6)
}

fn truncating_label(
    id: impl Into<ElementId>,
    text: SharedString,
    capacity: usize,
) -> Stateful<Div> {
    div()
        .id(id)
        .truncate()
        .when(text.chars().count() > capacity, {
            let full = text.clone();
            move |this| this.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
        })
        .child(text)
}

impl OxidalApp {
    fn render_session_row(&self, item: &Session, cx: &mut Context<Self>) -> impl IntoElement {
        let id = item.id;
        let weak_app = cx.weak_entity();
        let selected = self.selected_session == Some(id);
        let open = self.is_open(id);
        let group_name = SharedString::from(format!("session-{id}"));
        let name = SharedString::from(item.name.clone());
        let icon = item.display_icon();
        let color = item.color.hsla();
        let accent = color.unwrap_or(cx.theme().primary);
        let detail = SharedString::from(match item.kind {
            SessionKind::Local => item.detail(),
            kind => format!("{} · {}", kind.label(), item.detail()),
        });
        let capacity = self.label_capacity(cx);
        let hover_bg = cx.theme().sidebar_accent.opacity(0.5);

        h_flex()
            .id(SharedString::from(format!("session-{id}")))
            .group(group_name.clone())
            .relative()
            .items_center()
            .gap_2()
            .px_2()
            .pt(px(2.))
            .pb_1()
            .mx_1()
            .rounded_md()
            .border_t_2()
            .border_color(gpui_kit::transparent_black())
            .cursor_pointer()
            .when(selected, |this| {
                this.bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(6.))
                            .bottom(px(6.))
                            .w(px(3.))
                            .rounded_full()
                            .bg(accent),
                    )
            })
            .when(!selected, |this| {
                this.hover(move |style| style.bg(hover_bg))
            })
            .on_click(
                cx.listener(move |view, event: &gpui_kit::ClickEvent, window, cx| {
                    if event.click_count() >= 2 {
                        view.connect_session(id, window, cx);
                    } else {
                        view.selected_session = Some(id);
                        cx.notify();
                    }
                }),
            )
            .when(self.is_dragging(DragSource::Session(id), cx), |this| {
                this.opacity(0.4)
            })
            .on_drag(
                SessionDrag {
                    id,
                    preview: DragPreview {
                        icon: icon.clone(),
                        color,
                        label: name.clone(),
                        kind: PreviewKind::Row {
                            detail: detail.clone(),
                        },
                        grab: Default::default(),
                    },
                },
                move |drag, grab, _, cx| {
                    start_drag(
                        &weak_app,
                        DragSource::Session(drag.id),
                        &drag.preview,
                        grab,
                        cx,
                    )
                },
            )
            .drag_over::<SessionDrag>(move |style, drag, _, cx| {
                if drag.id == id {
                    style
                } else {
                    style.border_color(cx.theme().primary)
                }
            })
            .on_drop(cx.listener(move |view, drag: &SessionDrag, _, cx| {
                if drag.id != id {
                    view.move_session(drag.id, DropTarget::Before(id), cx);
                }
            }))
            .child(
                div()
                    .flex_none()
                    .size(px(24.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(match color {
                        Some(color) => color.opacity(0.18),
                        None => cx.theme().sidebar_foreground.opacity(0.1),
                    })
                    .child(
                        Icon::empty()
                            .path(icon)
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
                            .min_w_0()
                            .child(
                                truncating_label(
                                    SharedString::from(format!("name-{id}")),
                                    name.clone(),
                                    capacity,
                                )
                                .min_w_0()
                                .text_sm(),
                            )
                            .when(open, |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .size(px(6.))
                                        .rounded_full()
                                        .bg(cx.theme().success),
                                )
                            }),
                    )
                    .child(
                        truncating_label(
                            SharedString::from(format!("detail-{id}")),
                            detail,
                            capacity,
                        )
                        .text_xs()
                        .text_color(cx.theme().muted_foreground),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .invisible()
                    .group_hover(group_name, |this| this.visible())
                    .child(
                        Button::new(SharedString::from(format!("edit-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Settings2)
                            .tooltip("Edit")
                            .on_click(cx.listener(move |view, _, _window, cx| {
                                let Some(session) = view.session_with_secrets(id) else {
                                    return;
                                };
                                view.open_session_window(Some(session), cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Delete)
                            .tooltip("Delete")
                            .on_click(cx.listener(move |_view, _, _window, cx| {
                                let weak_app = cx.weak_entity();
                                let name = name.clone();
                                crate::dialog_window::open(
                                    "Delete Session",
                                    px(400.),
                                    cx,
                                    move |dialog, _window, _cx| {
                                        let weak_app = weak_app.clone();
                                        dialog
                                            .child(format!(
                                                "Delete \"{name}\"? This also removes its saved \
                                                 password and closes any open tabs for it."
                                            ))
                                            .button(
                                                Button::new("cancel").label("Cancel").on_click(
                                                    |_, window, _| window.remove_window(),
                                                ),
                                            )
                                            .button(
                                                Button::new("delete")
                                                    .danger()
                                                    .label("Delete")
                                                    .on_click(move |_, window, cx| {
                                                        let _ = weak_app.update(cx, |app, cx| {
                                                            app.delete_session(id, cx);
                                                        });
                                                        window.remove_window();
                                                    }),
                                            )
                                    },
                                );
                            })),
                    ),
            )
    }

    fn label_capacity(&self, cx: &gpui_kit::App) -> usize {
        label_capacity(self.sidebar_width(cx))
    }

    fn render_folder_row(
        &self,
        folder: &SessionFolder,
        count: usize,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let folder_id = folder.id;
        let group_name = SharedString::from(format!("folder-{folder_id}"));
        let capacity = self.label_capacity(cx);
        let hover_bg = cx.theme().sidebar_accent.opacity(0.5);
        let weak_app = cx.weak_entity();
        let preview = DragPreview {
            icon: folder.display_icon(),
            color: folder.color.hsla(),
            label: SharedString::from(folder.name.clone()),
            kind: PreviewKind::Row {
                detail: SharedString::from(match count {
                    1 => "1 session".to_string(),
                    n => format!("{n} sessions"),
                }),
            },
            grab: Default::default(),
        };

        h_flex()
            .id(SharedString::from(format!("folder-{folder_id}")))
            .group(group_name.clone())
            .items_center()
            .gap_1()
            .px_2()
            .pt(px(2.))
            .pb_1()
            .mx_1()
            .rounded_md()
            .border_t_2()
            .border_color(gpui_kit::transparent_black())
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .when(
                self.is_dragging(DragSource::Folder(folder_id), cx),
                |this| this.opacity(0.4),
            )
            .on_drag(
                FolderDrag {
                    id: folder_id,
                    preview,
                },
                move |drag, grab, _, cx| {
                    start_drag(
                        &weak_app,
                        DragSource::Folder(drag.id),
                        &drag.preview,
                        grab,
                        cx,
                    )
                },
            )
            .drag_over::<SessionDrag>(|style, _, _, cx| style.bg(cx.theme().primary.opacity(0.12)))
            .on_drop(cx.listener(move |view, drag: &SessionDrag, _, cx| {
                view.move_session(drag.id, DropTarget::Into(Some(folder_id)), cx);
            }))
            .drag_over::<FolderDrag>(move |style, drag, _, cx| {
                if drag.id == folder_id {
                    style
                } else {
                    style.border_color(cx.theme().primary)
                }
            })
            .on_drop(cx.listener(move |view, drag: &FolderDrag, _, cx| {
                if drag.id != folder_id {
                    view.move_folder(drag.id, Some(folder_id), cx);
                }
            }))
            .on_click(cx.listener(move |view, _, _, cx| {
                view.toggle_folder_collapsed(folder_id, cx);
            }))
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall()
                .text_color(cx.theme().muted_foreground),
            )
            .child(
                Icon::empty()
                    .path(folder.display_icon())
                    .small()
                    .when_some(folder.color.hsla(), |this, color| this.text_color(color)),
            )
            .child(
                truncating_label(
                    SharedString::from(format!("folder-name-{folder_id}")),
                    SharedString::from(folder.name.clone()),
                    capacity,
                )
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD),
            )
            .child(
                div()
                    .flex_none()
                    .px_1p5()
                    .rounded_full()
                    .text_xs()
                    .bg(cx.theme().sidebar_accent)
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(count.to_string())),
            )
            .child(
                h_flex()
                    .gap_1()
                    .invisible()
                    .group_hover(group_name, |this| this.visible())
                    .child(
                        Button::new(SharedString::from(format!("edit-folder-{folder_id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Settings2)
                            .tooltip("Edit")
                            .on_click(cx.listener(move |view, _, _window, cx| {
                                let Some(folder) =
                                    view.folders.iter().find(|f| f.id == folder_id).cloned()
                                else {
                                    return;
                                };
                                let weak_app = cx.weak_entity();
                                session_dialog::open_edit_folder_dialog(folder, weak_app, cx);
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("delete-folder-{folder_id}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Delete)
                            .tooltip("Delete Folder")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.delete_folder(folder_id, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_sessions_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.filter_query(cx);
        let filtering = !query.is_empty();
        let muted = cx.theme().muted_foreground;
        let mut rows: Vec<gpui_kit::AnyElement> = Vec::new();

        for folder in &self.folders {
            let folder_id = folder.id;
            let children: Vec<&Session> = self
                .sessions
                .iter()
                .filter(|s| s.folder_id == Some(folder_id) && matches_filter(s, &query))
                .collect();
            if filtering && children.is_empty() {
                continue;
            }
            let collapsed = !filtering && self.collapsed_folders.contains(&folder_id);
            rows.push(self.render_folder_row(folder, children.len(), collapsed, cx));

            if !collapsed && !children.is_empty() {
                let nested: Vec<gpui_kit::AnyElement> = children
                    .into_iter()
                    .map(|item| self.render_session_row(item, cx).into_any_element())
                    .collect();
                rows.push(
                    v_flex()
                        .ml(FOLDER_GUIDE_INDENT)
                        .pl_1()
                        .border_l_1()
                        .border_color(cx.theme().sidebar_border)
                        .children(nested)
                        .into_any_element(),
                );
            }
        }

        let loose: Vec<&Session> = self
            .sessions
            .iter()
            .filter(|s| s.folder_id.is_none() && matches_filter(s, &query))
            .collect();
        if !loose.is_empty() {
            if !rows.is_empty() {
                let first = loose[0].id;
                rows.push(
                    h_flex()
                        .id("ungrouped-sessions")
                        .items_center()
                        .gap_2()
                        .mx_1()
                        .mt_2()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .drag_over::<SessionDrag>(|style, _, _, cx| {
                            style.bg(cx.theme().primary.opacity(0.12))
                        })
                        .on_drop(cx.listener(move |view, drag: &SessionDrag, _, cx| {
                            view.move_session(drag.id, DropTarget::Before(first), cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(muted)
                                .child("Other"),
                        )
                        .child(div().flex_1().h(px(1.)).bg(cx.theme().sidebar_border))
                        .into_any_element(),
                );
            }
            rows.extend(
                loose
                    .iter()
                    .map(|item| self.render_session_row(item, cx).into_any_element()),
            );
        }

        if rows.is_empty() {
            rows.push(
                div()
                    .px_3()
                    .py_4()
                    .text_xs()
                    .text_color(muted)
                    .child(if filtering {
                        "No sessions match"
                    } else {
                        "No sessions yet. Press + to add one."
                    })
                    .into_any_element(),
            );
        }

        rows.push(
            div()
                .id("sessions-drop-tail")
                .flex_1()
                .min_h(px(32.))
                .mx_1()
                .rounded_md()
                .drag_over::<SessionDrag>(|style, _, _, cx| {
                    style.bg(cx.theme().primary.opacity(0.12))
                })
                .on_drop(cx.listener(|view, drag: &SessionDrag, _, cx| {
                    view.move_session(drag.id, DropTarget::Into(None), cx);
                }))
                .drag_over::<FolderDrag>(|style, _, _, cx| {
                    style.bg(cx.theme().primary.opacity(0.12))
                })
                .on_drop(cx.listener(|view, drag: &FolderDrag, _, cx| {
                    view.move_folder(drag.id, None, cx);
                }))
                .into_any_element(),
        );

        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Sessions"),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("new-folder")
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(Lucide::FolderPlus))
                                    .tooltip("New Folder")
                                    .on_click(cx.listener(|_view, _, _window, cx| {
                                        let weak_app = cx.weak_entity();
                                        session_dialog::open_new_folder_dialog(weak_app, cx);
                                    })),
                            )
                            .child(
                                Button::new("add")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Plus)
                                    .tooltip("New Session")
                                    .on_click(cx.listener(|view, _, _window, cx| {
                                        view.open_session_window(None, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div().px_2().pb_2().child(
                    Input::new(&self.filter)
                        .small()
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(muted))
                        .cleanable(true),
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
}
