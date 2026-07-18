#![windows_subsystem = "windows"]

mod app;
mod credentials;
mod session;
mod session_dialog;
mod settings;
mod settings_view;
mod sftp;
mod ssh_client;
mod terminal;
mod update;

use gpui::{
    App, AppContext as _, Bounds, KeyBinding, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, px, size,
};
use gpui_component::{Root, Theme, ThemeMode, TitleBar};

use crate::app::OxidalApp;
use crate::terminal::view::{CopySelection, PasteClipboard, SendTab, SendTabPrev};

fn main() {
    let application = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    application.run(move |cx: &mut App| {
        gpui_component::init(cx);

        cx.bind_keys([
            KeyBinding::new("tab", SendTab, Some("Terminal")),
            KeyBinding::new("shift-tab", SendTabPrev, Some("Terminal")),
            KeyBinding::new("ctrl-shift-c", CopySelection, Some("Terminal")),
            KeyBinding::new("ctrl-shift-v", PasteClipboard, Some("Terminal")),
        ]);

        let settings = settings::load_settings();
        let opacity = settings.opacity;
        let mode = if settings.dark_mode {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        Theme::change(mode, None, cx);
        cx.set_global(settings);

        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitleBar::title_bar_options()),
            window_min_size: Some(size(px(800.), px(560.))),
            window_background: if opacity < 1.0 {
                WindowBackgroundAppearance::Transparent
            } else {
                WindowBackgroundAppearance::Opaque
            },
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            settings::apply_window_opacity(window, cx);
            let view = cx.new(|cx| OxidalApp::new(window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open window");
    });
}
