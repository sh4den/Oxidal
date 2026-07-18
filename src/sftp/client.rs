use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileType;

use super::{SftpClient, SftpCommand, SftpEntry, SftpEvent, join_remote};
use crate::ssh_client;

const CHUNK_SIZE: usize = 64 * 1024;

pub fn spawn(
    host: String,
    port: u16,
    username: String,
    password: SecretString,
    private_key_path: Option<String>,
    initial_path: String,
) -> SftpClient {
    let (out_tx, out_rx) = async_channel::unbounded::<SftpEvent>();
    let (cmd_tx, cmd_rx) = async_channel::unbounded::<SftpCommand>();

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = out_tx.send_blocking(SftpEvent::Closed(Some(e.to_string())));
                return;
            }
        };

        let result = runtime.block_on(run(
            host,
            port,
            username,
            password,
            private_key_path,
            initial_path,
            out_tx.clone(),
            cmd_rx,
        ));
        let _ = out_tx.send_blocking(SftpEvent::Closed(result.err().map(|e| e.to_string())));
    });

    SftpClient {
        events: out_rx,
        commands: cmd_tx,
    }
}

async fn run(
    host: String,
    port: u16,
    username: String,
    password: SecretString,
    private_key_path: Option<String>,
    initial_path: String,
    out_tx: async_channel::Sender<SftpEvent>,
    cmd_rx: async_channel::Receiver<SftpCommand>,
) -> anyhow::Result<()> {
    let session = ssh_client::connect(host, port, username, password, private_key_path).await?;

    let channel = session.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let stream = channel.into_stream();
    let sftp = SftpSession::new(stream).await?;

    let mut current_dir = sftp
        .canonicalize(initial_path.clone())
        .await
        .unwrap_or(initial_path);

    list_and_send(&sftp, current_dir.clone(), &out_tx).await;

    while let Ok(cmd) = cmd_rx.recv().await {
        match cmd {
            SftpCommand::List { path } => {
                current_dir = path.clone();
                list_and_send(&sftp, path, &out_tx).await;
            }
            SftpCommand::CreateDir { name } => {
                let path = join_remote(&current_dir, &name);
                if let Err(err) = sftp.create_dir(path).await {
                    send_error(&out_tx, format!("Couldn't create folder: {err}")).await;
                }
                list_and_send(&sftp, current_dir.clone(), &out_tx).await;
            }
            SftpCommand::Rename { from, to } => {
                if let Err(err) = sftp.rename(from, to).await {
                    send_error(&out_tx, format!("Couldn't rename: {err}")).await;
                }
                list_and_send(&sftp, current_dir.clone(), &out_tx).await;
            }
            SftpCommand::RemoveFile { path } => {
                if let Err(err) = sftp.remove_file(path).await {
                    send_error(&out_tx, format!("Couldn't delete file: {err}")).await;
                }
                list_and_send(&sftp, current_dir.clone(), &out_tx).await;
            }
            SftpCommand::RemoveDir { path } => {
                if let Err(err) = sftp.remove_dir(path).await {
                    send_error(&out_tx, format!("Couldn't delete folder: {err}")).await;
                }
                list_and_send(&sftp, current_dir.clone(), &out_tx).await;
            }
            SftpCommand::Upload { local, remote } => {
                if let Err(err) = do_upload(&sftp, &local, &remote, &out_tx).await {
                    let _ = out_tx
                        .send(SftpEvent::TransferFinished {
                            error: Some(err.to_string()),
                        })
                        .await;
                }
                list_and_send(&sftp, current_dir.clone(), &out_tx).await;
            }
            SftpCommand::Download {
                remote,
                local,
                open_when_done,
            } => match do_download(&sftp, &remote, &local, &out_tx).await {
                Ok(()) if open_when_done => {
                    if let Err(err) = open::that_detached(&local) {
                        send_error(&out_tx, format!("Couldn't open {}: {err}", local.display()))
                            .await;
                    }
                }
                Ok(()) => {}
                Err(err) => {
                    let _ = out_tx
                        .send(SftpEvent::TransferFinished {
                            error: Some(err.to_string()),
                        })
                        .await;
                }
            },
        }
    }

    Ok(())
}

async fn send_error(out_tx: &async_channel::Sender<SftpEvent>, message: String) {
    let _ = out_tx.send(SftpEvent::Error(message)).await;
}

async fn list_and_send(
    sftp: &SftpSession,
    path: String,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    match read_dir(sftp, &path).await {
        Ok(entries) => {
            let _ = out_tx.send(SftpEvent::Listing { path, entries }).await;
        }
        Err(err) => send_error(out_tx, format!("Couldn't list {path}: {err}")).await,
    }
}

async fn read_dir(sftp: &SftpSession, path: &str) -> anyhow::Result<Vec<SftpEntry>> {
    let read_dir = sftp.read_dir(path).await?;
    let mut entries: Vec<SftpEntry> = read_dir
        .filter(|entry| {
            let name = entry.file_name();
            name != "." && name != ".."
        })
        .map(|entry| {
            let metadata = entry.metadata();
            SftpEntry {
                name: entry.file_name(),
                path: entry.path(),
                is_dir: matches!(entry.file_type(), FileType::Dir),
                size: metadata.len(),
                modified: metadata.mtime.map(|t| t as u64),
                permissions: metadata.permissions,
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

async fn do_upload(
    sftp: &SftpSession,
    local: &std::path::Path,
    remote: &str,
    out_tx: &async_channel::Sender<SftpEvent>,
) -> anyhow::Result<()> {
    let mut local_file = tokio::fs::File::open(local).await?;
    let total = local_file.metadata().await?.len();
    let label = local
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| remote.to_string());

    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label,
            total: Some(total),
        })
        .await;

    let mut remote_file = sftp.create(remote).await?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut transferred = 0u64;
    loop {
        let n = local_file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        remote_file.write_all(&buf[..n]).await?;
        transferred += n as u64;
        let _ = out_tx
            .send(SftpEvent::TransferProgress { transferred })
            .await;
    }
    remote_file.shutdown().await?;

    let _ = out_tx
        .send(SftpEvent::TransferFinished { error: None })
        .await;
    Ok(())
}

async fn do_download(
    sftp: &SftpSession,
    remote: &str,
    local: &std::path::Path,
    out_tx: &async_channel::Sender<SftpEvent>,
) -> anyhow::Result<()> {
    let metadata = sftp.metadata(remote).await?;
    let label = local
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| remote.to_string());

    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label,
            total: Some(metadata.len()),
        })
        .await;

    let mut remote_file = sftp.open(remote).await?;
    let mut local_file = tokio::fs::File::create(local).await?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut transferred = 0u64;
    loop {
        let n = remote_file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        local_file.write_all(&buf[..n]).await?;
        transferred += n as u64;
        let _ = out_tx
            .send(SftpEvent::TransferProgress { transferred })
            .await;
    }
    remote_file.shutdown().await?;
    local_file.flush().await?;

    let _ = out_tx
        .send(SftpEvent::TransferFinished { error: None })
        .await;
    Ok(())
}
