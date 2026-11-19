use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, canvas, div, prelude::FluentBuilder as _, px,
};

use gpui_kit::component::button::Button;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::ButtonVariants as _,
    h_flex,
    tab::{Tab, TabBar},
    v_flex,
};

use crate::app::drag::{DragPreview, DragSource, PreviewKind, TabDrag, start_drag};
use crate::app::{OxidalApp, TabContent};

impl OxidalApp {
    pub(super) fn render_workspace(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.tabs.is_empty() {
            return self.render_welcome(cx).into_any_element();
        }

        let active_index = self.active_tab.unwrap_or(0);
        let weak_app = cx.weak_entity();
        let tabs_overflow = self.tab_scroll.max_offset().x > px(0.);
        let new_tab_button = Button::new("new-tab-from-selection")
            .ghost()
            .xsmall()
            .icon(IconName::Plus)
            .tooltip("Connect selected session")
            .on_click(cx.listener(|view, _, window, cx| {
                if let Some(id) = view.selected_session {
                    view.connect_session(id, window, cx);
                }
            }));
        let tab_bar = TabBar::new("open-tabs")
            .track_scroll(&self.tab_scroll)
            .selected_index(active_index)
            .on_click(cx.listener(|view, index: &usize, _, cx| {
                view.active_tab = Some(*index);
                cx.notify();
            }))
            .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                let group_name = SharedString::from(format!("tab-{index}"));
                let preview = DragPreview {
                    icon: tab.icon.clone(),
                    color: tab.icon_color,
                    label: tab.title.clone(),
                    kind: PreviewKind::Tab,
                    grab: Default::default(),
                };
                let weak_app = weak_app.clone();
                Tab::new()
                    .group(group_name.clone())
                    .when(self.is_dragging(DragSource::Tab(index), cx), |this| {
                        this.opacity(0.4)
                    })
                    .on_drag(TabDrag { index, preview }, move |drag, grab, _, cx| {
                        start_drag(
                            &weak_app,
                            DragSource::Tab(drag.index),
                            &drag.preview,
                            grab,
                            cx,
                        )
                    })
                    .drag_over::<TabDrag>(move |style, drag, _, cx| {
                        if drag.index == index {
                            style
                        } else {
                            style.bg(cx.theme().primary.opacity(0.12))
                        }
                    })
                    .on_drop(cx.listener(move |view, drag: &TabDrag, _, cx| {
                        if drag.index != index {
                            view.move_tab(drag.index, index, cx);
                        }
                    }))
                    .prefix(
                        Icon::empty()
                            .path(tab.icon.clone())
                            .xsmall()
                            .when_some(tab.icon_color, |this, color| this.text_color(color)),
                    )
                    .pl_3()
                    .pr_2()
                    .label(tab.title.clone())
                    .suffix(
                        Button::new(SharedString::from(format!("close-tab-{index}")))
                            .ghost()
                            .xsmall()
                            .rounded(px(10.))
                            .icon(Icon::new(IconName::Close).with_size(px(11.)))
                            .when(index != active_index, |this| this.invisible())
                            .group_hover(group_name, |this| this.visible())
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.close_tab(index, cx);
                            })),
                    )
            }))
            .map(|this| {
                if tabs_overflow {
                    this.suffix(new_tab_button)
                } else {
                    this.last_empty_space(h_flex().px_1().child(new_tab_button))
                        .suffix(div())
                }
            });
        let entity_id = cx.entity_id();
        let tab_scroll = self.tab_scroll.clone();
        let overflow_watch = canvas(
            move |_, _, cx| {
                if (tab_scroll.max_offset().x > px(0.)) != tabs_overflow {
                    cx.notify(entity_id);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0();

        let content = self.tabs.get(active_index).map(|tab| match &tab.content {
            TabContent::Terminal(view) => view.clone().into_any_element(),
            TabContent::SshSession { terminal, .. } => terminal.clone().into_any_element(),
            TabContent::Sftp(view) => view.clone().into_any_element(),
            TabContent::Settings(view) => view.clone().into_any_element(),
            TabContent::Message(msg) => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Icon::new(IconName::TriangleAlert).with_size(px(32.)))
                .child(
                    div()
                        .text_sm()
                        .max_w(px(420.))
                        .text_center()
                        .child(msg.clone()),
                )
                .into_any_element(),
        });

        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(tab_bar)
            .child(overflow_watch)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .children(content),
            )
            .into_any_element()
    }
}
