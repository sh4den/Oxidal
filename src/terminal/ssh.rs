use russh::{Channel, ChannelMsg};
use secrecy::SecretString;

use super::backend::{Backend, BackendEvent};
use super::stats::{self, RemoteStats};
use crate::ssh_client;

pub fn spawn(
    host: String,
    port: u16,
    username: String,
    password: SecretString,
    private_key_path: Option<String>,
    rows: u16,
    cols: u16,
) -> (Backend, async_channel::Receiver<RemoteStats>) {
    let (out_tx, out_rx) = async_channel::unbounded::<BackendEvent>();
    let (in_tx, in_rx) = async_channel::unbounded::<Vec<u8>>();
    let (resize_tx, resize_rx) = async_channel::unbounded::<(u16, u16)>();
    let (stats_tx, stats_rx) = async_channel::unbounded::<RemoteStats>();

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
            stats_tx,
        ));
        let _ = out_tx.send_blocking(BackendEvent::Closed(result.err().map(|e| e.to_string())));
    });

    (Backend::new(out_rx, in_tx, resize_tx), stats_rx)
}

async fn run(
    host: String,
    port: u16,
    username: String,
    password: SecretString,
    private_key_path: Option<String>,
    rows: u16,
    cols: u16,
    out_tx: async_channel::Sender<BackendEvent>,
    in_rx: async_channel::Receiver<Vec<u8>>,
    resize_rx: async_channel::Receiver<(u16, u16)>,
    stats_tx: async_channel::Sender<RemoteStats>,
) -> anyhow::Result<()> {
    let session = ssh_client::connect(host, port, username, password, private_key_path).await?;

    let mut channel = session.channel_open_session().await?;
    channel
        .request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
        .await?;
    channel.request_shell(false).await?;

    let mut monitor = match session.channel_open_session().await {
        Ok(ch) => match ch.exec(true, stats::MONITOR_SCRIPT).await {
            Ok(()) => Some(ch),
            Err(_) => None,
        },
        Err(_) => None,
    };
    let mut frames = stats::FrameSplitter::default();
    let mut parser = stats::StatsParser::new(port);

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
            msg = wait_monitor(&mut monitor) => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        for frame in frames.push(data) {
                            let _ = stats_tx.send(parser.parse_frame(&frame)).await;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { .. })
                    | Some(ChannelMsg::Eof)
                    | Some(ChannelMsg::Close)
                    | None => monitor = None,
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

async fn wait_monitor(monitor: &mut Option<Channel<russh::client::Msg>>) -> Option<ChannelMsg> {
    match monitor {
        Some(channel) => channel.wait().await,
        None => std::future::pending().await,
    }
}
