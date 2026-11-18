use super::*;

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
        .expect("time")
        .with_timezone(&chrono::Utc)
}

#[test]
fn quoting_keeps_shell_characters_literal() {
    assert_eq!(quote("plain"), "'plain'");
    assert_eq!(quote("it's"), r"'it'\''s'");
    assert_eq!(quote("$(rm -rf ~); `x` \"y\""), "'$(rm -rf ~); `x` \"y\"'");
    assert_eq!(quote(""), "''");
}

#[test]
fn listings_start_with_the_resolved_folder() {
    let raw = b"/home/user\n\
        total 8\n\
        drwxr-xr-x 2 user staff 4096 Jan 2 2025 docs\n\
        -rw-r--r-- 1 user staff 12 Jun 14 09:30 notes.txt\n\
        lrwxrwxrwx 1 user staff 9 Jun 14 09:30 link -> notes.txt\n";
    let (dir, entries) = parse_listing(raw, now()).expect("listing");

    assert_eq!(dir, "/home/user");
    assert_eq!(entries.len(), 3);
    let docs = entries.iter().find(|e| e.name == "docs").expect("docs");
    assert!(docs.is_dir);
    assert_eq!(docs.path, "/home/user/docs");
    let notes = entries
        .iter()
        .find(|e| e.name == "notes.txt")
        .expect("notes");
    assert_eq!(notes.size, 12);
    assert_eq!(notes.owner.as_deref(), Some("user"));
    let link = entries.iter().find(|e| e.name == "link").expect("link");
    assert!(link.is_symlink);
}

#[test]
fn listings_of_the_root_join_paths_without_a_double_slash() {
    let raw = b"/\ndrwxr-xr-x 2 root root 4096 Jan 2 2025 etc\n";
    let (dir, entries) = parse_listing(raw, now()).expect("listing");
    assert_eq!(dir, "/");
    assert_eq!(entries[0].path, "/etc");
}

#[test]
fn a_listing_without_a_folder_line_is_rejected() {
    assert!(parse_listing(b"", now()).is_err());
    assert!(parse_listing(b"total 0\n", now()).is_err());
}

#[test]
fn file_headers_give_the_size() {
    assert_eq!(
        parse_file_header("0644 1234 notes.txt").expect("header"),
        1234
    );
    assert_eq!(
        parse_file_header("0600 0 name with spaces").expect("header"),
        0
    );
}

#[test]
fn malformed_file_headers_are_rejected() {
    assert!(parse_file_header("0644 12").is_err());
    assert!(parse_file_header("644 12 name").is_err());
    assert!(parse_file_header("0899 12 name").is_err());
    assert!(parse_file_header("0644 -1 name").is_err());
    assert!(parse_file_header("0644 big name").is_err());
}

#[test]
fn targets_split_into_folder_and_name() {
    assert_eq!(
        split_target("/home/user/notes.txt").expect("target"),
        ("/home/user".to_string(), "notes.txt")
    );
    assert_eq!(
        split_target("/notes.txt").expect("target"),
        ("/".to_string(), "notes.txt")
    );
}

#[test]
fn targets_scp_cannot_name_are_rejected() {
    assert!(split_target("/home/user/..").is_err());
    assert!(split_target("/home/user/.").is_err());
    assert!(split_target("/").is_err());
    assert!(split_target("/home/user/bad\nname").is_err());
}

#[test]
fn acks_report_the_remote_message() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let mut ok: &[u8] = &[0];
        assert!(read_ack(&mut ok).await.is_ok());

        let mut failed: &[u8] = b"\x01scp: /etc/shadow: Permission denied\n";
        let err = read_ack(&mut failed).await.expect_err("error");
        assert_eq!(err.to_string(), "/etc/shadow: Permission denied");

        let mut closed: &[u8] = &[];
        assert_eq!(
            read_ack(&mut closed).await.expect_err("closed").to_string(),
            NO_SCP
        );
    });
}

#[test]
fn mkdir_batches_stay_under_the_script_limit() {
    let dirs: Vec<String> = (0..5000)
        .map(|ix| format!("/srv/data/folder-{ix:05}"))
        .collect();
    let scripts = mkdir_scripts(&dirs);

    assert!(scripts.len() > 1);
    assert!(scripts.iter().all(|script| script.len() <= MAX_SCRIPT));
    assert!(
        scripts
            .iter()
            .all(|script| script.starts_with("mkdir -p -- "))
    );
    let total: usize = scripts
        .iter()
        .map(|script| script.matches("'/srv/data/").count())
        .sum();
    assert_eq!(total, dirs.len());
    assert!(mkdir_scripts(&[]).is_empty());
}

#[test]
fn moves_refuse_to_replace_an_existing_target() {
    let script = move_script("/a/it's", "/b/x");
    assert!(script.starts_with("if [ -e '/b/x' ] || [ -L '/b/x' ]; then"));
    assert!(script.ends_with(r"mv -- '/a/it'\''s' '/b/x'"));
    assert!(script.contains(&format!("exit {EXISTS_STATUS}")));
}

#[test]
fn shell_prefixes_are_stripped_from_errors() {
    for (raw, expected) in [
        (
            "sh: can't create /home/u/a.txt: File exists",
            "can't create /home/u/a.txt: File exists",
        ),
        (
            "sh: 1: cannot create /home/u/a.txt: File exists",
            "cannot create /home/u/a.txt: File exists",
        ),
        (
            "sh: line 1: /home/u/a.txt: cannot overwrite existing file",
            "/home/u/a.txt: cannot overwrite existing file",
        ),
        (
            "sh:1: file exists: /home/u/a.txt",
            "file exists: /home/u/a.txt",
        ),
        (
            "rmdir: '/home/u/docs': Directory not empty",
            "rmdir: '/home/u/docs': Directory not empty",
        ),
        ("sh: 12abc: odd", "12abc: odd"),
        ("sh:", "sh:"),
    ] {
        assert_eq!(strip_shell_prefix(raw), expected, "{raw}");
    }
}
