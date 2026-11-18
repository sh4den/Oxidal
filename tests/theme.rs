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
    theme.set_color(slot, ThemeMode::Dark, gpui_kit::rgb(0xFF0000).into());

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
    theme.set_color(slot, ThemeMode::Light, gpui_kit::rgb(0xFFFFFF).into());

    assert_eq!(
        theme.terminal_palette(ThemeMode::Light).background,
        gpui_kit::rgb(0xFFFFFF).into()
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
    theme.set_color(slot, ThemeMode::Dark, gpui_kit::rgb(0x101820).into());

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
