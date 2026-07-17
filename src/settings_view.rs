use gpui::{
    div, prelude::FluentBuilder as _, px, AppContext as _, Context, FontWeight, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectState},
    slider::{Slider, SliderEvent, SliderState},
    v_flex, ActiveTheme as _, IconName, IndexPath, Theme, ThemeMode, WindowExt as _,
};

use crate::settings::{self, AppSettings};

/// The "Settings" tab: terminal font and appearance (light/dark) preferences.
pub struct SettingsView {
    font_select: gpui::Entity<SelectState<SearchableVec<SharedString>>>,
    font_size_input: gpui::Entity<InputState>,
    opacity_slider: gpui::Entity<SliderState>,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = cx.global::<AppSettings>().clone();

        // Every font family the OS reports (DirectWrite on Windows, CoreText
        // on macOS, fontconfig on Linux), searchable in the dropdown. The
        // saved family is kept selectable even if it's no longer installed.
        let mut fonts: Vec<SharedString> = cx
            .text_system()
            .all_font_names()
            .into_iter()
            .map(SharedString::from)
            .collect();
        fonts.sort_by_key(|name| name.to_lowercase());
        fonts.dedup();
        let current_font = SharedString::from(current.font_family.clone());
        if !fonts.contains(&current_font) {
            fonts.insert(0, current_font.clone());
        }
        let selected = fonts
            .iter()
            .position(|font| *font == current_font)
            .map(|ix| IndexPath::default().row(ix));
        let font_select = cx.new(|cx| {
            SelectState::new(SearchableVec::new(fonts), selected, window, cx).searchable(true)
        });

        let font_size_input = cx
            .new(|cx| InputState::new(window, cx).default_value(current.font_size.to_string()));

        let opacity_slider = cx.new(|_| {
            SliderState::new()
                .min(0.3)
                .max(1.0)
                .step(0.05)
                .default_value(current.opacity.clamp(0.3, 1.0))
        });
        cx.subscribe_in(
            &opacity_slider,
            window,
            |_view, _, event: &SliderEvent, window, cx| match event {
                SliderEvent::Change(value) => {
                    cx.global_mut::<AppSettings>().opacity = value.start();
                    settings::apply_window_opacity(window, cx);
                }
                SliderEvent::Release(_) => {
                    settings::save_settings(cx.global::<AppSettings>());
                }
            },
        )
        .detach();

        cx.observe_global::<AppSettings>(|_, cx| cx.notify()).detach();

        Self {
            font_select,
            font_size_input,
            opacity_slider,
        }
    }

    fn apply_font(&self, window: &mut Window, cx: &mut Context<Self>) {
        let family = self
            .font_select
            .read(cx)
            .selected_value()
            .map(|font| font.to_string())
            .unwrap_or_default();
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
        // Theme::change rebuilt the colors, dropping the translucency tint.
        settings::apply_window_opacity(window, cx);
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
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .max_w(px(360.))
                            .pt_2()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Window Opacity"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{:.0}%",
                                                cx.global::<AppSettings>().opacity * 100.0
                                            )),
                                    ),
                            )
                            .child(Slider::new(&self.opacity_slider)),
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
                            .child(
                                Select::new(&self.font_select)
                                    .placeholder("Select a font")
                                    .search_placeholder("Search fonts..."),
                            ),
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
