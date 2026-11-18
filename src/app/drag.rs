use crate::session::{Session, SessionFolder};

use gpui_kit::{
    AppContext as _, Context, Entity, FontWeight, Hsla, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use uuid::Uuid;

use super::OxidalApp;

#[derive(Clone)]
pub(super) struct DragPreview {
    pub(super) icon: SharedString,
    pub(super) color: Option<Hsla>,
    pub(super) label: SharedString,
    pub(super) kind: PreviewKind,
    pub(super) grab: gpui_kit::Point<gpui_kit::Pixels>,
}

#[derive(Clone)]
pub(super) enum PreviewKind {
    Row { detail: SharedString },
    Tab,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DragSource {
    Session(Uuid),
    Folder(Uuid),
    Tab(usize),
}

pub(super) fn start_drag(
    app: &WeakEntity<OxidalApp>,
    source: DragSource,
    preview: &DragPreview,
    grab: gpui_kit::Point<gpui_kit::Pixels>,
    cx: &mut gpui_kit::App,
) -> Entity<DragPreview> {
    let _ = app.update(cx, |app, cx| {
        app.dragging = Some(source);
        cx.notify();
    });
    let preview = DragPreview {
        grab,
        ..preview.clone()
    };
    cx.new(|_| preview)
}

#[derive(Clone)]
pub(super) struct SessionDrag {
    pub(super) id: Uuid,
    pub(super) preview: DragPreview,
}

#[derive(Clone)]
pub(super) struct FolderDrag {
    pub(super) id: Uuid,
    pub(super) preview: DragPreview,
}

#[derive(Clone)]
pub(super) struct TabDrag {
    pub(super) index: usize,
    pub(super) preview: DragPreview,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let accent = self.color.unwrap_or(theme.primary);

        let detail = match &self.kind {
            PreviewKind::Row { detail } => detail.clone(),
            PreviewKind::Tab => {
                return h_flex()
                    .h(px(32.))
                    .items_center()
                    .gap_2()
                    .pl_3()
                    .pr_2()
                    .bg(theme.tab_active)
                    .text_color(theme.tab_active_foreground)
                    .font_family(theme.font_family.clone())
                    .text_sm()
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .child(
                        Icon::empty()
                            .path(self.icon.clone())
                            .xsmall()
                            .when_some(self.color, |this, color| this.text_color(color)),
                    )
                    .child(self.label.clone())
                    .child(
                        div()
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(IconName::Close).with_size(px(11.))),
                    )
                    .into_any_element();
            }
        };

        let card = h_flex()
            .items_center()
            .gap_2()
            .min_w(px(160.))
            .max_w(px(280.))
            .pl_1p5()
            .pr_3()
            .py_1p5()
            .rounded_lg()
            .border_1()
            .border_color(accent.opacity(0.5))
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .font_family(theme.font_family.clone())
            .shadow_lg()
            .child(
                div()
                    .flex_none()
                    .size(px(24.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(accent.opacity(0.18))
                    .child(
                        Icon::empty()
                            .path(self.icon.clone())
                            .small()
                            .text_color(self.color.unwrap_or(theme.popover_foreground)),
                    ),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.label.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    ),
            );

        div()
            .pl((self.grab.x - px(20.)).max(px(0.)))
            .pt((self.grab.y - px(20.)).max(px(0.)))
            .child(card)
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
pub(super) enum DropTarget {
    Before(Uuid),
    Into(Option<Uuid>),
}

pub(super) fn reorder_sessions(sessions: &mut Vec<Session>, id: Uuid, target: DropTarget) -> bool {
    let Some(index) = sessions.iter().position(|s| s.id == id) else {
        return false;
    };
    let mut session = sessions.remove(index);
    match target {
        DropTarget::Before(target_id) => {
            let Some(at) = sessions.iter().position(|s| s.id == target_id) else {
                sessions.insert(index, session);
                return false;
            };
            session.folder_id = sessions[at].folder_id;
            sessions.insert(at, session);
        }
        DropTarget::Into(folder_id) => {
            session.folder_id = folder_id;
            sessions.push(session);
        }
    }
    true
}

pub(super) fn reorder_folders(
    folders: &mut Vec<SessionFolder>,
    id: Uuid,
    before: Option<Uuid>,
) -> bool {
    let Some(index) = folders.iter().position(|f| f.id == id) else {
        return false;
    };
    let folder = folders.remove(index);
    let at = match before {
        None => folders.len(),
        Some(target) => match folders.iter().position(|f| f.id == target) {
            Some(at) => at,
            None => {
                folders.insert(index, folder);
                return false;
            }
        },
    };
    folders.insert(at, folder);
    true
}

impl OxidalApp {
    pub(super) fn is_dragging(&self, source: DragSource, cx: &gpui_kit::App) -> bool {
        cx.has_active_drag() && self.dragging == Some(source)
    }
}

#[cfg(test)]
#[path = "../../tests/app/drag.rs"]
mod tests;
