use gpui_kit::assets::IconName as Lucide;
use std::fs;
use std::path::PathBuf;

use gpui_kit::component::{IconName, IconNamed};
use gpui_kit::{Hsla, SharedString, rgb};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::settings::config_dir;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionKind {
    Ssh,
    Sftp,
    Ftp,
    Telnet,
    Rdp,
    Serial,
    Local,
}

impl SessionKind {
    pub const ALL: [SessionKind; 7] = [
        SessionKind::Ssh,
        SessionKind::Sftp,
        SessionKind::Ftp,
        SessionKind::Telnet,
        SessionKind::Rdp,
        SessionKind::Serial,
        SessionKind::Local,
    ];

    pub fn icon(self) -> IconName {
        match self {
            SessionKind::Ssh | SessionKind::Local => IconName::SquareTerminal,
            SessionKind::Sftp => IconName::Folder,
            SessionKind::Ftp => IconName::FolderOpen,
            SessionKind::Telnet => IconName::Network,
            SessionKind::Rdp => IconName::LayoutDashboard,
            SessionKind::Serial => IconName::Cpu,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SessionKind::Ssh => "SSH",
            SessionKind::Sftp => "SFTP",
            SessionKind::Ftp => "FTP",
            SessionKind::Telnet => "Telnet",
            SessionKind::Rdp => "RDP",
            SessionKind::Serial => "Serial",
            SessionKind::Local => "Local",
        }
    }

    pub fn is_available(self) -> bool {
        !matches!(self, SessionKind::Rdp)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FtpSecurity {
    #[default]
    Explicit,
    Implicit,
    None,
}

impl FtpSecurity {
    pub const IMPLICIT_PORT: u16 = 990;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemIcon(Glyph);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Glyph {
    Lucide(Lucide),
    Os(usize),
}

const OS_ICONS: &[(&str, &str)] = &[
    ("os/linux", "Linux"),
    ("os/ubuntu", "Ubuntu"),
    ("os/debian", "Debian"),
    ("os/raspberrypi", "Raspberry Pi OS"),
    ("os/alpinelinux", "Alpine Linux"),
    ("os/archlinux", "Arch Linux"),
    ("os/manjaro", "Manjaro"),
    ("os/endeavouros", "EndeavourOS"),
    ("os/fedora", "Fedora"),
    ("os/centos", "CentOS"),
    ("os/redhat", "Red Hat"),
    ("os/rockylinux", "Rocky Linux"),
    ("os/almalinux", "AlmaLinux"),
    ("os/opensuse", "openSUSE"),
    ("os/gentoo", "Gentoo"),
    ("os/linuxmint", "Linux Mint"),
    ("os/nixos", "NixOS"),
    ("os/kalilinux", "Kali Linux"),
    ("os/popos", "Pop!_OS"),
    ("os/voidlinux", "Void Linux"),
    ("os/elementary", "elementary OS"),
    ("os/freebsd", "FreeBSD"),
    ("os/openbsd", "OpenBSD"),
    ("os/netbsd", "NetBSD"),
    ("os/apple", "macOS"),
];

const SUGGESTED_ICONS: &[&str] = &[
    "square-terminal",
    "code",
    "server",
    "layout-grid",
    "container",
    "database",
    "hard-drive",
    "cpu",
    "memory-stick",
    "usb",
    "network",
    "router",
    "wifi",
    "signal",
    "globe",
    "cloud",
    "brick-wall",
    "shield",
    "lock",
    "key",
    "layers",
    "package",
    "git-branch",
    "monitor",
    "gauge",
    "activity",
    "zap",
    "plug",
    "clock",
    "flask-conical",
    "wrench",
    "bot",
    "building-2",
    "user",
    "folder",
    "folder-open",
    "star",
    "heart",
    "bell",
    "inbox",
    "map",
    "chart-pie",
    "github",
    "book-open",
    "layout-dashboard",
    "frame",
    "palette",
];

struct IconCatalog {
    icons: Vec<(ItemIcon, String)>,
    by_name: std::collections::HashMap<String, ItemIcon>,
}

static CATALOG: std::sync::LazyLock<IconCatalog> = std::sync::LazyLock::new(|| {
    let icons: Vec<(ItemIcon, String)> = Lucide::ALL
        .iter()
        .map(|icon| {
            let path = icon.path();
            let name = path
                .trim_start_matches("icons/")
                .trim_end_matches(".svg")
                .to_string();
            (ItemIcon(Glyph::Lucide(*icon)), name)
        })
        .collect();
    let by_name = icons
        .iter()
        .map(|(icon, name)| (name.clone(), *icon))
        .chain(
            OS_ICONS
                .iter()
                .enumerate()
                .map(|(index, (name, _))| (name.to_string(), ItemIcon(Glyph::Os(index)))),
        )
        .collect();
    IconCatalog { icons, by_name }
});

impl ItemIcon {
    pub const UNKNOWN: ItemIcon = ItemIcon(Glyph::Lucide(Lucide::CircleQuestionMark));

    pub fn from_name(name: &str) -> Option<Self> {
        CATALOG.by_name.get(name).copied()
    }

    pub fn name(self) -> &'static str {
        match self.0 {
            Glyph::Os(index) => OS_ICONS[index].0,
            Glyph::Lucide(_) => CATALOG
                .icons
                .iter()
                .find(|(icon, _)| *icon == self)
                .map_or("", |(_, name)| name.as_str()),
        }
    }

    pub fn label(self) -> &'static str {
        match self.0 {
            Glyph::Os(index) => OS_ICONS[index].1,
            Glyph::Lucide(_) => self.name(),
        }
    }

    pub fn is_os(self) -> bool {
        matches!(self.0, Glyph::Os(_))
    }

    pub fn os_logos() -> impl Iterator<Item = ItemIcon> {
        (0..OS_ICONS.len()).map(|index| ItemIcon(Glyph::Os(index)))
    }

    pub fn search_os(query: &str) -> impl Iterator<Item = ItemIcon> + '_ {
        let words = query_words(query);
        Self::os_logos().filter(move |icon| {
            let haystack = format!("{} {}", icon.name(), icon.label()).to_lowercase();
            words.iter().all(|word| haystack.contains(word.as_str()))
        })
    }

    /// The short list shown before the user searches.
    pub fn suggested() -> impl Iterator<Item = ItemIcon> {
        SUGGESTED_ICONS
            .iter()
            .filter_map(|name| CATALOG.by_name.get(*name).copied())
    }

    /// Every icon whose name contains all the words of `query`, in catalog order.
    pub fn search(query: &str) -> impl Iterator<Item = ItemIcon> + '_ {
        let words = query_words(query);
        CATALOG
            .icons
            .iter()
            .filter(move |(_, name)| words.iter().all(|word| name.contains(word.as_str())))
            .map(|(icon, _)| *icon)
    }
}

fn query_words(query: &str) -> Vec<String> {
    query
        .split(|c: char| c.is_whitespace() || c == '-')
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

impl IconNamed for ItemIcon {
    fn path(self) -> SharedString {
        match self.0 {
            Glyph::Lucide(icon) => icon.path(),
            Glyph::Os(index) => format!("icons/oxidal/{}.svg", OS_ICONS[index].0).into(),
        }
    }
}

impl Serialize for ItemIcon {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

fn lenient_icon<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ItemIcon>, D::Error> {
    let name: Option<String> = Option::deserialize(deserializer)?;
    Ok(name.map(|name| ItemIcon::from_name(&name).unwrap_or(ItemIcon::UNKNOWN)))
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

#[derive(Clone, Serialize, Deserialize)]
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
    #[serde(skip)]
    pub key_passphrase: SecretString,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    #[serde(default)]
    pub private_key_path: Option<String>,
    #[serde(default)]
    pub proxy_host: String,
    #[serde(default = "default_proxy_port")]
    pub proxy_port: u16,
    #[serde(default)]
    pub proxy_username: String,
    #[serde(skip)]
    pub proxy_password: SecretString,
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    #[serde(default)]
    pub show_hidden_files: bool,
    #[serde(default = "default_monitoring")]
    pub monitoring: bool,
    #[serde(default)]
    pub ftp_security: FtpSecurity,
    #[serde(default)]
    pub ftp_active_mode: bool,
    #[serde(default, deserialize_with = "lenient_icon")]
    pub icon: Option<ItemIcon>,
    #[serde(default)]
    pub color: ItemColor,
    #[serde(skip)]
    secrets_loaded: bool,
}

fn default_monitoring() -> bool {
    true
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionFolder {
    pub id: Uuid,
    pub name: String,
    #[serde(default, deserialize_with = "lenient_icon")]
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

pub fn default_proxy_port() -> u16 {
    1080
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
            key_passphrase: SecretString::default(),
            baud_rate: default_baud_rate(),
            private_key_path: None,
            proxy_host: String::new(),
            proxy_port: default_proxy_port(),
            proxy_username: String::new(),
            proxy_password: SecretString::default(),
            folder_id: None,
            show_hidden_files: false,
            monitoring: default_monitoring(),
            ftp_security: FtpSecurity::default(),
            ftp_active_mode: false,
            icon: None,
            color: ItemColor::default(),
            secrets_loaded: true,
        }
    }

    pub fn secrets_loaded(&self) -> bool {
        self.secrets_loaded
    }

    pub fn ensure_secrets(&mut self) {
        if self.secrets_loaded {
            return;
        }
        if let Some(password) = crate::credentials::load_password(self.id) {
            self.password = password;
        }
        if let Some(passphrase) = crate::credentials::load_key_passphrase(self.id) {
            self.key_passphrase = passphrase;
        }
        if let Some(password) = crate::credentials::load_proxy_password(self.id) {
            self.proxy_password = password;
        }
        self.secrets_loaded = true;
    }

    pub fn display_icon(&self) -> SharedString {
        match self.icon {
            Some(icon) => icon.path(),
            None => self.kind.icon().path(),
        }
    }

    pub fn credentials(&self) -> crate::ssh_client::SshCredentials {
        crate::ssh_client::SshCredentials::new(
            self.username.clone(),
            self.password.clone(),
            self.private_key_path.clone(),
            self.key_passphrase.clone(),
        )
    }

    pub fn ftp_target(&self) -> crate::sftp::FtpTarget {
        crate::sftp::FtpTarget {
            host: self.host.trim().to_string(),
            port: self.port,
            username: self.username.trim().to_string(),
            password: self.password.clone(),
            security: self.ftp_security,
            active_mode: self.ftp_active_mode,
            proxy: self.proxy(),
        }
    }

    pub fn proxy(&self) -> Option<crate::proxy::ProxyConfig> {
        let host = self.proxy_host.trim();
        (!host.is_empty()).then(|| crate::proxy::ProxyConfig {
            host: host.to_string(),
            port: self.proxy_port,
            username: self.proxy_username.trim().to_string(),
            password: self.proxy_password.clone(),
        })
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
            SessionKind::Ssh
            | SessionKind::Sftp
            | SessionKind::Ftp
            | SessionKind::Telnet
            | SessionKind::Rdp => {
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

    pub fn default_name(&self) -> String {
        let host = self.host.trim();
        match self.kind {
            SessionKind::Local => "Local shell".to_string(),
            _ if host.is_empty() => self.kind.label().to_string(),
            SessionKind::Serial => host.to_string(),
            SessionKind::Ssh
            | SessionKind::Sftp
            | SessionKind::Ftp
            | SessionKind::Telnet
            | SessionKind::Rdp => {
                let user = self.username.trim();
                if user.is_empty() {
                    host.to_string()
                } else {
                    format!("{host} ({user})")
                }
            }
        }
    }
}

impl SessionKind {
    pub fn default_port(self) -> u16 {
        match self {
            SessionKind::Ssh | SessionKind::Sftp => 22,
            SessionKind::Ftp => 21,
            SessionKind::Telnet => 23,
            SessionKind::Rdp => 3389,
            SessionKind::Serial | SessionKind::Local => 0,
        }
    }
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
        session.name = session.name.trim().to_string();
    }
    sessions
}

pub fn save_sessions(sessions: &[Session]) {
    if crate::settings::prepare_config_dir().is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(sessions) {
        let _ = crate::settings::write_private(&sessions_path(), &json);
    }
}

pub fn load_folders() -> Vec<SessionFolder> {
    let path = folders_path();
    let mut folders: Vec<SessionFolder> = match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    for folder in &mut folders {
        folder.name = folder.name.trim().to_string();
    }
    folders
}

pub fn save_folders(folders: &[SessionFolder]) {
    if crate::settings::prepare_config_dir().is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(folders) {
        let _ = crate::settings::write_private(&folders_path(), &json);
    }
}

fn default_sessions() -> Vec<Session> {
    vec![Session::new("Local shell", SessionKind::Local)]
}

#[cfg(test)]
#[path = "../tests/session.rs"]
mod tests;
