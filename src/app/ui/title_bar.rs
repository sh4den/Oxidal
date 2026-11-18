use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    prelude::FluentBuilder as _,
};

use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{IconName, Sizable as _, TitleBar, button::ButtonVariants as _, h_flex};

use super::about::open_about_dialog;
use crate::app::{OxidalApp, UpdateState};

impl OxidalApp {
    pub(super) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let update_button = match &self.update_state {
            UpdateState::Idle => None,
            UpdateState::Available(found) => Some(
                Button::new("update")
                    .primary()
                    .small()
                    .icon(IconName::ArrowDown)
                    .label("Download update")
                    .tooltip(format!("Version {}", found.version))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.start_update_download(cx);
                    })),
            ),
            UpdateState::Downloading(_) => Some(
                Button::new("update")
                    .ghost()
                    .small()
                    .loading(true)
                    .loading_icon(IconName::Loader)
                    .label("Downloading update"),
            ),
            UpdateState::Ready(_) => Some(
                Button::new("update")
                    .primary()
                    .small()
                    .icon(IconName::Redo2)
                    .label("Restart to update")
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.restart_to_update(window, cx);
                    })),
            ),
        };

        let this = cx.entity();

        TitleBar::new()
            .child(
                Button::new("application-menu")
                    .icon(IconName::Menu)
                    .ghost()
                    .small()
                    .dropdown_menu(move |menu, _window, _cx| {
                        let this = this.clone();
                        menu.item(
                            PopupMenuItem::new("Settings")
                                .icon(IconName::Settings)
                                .on_click(move |_, window, cx| {
                                    this.update(cx, |view, cx| {
                                        view.open_settings_tab(window, cx);
                                    });
                                }),
                        )
                        .item(PopupMenuItem::new("About").icon(IconName::Info).on_click(
                            |_, window, cx| {
                                open_about_dialog(window, cx);
                            },
                        ))
                        .separator()
                        .item(
                            PopupMenuItem::new("Exit")
                                .icon(IconName::WindowClose)
                                .on_click(|_, _, cx| {
                                    cx.quit();
                                }),
                        )
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .pr_2()
                    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation()
                    })
                    .when_some(update_button, |this, button| this.child(button)),
            )
    }
}
