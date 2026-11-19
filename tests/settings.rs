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
fn settings_saved_before_edited_file_uploads_leave_the_choice_unmade() {
    let existing = r#"{
            "font_family": "Menlo",
            "font_size": 14.0,
            "dark_mode": false
        }"#;

    let settings: AppSettings =
        serde_json::from_str(existing).expect("a file from before the setting should load");
    assert!(
        settings.edit_upload.is_none(),
        "no stored choice means the first save asks"
    );

    let settings = AppSettings {
        edit_upload: Some(EditUploadMode::Auto),
        ..AppSettings::default()
    };
    let json = serde_json::to_string(&settings).expect("serialize");
    let restored: AppSettings = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.edit_upload, Some(EditUploadMode::Auto));
}

#[test]
fn the_open_mode_starts_unchosen_and_survives_a_round_trip() {
    let existing = r#"{
            "font_family": "Menlo",
            "font_size": 14.0,
            "dark_mode": false
        }"#;

    let settings: AppSettings =
        serde_json::from_str(existing).expect("a file from before the setting should load");
    assert!(
        settings.remote_open.is_none(),
        "no stored choice means the first double click asks"
    );

    let settings = AppSettings {
        remote_open: Some(RemoteOpenMode::Editor),
        ..AppSettings::default()
    };
    let json = serde_json::to_string(&settings).expect("serialize");
    let restored: AppSettings = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.remote_open, Some(RemoteOpenMode::Editor));
}

#[test]
fn the_default_width_sits_inside_the_allowed_range() {
    assert!(default_sidebar_width() >= SIDEBAR_MIN_WIDTH);
    assert!(default_sidebar_width() <= SIDEBAR_MAX_WIDTH);
}
