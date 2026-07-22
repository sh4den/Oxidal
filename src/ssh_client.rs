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

        anyhow::Ok(())
    };

    match tokio::time::timeout(AUTH_TIMEOUT, attempt).await {
        Ok(result) => result?,
        Err(_) => anyhow::bail!("Timed out authenticating to {host}:{port}"),
    }

    Ok(session)
}
