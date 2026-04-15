use std::sync::Arc;
use std::time::Duration;

use russh::client;
use russh::ChannelMsg;

use super::backend::{Backend, BackendEvent};

struct SshHandler;

impl client::Handler for SshHandler {
    type Error = russh::Error;

    // TODO: verify against a known_hosts store instead of trusting blindly.
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// Connect to an SSH server and start an interactive shell over a PTY channel.
/// The connection runs on a dedicated background thread with its own tokio
/// runtime; connection failures surface as `BackendEvent::Closed(Some(..))`.
///
/// If `private_key_path` is set, public-key authentication is tried first;
/// otherwise (or if that fails) password authentication is used.
pub fn spawn(
    host: String,
    port: u16,
    username: String,
    password: String,
    private_key_path: Option<String>,
    rows: u16,
    cols: u16,
) -> Backend {
    let (out_tx, out_rx) = async_channel::unbounded::<BackendEvent>();
    let (in_tx, in_rx) = async_channel::unbounded::<Vec<u8>>();
    let (resize_tx, resize_rx) = async_channel::unbounded::<(u16, u16)>();

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = out_tx.send_blocking(BackendEvent::Closed(Some(e.to_string())));
                return;
            }
        };

        let result = runtime.block_on(run(
            host,
            port,
            username,
            password,
            private_key_path,
            rows,
            cols,
            out_tx.clone(),
            in_rx,
            resize_rx,
        ));
        let _ = out_tx.send_blocking(BackendEvent::Closed(result.err().map(|e| e.to_string())));
    });

    Backend::new(out_rx, in_tx, resize_tx)
}

async fn run(
    host: String,
    port: u16,
    username: String,
    password: String,
    private_key_path: Option<String>,
    rows: u16,
    cols: u16,
    out_tx: async_channel::Sender<BackendEvent>,
    in_rx: async_channel::Receiver<Vec<u8>>,
    resize_rx: async_channel::Receiver<(u16, u16)>,
) -> anyhow::Result<()> {
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(60)),
        ..Default::default()
    });

    let mut session = client::connect(config, (host.as_str(), port), SshHandler).await?;

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

    let mut channel = session.channel_open_session().await?;
    channel
        .request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
        .await?;
    channel.request_shell(false).await?;

    loop {
        tokio::select! {
            input = in_rx.recv() => {
                match input {
                    Ok(bytes) => { channel.data(&bytes[..]).await?; }
                    Err(_) => break,
                }
            }
            resize = resize_rx.recv() => {
                if let Ok((rows, cols)) = resize {
                    channel.window_change(cols as u32, rows as u32, 0, 0).await?;
                }
            }
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        if out_tx.send(BackendEvent::Data(data.to_vec())).await.is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { .. }) | None => break,
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
