use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const ATTEMPTS: u32 = 64;

pub fn private_dir(prefix: &str) -> io::Result<PathBuf> {
    let base = std::env::temp_dir();
    let pid = std::process::id();

    for attempt in 0..ATTEMPTS {
        let candidate = base.join(format!("{prefix}-{pid}-{}", suffix(attempt)));
        match std::fs::create_dir(&candidate) {
            Ok(()) => {
                harden(&candidate)?;
                return Ok(candidate);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a private temporary directory",
    ))
}

fn suffix(attempt: u32) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{seq:x}-{attempt:x}")
}

#[cfg(unix)]
fn harden(dir: &std::path::Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn harden(_dir: &std::path::Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
