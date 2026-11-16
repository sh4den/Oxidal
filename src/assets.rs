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
    "scroll-hint",
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
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items = gpui_kit::assets::AllAssets.list(path)?;
        items.extend(
            BUNDLED
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        Ok(items)
    }
}
