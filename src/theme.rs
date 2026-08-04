use std::collections::BTreeMap;

use gpui::{Global, Hsla};
use gpui_component::{Colorize as _, ThemeColor, ThemeConfig, ThemeConfigColors, ThemeMode};
use serde::{Deserialize, Serialize};

use crate::terminal::grid::TerminalPalette;

impl Global for TerminalPalette {}

pub struct ColorSlot {
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
}

macro_rules! ui_slots {
    ($(($key:literal, $label:literal, $group:expr, $field:ident)),* $(,)?) => {
        pub const UI_SLOTS: &[ColorSlot] = &[
            $(ColorSlot { key: $key, label: $label, group: $group }),*
        ];

        fn apply_ui_slot(colors: &mut ThemeConfigColors, key: &str, value: &str) {
            match key {
                $($key => colors.$field = Some(value.to_string().into()),)*
                _ => {}
            }
        }

        fn base_ui_color(key: &str, base: &ThemeColor) -> Option<Hsla> {
            match key {
                $($key => Some(base.$field),)*
                _ => None,
            }
        }
    };
}

pub const GROUP_SURFACES: &str = "Surfaces";
pub const GROUP_ACCENTS: &str = "Accents";
pub const GROUP_LINES: &str = "Lines & muted";
pub const GROUP_STATUS: &str = "Status";

ui_slots![
    ("background", "Background", GROUP_SURFACES, background),
    ("foreground", "Text", GROUP_SURFACES, foreground),
    ("sidebar", "Sidebar", GROUP_SURFACES, sidebar),
    (
        "sidebar_foreground",
        "Sidebar text",
        GROUP_SURFACES,
        sidebar_foreground
    ),
    ("title_bar", "Title bar", GROUP_SURFACES, title_bar),
    ("tab_bar", "Tab bar", GROUP_SURFACES, tab_bar),
    ("tab_active", "Active tab", GROUP_SURFACES, tab_active),
    ("popover", "Menus & dialogs", GROUP_SURFACES, popover),
    (
        "popover_foreground",
        "Menu text",
        GROUP_SURFACES,
        popover_foreground
    ),
    ("primary", "Primary", GROUP_ACCENTS, primary),
    (
        "primary_foreground",
        "Primary text",
        GROUP_ACCENTS,
        primary_foreground
    ),
    ("accent", "Hover", GROUP_ACCENTS, accent),
    (
        "accent_foreground",
        "Hover text",
        GROUP_ACCENTS,
        accent_foreground
    ),
    (
        "sidebar_accent",
        "Selected row",
        GROUP_ACCENTS,
        sidebar_accent
    ),
    ("selection", "Text selection", GROUP_ACCENTS, selection),
    ("ring", "Focus ring", GROUP_ACCENTS, ring),
    ("link", "Links", GROUP_ACCENTS, link),
    ("border", "Border", GROUP_LINES, border),
    (
        "sidebar_border",
        "Sidebar border",
        GROUP_LINES,
        sidebar_border
    ),
    ("input", "Input border", GROUP_LINES, input),
    ("muted", "Muted surface", GROUP_LINES, muted),
    (
        "muted_foreground",
        "Muted text",
        GROUP_LINES,
        muted_foreground
    ),
    ("scrollbar_thumb", "Scrollbar", GROUP_LINES, scrollbar_thumb),
    ("danger", "Danger", GROUP_STATUS, danger),
    ("warning", "Warning", GROUP_STATUS, warning),
    ("success", "Success", GROUP_STATUS, success),
    ("info", "Info", GROUP_STATUS, info),
];

pub const GROUP_TERMINAL: &str = "Terminal";
pub const GROUP_ANSI: &str = "ANSI";
pub const GROUP_ANSI_BRIGHT: &str = "ANSI bright";

pub const TERMINAL_SLOTS: &[ColorSlot] = &[
    ColorSlot {
        key: "terminal.background",
        label: "Background",
        group: GROUP_TERMINAL,
    },
    ColorSlot {
        key: "terminal.foreground",
        label: "Text",
        group: GROUP_TERMINAL,
    },
    ColorSlot {
        key: "terminal.cursor",
        label: "Cursor",
        group: GROUP_TERMINAL,
    },
    ColorSlot {
        key: "terminal.selection",
        label: "Selection",
        group: GROUP_TERMINAL,
    },
    ColorSlot {
        key: "ansi.0",
        label: "Black",
        group: GROUP_ANSI,
    },
    ColorSlot {
        key: "ansi.1",
        label: "Red",
        group: GROUP_ANSI,
    },
    ColorSlot {
        key: "ansi.2",
        label: "Green",
        group: GROUP_ANSI,
    },
    ColorSlot {
        key: "ansi.3",
        label: "Yellow",
        group: GROUP_ANSI,
    },
    ColorSlot {
        key: "ansi.4",
        label: "Blue",
        group: GROUP_ANSI,
    },
    ColorSlot {
        key: "ansi.5",
        label: "Magenta",
        group: GROUP_ANSI,
    },
    ColorSlot {
        key: "ansi.6",
        label: "Cyan",
        group: GROUP_ANSI,
    },
    ColorSlot {
        key: "ansi.7",
        label: "White",
        group: GROUP_ANSI,
    },
    ColorSlot {
        key: "ansi.8",
        label: "Black",
        group: GROUP_ANSI_BRIGHT,
    },
    ColorSlot {
        key: "ansi.9",
        label: "Red",
        group: GROUP_ANSI_BRIGHT,
    },
    ColorSlot {
        key: "ansi.10",
        label: "Green",
        group: GROUP_ANSI_BRIGHT,
    },
    ColorSlot {
        key: "ansi.11",
        label: "Yellow",
        group: GROUP_ANSI_BRIGHT,
    },
    ColorSlot {
        key: "ansi.12",
        label: "Blue",
        group: GROUP_ANSI_BRIGHT,
    },
    ColorSlot {
        key: "ansi.13",
        label: "Magenta",
        group: GROUP_ANSI_BRIGHT,
    },
    ColorSlot {
        key: "ansi.14",
        label: "Cyan",
        group: GROUP_ANSI_BRIGHT,
    },
    ColorSlot {
        key: "ansi.15",
        label: "White",
        group: GROUP_ANSI_BRIGHT,
    },
];

/// Groups in the order the editor lays them out.
pub const GROUPS: &[&str] = &[
    GROUP_SURFACES,
    GROUP_ACCENTS,
    GROUP_LINES,
    GROUP_STATUS,
    GROUP_TERMINAL,
    GROUP_ANSI,
    GROUP_ANSI_BRIGHT,
];

pub fn all_slots() -> impl Iterator<Item = &'static ColorSlot> {
    UI_SLOTS.iter().chain(TERMINAL_SLOTS)
}

fn base_terminal_color(key: &str) -> Option<Hsla> {
    let base = TerminalPalette::default();
    match key {
        "terminal.background" => Some(base.background),
        "terminal.foreground" => Some(base.foreground),
        "terminal.cursor" => Some(base.cursor),
        "terminal.selection" => Some(base.selection),
        _ => key
            .strip_prefix("ansi.")
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| base.ansi.get(index).copied()),
    }
}

pub fn parse_color(value: &str) -> Option<Hsla> {
    Hsla::parse_hex(value).ok()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModeColors {
    #[serde(default)]
    pub ui: BTreeMap<String, String>,
    #[serde(default)]
    pub terminal: BTreeMap<String, String>,
}

impl ModeColors {
    fn keep_known(&mut self) {
        self.ui.retain(|key, value| {
            base_ui_color(key, &ThemeColor::light()).is_some() && is_hex(value)
        });
        self.terminal
            .retain(|key, value| base_terminal_color(key).is_some() && is_hex(value));
    }
}

fn is_hex(value: &str) -> bool {
    parse_color(value).is_some()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ThemeSettings {
    #[serde(default)]
    pub light: ModeColors,
    #[serde(default)]
    pub dark: ModeColors,
}

impl ThemeSettings {
    pub fn colors(&self, mode: ThemeMode) -> &ModeColors {
        if mode.is_dark() {
            &self.dark
        } else {
            &self.light
        }
    }

    pub fn colors_mut(&mut self, mode: ThemeMode) -> &mut ModeColors {
        if mode.is_dark() {
            &mut self.dark
        } else {
            &mut self.light
        }
    }

    pub fn is_customized(&self, mode: ThemeMode) -> bool {
        let colors = self.colors(mode);
        !colors.ui.is_empty() || !colors.terminal.is_empty()
    }

    pub fn ui_color(&self, key: &str, mode: ThemeMode) -> Hsla {
        let base = if mode.is_dark() {
            ThemeColor::dark()
        } else {
            ThemeColor::light()
        };
        self.colors(mode)
            .ui
            .get(key)
            .and_then(|value| parse_color(value))
            .or_else(|| base_ui_color(key, &base))
            .unwrap_or_default()
    }

    pub fn terminal_color(&self, key: &str, mode: ThemeMode) -> Hsla {
        self.colors(mode)
            .terminal
            .get(key)
            .and_then(|value| parse_color(value))
            .or_else(|| base_terminal_color(key))
            .unwrap_or_default()
    }

    pub fn color(&self, slot: &ColorSlot, mode: ThemeMode) -> Hsla {
        if slot.key.starts_with("terminal.") || slot.key.starts_with("ansi.") {
            self.terminal_color(slot.key, mode)
        } else {
            self.ui_color(slot.key, mode)
        }
    }

    pub fn set_color(&mut self, slot: &ColorSlot, mode: ThemeMode, color: Hsla) {
        let hex = color.to_hex();
        let colors = self.colors_mut(mode);
        if slot.key.starts_with("terminal.") || slot.key.starts_with("ansi.") {
            colors.terminal.insert(slot.key.to_string(), hex);
        } else {
            colors.ui.insert(slot.key.to_string(), hex);
        }
    }

    pub fn reset(&mut self, mode: ThemeMode) {
        *self.colors_mut(mode) = ModeColors::default();
    }

    /// Every slot spelled out with the color it currently resolves to, so an
    /// exported file is a complete document rather than a sparse diff.
    pub fn resolved(&self) -> Self {
        let mut resolved = Self::default();
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            for slot in UI_SLOTS {
                resolved
                    .colors_mut(mode)
                    .ui
                    .insert(slot.key.to_string(), self.ui_color(slot.key, mode).to_hex());
            }
            for slot in TERMINAL_SLOTS {
                resolved.colors_mut(mode).terminal.insert(
                    slot.key.to_string(),
                    self.terminal_color(slot.key, mode).to_hex(),
                );
            }
        }
        resolved
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut parsed: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        parsed.light.keep_known();
        parsed.dark.keep_known();
        if !parsed.is_customized(ThemeMode::Light) && !parsed.is_customized(ThemeMode::Dark) {
            return Err("no usable colors in this file".to_string());
        }
        Ok(parsed)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.resolved()).unwrap_or_default()
    }

    /// Lays the customized colors over a base theme, so every slot the user
    /// never touched keeps whatever the base theme says.
    pub fn config_over(&self, mut base: ThemeConfig, mode: ThemeMode) -> ThemeConfig {
        base.mode = mode;
        // A name of its own keeps the registry from swapping our colors back out
        // for the stock theme it was cloned from.
        base.name = if mode.is_dark() {
            "Oxidal Dark".into()
        } else {
            "Oxidal Light".into()
        };
        for (key, value) in &self.colors(mode).ui {
            if is_hex(value) {
                apply_ui_slot(&mut base.colors, key, value);
            }
        }
        base
    }

    pub fn terminal_palette(&self, mode: ThemeMode) -> TerminalPalette {
        let mut palette = TerminalPalette {
            background: self.terminal_color("terminal.background", mode),
            foreground: self.terminal_color("terminal.foreground", mode),
            cursor: self.terminal_color("terminal.cursor", mode),
            selection: self.terminal_color("terminal.selection", mode),
            ansi: TerminalPalette::default().ansi,
        };
        for (index, color) in palette.ansi.iter_mut().enumerate() {
            *color = self.terminal_color(&format!("ansi.{index}"), mode);
        }
        palette
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_untouched_theme_resolves_to_the_built_in_colors() {
        let theme = ThemeSettings::default();

        assert_eq!(
            theme.ui_color("background", ThemeMode::Dark),
            ThemeColor::dark().background,
            "an unset slot must fall back to the library default"
        );
        assert_eq!(
            theme.terminal_palette(ThemeMode::Light).ansi,
            TerminalPalette::default().ansi
        );
    }

    #[test]
    fn a_customized_color_is_laid_over_the_base_theme() {
        let mut base = ThemeConfig::default();
        base.colors.primary = Some("neutral-900".into());
        base.colors.background = Some("white".into());

        let mut theme = ThemeSettings::default();
        let slot = UI_SLOTS
            .iter()
            .find(|slot| slot.key == "primary")
            .expect("primary is a slot");
        theme.set_color(slot, ThemeMode::Dark, gpui::rgb(0xFF0000).into());

        let config = theme.config_over(base.clone(), ThemeMode::Dark);
        assert_eq!(config.colors.primary.as_deref(), Some("#FF0000"));
        assert_eq!(
            config.colors.background.as_deref(),
            Some("white"),
            "untouched slots keep whatever the base theme says"
        );
        assert_eq!(
            theme
                .config_over(base, ThemeMode::Light)
                .colors
                .primary
                .as_deref(),
            Some("neutral-900"),
            "editing one mode must not touch the other"
        );
    }

    #[test]
    fn each_mode_keeps_its_own_terminal_palette() {
        let mut theme = ThemeSettings::default();
        let slot = TERMINAL_SLOTS
            .iter()
            .find(|slot| slot.key == "terminal.background")
            .expect("terminal background is a slot");
        theme.set_color(slot, ThemeMode::Light, gpui::rgb(0xFFFFFF).into());

        assert_eq!(
            theme.terminal_palette(ThemeMode::Light).background,
            gpui::rgb(0xFFFFFF).into()
        );
        assert_eq!(
            theme.terminal_palette(ThemeMode::Dark).background,
            TerminalPalette::default().background
        );
    }

    #[test]
    fn an_exported_theme_comes_back_unchanged() {
        let mut theme = ThemeSettings::default();
        let slot = UI_SLOTS
            .iter()
            .find(|slot| slot.key == "sidebar")
            .expect("sidebar is a slot");
        theme.set_color(slot, ThemeMode::Dark, gpui::rgb(0x101820).into());

        let restored = ThemeSettings::from_json(&theme.to_json()).expect("exported themes import");

        assert_eq!(
            restored.ui_color("sidebar", ThemeMode::Dark),
            theme.ui_color("sidebar", ThemeMode::Dark)
        );
        assert_eq!(
            restored.ui_color("background", ThemeMode::Light),
            theme.ui_color("background", ThemeMode::Light),
            "resolved defaults survive the round trip"
        );
    }

    #[test]
    fn junk_in_an_imported_file_is_dropped_rather_than_applied() {
        let json = r##"{
            "dark": {
                "ui": { "background": "#123456", "not_a_slot": "#FFFFFF", "border": "purple" },
                "terminal": { "ansi.99": "#FFFFFF", "ansi.1": "#FF0000" }
            }
        }"##;

        let imported = ThemeSettings::from_json(json).expect("the usable colors are kept");
        let dark = imported.colors(ThemeMode::Dark);

        assert_eq!(
            dark.ui.get("background").map(String::as_str),
            Some("#123456")
        );
        assert!(!dark.ui.contains_key("not_a_slot"), "unknown keys go");
        assert!(!dark.ui.contains_key("border"), "unparseable colors go");
        assert!(!dark.terminal.contains_key("ansi.99"));
        assert_eq!(
            dark.terminal.get("ansi.1").map(String::as_str),
            Some("#FF0000")
        );
    }

    #[test]
    fn a_file_with_nothing_usable_is_refused() {
        assert!(ThemeSettings::from_json("{}").is_err());
        assert!(ThemeSettings::from_json("not json").is_err());
    }
}
