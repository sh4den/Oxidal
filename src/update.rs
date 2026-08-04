use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};

const RELEASES_URL: &str = "https://api.github.com/repos/sh4den/Oxidal/releases/latest";
const USER_AGENT: &str = concat!("Oxidal/", env!("CARGO_PKG_VERSION"));
const CHECKSUMS_ASSET: &str = "SHA256SUMS";
const MAX_ASSET_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CHECKSUMS_BYTES: u64 = 64 * 1024;

const TRUSTED_HOSTS: &[&str] = &[
    "github.com",
    "api.github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

#[derive(Clone)]
pub struct AvailableUpdate {
    pub version: String,
    pub asset_name: String,
    pub asset_url: String,
    checksums_url: String,
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

    #[cfg(target_os = "macos")]
    resign(&exe);

    relaunch(&exe)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn app_bundle(exe: &Path) -> Option<&Path> {
    exe.ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
}

#[cfg(target_os = "macos")]
fn resign(exe: &Path) {
    let target = app_bundle(exe).unwrap_or(exe);
    let _ = std::process::Command::new("/usr/bin/codesign")
        .arg("--force")
        .arg("--sign")
        .arg("-")
        .arg(target)
        .status();
}

#[cfg(target_os = "macos")]
fn relaunch(exe: &Path) -> anyhow::Result<()> {
    match app_bundle(exe) {
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

fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    host.split(':').next().filter(|host| !host.is_empty())
}

fn is_trusted(url: &str) -> bool {
    host_of(url).is_some_and(|host| TRUSTED_HOSTS.contains(&host))
}

fn fetch_latest() -> Option<AvailableUpdate> {
    let release: Release = ureq::get(RELEASES_URL)
        .header("User-Agent", USER_AGENT)
        .call()
        .ok()?
        .body_mut()
        .read_json()
        .ok()?;
    let version = release.tag_name.trim_start_matches(['v', 'V']).to_string();
    if !is_newer(&version, env!("CARGO_PKG_VERSION")) {
        return None;
    }

    let checksums = release
        .assets
        .iter()
        .find(|asset| asset.name == CHECKSUMS_ASSET)?;
    let asset = pick_asset(&release.assets)?;
    if !is_trusted(&asset.browser_download_url) || !is_trusted(&checksums.browser_download_url) {
        return None;
    }

    Some(AvailableUpdate {
        version,
        asset_name: asset.name.clone(),
        asset_url: asset.browser_download_url.clone(),
        checksums_url: checksums.browser_download_url.clone(),
    })
}

fn fetch_asset(update: &AvailableUpdate) -> anyhow::Result<PathBuf> {
    if !is_trusted(&update.asset_url) || !is_trusted(&update.checksums_url) {
        anyhow::bail!("the release points somewhere other than GitHub, so it was not downloaded");
    }

    let expected = expected_digest(&update.checksums_url, &update.asset_name)?;

    let dir = crate::tempdir::private_dir("oxidal-update")?;
    let path = dir.join(update.asset_name.replace(['/', '\\', ':'], "_"));

    let digest = match stream_to_file(&update.asset_url, &path) {
        Ok(digest) => digest,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
    };

    if digest != expected {
        let _ = std::fs::remove_dir_all(&dir);
        anyhow::bail!(
            "the downloaded update did not match the checksum published with the release, so it \
             was discarded"
        );
    }

    Ok(path)
}

fn stream_to_file(url: &str, path: &Path) -> anyhow::Result<String> {
    let mut response = ureq::get(url).header("User-Agent", USER_AGENT).call()?;
    let mut file = std::fs::File::create(path)?;
    let mut reader = response.body_mut().as_reader();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;

    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > MAX_ASSET_BYTES {
            anyhow::bail!("the update is larger than {MAX_ASSET_BYTES} bytes, so it was refused");
        }
        hasher.update(&buf[..read]);
        std::io::Write::write_all(&mut file, &buf[..read])?;
    }

    Ok(hex(&hasher.finalize()))
}

fn expected_digest(checksums_url: &str, asset_name: &str) -> anyhow::Result<String> {
    let mut response = ureq::get(checksums_url)
        .header("User-Agent", USER_AGENT)
        .call()?;
    let mut text = String::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_CHECKSUMS_BYTES)
        .read_to_string(&mut text)?;

    find_digest(&text, asset_name).ok_or_else(|| {
        anyhow::anyhow!("the release publishes no checksum for {asset_name}, so it was not applied")
    })
}

fn find_digest(checksums: &str, asset_name: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let (digest, name) = line.split_once(char::is_whitespace)?;
        let name = name.trim().trim_start_matches('*');
        let digest = digest.trim().to_lowercase();
        let matches = name == asset_name
            && digest.len() == 64
            && digest.bytes().all(|b| b.is_ascii_hexdigit());
        matches.then_some(digest)
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
        .filter(|asset| asset.name != CHECKSUMS_ASSET && !matches_any(&asset.name, skip_keys))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_github_urls_are_downloaded_from() {
        for url in [
            "https://github.com/sh4den/Oxidal/releases/download/v1/Oxidal-linux-amd64",
            "https://objects.githubusercontent.com/x",
            "https://release-assets.githubusercontent.com/x",
        ] {
            assert!(is_trusted(url), "{url} should be trusted");
        }

        for url in [
            "http://github.com/sh4den/Oxidal/x",
            "https://github.com.evil.test/x",
            "https://evil.test/x",
            "https://evil.test/?a=github.com",
            "https://github.com@evil.test/x",
            "file:///etc/passwd",
            "",
        ] {
            assert!(!is_trusted(url), "{url} must not be trusted");
        }
    }

    #[test]
    fn a_checksum_is_matched_to_its_own_asset() {
        let digest = "a".repeat(64);
        let other = "b".repeat(64);
        let checksums = format!(
            "{other}  Oxidal-windows-x86_64.exe\n{digest}  Oxidal-linux-amd64\n{other}  \
             Oxidal-macos-intel\n"
        );

        assert_eq!(
            find_digest(&checksums, "Oxidal-linux-amd64"),
            Some(digest),
            "the line for the asset we are downloading is the one that counts"
        );
        assert_eq!(
            find_digest(&checksums, "Oxidal-linux-arm64"),
            None,
            "an asset with no published checksum has no digest"
        );
    }

    #[test]
    fn binary_marked_checksum_lines_are_understood() {
        let digest = "c".repeat(64);
        let checksums = format!("{digest} *Oxidal-windows-x86_64.exe\n");

        assert_eq!(
            find_digest(&checksums, "Oxidal-windows-x86_64.exe"),
            Some(digest),
            "sha256sum's binary marker is not part of the file name"
        );
    }

    #[test]
    fn a_malformed_checksum_line_is_not_accepted() {
        for line in [
            "not-a-digest  Oxidal-linux-amd64",
            "abc  Oxidal-linux-amd64",
            "Oxidal-linux-amd64",
            "",
        ] {
            assert_eq!(
                find_digest(line, "Oxidal-linux-amd64"),
                None,
                "{line:?} is not a usable checksum"
            );
        }
    }

    #[test]
    fn the_checksums_file_is_never_offered_as_the_update() {
        let assets = vec![
            Asset {
                name: CHECKSUMS_ASSET.to_string(),
                browser_download_url: "https://github.com/x/SHA256SUMS".to_string(),
            },
            Asset {
                name: "Oxidal-linux-amd64".to_string(),
                browser_download_url: "https://github.com/x/Oxidal-linux-amd64".to_string(),
            },
        ];

        let picked = pick_asset(&assets).expect("an asset should be picked");
        assert_ne!(
            picked.name, CHECKSUMS_ASSET,
            "the manifest is not a thing to run"
        );
    }

    #[test]
    fn hashing_matches_the_published_digest_format() {
        let mut hasher = Sha256::new();
        hasher.update(b"abc");
        assert_eq!(
            hex(&hasher.finalize()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn newer_versions_are_recognised() {
        assert!(is_newer("0.4.2", "0.4.1"));
        assert!(is_newer("0.5.0", "0.4.9"));
        assert!(!is_newer("0.4.1", "0.4.1"));
        assert!(!is_newer("0.4.0", "0.4.1"));
    }
}
