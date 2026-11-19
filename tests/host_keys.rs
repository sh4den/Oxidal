use super::*;

const FIRST_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
const SECOND_KEY: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHA\
                              yNTYAAABBBEmKSENjQEezOmxkZMy7opKgwFB9nkt5YRrYMjNuG5N87uRgg6CLrb\
                              o5wAdT/y6v0mKV0U2w0WZ2YB/++Tpockg=";

fn key(openssh: &str) -> PublicKey {
    PublicKey::from_openssh(openssh).expect("a well formed public key")
}

#[test]
fn a_host_can_carry_more_than_one_trusted_key() {
    let dir = crate::tempdir::private_dir("oxidal-hosts").expect("dir");
    let path = dir.join("known_hosts");
    std::fs::write(&path, format!("vault.test {FIRST_KEY}\n")).expect("file");

    let sshd = key(FIRST_KEY);
    let dropbear = key(SECOND_KEY);

    assert!(
        matches!(
            assess("vault.test", 22, &sshd, &path),
            Ok(KeyStatus::Trusted)
        ),
        "the recorded key must connect without a prompt"
    );
    assert!(
        matches!(
            assess("other.test", 22, &sshd, &path),
            Ok(KeyStatus::Unknown)
        ),
        "a host with no entries is unknown, not conflicting"
    );
    match assess("vault.test", 22, &dropbear, &path) {
        Ok(KeyStatus::Conflicting(stored)) => {
            assert_eq!(stored.len(), 1, "every stored key is shown for comparison");
            assert!(stored[0].starts_with("ssh-ed25519 SHA256:"));
        }
        _ => panic!("a second identity must surface as a conflict, not an error"),
    }

    russh::keys::known_hosts::learn_known_hosts_path("vault.test", 22, &dropbear, &path)
        .expect("append");

    assert!(
        matches!(
            assess("vault.test", 22, &dropbear, &path),
            Ok(KeyStatus::Trusted)
        ),
        "once accepted, the second identity connects without a prompt"
    );
    assert!(
        matches!(
            assess("vault.test", 22, &sshd, &path),
            Ok(KeyStatus::Trusted)
        ),
        "accepting the second identity must not evict the first"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_nonstandard_port_keeps_its_own_identities() {
    let dir = crate::tempdir::private_dir("oxidal-ports").expect("dir");
    let path = dir.join("known_hosts");
    std::fs::write(
        &path,
        format!("vault.test {FIRST_KEY}\n[vault.test]:2222 {SECOND_KEY}\n"),
    )
    .expect("file");

    let sshd = key(FIRST_KEY);
    let dropbear = key(SECOND_KEY);

    assert!(matches!(
        assess("vault.test", 2222, &dropbear, &path),
        Ok(KeyStatus::Trusted)
    ));
    assert!(
        matches!(
            assess("vault.test", 2222, &sshd, &path),
            Ok(KeyStatus::Conflicting(_))
        ),
        "port 22's key does not vouch for port 2222"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
