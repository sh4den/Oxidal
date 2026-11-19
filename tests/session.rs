use super::*;

#[test]
fn icons_round_trip_by_lucide_name() {
    let folder: SessionFolder = serde_json::from_str(
        r#"{"id":"6f1d0c3e-0000-4000-8000-0000000000f1","name":"A","icon":"brick-wall"}"#,
    )
    .expect("folder");
    assert_eq!(folder.icon.map(ItemIcon::name), Some("brick-wall"));

    let json = serde_json::to_string(&folder).expect("json");
    let again: SessionFolder = serde_json::from_str(&json).expect("round trip");
    assert_eq!(again.icon, folder.icon);
}

#[test]
fn an_unknown_icon_shows_a_question_mark() {
    for name in ["Firewall", "no-such-icon"] {
        let folder: SessionFolder = serde_json::from_str(&format!(
            r#"{{"id":"6f1d0c3e-0000-4000-8000-0000000000f1","name":"A","icon":"{name}"}}"#
        ))
        .expect("folder still loads");
        assert_eq!(folder.icon, Some(ItemIcon::UNKNOWN), "{name}");
    }
}

#[test]
fn searching_icons_matches_every_word() {
    let names: Vec<&str> = ItemIcon::search("shield lock")
        .map(ItemIcon::name)
        .collect();
    assert!(names.contains(&"shield-lock"), "{names:?}");
    assert!(
        names
            .iter()
            .all(|name| name.contains("shield") && name.contains("lock"))
    );
    assert_eq!(ItemIcon::suggested().count(), SUGGESTED_ICONS.len());
}

#[test]
fn os_logos_round_trip_and_are_bundled() {
    let folder: SessionFolder = serde_json::from_str(
        r#"{"id":"6f1d0c3e-0000-4000-8000-0000000000f1","name":"A","icon":"os/ubuntu"}"#,
    )
    .expect("folder");
    let icon = folder.icon.expect("icon");
    assert!(icon.is_os());
    assert_eq!(icon.label(), "Ubuntu");
    let json = serde_json::to_string(&folder).expect("json");
    assert!(json.contains(r#""icon":"os/ubuntu""#), "{json}");

    assert_ne!(
        ItemIcon::from_name("apple"),
        ItemIcon::from_name("os/apple")
    );
    for logo in ItemIcon::os_logos() {
        let path = logo.path();
        let data = gpui_kit::AssetSource::load(&crate::assets::Assets, &path).expect("load");
        assert!(data.is_some(), "{path} is not bundled");
    }
}

#[test]
fn searching_os_logos_matches_labels() {
    let names: Vec<&str> = ItemIcon::search_os("mint").map(ItemIcon::name).collect();
    assert_eq!(names, ["os/linuxmint"]);
    assert_eq!(ItemIcon::search_os("macos").count(), 1);
}

fn session(kind: SessionKind, host: &str, user: &str, port: u16) -> Session {
    let mut session = Session::new("", kind);
    session.host = host.to_string();
    session.username = user.to_string();
    session.port = port;
    session
}

#[test]
fn default_names_follow_host_and_user() {
    let ssh = session(SessionKind::Ssh, "10.0.0.5", "root", 22);
    assert_eq!(ssh.default_name(), "10.0.0.5 (root)");

    let telnet = session(SessionKind::Telnet, " router.lan ", "", 2323);
    assert_eq!(telnet.default_name(), "router.lan");

    let serial = session(SessionKind::Serial, "COM3", "", 0);
    assert_eq!(serial.default_name(), "COM3");
}

#[test]
fn default_names_fall_back_without_a_host() {
    assert_eq!(
        session(SessionKind::Sftp, "", "root", 22).default_name(),
        "SFTP"
    );
    assert_eq!(
        session(SessionKind::Serial, "", "", 0).default_name(),
        "Serial"
    );
    assert_eq!(
        session(SessionKind::Local, "", "", 0).default_name(),
        "Local shell"
    );
}
