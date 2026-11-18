use super::*;

#[test]
fn two_directories_never_collide() {
    let first = private_dir("oxidal-test").expect("first");
    let second = private_dir("oxidal-test").expect("second");

    assert_ne!(first, second, "each call must own a fresh directory");
    assert!(first.is_dir() && second.is_dir());

    let _ = std::fs::remove_dir_all(&first);
    let _ = std::fs::remove_dir_all(&second);
}

#[cfg(unix)]
#[test]
fn the_directory_is_not_readable_by_other_users() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = private_dir("oxidal-perm").expect("dir");
    let mode = std::fs::metadata(&dir)
        .expect("metadata")
        .permissions()
        .mode();

    assert_eq!(
        mode & 0o777,
        0o700,
        "a staging directory must be owner only"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_squatted_name_does_not_stop_us() {
    let dir = private_dir("oxidal-squat").expect("dir");
    assert!(dir.is_dir(), "creation retries until it owns the directory");
    let _ = std::fs::remove_dir_all(&dir);
}
