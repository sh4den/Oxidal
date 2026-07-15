mod app;
mod session;
mod session_dialog;
mod settings;
mod settings_view;
mod terminal;

use gpui::{px, size, App, AppContext as _, Bounds, WindowBounds, WindowOptions};
use gpui_component::{Root, Theme, ThemeMode, TitleBar};

use crate::app::OxidalApp;

fn main() {
    let application = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    application.run(move |cx: &mut App| {
        // Must be called before using any GPUI Component features.
        gpui_component::init(cx);

        let settings = settings::load_settings();
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
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            let view = cx.new(|cx| OxidalApp::new(window, cx));
            // First level child of the window must be a Root.
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open window");
    });
}
