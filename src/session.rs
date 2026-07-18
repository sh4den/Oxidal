use std::fs;
use std::path::PathBuf;

use gpui_component::IconName;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The kind of a session, mirroring MobaXterm's session categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionKind {
    Ssh,
    Sftp,
    Rdp,
    Serial,
    Local,
}

impl SessionKind {
    pub const ALL: [SessionKind; 5] = [
        SessionKind::Ssh,
        SessionKind::Sftp,
        SessionKind::Rdp,
        SessionKind::Serial,
        SessionKind::Local,
    ];

    pub fn icon(self) -> IconName {
        match self {
            SessionKind::Ssh | SessionKind::Local => IconName::SquareTerminal,
            SessionKind::Sftp => IconName::Folder,
            SessionKind::Rdp => IconName::LayoutDashboard,
            SessionKind::Serial => IconName::Cpu,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SessionKind::Ssh => "SSH",
            SessionKind::Sftp => "SFTP",
            SessionKind::Rdp => "RDP",
            SessionKind::Serial => "Serial",
            SessionKind::Local => "Local",
        }
    }
}

/// A saved connection entry shown in the sessions sidebar.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub name: String,
    pub kind: SessionKind,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    /// Never written to `sessions.json`. Persisted separately in the OS
    /// credential vault (see `credentials`) and re-attached on load. Held
    /// as a [`SecretString`] so it's redacted from `Debug` output and
    /// zeroized in memory on drop.
    #[serde(skip)]
    pub password: SecretString,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    /// Path to a private key file for SSH public-key authentication. Tried
    /// before falling back to password auth. Optional.
    #[serde(default)]
    pub private_key_path: Option<String>,
    /// The folder this session is filed under in the sidebar, if any.
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    /// Whether the SFTP explorer for this session shows dotfiles. Toggled
    /// from the explorer's "…" menu and remembered per session.
    #[serde(default)]
    pub show_hidden_files: bool,
}

/// A group of sessions shown together in the sidebar.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionFolder {
    pub id: Uuid,
    pub name: String,
}

impl SessionFolder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
        }
    }
}

fn default_baud_rate() -> u32 {
    115_200
}

impl Session {
    pub fn new(name: impl Into<String>, kind: SessionKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            host: String::new(),
            port: kind.default_port(),
            username: String::new(),
            password: SecretString::default(),
            baud_rate: default_baud_rate(),
            private_key_path: None,
            folder_id: None,
            show_hidden_files: false,
        }
    }

    /// A short human-readable summary shown under the session name.
    pub fn detail(&self) -> String {
        match self.kind {
            SessionKind::Local => "Local shell".to_string(),
            SessionKind::Serial => {
                if self.host.is_empty() {
                    "No port configured".to_string()
                } else {
                    self.host.clone()
                }
            }
            SessionKind::Ssh | SessionKind::Sftp | SessionKind::Rdp => {
                if self.host.is_empty() {
                    "No host configured".to_string()
                } else if self.username.is_empty() {
                    format!("{}:{}", self.host, self.port)
                } else {
                    format!("{}@{}:{}", self.username, self.host, self.port)
                }
            }
        }
    }
}

impl SessionKind {
    pub fn default_port(self) -> u16 {
        match self {
            SessionKind::Ssh | SessionKind::Sftp => 22,
            SessionKind::Rdp => 3389,
            SessionKind::Serial | SessionKind::Local => 0,
        }
    }
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Oxidal")
}

fn sessions_path() -> PathBuf {
    config_dir().join("sessions.json")
}

fn folders_path() -> PathBuf {
    config_dir().join("folders.json")
}

pub fn load_sessions() -> Vec<Session> {
    let path = sessions_path();
    let mut sessions: Vec<Session> = match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => default_sessions(),
    };
    for session in &mut sessions {
        if let Some(password) = crate::credentials::load_password(session.id) {
            session.password = password;
        }
    }
    sessions
}

pub fn save_sessions(sessions: &[Session]) {
    let dir = config_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(sessions) {
        let _ = fs::write(sessions_path(), json);
    }
}

pub fn load_folders() -> Vec<SessionFolder> {
    let path = folders_path();
    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn save_folders(folders: &[SessionFolder]) {
    let dir = config_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(folders) {
        let _ = fs::write(folders_path(), json);
    }
}

fn default_sessions() -> Vec<Session> {
    vec![Session::new("Local shell", SessionKind::Local)]
}
