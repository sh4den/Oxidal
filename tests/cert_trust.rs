use super::*;

#[test]
fn a_trusted_certificate_is_remembered_per_host_and_port() {
    let dir = crate::tempdir::private_dir("oxidal-certs").expect("dir");
    let path = dir.join("trusted_certificates.json");

    assert!(stored_for(&path, "files.test", 21).is_empty());

    remember(&path, "Files.Test", 21, "AA:BB").expect("write");
    remember(&path, "files.test", 21, "AA:BB").expect("write again");
    remember(&path, "files.test", 21, "CC:DD").expect("second");

    assert_eq!(
        stored_for(&path, "files.test", 21),
        vec!["AA:BB".to_string(), "CC:DD".to_string()],
        "host names compare case-insensitively and duplicates are not stored twice"
    );
    assert!(
        stored_for(&path, "files.test", 990).is_empty(),
        "a certificate trusted on one port does not vouch for another"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fingerprints_are_colon_separated_sha256() {
    let fp = fingerprint(b"abc");
    assert_eq!(fp.len(), 32 * 3 - 1);
    assert!(fp.starts_with("BA:78:16:BF"));
}
