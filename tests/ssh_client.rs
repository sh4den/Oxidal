use super::*;

// Fixture from russh's own test suite; the passphrase is "blabla".
const ENCRYPTED_PKCS8_KEY: &str = "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIFLTBXBgkqhkiG9w0BBQ0wSjApBgkqhkiG9w0BBQwwHAQITo1O0b8YrS0CAggA\nMAwGCCqGSIb3DQIJBQAwHQYJYIZIAWUDBAEqBBBtLH4T1KOfo1GGr7salhR8BIIE\n0KN9ednYwcTGSX3hg7fROhTw7JAJ1D4IdT1fsoGeNu2BFuIgF3cthGHe6S5zceI2\nMpkfwvHbsOlDFWMUIAb/VY8/iYxhNmd5J6NStMYRC9NC0fVzOmrJqE1wITqxtORx\nIkzqkgFUbaaiFFQPepsh5CvQfAgGEWV329SsTOKIgyTj97RxfZIKA+TR5J5g2dJY\nj346SvHhSxJ4Jc0asccgMb0HGh9UUDzDSql0OIdbnZW5KzYJPOx+aDqnpbz7UzY/\nP8N0w/pEiGmkdkNyvGsdttcjFpOWlLnLDhtLx8dDwi/sbEYHtpMzsYC9jPn3hnds\nTcotqjoSZ31O6rJD4z18FOQb4iZs3MohwEdDd9XKblTfYKM62aQJWH6cVQcg+1C7\njX9l2wmyK26Tkkl5Qg/qSfzrCveke5muZgZkFwL0GCcgPJ8RixSB4GOdSMa/hAMU\nkvFAtoV2GluIgmSe1pG5cNMhurxM1dPPf4WnD+9hkFFSsMkTAuxDZIdDk3FA8zof\nYhv0ZTfvT6V+vgH3Hv7Tqcxomy5Qr3tj5vvAqqDU6k7fC4FvkxDh2mG5ovWvc4Nb\nXv8sed0LGpYitIOMldu6650LoZAqJVv5N4cAA2Edqldf7S2Iz1QnA/usXkQd4tLa\nZ80+sDNv9eCVkfaJ6kOVLk/ghLdXWJYRLenfQZtVUXrPkaPpNXgD0dlaTN8KuvML\nUw/UGa+4ybnPsdVflI0YkJKbxouhp4iB4S5ACAwqHVmsH5GRnujf10qLoS7RjDAl\no/wSHxdT9BECp7TT8ID65u2mlJvH13iJbktPczGXt07nBiBse6OxsClfBtHkRLzE\nQF6UMEXsJnIIMRfrZQnduC8FUOkfPOSXc8r9SeZ3GhfbV/DmWZvFPCpjzKYPsM5+\nN8Bw/iZ7NIH4xzNOgwdp5BzjH9hRtCt4sUKVVlWfEDtTnkHNOusQGKu7HkBF87YZ\nRN/Nd3gvHob668JOcGchcOzcsqsgzhGMD8+G9T9oZkFCYtwUXQU2XjMN0R4VtQgZ\nrAxWyQau9xXMGyDC67gQ5xSn+oqMK0HmoW8jh2LG/cUowHFAkUxdzGadnjGhMOI2\nzwNJPIjF93eDF/+zW5E1l0iGdiYyHkJbWSvcCuvTwma9FIDB45vOh5mSR+YjjSM5\nnq3THSWNi7Cxqz12Q1+i9pz92T2myYKBBtu1WDh+2KOn5DUkfEadY5SsIu/Rb7ub\n5FBihk2RN3y/iZk+36I69HgGg1OElYjps3D+A9AjVby10zxxLAz8U28YqJZm4wA/\nT0HLxBiVw+rsHmLP79KvsT2+b4Diqih+VTXouPWC/W+lELYKSlqnJCat77IxgM9e\nYIhzD47OgWl33GJ/R10+RDoDvY4koYE+V5NLglEhbwjloo9Ryv5ywBJNS7mfXMsK\n/uf+l2AscZTZ1mhtL38efTQCIRjyFHc3V31DI0UdETADi+/Omz+bXu0D5VvX+7c6\nb1iVZKpJw8KUjzeUV8yOZhvGu3LrQbhkTPVYL555iP1KN0Eya88ra+FUKMwLgjYr\nJkUx4iad4dTsGPodwEP/Y9oX/Qk3ZQr+REZ8lg6IBoKKqqrQeBJ9gkm1jfKE6Xkc\nCog3JMeTrb3LiPHgN6gU2P30MRp6L1j1J/MtlOAr5rux\n-----END ENCRYPTED PRIVATE KEY-----\n";

// Throwaway ed25519 key generated for these tests only, passphrase "blabla".
const ENCRYPTED_OPENSSH_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABAZxeNw3y\naJl4afi0JF1vltAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIKFdsZ6ZxaXHVZup\nc+n2plMXywolMTxz22GLuBuHbCYVAAAAoDGaeBiR06bkYvD+13Gu7sQewMfGaBpLafdnns\nzN4kh2k3WiM3EU7/v7RgZSQfepKFbwoUlu1PTmB7pk8KUK464cqxfdEZmOQ2a0+DvEk1wo\nSTQGrCQnmTmbSx+j1BEMZ+R3ay2P0f4F0BQo5gce6//9CQE7Q6eKXqDOyOEZ4YclwNYShs\n3uZy/rQXLjfd58pJgo398ph2RdNwKY9DM4p1o=\n-----END OPENSSH PRIVATE KEY-----\n";

fn write_key(name: &str) -> std::path::PathBuf {
    write_key_text(name, ENCRYPTED_PKCS8_KEY)
}

fn write_key_text(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("oxidal-{name}-{}.key", std::process::id()));
    std::fs::write(&path, contents.as_bytes()).expect("write key");
    path
}

fn credentials(path: &std::path::Path, passphrase: &str) -> SshCredentials {
    SshCredentials::new(
        "someone".to_string(),
        SecretString::default(),
        Some(path.display().to_string()),
        SecretString::from(passphrase),
    )
}

#[test]
fn the_right_passphrase_unlocks_an_encrypted_key() {
    let path = write_key("good");
    let credentials = credentials(&path, "blabla");

    assert!(
        load_key(
            credentials.key_path().expect("key path"),
            credentials.passphrase()
        )
        .is_ok(),
        "an encrypted key should load once its passphrase is supplied"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_encrypted_key_without_a_passphrase_says_so() {
    let path = write_key("missing");
    let credentials = credentials(&path, "");

    assert!(
        credentials.passphrase().is_none(),
        "an empty passphrase must not be handed to the key loader"
    );
    let err = load_key(credentials.key_path().expect("key path"), None)
        .expect_err("an encrypted key cannot load without its passphrase")
        .to_string();
    assert!(
        err.contains("passphrase"),
        "the error should point at the passphrase, got: {err}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_wrong_passphrase_is_reported_as_such() {
    let path = write_key("wrong");
    let credentials = credentials(&path, "not-the-passphrase");

    let err = load_key(
        credentials.key_path().expect("key path"),
        credentials.passphrase(),
    )
    .expect_err("a wrong passphrase cannot unlock the key")
    .to_string();
    assert!(
        err.contains("passphrase"),
        "the error should point at the passphrase, got: {err}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_openssh_key_unlocks_with_its_passphrase() {
    let path = write_key_text("openssh-good", ENCRYPTED_OPENSSH_KEY);

    assert!(is_encrypted_key(&path.display().to_string()));
    assert!(
        load_key(&path.display().to_string(), Some("blabla")).is_ok(),
        "the format ssh-keygen actually produces should unlock"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_openssh_key_without_a_passphrase_says_so() {
    let path = write_key_text("openssh-missing", ENCRYPTED_OPENSSH_KEY);

    let err = load_key(&path.display().to_string(), None)
        .expect_err("an encrypted key cannot load without its passphrase")
        .to_string();
    assert!(
        err.contains("passphrase"),
        "the error should point at the passphrase, got: {err}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_unencrypted_key_is_not_mistaken_for_an_encrypted_one() {
    let path = write_key_text("plain", "-----BEGIN OPENSSH PRIVATE KEY-----\nnot a key\n");

    assert!(!is_encrypted_key(&path.display().to_string()));
    let err = load_key(&path.display().to_string(), None)
        .expect_err("garbage is not loadable")
        .to_string();
    assert!(
        !err.contains("passphrase"),
        "a malformed key should not be blamed on a passphrase, got: {err}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_blank_key_path_is_treated_as_no_key() {
    for path in [None, Some(String::new()), Some("   ".to_string())] {
        let credentials = SshCredentials::new(
            String::new(),
            SecretString::default(),
            path,
            SecretString::default(),
        );
        assert!(credentials.key_path().is_none());
    }
}

#[test]
fn legacy_algorithms_are_off_unless_asked_for() {
    let credentials = SshCredentials::default();
    assert!(!credentials.legacy_algorithms);

    let preferred = preferred(false);
    assert!(!preferred.kex.iter().any(|name| LEGACY_KEX.contains(name)));
    assert!(
        !preferred
            .cipher
            .iter()
            .any(|name| LEGACY_CIPHERS.contains(name))
    );
    assert!(!preferred.mac.iter().any(|name| LEGACY_MACS.contains(name)));
}

#[test]
fn legacy_algorithms_rank_below_every_modern_one() {
    let defaults = Preferred::default();
    let legacy = preferred(true);

    let first_legacy = legacy
        .kex
        .iter()
        .position(|name| LEGACY_KEX.contains(name))
        .expect("legacy kex present");
    let extensions = legacy
        .kex
        .iter()
        .position(|name| *name == kex::EXTENSION_SUPPORT_AS_CLIENT)
        .expect("extension markers kept");
    assert!(
        legacy.kex[..first_legacy]
            .iter()
            .all(|name| defaults.kex.contains(name)),
        "only modern kex may come before the legacy ones"
    );
    assert_eq!(
        &legacy.kex[first_legacy..extensions],
        LEGACY_KEX,
        "legacy kex must sit just before the extension markers"
    );

    assert_eq!(
        &legacy.cipher[..defaults.cipher.len()],
        &defaults.cipher[..]
    );
    assert_eq!(&legacy.cipher[defaults.cipher.len()..], LEGACY_CIPHERS);
    assert_eq!(&legacy.mac[..defaults.mac.len()], &defaults.mac[..]);
    assert_eq!(&legacy.mac[defaults.mac.len()..], LEGACY_MACS);
}

fn no_common(kind: russh::AlgorithmKind, theirs: &[&str]) -> russh::Error {
    russh::Error::NoCommonAlgo {
        kind,
        ours: Vec::new(),
        theirs: theirs.iter().map(|name| name.to_string()).collect(),
    }
}

#[test]
fn a_sha1_only_server_is_pointed_at_legacy_algorithms() {
    let err = no_common(
        russh::AlgorithmKind::Kex,
        &[
            "diffie-hellman-group-exchange-sha1",
            "diffie-hellman-group14-sha1",
        ],
    );
    assert!(legacy_would_help(&err));

    let err = no_common(russh::AlgorithmKind::Cipher, &["aes128-cbc", "3des-cbc"]);
    assert!(legacy_would_help(&err));
}

#[test]
fn legacy_algorithms_are_not_suggested_when_they_would_not_help() {
    let err = no_common(russh::AlgorithmKind::Cipher, &["3des-cbc"]);
    assert!(!legacy_would_help(&err));

    let err = no_common(russh::AlgorithmKind::Key, &["ssh-dss"]);
    assert!(!legacy_would_help(&err));

    assert!(!legacy_would_help(&russh::Error::Disconnect));
}
