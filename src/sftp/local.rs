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
                LocalCommand::CreateFile { name } => {
                    if let Err(err) = create_file(&current, &name) {
                        send_error(&out_tx, format!("Couldn't create file: {err}"));
                    }
                    list_and_send(&current, &out_tx);
                }
                LocalCommand::Rename { from, to } => {
                    if let Err(err) = fs::rename(&from, &to) {
                        send_error(&out_tx, format!("Couldn't rename: {err}"));
                    }
                    list_and_send(&current, &out_tx);
                }
                LocalCommand::Move { from, to } => match move_entry(&from, &to) {
                    Ok(()) => {
                        list_and_send(&current, &out_tx);
                    }
                    Err(message) => send_error(&out_tx, message),
                },
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

fn create_file(dir: &str, name: &str) -> std::io::Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(Path::new(dir).join(name))
        .map(drop)
}

fn move_entry(from: &str, to: &str) -> Result<(), String> {
    let destination = Path::new(to);
    if destination.symlink_metadata().is_ok() {
        let name = destination
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| to.to_string());
        return Err(format!(
            "Couldn't move: \"{}\" already exists there",
            super::display_name(&name)
        ));
    }
    fs::rename(from, destination).map_err(|err| format!("Couldn't move: {err}"))
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

    super::sort_entries(&mut entries);
    entries.shrink_to_fit();

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
#[path = "../../tests/sftp/local.rs"]
mod tests;
