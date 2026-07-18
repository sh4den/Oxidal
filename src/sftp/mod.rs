mod client;
mod panel;

pub use panel::SftpPanel;

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct SftpEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<u64>,
    pub permissions: Option<u32>,
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
}

pub use client::spawn;

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

fn join_remote(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
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
