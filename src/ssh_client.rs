use std::sync::Arc;
use std::time::Duration;

use russh::client;

/// Trust-on-first-use host key handler shared by the terminal and SFTP SSH
/// backends. Both need an authenticated `client::Handle` before opening their
/// own channel (a shell PTY vs. an SFTP subsystem), so the connect+auth
/// dance lives here once instead of being duplicated per backend.
pub struct Handler;

impl client::Handler for Handler {
    type Error = russh::Error;

    // TODO: verify against a known_hosts store instead of trusting blindly.
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// Connect and authenticate to an SSH server. If `private_key_path` is set,
/// public-key authentication is tried first; otherwise (or if that fails)
/// password authentication is used.
pub async fn connect(
    host: String,
    port: u16,
    username: String,
    password: String,
    private_key_path: Option<String>,
) -> anyhow::Result<client::Handle<Handler>> {
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(60)),
        ..Default::default()
    });

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
        let auth = session.authenticate_password(username, password).await?;
        if !auth.success() {
            anyhow::bail!("SSH authentication failed");
        }
    }

    Ok(session)
}
