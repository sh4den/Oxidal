use std::fs;
use std::path::PathBuf;

use gpui::{App, Global, Window, WindowBackgroundAppearance};
use gpui_component::Theme;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub font_family: String,
    pub font_size: f32,
    pub dark_mode: bool,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    #[serde(default)]
    pub download_dir: Option<String>,
    #[serde(default = "default_show_column_hint")]
    pub show_column_hint: bool,
}

fn default_show_column_hint() -> bool {
    true
}

impl Global for AppSettings {}

impl AppSettings {
    pub fn resolved_download_dir(&self) -> PathBuf {
        self.download_dir
            .as_ref()
            .map(|dir| dir.trim())
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_download_dir)
    }
}

pub fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            font_family: default_font_family(),
            font_size: 14.0,
            dark_mode: false,
            opacity: default_opacity(),
            sidebar_width: default_sidebar_width(),
            download_dir: None,
            show_column_hint: default_show_column_hint(),
        }
    }
}

fn default_opacity() -> f32 {
    1.0
}

pub const SIDEBAR_MIN_WIDTH: f32 = 240.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 800.0;

fn default_sidebar_width() -> f32 {
    320.0
}

pub fn apply_window_opacity(window: &mut Window, cx: &mut App) {
    let opacity = cx.global::<AppSettings>().opacity.clamp(0.3, 1.0);
    let translucent = opacity < 1.0;
    window.set_background_appearance(if translucent {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Opaque
    });

    let mode = cx.global::<Theme>().mode;
    Theme::change(mode, None, cx);

    if translucent {
        let theme = Theme::global_mut(cx);
        theme.colors.sidebar.a = 0.0;
        theme.colors.tab_bar.a = 0.0;
        theme.colors.title_bar.a = 0.0;
        theme.colors.overlay.a = 0.0;
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

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Oxidal")
}

pub fn prepare_config_dir() -> std::io::Result<PathBuf> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

pub fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    let staging = path.with_extension("tmp");
    let _ = fs::remove_file(&staging);
    fs::write(&staging, contents)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o600))?;
    }

    fs::rename(&staging, path)
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
    if prepare_config_dir().is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = write_private(&settings_path(), &json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_saved_before_the_sidebar_was_resizable_keep_their_values() {
        let existing = r#"{
            "font_family": "0xProto Nerd Font Mono",
            "font_size": 14.0,
            "dark_mode": true,
            "opacity": 1.0
        }"#;

        let settings: AppSettings =
            serde_json::from_str(existing).expect("a file without the new field should still load");

        assert_eq!(settings.font_family, "0xProto Nerd Font Mono");
        assert!(settings.dark_mode, "existing preferences must survive");
        assert_eq!(settings.sidebar_width, default_sidebar_width());
    }

    #[test]
    fn the_sidebar_width_survives_a_round_trip() {
        let settings = AppSettings {
            sidebar_width: 512.0,
            ..AppSettings::default()
        };

        let json = serde_json::to_string(&settings).expect("serialize");
        let restored: AppSettings = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.sidebar_width, 512.0);
    }

    #[test]
    fn the_download_folder_falls_back_to_the_one_the_os_uses() {
        let existing = r#"{
            "font_family": "Menlo",
            "font_size": 14.0,
            "dark_mode": false
        }"#;

        let settings: AppSettings =
            serde_json::from_str(existing).expect("a file from before the setting should load");
        assert_eq!(
            settings.resolved_download_dir(),
            default_download_dir(),
            "an unset folder means the OS downloads folder"
        );

        let blank = AppSettings {
            download_dir: Some("   ".to_string()),
            ..AppSettings::default()
        };
        assert_eq!(
            blank.resolved_download_dir(),
            default_download_dir(),
            "a cleared field must not send downloads to a blank path"
        );
    }

    #[test]
    fn a_chosen_download_folder_survives_a_round_trip() {
        let settings = AppSettings {
            download_dir: Some("/srv/incoming".to_string()),
            ..AppSettings::default()
        };

        let json = serde_json::to_string(&settings).expect("serialize");
        let restored: AppSettings = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(
            restored.resolved_download_dir(),
            PathBuf::from("/srv/incoming")
        );
    }

    #[test]
    fn the_default_width_sits_inside_the_allowed_range() {
        assert!(default_sidebar_width() >= SIDEBAR_MIN_WIDTH);
        assert!(default_sidebar_width() <= SIDEBAR_MAX_WIDTH);
    }
}
