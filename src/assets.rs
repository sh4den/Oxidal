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
    "firewall",
    "flask",
    "gauge",
    "git-branch",
    "key",
    "layers",
    "lock",
    "monitor",
    "package",
    "plug",
    "router",
    "server",
    "shield",
    "signal",
    "usb",
    "wifi",
    "wrench",
    "zap",
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
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
