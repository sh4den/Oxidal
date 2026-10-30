use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

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
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items = gpui_component_assets::Assets.list(path)?;
        items.extend(
            BUNDLED
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        Ok(items)
    }
}
