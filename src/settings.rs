use std::fs;
use std::path::PathBuf;

use gpui::Global;
use serde::{Deserialize, Serialize};

/// User-configurable application preferences: terminal font and theme mode.
/// Stored as a GPUI global so any view can read the live value with
/// `cx.global::<AppSettings>()`, and react to changes via `cx.observe_global`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    pub font_family: String,
    pub font_size: f32,
    pub dark_mode: bool,
}

impl Global for AppSettings {}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            font_family: default_font_family(),
            font_size: 14.0,
            dark_mode: false,
        }
    }
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
