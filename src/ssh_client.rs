use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client;
use secrecy::{ExposeSecret as _, SecretString};

use crate::proxy::ProxyConfig;

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
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let checked = match server_public_key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => {
                crate::host_keys::verify(&self.host, self.port, key).await
            }
            russh::keys::PublicKeyOrCertificate::Certificate(_) => Err(format!(
                "{} offered a host certificate, which Oxidal cannot verify",
                self.host
            )),
        };

        match checked {
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
    proxy: Option<ProxyConfig>,
) -> anyhow::Result<client::Handle<Handler>> {
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        window_size: 16 * 1024 * 1024,
        ..Default::default()
    });

    let rejection = Arc::new(Mutex::new(None));
    let handler = Handler {
        host: host.clone(),
        port,
        rejection: rejection.clone(),
    };

    let stream = crate::proxy::open_stream(&host, port, proxy.as_ref(), CONNECT_TIMEOUT).await?;

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
#[path = "../tests/ssh_client.rs"]
mod tests;
