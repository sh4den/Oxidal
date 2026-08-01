use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{LocalClient, LocalCommand, SftpEntry, SftpEvent};

pub fn spawn(initial: PathBuf) -> LocalClient {
    let (out_tx, out_rx) = async_channel::unbounded::<SftpEvent>();
    let (cmd_tx, cmd_rx) = async_channel::unbounded::<LocalCommand>();

    std::thread::spawn(move || {
        let mut current = path_string(&initial);
        list_and_send(&current, &out_tx);

        while let Ok(cmd) = cmd_rx.recv_blocking() {
            match cmd {
                LocalCommand::List { path } => {
                    if let Some(listed) = list_and_send(&path, &out_tx) {
                        current = listed;
                    }
                }
                LocalCommand::CreateDir { name } => {
                    if let Err(err) = fs::create_dir(Path::new(&current).join(name)) {
                        send_error(&out_tx, format!("Couldn't create folder: {err}"));
                    }
                    list_and_send(&current, &out_tx);
                }
                LocalCommand::Rename { from, to } => {
                    if let Err(err) = fs::rename(&from, &to) {
                        send_error(&out_tx, format!("Couldn't rename: {err}"));
                    }
                    list_and_send(&current, &out_tx);
                }
                LocalCommand::RemoveFile { path } => {
                    if let Err(err) = fs::remove_file(&path) {
                        send_error(&out_tx, format!("Couldn't delete file: {err}"));
                    }
                    list_and_send(&current, &out_tx);
                }
                LocalCommand::RemoveDir { path } => {
                    if let Err(err) = fs::remove_dir(&path) {
                        send_error(&out_tx, format!("Couldn't delete folder: {err}"));
                    }
                    list_and_send(&current, &out_tx);
                }
            }
        }
    });

    LocalClient {
        events: out_rx,
        commands: cmd_tx,
    }
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub fn join_local(dir: &str, name: &str) -> String {
    path_string(&Path::new(dir).join(name))
}

pub fn parent_local(path: &str) -> String {
    match Path::new(path).parent() {
        Some(parent) => path_string(parent),
        None => path.to_string(),
    }
}

pub fn has_parent(path: &str) -> bool {
    Path::new(path).parent().is_some()
}

fn send_error(out_tx: &async_channel::Sender<SftpEvent>, message: String) {
    let _ = out_tx.send_blocking(SftpEvent::Error(message));
}

fn list_and_send(path: &str, out_tx: &async_channel::Sender<SftpEvent>) -> Option<String> {
    match read_dir(path) {
        Ok(entries) => {
            let resolved = canonical_display(path);
            let _ = out_tx.send_blocking(SftpEvent::Listing {
                path: resolved.clone(),
                entries,
            });
            Some(resolved)
        }
        Err(err) => {
            send_error(out_tx, format!("Couldn't list {path}: {err}"));
            None
        }
    }
}

fn canonical_display(path: &str) -> String {
    match fs::canonicalize(path) {
        Ok(resolved) => {
            let text = path_string(&resolved);
            match text.strip_prefix(r"\\?\") {
                Some(stripped) => stripped.to_string(),
                None => text,
            }
        }
        Err(_) => path.to_string(),
    }
}

fn read_dir(path: &str) -> std::io::Result<Vec<SftpEntry>> {
    let mut entries: Vec<SftpEntry> = Vec::new();

    for entry in fs::read_dir(path)? {
        let Ok(entry) = entry else {
            continue;
        };
        let entry_path = entry.path();
        let is_symlink = entry
            .file_type()
            .map(|file_type| file_type.is_symlink())
            .unwrap_or(false);
        let metadata = fs::metadata(&entry_path).or_else(|_| entry.metadata()).ok();

        entries.push(SftpEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            path: path_string(&entry_path),
            is_dir: metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false),
            is_symlink,
            size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: metadata.as_ref().and_then(|m| unix_secs(m.modified().ok())),
            accessed: metadata.as_ref().and_then(|m| unix_secs(m.accessed().ok())),
            permissions: metadata.as_ref().and_then(mode_of),
            owner: metadata.as_ref().and_then(owner_of),
            group: metadata.as_ref().and_then(group_of),
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

fn unix_secs(time: Option<SystemTime>) -> Option<u64> {
    time?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_secs())
}

#[cfg(unix)]
fn mode_of(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    Some(metadata.mode() & 0o7777)
}

#[cfg(not(unix))]
fn mode_of(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
fn owner_of(metadata: &fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt as _;
    Some(metadata.uid().to_string())
}

#[cfg(not(unix))]
fn owner_of(_metadata: &fs::Metadata) -> Option<String> {
    None
}

#[cfg(unix)]
fn group_of(metadata: &fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt as _;
    Some(metadata.gid().to_string())
}

#[cfg(not(unix))]
fn group_of(_metadata: &fs::Metadata) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joining_and_walking_up_are_inverse() {
        let dir = path_string(&std::env::temp_dir());
        let child = join_local(&dir, "oxidal-child");

        assert!(child.starts_with(&dir));
        assert!(child.ends_with("oxidal-child"));
        assert_eq!(parent_local(&child), dir.trim_end_matches('/').to_string());
    }

    #[test]
    fn the_filesystem_root_has_no_parent() {
        let root = path_string(Path::new("/"));
        assert!(!has_parent(&root));
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
}
