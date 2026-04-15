use gpui::{
    div, prelude::FluentBuilder as _, px, AppContext as _, Context, FontWeight, IntoElement,
    ParentElement as _, Render, Styled as _, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex, ActiveTheme as _, IconName, Theme, ThemeMode, WindowExt as _,
};

use crate::settings::{self, AppSettings};

/// The "Settings" tab: terminal font and appearance (light/dark) preferences.
pub struct SettingsView {
    font_family_input: gpui::Entity<InputState>,
    font_size_input: gpui::Entity<InputState>,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = cx.global::<AppSettings>().clone();
        let font_family_input =
            cx.new(|cx| InputState::new(window, cx).default_value(current.font_family.clone()));
        let font_size_input = cx
            .new(|cx| InputState::new(window, cx).default_value(current.font_size.to_string()));

        cx.observe_global::<AppSettings>(|_, cx| cx.notify()).detach();

        Self {
            font_family_input,
            font_size_input,
        }
    }

    fn apply_font(&self, window: &mut Window, cx: &mut Context<Self>) {
        let family = self.font_family_input.read(cx).value().to_string();
        let size: f32 = self
            .font_size_input
            .read(cx)
            .value()
            .to_string()
            .parse::<f32>()
            .unwrap_or(14.0)
            .clamp(8.0, 32.0);

        {
            let global = cx.global_mut::<AppSettings>();
            global.font_family = if family.trim().is_empty() {
                global.font_family.clone()
            } else {
                family
            };
            global.font_size = size;
        }
        settings::save_settings(cx.global::<AppSettings>());
        window.push_notification("Terminal font updated", cx);
    }

    fn set_dark_mode(&self, dark: bool, window: &mut Window, cx: &mut Context<Self>) {
        Theme::change(
            if dark { ThemeMode::Dark } else { ThemeMode::Light },
            Some(window),
            cx,
        );
        cx.global_mut::<AppSettings>().dark_mode = dark;
        settings::save_settings(cx.global::<AppSettings>());
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_dark = cx.theme().mode.is_dark();

        v_flex()
            .size_full()
            .p_6()
            .gap_6()
            .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child("Settings"))
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Appearance"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("theme-light")
                                    .icon(IconName::Sun)
                                    .label("Light")
                                    .when(!is_dark, |b| b.primary())
                                    .when(is_dark, |b| b.outline())
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.set_dark_mode(false, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("theme-dark")
                                    .icon(IconName::Moon)
                                    .label("Dark")
                                    .when(is_dark, |b| b.primary())
                                    .when(!is_dark, |b| b.outline())
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.set_dark_mode(true, window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .max_w(px(360.))
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Terminal Font"))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Font Family"))
                            .child(Input::new(&self.font_family_input)),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Font Size (px)"))
                            .child(Input::new(&self.font_size_input)),
                    )
                    .child(
                        Button::new("apply-font")
                            .primary()
                            .label("Apply")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.apply_font(window, cx);
                            })),
                    ),
            )
    }
}
