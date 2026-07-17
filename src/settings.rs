use std::fs;
use std::path::PathBuf;

use gpui::{App, Global, Window, WindowBackgroundAppearance};
use gpui_component::Theme;
use serde::{Deserialize, Serialize};

/// User-configurable application preferences: terminal font and theme mode.
/// Stored as a GPUI global so any view can read the live value with
/// `cx.global::<AppSettings>()`, and react to changes via `cx.observe_global`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    pub font_family: String,
    pub font_size: f32,
    pub dark_mode: bool,
    /// Window background opacity, `0.3..=1.0`. Below 1.0 the window becomes
    /// translucent (glass-terminal style); text stays fully opaque.
    #[serde(default = "default_opacity")]
    pub opacity: f32,
}

impl Global for AppSettings {}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            font_family: default_font_family(),
            font_size: 14.0,
            dark_mode: false,
            opacity: default_opacity(),
        }
    }
}

fn default_opacity() -> f32 {
    1.0
}

/// Push the saved window opacity to the platform window and the theme.
/// `WindowBackgroundAppearance::Transparent` is the plain alpha mode
/// supported by macOS, Windows (DirectComposition), and composited Linux.
///
/// Exactly one translucent layer carries the effect: `tokens.background`,
/// the full-window quad painted by gpui-component's `Root`. The chrome
/// surfaces stacked on top of it (app background, sidebar, tab bar, title
/// bar) are made fully transparent instead of translucent — several stacked
/// alpha layers would compound right back toward opaque.
///
/// Theme colors are rebuilt by `Theme::change`, so this must be re-run
/// after every theme-mode switch.
pub fn apply_window_opacity(window: &mut Window, cx: &mut App) {
    let opacity = cx.global::<AppSettings>().opacity.clamp(0.3, 1.0);
    let translucent = opacity < 1.0;
    window.set_background_appearance(if translucent {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Opaque
    });

    // Rebuild pristine colors for the current mode first, so every tint
    // below starts from the theme's real values no matter how many times
    // this runs (and so 100% opacity restores everything, including the
    // dialog scrim).
    let mode = cx.global::<Theme>().mode;
    Theme::change(mode, None, cx);

    if translucent {
        let theme = Theme::global_mut(cx);
        // `colors.background` stays pristine on purpose: input and select
        // fields derive their fill from it, and translucent form controls
        // are unreadable. Nothing large paints it directly — the big
        // surfaces paint the tokens below.
        theme.colors.sidebar.a = 0.0;
        theme.colors.tab_bar.a = 0.0;
        theme.colors.title_bar.a = 0.0;
        // The dialog scrim covers the whole window; left dark it reads as
        // "transparency turned off" every time a dialog opens.
        theme.colors.overlay.a = 0.0;
        // Tokens are derived copies of the colors, and they — not the
        // colors — are what Root, TabBar, and Dialog actually paint.
        theme.tokens = (&theme.colors).into();
        theme.tokens.background = {
            let mut base = theme.tokens.background.color;
            base.a = opacity;
            base.into()
        };
    }
    window.refresh();
}

fn default_font_family() -> String {
    if cfg!(target_os = "windows") {
        "Consolas".to_string()
    } else if cfg!(target_os = "macos") {
        "Menlo".to_string()
    } else {
        "DejaVu Sans Mono".to_string()
    }
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Oxidal")
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn load_settings() -> AppSettings {
    match fs::read_to_string(settings_path()) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => AppSettings::default(),
    }
}

pub fn save_settings(settings: &AppSettings) {
    let dir = config_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = fs::write(settings_path(), json);
    }
}
