use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpStream;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(150);
const AUTH_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Handler {
    host: String,
    port: u16,
    rejection: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        match crate::host_keys::verify(&self.host, self.port, server_public_key).await {
            Ok(()) => Ok(true),
            Err(message) => {
                if let Ok(mut rejection) = self.rejection.lock() {
                    *rejection = Some(message);
                }
                Ok(false)
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct SshCredentials {
    pub username: String,
    pub password: SecretString,
    pub private_key_path: Option<String>,
    pub key_passphrase: SecretString,
}

impl SshCredentials {
    pub fn new(
        username: String,
        password: SecretString,
        private_key_path: Option<String>,
        key_passphrase: SecretString,
    ) -> Self {
        Self {
            username,
            password,
            private_key_path,
            key_passphrase,
        }
    }

    fn key_path(&self) -> Option<&str> {
        self.private_key_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
    }

    fn passphrase(&self) -> Option<&str> {
        let passphrase = self.key_passphrase.expose_secret();
        (!passphrase.is_empty()).then_some(passphrase)
    }
}

fn is_encrypted_key(path: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.contains("ENCRYPTED")
        || russh::keys::PrivateKey::from_openssh(&text)
            .map(|key| key.is_encrypted())
            .unwrap_or(false)
}

fn load_key(path: &str, passphrase: Option<&str>) -> anyhow::Result<russh::keys::PrivateKey> {
    russh::keys::load_secret_key(path, passphrase).map_err(|err| {
        if passphrase.is_some() {
            anyhow::anyhow!("couldn't unlock private key {path}, check the passphrase: {err}")
        } else if matches!(err, russh::keys::Error::KeyIsEncrypted) || is_encrypted_key(path) {
            anyhow::anyhow!(
                "private key {path} is protected by a passphrase; add it to the session and try again"
            )
        } else {
            anyhow::anyhow!("failed to load private key {path}: {err}")
        }
    })
}

pub async fn connect(
    host: String,
    port: u16,
    credentials: SshCredentials,
) -> anyhow::Result<client::Handle<Handler>> {
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });

    let rejection = Arc::new(Mutex::new(None));
    let handler = Handler {
        host: host.clone(),
        port,
        rejection: rejection.clone(),
    };

    let stream = match tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => anyhow::bail!("Could not reach {host}:{port}: {e}"),
        Err(_) => anyhow::bail!("Timed out connecting to {host}:{port}"),
    };

    let mut session = match tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        client::connect_stream(config, stream, handler),
    )
    .await
    {
        Ok(Ok(session)) => session,
        Ok(Err(e)) => {
            let rejected = rejection.lock().ok().and_then(|mut slot| slot.take());
            match rejected {
                Some(message) => anyhow::bail!(message),
                None => return Err(e.into()),
            }
        }
        Err(_) => anyhow::bail!("Timed out during the SSH handshake with {host}:{port}"),
    };

    let attempt = async {
        let mut authenticated = false;
        let key_configured = credentials.key_path().is_some();
        if let Some(key_path) = credentials.key_path() {
            let key_pair = load_key(key_path, credentials.passphrase())?;
            let hash_alg = session.best_supported_rsa_hash().await?.flatten();
            let auth = session
                .authenticate_publickey(
                    credentials.username.clone(),
                    russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key_pair), hash_alg),
                )
                .await?;
            authenticated = auth.success();
        }

        if !authenticated {
            let password = credentials.password.expose_secret();
            if password.is_empty() {
                if key_configured {
                    anyhow::bail!(
                        "the server rejected the private key, and no password is set for this \
                         session"
                    );
                }
                anyhow::bail!("no private key or password is set for this session");
            }

            let auth = session
                .authenticate_password(credentials.username.clone(), password)
                .await?;
            if !auth.success() {
                if key_configured {
                    anyhow::bail!("the server rejected both the private key and the password");
                }
                anyhow::bail!("SSH authentication failed");
            }
        }

        anyhow::Ok(())
    };

    match tokio::time::timeout(AUTH_TIMEOUT, attempt).await {
        Ok(result) => result?,
        Err(_) => anyhow::bail!("Timed out authenticating to {host}:{port}"),
    }

    Ok(session)
}

#[cfg(test)]
mod tests {
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
}
