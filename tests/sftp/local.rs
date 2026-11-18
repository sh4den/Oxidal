use super::*;

#[test]
fn joining_and_walking_up_are_inverse() {
    let dir = path_string(&std::env::temp_dir());
    let child = join_local(&dir, "oxidal-child");

    assert!(child.starts_with(&dir));
    assert!(child.ends_with("oxidal-child"));
    assert_eq!(
        parent_local(&child),
        dir.trim_end_matches(['/', '\\']).to_string()
    );
}

#[test]
fn the_filesystem_root_has_no_parent() {
    let root = path_string(Path::new("/"));
    assert!(!has_parent(&root));
}

#[test]
fn creating_a_file_never_overwrites_an_existing_one() {
    let base = std::env::temp_dir().join(format!("oxidal-create-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("dir");
    let dir = path_string(&base);

    create_file(&dir, "notes.txt").expect("create");
    assert_eq!(fs::read(base.join("notes.txt")).expect("read"), b"");

    fs::write(base.join("notes.txt"), b"keep me").expect("write");
    let err = create_file(&dir, "notes.txt").expect_err("exists");
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(base.join("notes.txt")).expect("read"), b"keep me");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn moving_into_a_folder_never_replaces_what_is_there() {
    let base = std::env::temp_dir().join(format!("oxidal-move-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("archive")).expect("dir");
    fs::write(base.join("report.txt"), b"new").expect("file");
    fs::write(base.join("archive").join("report.txt"), b"old").expect("file");
    fs::write(base.join("notes.txt"), b"notes").expect("file");

    let err = move_entry(
        &path_string(&base.join("report.txt")),
        &path_string(&base.join("archive").join("report.txt")),
    )
    .expect_err("a clash must be refused");
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(
        fs::read(base.join("archive").join("report.txt")).expect("read"),
        b"old",
        "the file already in the folder is untouched"
    );
    assert!(base.join("report.txt").exists(), "and the source stays put");

    move_entry(
        &path_string(&base.join("notes.txt")),
        &path_string(&base.join("archive").join("notes.txt")),
    )
    .expect("a free name moves");
    assert!(!base.join("notes.txt").exists());
    assert_eq!(
        fs::read(base.join("archive").join("notes.txt")).expect("read"),
        b"notes"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn listing_reports_directories_first_then_names() {
    let base = std::env::temp_dir().join(format!("oxidal-local-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("zeta-dir")).expect("dir");
    fs::create_dir_all(base.join("alpha-dir")).expect("dir");
    fs::write(base.join("beta.txt"), b"hello").expect("file");

    let entries = read_dir(&path_string(&base)).expect("listing");
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["alpha-dir", "zeta-dir", "beta.txt"]);

    let file = entries.iter().find(|e| e.name == "beta.txt").expect("file");
    assert!(!file.is_dir);
    assert_eq!(file.size, 5);
    assert!(
        entries
            .iter()
            .find(|e| e.name == "alpha-dir")
            .unwrap()
            .is_dir
    );

    let _ = fs::remove_dir_all(&base);
}
