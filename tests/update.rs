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
