use std::path::{Path, PathBuf};

use serde::Deserialize;

const RELEASES_URL: &str = "https://api.github.com/repos/sh4den/Oxidal/releases/latest";
const USER_AGENT: &str = concat!("Oxidal/", env!("CARGO_PKG_VERSION"));

#[derive(Clone)]
pub struct AvailableUpdate {
    pub version: String,
    pub asset_name: String,
    pub asset_url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub fn check() -> async_channel::Receiver<AvailableUpdate> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        cleanup_previous();
        if let Some(update) = fetch_latest() {
            let _ = tx.send_blocking(update);
        }
    });
    rx
}

pub fn download(update: AvailableUpdate) -> async_channel::Receiver<Result<PathBuf, String>> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = fetch_asset(&update).map_err(|e| e.to_string());
        let _ = tx.send_blocking(result);
    });
    rx
}

pub fn apply_and_restart(downloaded: &Path) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let backup = sibling(&exe, ".old");

    let _ = std::fs::remove_file(&backup);
    std::fs::rename(&exe, &backup)?;
    if let Err(e) = place(downloaded, &exe) {
        let _ = std::fs::rename(&backup, &exe);
        return Err(e);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
    }

    relaunch(&exe)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn relaunch(exe: &Path) -> anyhow::Result<()> {
    let bundle = exe
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"));
    match bundle {
        Some(bundle) => {
            std::process::Command::new("open")
                .arg("-n")
                .arg(bundle)
                .spawn()?;
        }
        None => {
            std::process::Command::new(exe).spawn()?;
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn relaunch(exe: &Path) -> anyhow::Result<()> {
    std::process::Command::new(exe).spawn()?;
    Ok(())
}

fn place(from: &Path, to: &Path) -> anyhow::Result<()> {
    if std::fs::rename(from, to).is_err() {
        std::fs::copy(from, to)?;
        let _ = std::fs::remove_file(from);
    }
    Ok(())
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn cleanup_previous() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(sibling(&exe, ".old"));
    }
}

fn fetch_latest() -> Option<AvailableUpdate> {
    let release: Release = ureq::get(RELEASES_URL)
        .set("User-Agent", USER_AGENT)
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let version = release.tag_name.trim_start_matches(['v', 'V']).to_string();
    if !is_newer(&version, env!("CARGO_PKG_VERSION")) {
        return None;
    }
    let asset = pick_asset(&release.assets)?;
    Some(AvailableUpdate {
        version,
        asset_name: asset.name.clone(),
        asset_url: asset.browser_download_url.clone(),
    })
}

fn fetch_asset(update: &AvailableUpdate) -> anyhow::Result<PathBuf> {
    let dir = std::env::temp_dir().join("oxidal-update");
    std::fs::create_dir_all(&dir)?;
    let file_name = update.asset_name.replace(['/', '\\', ':'], "_");
    let path = dir.join(file_name);
    let response = ureq::get(&update.asset_url)
        .set("User-Agent", USER_AGENT)
        .call()?;
    let mut file = std::fs::File::create(&path)?;
    std::io::copy(&mut response.into_reader(), &mut file)?;
    Ok(path)
}

fn pick_asset(assets: &[Asset]) -> Option<&Asset> {
    let os_keys: &[&str] = match std::env::consts::OS {
        "windows" => &["windows", "win64", "win32", "win"],
        "macos" => &["macos", "darwin", "mac", "apple", "osx"],
        _ => &["linux"],
    };
    let arch_keys: &[&str] = match std::env::consts::ARCH {
        "x86_64" => &["x86_64", "amd64", "x64", "intel"],
        "aarch64" => &["aarch64", "arm64", "silicon"],
        _ => &[],
    };
    let skip_keys: &[&str] = &[
        ".zip",
        ".tar",
        ".dmg",
        ".deb",
        ".msi",
        ".appimage",
        "setup",
        "installer",
    ];
    let matches_any = |name: &str, keys: &[&str]| {
        let name = name.to_lowercase();
        keys.iter().any(|key| name.contains(key))
    };

    let updatable: Vec<&Asset> = assets
        .iter()
        .filter(|asset| !matches_any(&asset.name, skip_keys))
        .collect();
    let mut candidates: Vec<&Asset> = updatable
        .iter()
        .filter(|asset| matches_any(&asset.name, os_keys))
        .copied()
        .collect();
    if candidates.is_empty() && updatable.len() == 1 {
        candidates.push(updatable[0]);
    }
    candidates
        .iter()
        .find(|asset| matches_any(&asset.name, arch_keys))
        .copied()
        .or_else(|| candidates.first().copied())
}

fn is_newer(latest: &str, current: &str) -> bool {
    version_parts(latest) > version_parts(current)
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| {
            part.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}
