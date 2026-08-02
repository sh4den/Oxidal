mod client;
mod local;
mod panel;
mod workspace;

pub use panel::SftpPanel;
pub use workspace::SftpWorkspace;

use std::path::PathBuf;

#[derive(Clone)]
pub struct SftpEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<u64>,
    pub accessed: Option<u64>,
    pub permissions: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
}

enum SftpCommand {
    List {
        path: String,
    },
    CreateDir {
        name: String,
    },
    Rename {
        from: String,
        to: String,
    },
    RemoveFile {
        path: String,
    },
    RemoveDir {
        path: String,
    },
    Upload {
        local: PathBuf,
        remote: String,
    },
    Download {
        remote: String,
        local: PathBuf,
        open_when_done: bool,
    },
    UploadDir {
        local: PathBuf,
        remote: String,
    },
    DownloadDir {
        remote: String,
        local: PathBuf,
    },
}

enum LocalCommand {
    List { path: String },
    CreateDir { name: String },
    Rename { from: String, to: String },
    RemoveFile { path: String },
    RemoveDir { path: String },
}

enum SftpEvent {
    Listing {
        path: String,
        entries: Vec<SftpEntry>,
    },
    Error(String),
    TransferStarted {
        label: String,
        total: Option<u64>,
    },
    TransferProgress {
        transferred: u64,
    },
    TransferFinished {
        error: Option<String>,
    },
    Closed(Option<String>),
}

#[derive(Clone)]
pub struct SftpClient {
    events: async_channel::Receiver<SftpEvent>,
    commands: async_channel::Sender<SftpCommand>,
}

impl SftpClient {
    pub fn list(&self, path: impl Into<String>) {
        let _ = self
            .commands
            .send_blocking(SftpCommand::List { path: path.into() });
    }

    pub fn create_dir(&self, name: impl Into<String>) {
        let _ = self
            .commands
            .send_blocking(SftpCommand::CreateDir { name: name.into() });
    }

    pub fn rename(&self, from: impl Into<String>, to: impl Into<String>) {
        let _ = self.commands.send_blocking(SftpCommand::Rename {
            from: from.into(),
            to: to.into(),
        });
    }

    pub fn remove_file(&self, path: impl Into<String>) {
        let _ = self
            .commands
            .send_blocking(SftpCommand::RemoveFile { path: path.into() });
    }

    pub fn remove_dir(&self, path: impl Into<String>) {
        let _ = self
            .commands
            .send_blocking(SftpCommand::RemoveDir { path: path.into() });
    }

    pub fn upload(&self, local: PathBuf, remote: impl Into<String>) {
        let _ = self.commands.send_blocking(SftpCommand::Upload {
            local,
            remote: remote.into(),
        });
    }

    pub fn download(&self, remote: impl Into<String>, local: PathBuf) {
        let _ = self.commands.send_blocking(SftpCommand::Download {
            remote: remote.into(),
            local,
            open_when_done: false,
        });
    }

    pub fn download_and_open(&self, remote: impl Into<String>, local: PathBuf) {
        let _ = self.commands.send_blocking(SftpCommand::Download {
            remote: remote.into(),
            local,
            open_when_done: true,
        });
    }

    pub fn upload_dir(&self, local: PathBuf, remote: impl Into<String>) {
        let _ = self.commands.send_blocking(SftpCommand::UploadDir {
            local,
            remote: remote.into(),
        });
    }

    pub fn download_dir(&self, remote: impl Into<String>, local: PathBuf) {
        let _ = self.commands.send_blocking(SftpCommand::DownloadDir {
            remote: remote.into(),
            local,
        });
    }
}

#[derive(Clone)]
pub struct LocalClient {
    events: async_channel::Receiver<SftpEvent>,
    commands: async_channel::Sender<LocalCommand>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PanelSide {
    Local,
    Remote,
}

#[derive(Clone)]
pub struct FileDrag {
    pub side: PanelSide,
    pub entry_path: String,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Clone)]
pub enum FileClient {
    Local(LocalClient),
    Remote(SftpClient),
}

impl FileClient {
    fn events(&self) -> async_channel::Receiver<SftpEvent> {
        match self {
            FileClient::Local(client) => client.events.clone(),
            FileClient::Remote(client) => client.events.clone(),
        }
    }

    pub fn side(&self) -> PanelSide {
        match self {
            FileClient::Local(_) => PanelSide::Local,
            FileClient::Remote(_) => PanelSide::Remote,
        }
    }

    pub fn list(&self, path: impl Into<String>) {
        match self {
            FileClient::Local(client) => {
                let _ = client
                    .commands
                    .send_blocking(LocalCommand::List { path: path.into() });
            }
            FileClient::Remote(client) => client.list(path),
        }
    }

    pub fn create_dir(&self, name: impl Into<String>) {
        match self {
            FileClient::Local(client) => {
                let _ = client
                    .commands
                    .send_blocking(LocalCommand::CreateDir { name: name.into() });
            }
            FileClient::Remote(client) => client.create_dir(name),
        }
    }

    pub fn rename(&self, from: impl Into<String>, to: impl Into<String>) {
        match self {
            FileClient::Local(client) => {
                let _ = client.commands.send_blocking(LocalCommand::Rename {
                    from: from.into(),
                    to: to.into(),
                });
            }
            FileClient::Remote(client) => client.rename(from, to),
        }
    }

    pub fn remove_file(&self, path: impl Into<String>) {
        match self {
            FileClient::Local(client) => {
                let _ = client
                    .commands
                    .send_blocking(LocalCommand::RemoveFile { path: path.into() });
            }
            FileClient::Remote(client) => client.remove_file(path),
        }
    }

    pub fn remove_dir(&self, path: impl Into<String>) {
        match self {
            FileClient::Local(client) => {
                let _ = client
                    .commands
                    .send_blocking(LocalCommand::RemoveDir { path: path.into() });
            }
            FileClient::Remote(client) => client.remove_dir(path),
        }
    }
}

pub use client::spawn;
pub use local::{home_dir, spawn as spawn_local};

fn safe_local_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '_',
            c => c,
        })
        .collect();
    if sanitized.chars().all(|c| c == '.' || c == ' ') {
        "_".to_string()
    } else {
        sanitized
    }
}

fn unique_destination(dir: &std::path::Path, name: &str) -> PathBuf {
    let taken = dir.join(name);
    if !taken.exists() {
        return taken;
    }
    let as_path = std::path::Path::new(name);
    let (stem, extension) = match as_path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => (
            as_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(name),
            format!(".{ext}"),
        ),
        None => (name, String::new()),
    };
    for suffix in 1..1000 {
        let candidate = dir.join(format!("{stem} ({suffix}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    taken
}

fn join_remote(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn join_path(side: PanelSide, dir: &str, name: &str) -> String {
    match side {
        PanelSide::Local => local::join_local(dir, name),
        PanelSide::Remote => join_remote(dir, name),
    }
}

fn parent_path(side: PanelSide, path: &str) -> String {
    match side {
        PanelSide::Local => local::parent_local(path),
        PanelSide::Remote => parent_remote(path),
    }
}

fn has_parent(side: PanelSide, path: &str) -> bool {
    match side {
        PanelSide::Local => local::has_parent(path),
        PanelSide::Remote => path != "/",
    }
}

fn parent_remote(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some(("", _)) => "/".to_string(),
        Some((parent, _)) => parent.to_string(),
        None => "/".to_string(),
    }
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

pub fn format_modified(unix_secs: u64) -> String {
    match chrono::DateTime::from_timestamp(unix_secs as i64, 0) {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => String::new(),
    }
}

pub fn format_kind(entry: &SftpEntry) -> String {
    if entry.is_dir {
        return "Folder".to_string();
    }
    if entry.is_symlink {
        return "Symlink".to_string();
    }
    match entry.name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext.to_uppercase(),
        _ => "File".to_string(),
    }
}

pub fn format_permissions(is_dir: bool, mode: Option<u32>) -> String {
    let Some(mode) = mode else {
        return String::new();
    };
    let mut out = String::with_capacity(10);
    out.push(if is_dir { 'd' } else { '-' });
    for shift in [6u32, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        out.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        out.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        out.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
