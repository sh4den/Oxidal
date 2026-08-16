use std::path::PathBuf;

use gpui::{
    AppContext as _, Context, Div, Entity, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Colorize as _, Disableable as _, IconName, IndexPath, Sizable as _,
    ThemeMode, WindowExt as _,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{SearchableVec, Select, SelectState},
    slider::{Slider, SliderEvent, SliderState},
    v_flex,
};

use crate::settings::{self, AppSettings, EditUploadMode, RemoteOpenMode};
use crate::terminal::grid::TerminalPalette;
use crate::theme::{ColorSlot, GROUPS, ThemeSettings, all_slots};

const SWATCH_COLUMN: f32 = 188.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Theme,
    Terminal,
    Files,
}

impl Section {
    const ALL: &'static [Section] = &[Section::Theme, Section::Terminal, Section::Files];

    fn label(self) -> &'static str {
        match self {
            Section::Theme => "Theme",
            Section::Terminal => "Terminal",
            Section::Files => "Files",
        }
    }
}

pub struct SettingsView {
    section: Section,
    scroll: ScrollHandle,
    font_select: Entity<SelectState<SearchableVec<SharedString>>>,
    font_size_input: Entity<InputState>,
    opacity_slider: Entity<SliderState>,
    download_dir_input: Entity<InputState>,
    swatches: Vec<Entity<ColorPickerState>>,
    editing: ThemeMode,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = cx.global::<AppSettings>().clone();

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

        let font_size_input =
            cx.new(|cx| InputState::new(window, cx).default_value(current.font_size.to_string()));

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
                    settings::apply_appearance(window, cx);
                }
                SliderEvent::Release(_) => {
                    settings::save_settings(cx.global::<AppSettings>());
                }
            },
        )
        .detach();

        let download_dir_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(
                    settings::default_download_dir()
                        .to_string_lossy()
                        .to_string(),
                )
                .default_value(current.download_dir.clone().unwrap_or_default())
        });
        cx.subscribe(
            &download_dir_input,
            |view: &mut Self, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let folder = input.read(cx).value().to_string();
                    view.set_download_dir(folder, cx);
                }
            },
        )
        .detach();

        let editing = current.mode();
        let mut swatches = Vec::with_capacity(all_slots().count());
        for (index, slot) in all_slots().enumerate() {
            let color = current.theme.color(slot, editing);
            let state =
                cx.new(|cx| ColorPickerState::new(window, cx).default_value(color));
            cx.subscribe_in(
                &state,
                window,
                move |view, _, event: &ColorPickerEvent, window, cx| {
                    let ColorPickerEvent::Change(Some(color)) = event else {
                        return;
                    };
                    view.set_slot_color(index, *color, window, cx);
                },
            )
            .detach();
            swatches.push(state);
        }

        cx.observe_global::<AppSettings>(|_, cx| cx.notify())
            .detach();
        // Keeps the font preview in step with the picker and the size box.
        cx.observe(&font_select, |_, _, cx| cx.notify()).detach();
        cx.observe(&font_size_input, |_, _, cx| cx.notify()).detach();

        Self {
            section: Section::Theme,
            scroll: ScrollHandle::new(),
            font_select,
            font_size_input,
            opacity_slider,
            download_dir_input,
            swatches,
            editing,
        }
    }

    fn set_download_dir(&self, folder: String, cx: &mut Context<Self>) {
        let folder = folder.trim().to_string();
        cx.global_mut::<AppSettings>().download_dir = if folder.is_empty() {
            None
        } else {
            Some(folder)
        };
        settings::save_settings(cx.global::<AppSettings>());
        cx.notify();
    }

    fn pick_download_dir(&self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(SharedString::from("Choose")),
        });
        cx.spawn_in(window, async move |view, cx| {
            let Ok(Ok(Some(folder))) = rx.await else {
                return;
            };
            let Some(folder) = folder.first().map(|dir| dir.to_string_lossy().to_string()) else {
                return;
            };
            let _ = view.update_in(cx, |view, window, cx| {
                view.download_dir_input.update(cx, |state, cx| {
                    state.set_value(folder.clone(), window, cx);
                });
                view.set_download_dir(folder, cx);
            });
        })
        .detach();
    }

    fn reset_download_dir(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.download_dir_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.set_download_dir(String::new(), cx);
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

    fn set_column_hint(&self, show: bool, cx: &mut Context<Self>) {
        cx.global_mut::<AppSettings>().show_column_hint = show;
        settings::save_settings(cx.global::<AppSettings>());
        cx.refresh_windows();
    }

    fn set_edit_upload(&self, mode: EditUploadMode, cx: &mut Context<Self>) {
        cx.global_mut::<AppSettings>().edit_upload = Some(mode);
        settings::save_settings(cx.global::<AppSettings>());
        cx.notify();
    }

    fn set_remote_open(&self, mode: RemoteOpenMode, cx: &mut Context<Self>) {
        cx.global_mut::<AppSettings>().remote_open = Some(mode);
        settings::save_settings(cx.global::<AppSettings>());
        cx.notify();
    }

    fn set_dark_mode(&mut self, dark: bool, window: &mut Window, cx: &mut Context<Self>) {
        cx.global_mut::<AppSettings>().dark_mode = dark;
        settings::apply_appearance(window, cx);
        settings::save_settings(cx.global::<AppSettings>());
        self.set_editing(
            if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            window,
            cx,
        );
    }

    fn slot_at(index: usize) -> &'static ColorSlot {
        all_slots()
            .nth(index)
            .expect("swatches are built from the slot list")
    }

    fn set_slot_color(
        &mut self,
        index: usize,
        color: Hsla,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editing = self.editing;
        let slot = Self::slot_at(index);
        {
            let global = cx.global_mut::<AppSettings>();
            if global.theme.color(slot, editing) == color {
                return;
            }
            global.theme.set_color(slot, editing, color);
        }
        settings::save_settings(cx.global::<AppSettings>());
        if editing == cx.global::<AppSettings>().mode() {
            settings::apply_appearance(window, cx);
        }
        cx.notify();
    }

    /// Pushes whatever the theme currently says into every swatch, so switching
    /// the edited mode or loading a file leaves no stale color behind.
    fn sync_swatches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = cx.global::<AppSettings>().theme.clone();
        let editing = self.editing;
        for (index, state) in self.swatches.iter().enumerate() {
            let color = theme.color(Self::slot_at(index), editing);
            state.update(cx, |state, cx| state.set_value(color, window, cx));
        }
        cx.notify();
    }

    fn set_editing(&mut self, mode: ThemeMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing == mode {
            return;
        }
        self.editing = mode;
        self.sync_swatches(window, cx);
    }

    fn reset_colors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editing = self.editing;
        cx.global_mut::<AppSettings>().theme.reset(editing);
        settings::save_settings(cx.global::<AppSettings>());
        settings::apply_appearance(window, cx);
        self.sync_swatches(window, cx);
        window.push_notification("Colors reset to the built-in theme", cx);
    }

    fn export_theme(&self, window: &mut Window, cx: &mut Context<Self>) {
        let json = cx.global::<AppSettings>().theme.to_json();
        let start = settings::default_download_dir();
        let rx = cx.prompt_for_new_path(&start, Some("oxidal-theme.json"));
        cx.spawn_in(window, async move |view, cx| {
            let Ok(Ok(Some(path))) = rx.await else {
                return;
            };
            let written = std::fs::write(&path, json);
            let _ = view.update_in(cx, |_, window, cx| match written {
                Ok(()) => window.push_notification(
                    format!("Theme saved to {}", short_path(&path)),
                    cx,
                ),
                Err(err) => window.push_notification(format!("Could not save: {err}"), cx),
            });
        })
        .detach();
    }

    fn import_theme(&self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::from("Import")),
        });
        cx.spawn_in(window, async move |view, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.first().cloned() else {
                return;
            };
            let loaded = std::fs::read_to_string(&path)
                .map_err(|err| err.to_string())
                .and_then(|json| ThemeSettings::from_json(&json));

            let _ = view.update_in(cx, |view, window, cx| match loaded {
                Ok(theme) => {
                    cx.global_mut::<AppSettings>().theme = theme;
                    settings::save_settings(cx.global::<AppSettings>());
                    settings::apply_appearance(window, cx);
                    view.sync_swatches(window, cx);
                    window.push_notification(
                        format!("Imported {}", short_path(&path)),
                        cx,
                    );
                }
                Err(err) => window.push_notification(format!("Could not import: {err}"), cx),
            });
        })
        .detach();
    }
}

fn short_path(path: &PathBuf) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn group_title(title: impl Into<SharedString>, muted: Hsla) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(muted)
        .child(title.into())
}

/// One labelled control on its own line, with an optional hint underneath.
fn field(
    label: &'static str,
    hint: Option<&'static str>,
    control: impl IntoElement,
    muted: Hsla,
) -> impl IntoElement {
    v_flex()
        .gap_1p5()
        .child(
            v_flex()
                .gap_0p5()
                .child(div().text_sm().child(label))
                .when_some(hint, |this, hint| {
                    this.child(div().text_xs().text_color(muted).child(hint))
                }),
        )
        .child(control)
}

fn toggle_pair(
    left: (&'static str, &'static str, IconName),
    right: (&'static str, &'static str, IconName),
    left_selected: bool,
    on_left: impl Fn(&mut SettingsView, &mut Window, &mut Context<SettingsView>) + 'static,
    on_right: impl Fn(&mut SettingsView, &mut Window, &mut Context<SettingsView>) + 'static,
    cx: &mut Context<SettingsView>,
) -> Div {
    h_flex()
        .gap_1p5()
        .child(
            Button::new(left.0)
                .small()
                .icon(left.2)
                .label(left.1)
                .when(left_selected, |b| b.primary())
                .when(!left_selected, |b| b.outline())
                .on_click(cx.listener(move |view, _, window, cx| on_left(view, window, cx))),
        )
        .child(
            Button::new(right.0)
                .small()
                .icon(right.2)
                .label(right.1)
                .when(!left_selected, |b| b.primary())
                .when(left_selected, |b| b.outline())
                .on_click(cx.listener(move |view, _, window, cx| on_right(view, window, cx))),
        )
}

impl SettingsView {
    /// A row of tabs rather than a nav column: the window already stacks an icon
    /// rail and the session list down the left side.
    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.section;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let primary = cx.theme().primary;

        h_flex()
            .w_full()
            .flex_none()
            .gap_5()
            .px_8()
            .pt_5()
            .border_b_1()
            .border_color(cx.theme().border)
            .children(Section::ALL.iter().copied().map(|section| {
                let selected = section == active;
                div()
                    .id(section.label())
                    .pb_2p5()
                    .mb(px(-1.))
                    .text_sm()
                    .cursor_pointer()
                    .border_b_2()
                    .border_color(if selected { primary } else { gpui::transparent_black() })
                    .text_color(if selected { foreground } else { muted })
                    .when(selected, |this| this.font_weight(FontWeight::MEDIUM))
                    .when(!selected, |this| {
                        this.hover(|this| this.text_color(foreground))
                    })
                    .child(section.label())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.section = section;
                        view.scroll.set_offset(gpui::point(px(0.), px(0.)));
                        cx.notify();
                    }))
            }))
    }

    fn render_theme(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let is_dark = cx.global::<AppSettings>().dark_mode;
        let opacity = cx.global::<AppSettings>().opacity;
        let customized = cx.global::<AppSettings>().theme.is_customized(self.editing);

        let mode_toggle = toggle_pair(
            ("theme-light", "Light", IconName::Sun),
            ("theme-dark", "Dark", IconName::Moon),
            !is_dark,
            |view, window, cx| view.set_dark_mode(false, window, cx),
            |view, window, cx| view.set_dark_mode(true, window, cx),
            cx,
        );

        let theme = cx.global::<AppSettings>().theme.clone();
        let editing = self.editing;
        let foreground = cx.theme().foreground;
        let panel = cx.theme().muted.opacity(0.4);
        let hover = cx.theme().accent;

        let groups = GROUPS
            .iter()
            .map(|group| {
                let swatches = all_slots()
                    .enumerate()
                    .filter(|(_, slot)| slot.group == *group)
                    .map(|(index, slot)| {
                        h_flex()
                            .w(px(SWATCH_COLUMN))
                            .gap_2p5()
                            .items_center()
                            .px_2()
                            .py_1p5()
                            .rounded_md()
                            .hover(|this| this.bg(hover))
                            // The picker draws its own outline from the color it
                            // holds, which disappears when the color matches the
                            // page. This ring keeps every swatch findable.
                            .child(
                                div()
                                    .flex_none()
                                    .p(px(1.))
                                    .rounded(px(5.))
                                    .bg(border)
                                    .child(ColorPicker::new(&self.swatches[index]).small()),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(foreground)
                                            .truncate()
                                            .child(slot.label),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(theme.color(slot, editing).to_hex()),
                                    ),
                            )
                    })
                    .collect::<Vec<_>>();

                v_flex()
                    .gap_2()
                    .child(group_title(*group, muted))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_x_2()
                            .gap_y_1()
                            .p_2()
                            .rounded_lg()
                            .bg(panel)
                            .children(swatches),
                    )
            })
            .collect::<Vec<_>>();

        v_flex()
            .gap_5()
            .child(field(
                "Mode",
                Some("Light and dark keep their own colors. The swatches below edit the mode you are in."),
                mode_toggle,
                muted,
            ))
            .child(field(
                "Window opacity",
                Some("Lower values let the desktop show through the window."),
                v_flex()
                    .gap_1()
                    .max_w(px(320.))
                    .child(Slider::new(&self.opacity_slider))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{:.0}%", opacity * 100.0)),
                    ),
                muted,
            ))
            .child(div().w_full().h(px(1.)).bg(border))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_sm().child(if is_dark {
                        "Dark colors"
                    } else {
                        "Light colors"
                    }))
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(
                                Button::new("import-theme")
                                    .small()
                                    .outline()
                                    .icon(IconName::Inbox)
                                    .label("Import")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.import_theme(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("export-theme")
                                    .small()
                                    .outline()
                                    .icon(IconName::ExternalLink)
                                    .label("Export")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.export_theme(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("reset-theme")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Undo)
                                    .label("Reset")
                                    .disabled(!customized)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.reset_colors(window, cx);
                                    })),
                            ),
                    ),
            )
            .children(groups)
    }

    fn render_terminal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;

        let family = self
            .font_select
            .read(cx)
            .selected_value()
            .map(|font| font.to_string())
            .unwrap_or_else(|| cx.global::<AppSettings>().font_family.clone());
        let size = self
            .font_size_input
            .read(cx)
            .value()
            .to_string()
            .parse::<f32>()
            .unwrap_or(14.0)
            .clamp(8.0, 32.0);

        let palette = cx.global::<TerminalPalette>();
        let (surface, foreground, prompt, faded) = (
            palette.background,
            palette.foreground,
            palette.ansi[2],
            palette.ansi[8],
        );

        v_flex()
            .gap_5()
            .child(
                h_flex()
                    .gap_4()
                    .items_start()
                    .child(
                        div().w(px(300.)).child(field(
                            "Font family",
                            Some("Used by every terminal tab."),
                            Select::new(&self.font_select)
                                .placeholder("Select a font")
                                .search_placeholder("Search fonts..."),
                            muted,
                        )),
                    )
                    .child(div().w(px(140.)).child(field(
                        "Font size",
                        Some("8 to 32 pixels."),
                        Input::new(&self.font_size_input),
                        muted,
                    ))),
            )
            .child(
                v_flex()
                    .gap_1p5()
                    .child(div().text_sm().child("Preview"))
                    .child(
                        v_flex()
                            .w_full()
                            .gap_0p5()
                            .px_3()
                            .py_2p5()
                            .rounded_md()
                            .border_1()
                            .border_color(border)
                            .bg(surface)
                            .font_family(family)
                            .text_size(px(size))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().text_color(prompt).child("user@oxidal"))
                                    .child(div().text_color(foreground).child("~ %")),
                            )
                            .child(
                                div()
                                    .text_color(foreground)
                                    .child("The quick brown fox jumps over the lazy dog"),
                            )
                            .child(
                                div()
                                    .text_color(faded)
                                    .child("0123456789  il1 O0  {}[]()<>  =>  !=  --"),
                            ),
                    ),
            )
            .child(
                h_flex().child(
                    Button::new("apply-font")
                        .small()
                        .primary()
                        .label("Apply")
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.apply_font(window, cx);
                        })),
                ),
            )
    }

    fn render_files(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let show_column_hint = cx.global::<AppSettings>().show_column_hint;
        let hint_toggle = toggle_pair(
            ("column-hint-on", "Show", IconName::Eye),
            ("column-hint-off", "Hide", IconName::EyeOff),
            show_column_hint,
            |view, _, cx| view.set_column_hint(true, cx),
            |view, _, cx| view.set_column_hint(false, cx),
            cx,
        );

        let remote_open = cx.global::<AppSettings>().remote_open;
        let open_toggle = h_flex().gap_1p5().children(
            [
                (
                    "remote-open-editor",
                    "Built-in editor",
                    IconName::SquareTerminal,
                    RemoteOpenMode::Editor,
                ),
                (
                    "remote-open-default",
                    "Default app",
                    IconName::ExternalLink,
                    RemoteOpenMode::DefaultApp,
                ),
                (
                    "remote-open-ask",
                    "Ask each time",
                    IconName::Bell,
                    RemoteOpenMode::Ask,
                ),
            ]
            .map(|(id, label, icon, mode)| {
                let selected = remote_open == Some(mode);
                Button::new(id)
                    .small()
                    .icon(icon)
                    .label(label)
                    .when(selected, |b| b.primary())
                    .when(!selected, |b| b.outline())
                    .on_click(
                        cx.listener(move |view, _, _, cx| view.set_remote_open(mode, cx)),
                    )
            }),
        );

        let edit_upload = cx.global::<AppSettings>().edit_upload;
        let edit_upload_toggle = toggle_pair(
            ("edit-upload-ask", "Ask each time", IconName::Bell),
            ("edit-upload-auto", "Upload automatically", IconName::ArrowUp),
            edit_upload != Some(EditUploadMode::Auto),
            |view, _, cx| view.set_edit_upload(EditUploadMode::Ask, cx),
            |view, _, cx| view.set_edit_upload(EditUploadMode::Auto, cx),
            cx,
        );

        v_flex()
            .gap_5()
            .child(field(
                "Download folder",
                Some("Files downloaded from a remote folder are saved here."),
                v_flex()
                    .gap_2()
                    .max_w(px(420.))
                    .child(Input::new(&self.download_dir_input))
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(
                                Button::new("choose-download-dir")
                                    .outline()
                                    .small()
                                    .icon(IconName::FolderOpen)
                                    .label("Choose...")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.pick_download_dir(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("reset-download-dir")
                                    .ghost()
                                    .small()
                                    .label("Use default")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.reset_download_dir(window, cx);
                                    })),
                            ),
                    ),
                muted,
            ))
            .child(field(
                "Opening remote files",
                Some(
                    "How a double-clicked remote file opens. The built-in editor keeps the \
                     file in Oxidal's memory; the default app writes a temporary copy to \
                     this computer's disk.",
                ),
                open_toggle,
                muted,
            ))
            .child(field(
                "Edited remote files",
                Some(
                    "When a file opened from a remote server is saved on this computer, Oxidal \
                     can upload the changes back automatically or ask first.",
                ),
                edit_upload_toggle,
                muted,
            ))
            .child(field(
                "Column hint",
                Some(
                    "The strip that appears on hover when a file list has more columns than fit \
                     on screen.",
                ),
                hint_toggle,
                muted,
            ))
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.section {
            Section::Theme => self.render_theme(cx).into_any_element(),
            Section::Terminal => self.render_terminal(cx).into_any_element(),
            Section::Files => self.render_files(cx).into_any_element(),
        };

        v_flex()
            .size_full()
            .child(self.render_tabs(cx))
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(div().w_full().px_8().py_6().child(body)),
            )
    }
}
