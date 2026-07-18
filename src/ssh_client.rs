use std::sync::Arc;
use std::time::Duration;

use russh::client;
use secrecy::{ExposeSecret as _, SecretString};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Handler;

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

pub async fn connect(
    host: String,
    port: u16,
    username: String,
    password: SecretString,
    private_key_path: Option<String>,
) -> anyhow::Result<client::Handle<Handler>> {
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });

    let attempt = async {
        let mut session = client::connect(config, (host.as_str(), port), Handler).await?;

        let mut authenticated = false;
        if let Some(key_path) = private_key_path.filter(|p| !p.trim().is_empty()) {
            let key_pair = russh::keys::load_secret_key(&key_path, None)
                .map_err(|e| anyhow::anyhow!("failed to load private key {key_path}: {e}"))?;
            let hash_alg = session.best_supported_rsa_hash().await?.flatten();
            let auth = session
                .authenticate_publickey(
                    username.clone(),
                    russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key_pair), hash_alg),
                )
                .await?;
            authenticated = auth.success();
        }

        if !authenticated {
            let auth = session
                .authenticate_password(username, password.expose_secret())
                .await?;
            if !auth.success() {
                anyhow::bail!("SSH authentication failed");
            }
        }

        Ok(session)
    };

    match tokio::time::timeout(CONNECT_TIMEOUT, attempt).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!("Timed out connecting to {host}:{port}"),
    }
}
