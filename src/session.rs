use std::fs;
use std::path::PathBuf;

use gpui::{Hsla, SharedString, rgb};
use gpui_component::{IconName, IconNamed};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemIcon {
    Terminal,
    Code,
    Server,
    Cluster,
    Container,
    Database,
    Drive,
    Cpu,
    Memory,
    Usb,
    Network,
    Router,
    Wifi,
    Signal,
    Globe,
    Cloud,
    Firewall,
    Shield,
    Lock,
    Key,
    Layers,
    Package,
    GitBranch,
    Monitor,
    Gauge,
    Activity,
    Zap,
    Plug,
    Clock,
    Flask,
    Wrench,
    Bot,
    Building,
    User,
    Folder,
    FolderOpen,
    Star,
    Heart,
    Bell,
    Inbox,
    Map,
    Chart,
    Github,
    Book,
    Dashboard,
    Frame,
    Palette,
}

impl ItemIcon {
    pub const ALL: [ItemIcon; 47] = [
        ItemIcon::Terminal,
        ItemIcon::Code,
        ItemIcon::Server,
        ItemIcon::Cluster,
        ItemIcon::Container,
        ItemIcon::Database,
        ItemIcon::Drive,
        ItemIcon::Cpu,
        ItemIcon::Memory,
        ItemIcon::Usb,
        ItemIcon::Network,
        ItemIcon::Router,
        ItemIcon::Wifi,
        ItemIcon::Signal,
        ItemIcon::Globe,
        ItemIcon::Cloud,
        ItemIcon::Firewall,
        ItemIcon::Shield,
        ItemIcon::Lock,
        ItemIcon::Key,
        ItemIcon::Layers,
        ItemIcon::Package,
        ItemIcon::GitBranch,
        ItemIcon::Monitor,
        ItemIcon::Gauge,
        ItemIcon::Activity,
        ItemIcon::Zap,
        ItemIcon::Plug,
        ItemIcon::Clock,
        ItemIcon::Flask,
        ItemIcon::Wrench,
        ItemIcon::Bot,
        ItemIcon::Building,
        ItemIcon::User,
        ItemIcon::Folder,
        ItemIcon::FolderOpen,
        ItemIcon::Star,
        ItemIcon::Heart,
        ItemIcon::Bell,
        ItemIcon::Inbox,
        ItemIcon::Map,
        ItemIcon::Chart,
        ItemIcon::Github,
        ItemIcon::Book,
        ItemIcon::Dashboard,
        ItemIcon::Frame,
        ItemIcon::Palette,
    ];
}

impl IconNamed for ItemIcon {
    fn path(self) -> SharedString {
        let bundled = |name: &str| SharedString::from(format!("icons/oxidal/{name}.svg"));
        match self {
            ItemIcon::Terminal => IconName::SquareTerminal.path(),
            ItemIcon::Code => bundled("code"),
            ItemIcon::Server => bundled("server"),
            ItemIcon::Cluster => bundled("cluster"),
            ItemIcon::Container => bundled("container"),
            ItemIcon::Database => bundled("database"),
            ItemIcon::Drive => IconName::HardDrive.path(),
            ItemIcon::Cpu => IconName::Cpu.path(),
            ItemIcon::Memory => IconName::MemoryStick.path(),
            ItemIcon::Usb => bundled("usb"),
            ItemIcon::Network => IconName::Network.path(),
            ItemIcon::Router => bundled("router"),
            ItemIcon::Wifi => bundled("wifi"),
            ItemIcon::Signal => bundled("signal"),
            ItemIcon::Globe => IconName::Globe.path(),
            ItemIcon::Cloud => bundled("cloud"),
            ItemIcon::Firewall => bundled("firewall"),
            ItemIcon::Shield => bundled("shield"),
            ItemIcon::Lock => bundled("lock"),
            ItemIcon::Key => bundled("key"),
            ItemIcon::Layers => bundled("layers"),
            ItemIcon::Package => bundled("package"),
            ItemIcon::GitBranch => bundled("git-branch"),
            ItemIcon::Monitor => bundled("monitor"),
            ItemIcon::Gauge => bundled("gauge"),
            ItemIcon::Activity => bundled("activity"),
            ItemIcon::Zap => bundled("zap"),
            ItemIcon::Plug => bundled("plug"),
            ItemIcon::Clock => bundled("clock"),
            ItemIcon::Flask => bundled("flask"),
            ItemIcon::Wrench => bundled("wrench"),
            ItemIcon::Bot => IconName::Bot.path(),
            ItemIcon::Building => IconName::Building2.path(),
            ItemIcon::User => IconName::User.path(),
            ItemIcon::Folder => IconName::Folder.path(),
            ItemIcon::FolderOpen => IconName::FolderOpen.path(),
            ItemIcon::Star => IconName::Star.path(),
            ItemIcon::Heart => IconName::Heart.path(),
            ItemIcon::Bell => IconName::Bell.path(),
            ItemIcon::Inbox => IconName::Inbox.path(),
            ItemIcon::Map => IconName::Map.path(),
            ItemIcon::Chart => IconName::ChartPie.path(),
            ItemIcon::Github => IconName::Github.path(),
            ItemIcon::Book => IconName::BookOpen.path(),
            ItemIcon::Dashboard => IconName::LayoutDashboard.path(),
            ItemIcon::Frame => IconName::Frame.path(),
            ItemIcon::Palette => IconName::Palette.path(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemColor {
    #[default]
    Default,
    Red,
    Orange,
    Amber,
    Green,
    Teal,
    Blue,
    Indigo,
    Purple,
    Pink,
}

impl ItemColor {
    pub const ALL: [ItemColor; 10] = [
        ItemColor::Default,
        ItemColor::Red,
        ItemColor::Orange,
        ItemColor::Amber,
        ItemColor::Green,
        ItemColor::Teal,
        ItemColor::Blue,
        ItemColor::Indigo,
        ItemColor::Purple,
        ItemColor::Pink,
    ];

    pub fn hsla(self) -> Option<Hsla> {
        let hex = match self {
            ItemColor::Default => return None,
            ItemColor::Red => 0xef4444,
            ItemColor::Orange => 0xf97316,
            ItemColor::Amber => 0xf59e0b,
            ItemColor::Green => 0x22c55e,
            ItemColor::Teal => 0x14b8a6,
            ItemColor::Blue => 0x3b82f6,
            ItemColor::Indigo => 0x6366f1,
            ItemColor::Purple => 0xa855f7,
            ItemColor::Pink => 0xec4899,
        };
        Some(rgb(hex).into())
    }
}

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
    #[serde(skip)]
    pub password: SecretString,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    #[serde(default)]
    pub private_key_path: Option<String>,
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    #[serde(default)]
    pub show_hidden_files: bool,
    #[serde(default)]
    pub icon: Option<ItemIcon>,
    #[serde(default)]
    pub color: ItemColor,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionFolder {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub icon: Option<ItemIcon>,
    #[serde(default)]
    pub color: ItemColor,
}

impl SessionFolder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            icon: None,
            color: ItemColor::default(),
        }
    }

    pub fn display_icon(&self) -> SharedString {
        match self.icon {
            Some(icon) => icon.path(),
            None => IconName::Folder.path(),
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
            icon: None,
            color: ItemColor::default(),
        }
    }

    pub fn display_icon(&self) -> SharedString {
        match self.icon {
            Some(icon) => icon.path(),
            None => self.kind.icon().path(),
        }
    }

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
