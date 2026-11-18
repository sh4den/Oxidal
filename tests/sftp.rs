use super::*;

#[test]
fn a_name_that_reorders_itself_is_defused() {
    let spoofed = "evil\u{202e}gpj.exe";

    assert!(
        !display_name(spoofed).contains('\u{202e}'),
        "a right-to-left override must never reach the screen"
    );
    assert!(
        !safe_local_name(spoofed).contains('\u{202e}'),
        "nor the filesystem"
    );
    assert!(
        display_name("plan\u{200b}.txt").contains('\u{fffd}'),
        "a hidden character should be visible, not silently dropped"
    );
    assert_eq!(
        display_name("réunion été.txt"),
        "réunion été.txt",
        "ordinary accented names are left alone"
    );
}

#[test]
fn control_characters_never_reach_a_local_path() {
    assert_eq!(safe_local_name("a\nb\tc\0d"), "a_b_c_d");
    assert_eq!(safe_local_name("../../etc/passwd"), ".._.._etc_passwd");
    assert_eq!(safe_local_name(".."), "_");
    assert_eq!(safe_local_name("..."), "_");
    assert_eq!(safe_local_name("   "), "_");
}

#[test]
fn windows_hostile_names_are_neutralised() {
    assert_eq!(safe_local_name("report."), "report");
    assert_eq!(safe_local_name("report.txt "), "report.txt");
    assert_eq!(safe_local_name("CON"), "_CON");
    assert_eq!(safe_local_name("nul.txt"), "_nul.txt");
    assert_eq!(safe_local_name("a<b>c|d?e*f\"g"), "a_b_c_d_e_f_g");
    assert_eq!(
        safe_local_name("console.log"),
        "console.log",
        "a name that merely starts like a device name is fine"
    );
}

#[test]
fn a_remote_entry_moves_into_a_sibling_folder() {
    assert_eq!(
        move_destination(PanelSide::Remote, "/srv/app.log", "app.log", "/srv/logs"),
        Some("/srv/logs/app.log".to_string())
    );
    assert_eq!(
        move_destination(PanelSide::Remote, "/srv/www", "www", "/srv/old"),
        Some("/srv/old/www".to_string()),
        "a folder moves the same way as a file"
    );
    assert_eq!(
        move_destination(PanelSide::Remote, "/srv/logs/app.log", "app.log", "/srv"),
        Some("/srv/app.log".to_string()),
        "dropping on the parent moves it up a level"
    );
    assert_eq!(
        move_destination(PanelSide::Remote, "/app.log", "app.log", "/"),
        None,
        "dropping into the folder it already lives in does nothing"
    );
}

#[test]
fn a_remote_folder_never_moves_into_itself() {
    assert_eq!(
        move_destination(PanelSide::Remote, "/srv/www", "www", "/srv/www"),
        None
    );
    assert_eq!(
        move_destination(PanelSide::Remote, "/srv/www", "www", "/srv/www/static"),
        None,
        "nor into one of its own subfolders"
    );
    assert_eq!(
        move_destination(PanelSide::Remote, "/srv/www", "www", "/srv/www-old"),
        Some("/srv/www-old/www".to_string()),
        "a sibling that merely shares a prefix is a valid target"
    );
}

#[test]
fn a_local_entry_moves_into_a_folder_but_not_into_itself() {
    let base = std::env::temp_dir().to_string_lossy().to_string();
    let folder = local::join_local(&base, "photos");
    let file = local::join_local(&base, "cat.png");
    let inner = local::join_local(&folder, "2026");

    assert_eq!(
        move_destination(PanelSide::Local, &file, "cat.png", &folder),
        Some(local::join_local(&folder, "cat.png"))
    );
    assert_eq!(
        move_destination(PanelSide::Local, &folder, "photos", &folder),
        None
    );
    assert_eq!(
        move_destination(PanelSide::Local, &folder, "photos", &inner),
        None
    );
    assert_eq!(
        move_destination(PanelSide::Local, &file, "cat.png", &base),
        None,
        "the file is already in that folder"
    );
}

#[test]
fn a_second_download_of_the_same_name_lands_beside_the_first() {
    let base = std::env::temp_dir().join(format!("oxidal-unique-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("dir");

    assert_eq!(
        unique_destination(&base, "report.tar.gz"),
        base.join("report.tar.gz"),
        "a free name should be used as is"
    );

    std::fs::write(base.join("report.tar.gz"), b"first").expect("file");
    assert_eq!(
        unique_destination(&base, "report.tar.gz"),
        base.join("report.tar (1).gz"),
        "the counter goes before the last extension so the file still opens"
    );

    std::fs::write(base.join("report.tar (1).gz"), b"second").expect("file");
    assert_eq!(
        unique_destination(&base, "report.tar.gz"),
        base.join("report.tar (2).gz"),
        "the counter keeps climbing while names are taken"
    );

    std::fs::create_dir(base.join("logs")).expect("dir");
    assert_eq!(
        unique_destination(&base, "logs"),
        base.join("logs (1)"),
        "extensionless names, such as folders, get the counter appended"
    );

    std::fs::write(base.join(".bashrc"), b"x").expect("file");
    assert_eq!(
        unique_destination(&base, ".bashrc"),
        base.join(".bashrc (1)"),
        "a leading dot is part of the name, not an extension"
    );

    let _ = std::fs::remove_dir_all(&base);
}
