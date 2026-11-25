use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::{AlgorithmKind, Preferred, cipher, client, kex, mac};
use secrecy::{ExposeSecret as _, SecretString};

use crate::proxy::ProxyConfig;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(150);
const AUTH_TIMEOUT: Duration = Duration::from_secs(30);

const LEGACY_KEX: &[kex::Name] = &[kex::DH_GEX_SHA1, kex::DH_G14_SHA1, kex::DH_G1_SHA1];
const LEGACY_CIPHERS: &[cipher::Name] = &[
    cipher::AES_256_CBC,
    cipher::AES_192_CBC,
    cipher::AES_128_CBC,
];
const LEGACY_MACS: &[mac::Name] = &[mac::HMAC_SHA1_ETM, mac::HMAC_SHA1];

/// russh's defaults, with the SHA-1 and CBC algorithms that old devices still
/// require appended after every modern one, so they're only picked when the
/// server offers nothing better.
fn preferred(legacy: bool) -> Preferred {
    let defaults = Preferred::default();
    if !legacy {
        return defaults;
    }

    let mut kex_order = defaults.kex.to_vec();
    let extensions = kex_order
        .iter()
        .position(|name| *name == kex::EXTENSION_SUPPORT_AS_CLIENT)
        .unwrap_or(kex_order.len());
    kex_order.splice(extensions..extensions, LEGACY_KEX.iter().copied());

    Preferred {
        kex: kex_order.into(),
        cipher: [&defaults.cipher[..], LEGACY_CIPHERS].concat().into(),
        mac: [&defaults.mac[..], LEGACY_MACS].concat().into(),
        ..defaults
    }
}

/// Whether a failed negotiation would have succeeded with legacy algorithms on.
fn legacy_would_help(err: &russh::Error) -> bool {
    let russh::Error::NoCommonAlgo { kind, theirs, .. } = err else {
        return false;
    };
    match kind {
        AlgorithmKind::Kex => offers_any(theirs, LEGACY_KEX),
        AlgorithmKind::Cipher => offers_any(theirs, LEGACY_CIPHERS),
        AlgorithmKind::Mac => offers_any(theirs, LEGACY_MACS),
        _ => false,
    }
}

fn offers_any<N: AsRef<str>>(theirs: &[String], ours: &[N]) -> bool {
    theirs
        .iter()
        .any(|name| ours.iter().any(|ours| ours.as_ref() == name))
}

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
    pub legacy_algorithms: bool,
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
            legacy_algorithms: false,
        }
    }

    pub fn with_legacy_algorithms(mut self, legacy_algorithms: bool) -> Self {
        self.legacy_algorithms = legacy_algorithms;
        self
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
        preferred: preferred(credentials.legacy_algorithms),
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
                None if !credentials.legacy_algorithms && legacy_would_help(&e) => anyhow::bail!(
                    "{host} only offers outdated algorithms. Turn on Legacy Algorithms in the \
                     session's Auth settings to connect anyway ({e})"
                ),
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
