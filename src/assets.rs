use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

pub struct Assets;

macro_rules! bundled_icons {
    ($($name:literal),* $(,)?) => {
        &[$((
            concat!("icons/oxidal/", $name, ".svg"),
            include_bytes!(concat!("../assets/icons/", $name, ".svg")).as_slice(),
        )),*]
    };
}

const BUNDLED: &[(&str, &[u8])] = bundled_icons![
    "activity",
    "clock",
    "cloud",
    "cluster",
    "code",
    "container",
    "database",
    "file-plus",
    "firewall",
    "flask",
    "folder-plus",
    "gauge",
    "git-branch",
    "key",
    "layers",
    "list",
    "lock",
    "monitor",
    "os/almalinux",
    "os/alpinelinux",
    "os/apple",
    "os/archlinux",
    "os/centos",
    "os/debian",
    "os/elementary",
    "os/endeavouros",
    "os/fedora",
    "os/freebsd",
    "os/gentoo",
    "os/kalilinux",
    "os/linux",
    "os/linuxmint",
    "os/manjaro",
    "os/netbsd",
    "os/nixos",
    "os/openbsd",
    "os/opensuse",
    "os/popos",
    "os/raspberrypi",
    "os/redhat",
    "os/rockylinux",
    "os/ubuntu",
    "os/voidlinux",
    "package",
    "plug",
    "router",
    "scroll-hint",
    "server",
    "shield",
    "signal",
    "thermometer",
    "usb",
    "wifi",
    "wrench",
    "zap",
];

pub const LOGO: &str = "images/oxidal/logo.png";
const LOGO_PNG: &[u8] = include_bytes!("../packaging/linux/icons/256.png");

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == LOGO {
            return Ok(Some(Cow::Borrowed(LOGO_PNG)));
        }
        if let Some((_, data)) = BUNDLED.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(data)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items = gpui_kit::assets::Assets.list(path)?;
        items.extend(
            BUNDLED
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        Ok(items)
    }
}
