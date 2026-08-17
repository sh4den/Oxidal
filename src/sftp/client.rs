use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileType;

use super::{SftpClient, SftpCommand, SftpEntry, SftpEvent, join_remote, safe_local_name};
use crate::proxy::ProxyConfig;
use crate::ssh_client::{self, SshCredentials};

const CHUNK_SIZE: usize = 64 * 1024;
// Bounds how long the transport thread lingers waiting for the disconnect to flush.
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(2);

pub fn spawn(
    host: String,
    port: u16,
    credentials: SshCredentials,
    proxy: Option<ProxyConfig>,
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
            credentials,
            proxy,
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
    credentials: SshCredentials,
    proxy: Option<ProxyConfig>,
    initial_path: String,
    out_tx: async_channel::Sender<SftpEvent>,
    cmd_rx: async_channel::Receiver<SftpCommand>,
) -> anyhow::Result<()> {
    let session = ssh_client::connect(host, port, credentials, proxy).await?;

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
                do_upload(&sftp, &local, &remote, &out_tx).await;
                list_and_send(&sftp, current_dir.clone(), &out_tx).await;
            }
            SftpCommand::Download {
                remote,
                local,
                open_when_done,
            } => {
                if do_download(&sftp, &remote, &local, &out_tx).await
                    && open_when_done
                    && let Err(err) = open::that_detached(&local)
                {
                    send_error(&out_tx, format!("Couldn't open {}: {err}", local.display())).await;
                }
            }
            SftpCommand::Read { remote } => {
                let result = do_read(&sftp, &remote, &out_tx).await;
                let _ = out_tx.send(SftpEvent::FileLoaded { remote, result }).await;
            }
            SftpCommand::Write { remote, bytes, ack } => {
                do_write(&sftp, &remote, &bytes, &ack, &out_tx).await;
                list_and_send(&sftp, current_dir.clone(), &out_tx).await;
            }
            SftpCommand::UploadDir { local, remote } => {
                do_upload_dir(&sftp, &local, &remote, &out_tx).await;
                list_and_send(&sftp, current_dir.clone(), &out_tx).await;
            }
            SftpCommand::DownloadDir { remote, local } => {
                do_download_dir(&sftp, &remote, &local, &out_tx).await;
            }
        }
    }

    // Panel is gone: close the subsystem and the session rather than leaving the
    // socket to die with this thread's runtime.
    drop(sftp);
    let _ = session
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    let _ = tokio::time::timeout(DISCONNECT_TIMEOUT, session).await;

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
                is_symlink: matches!(entry.file_type(), FileType::Symlink),
                size: metadata.len(),
                modified: metadata.mtime.map(|t| t as u64),
                accessed: metadata.atime.map(|t| t as u64),
                permissions: metadata.permissions,
                owner: metadata
                    .user
                    .clone()
                    .or_else(|| metadata.uid.map(|uid| uid.to_string())),
                group: metadata
                    .group
                    .clone()
                    .or_else(|| metadata.gid.map(|gid| gid.to_string())),
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries.shrink_to_fit();

    Ok(entries)
}

fn transfer_label(path: &std::path::Path, fallback: &str) -> String {
    let raw = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| fallback.to_string());
    super::display_name(&raw)
}

async fn copy_up(
    sftp: &SftpSession,
    local: &std::path::Path,
    remote: &str,
    out_tx: &async_channel::Sender<SftpEvent>,
    done: &mut u64,
) -> anyhow::Result<()> {
    let mut local_file = tokio::fs::File::open(local).await?;
    let mut remote_file = sftp.create(remote).await?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = local_file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        remote_file.write_all(&buf[..n]).await?;
        *done += n as u64;
        let _ = out_tx
            .send(SftpEvent::TransferProgress { transferred: *done })
            .await;
    }
    remote_file.shutdown().await?;
    Ok(())
}

async fn create_new(local: &std::path::Path) -> std::io::Result<tokio::fs::File> {
    tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(local)
        .await
}

async fn copy_down(
    sftp: &SftpSession,
    remote: &str,
    local: &std::path::Path,
    out_tx: &async_channel::Sender<SftpEvent>,
    done: &mut u64,
) -> anyhow::Result<()> {
    let mut remote_file = sftp.open(remote).await?;
    let mut local_file = create_new(local).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow::anyhow!(
                "{} already exists, so nothing was written over it",
                local.display()
            )
        } else {
            anyhow::anyhow!("couldn't create {}: {err}", local.display())
        }
    })?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = remote_file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        local_file.write_all(&buf[..n]).await?;
        *done += n as u64;
        let _ = out_tx
            .send(SftpEvent::TransferProgress { transferred: *done })
            .await;
    }
    remote_file.shutdown().await?;
    local_file.flush().await?;
    Ok(())
}

async fn finish_transfer(
    out_tx: &async_channel::Sender<SftpEvent>,
    result: anyhow::Result<()>,
) -> bool {
    let ok = result.is_ok();
    let _ = out_tx
        .send(SftpEvent::TransferFinished {
            error: result.err().map(|e| e.to_string()),
        })
        .await;
    ok
}

async fn do_upload(
    sftp: &SftpSession,
    local: &std::path::Path,
    remote: &str,
    out_tx: &async_channel::Sender<SftpEvent>,
) -> bool {
    let total = match tokio::fs::metadata(local).await {
        Ok(metadata) => metadata.len(),
        Err(err) => {
            send_error(out_tx, format!("Couldn't read {}: {err}", local.display())).await;
            return false;
        }
    };

    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label: transfer_label(local, remote),
            total: Some(total),
        })
        .await;

    let mut done = 0u64;
    let result = copy_up(sftp, local, remote, out_tx, &mut done).await;
    finish_transfer(out_tx, result).await
}

async fn do_download(
    sftp: &SftpSession,
    remote: &str,
    local: &std::path::Path,
    out_tx: &async_channel::Sender<SftpEvent>,
) -> bool {
    let total = match sftp.metadata(remote).await {
        Ok(metadata) => metadata.len(),
        Err(err) => {
            send_error(out_tx, format!("Couldn't read {remote}: {err}")).await;
            return false;
        }
    };

    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label: transfer_label(local, remote),
            total: Some(total),
        })
        .await;

    let mut done = 0u64;
    let result = copy_down(sftp, remote, local, out_tx, &mut done).await;
    finish_transfer(out_tx, result).await
}

const MAX_READ: u64 = 10 * 1024 * 1024;

async fn do_read(
    sftp: &SftpSession,
    remote: &str,
    out_tx: &async_channel::Sender<SftpEvent>,
) -> Result<Vec<u8>, String> {
    let total = match sftp.metadata(remote).await {
        Ok(metadata) => metadata.len(),
        Err(err) => return Err(format!("Couldn't read {remote}: {err}")),
    };
    if total > MAX_READ {
        return Err(format!(
            "{remote} is {}, too large to open here",
            super::format_size(total)
        ));
    }

    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label: transfer_label(std::path::Path::new(remote), remote),
            total: Some(total),
        })
        .await;

    let result = async {
        let mut file = sftp.open(remote).await?;
        let mut buf = Vec::with_capacity(total as usize);
        let mut chunk = vec![0u8; CHUNK_SIZE];
        loop {
            let n = file.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.len() as u64 > MAX_READ {
                anyhow::bail!("the file grew past the size limit while reading");
            }
            let _ = out_tx
                .send(SftpEvent::TransferProgress {
                    transferred: buf.len() as u64,
                })
                .await;
        }
        file.shutdown().await?;
        anyhow::Ok(buf)
    }
    .await;

    let _ = out_tx
        .send(SftpEvent::TransferFinished { error: None })
        .await;
    result.map_err(|err| format!("Couldn't read {remote}: {err}"))
}

async fn do_write(
    sftp: &SftpSession,
    remote: &str,
    bytes: &[u8],
    ack: &async_channel::Sender<Option<String>>,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label: transfer_label(std::path::Path::new(remote), remote),
            total: Some(bytes.len() as u64),
        })
        .await;

    let result = async {
        let mut file = sftp.create(remote).await?;
        let mut done = 0u64;
        for chunk in bytes.chunks(CHUNK_SIZE) {
            file.write_all(chunk).await?;
            done += chunk.len() as u64;
            let _ = out_tx
                .send(SftpEvent::TransferProgress { transferred: done })
                .await;
        }
        file.shutdown().await?;
        anyhow::Ok(())
    }
    .await;

    let error = result.err().map(|err| err.to_string());
    let _ = out_tx
        .send(SftpEvent::TransferFinished {
            error: error.clone(),
        })
        .await;
    let _ = ack.send(error).await;
}

struct PlannedFile<L, R> {
    source: L,
    destination: R,
    size: u64,
}

async fn plan_upload(
    local_root: &std::path::Path,
    remote_root: &str,
) -> anyhow::Result<(Vec<String>, Vec<PlannedFile<std::path::PathBuf, String>>)> {
    let mut dirs = vec![remote_root.to_string()];
    let mut files = Vec::new();
    let mut pending = vec![(local_root.to_path_buf(), remote_root.to_string())];

    while let Some((local_dir, remote_dir)) = pending.pop() {
        let mut reader = tokio::fs::read_dir(&local_dir).await?;
        while let Some(entry) = reader.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            let remote_path = join_remote(&remote_dir, &name);
            let file_type = entry.file_type().await?;
            if file_type.is_dir() {
                dirs.push(remote_path.clone());
                pending.push((entry.path(), remote_path));
            } else if file_type.is_file() {
                files.push(PlannedFile {
                    size: entry.metadata().await.map(|m| m.len()).unwrap_or(0),
                    source: entry.path(),
                    destination: remote_path,
                });
            }
        }
    }

    Ok((dirs, files))
}

async fn plan_download(
    sftp: &SftpSession,
    remote_root: &str,
    local_root: &std::path::Path,
) -> anyhow::Result<(
    Vec<std::path::PathBuf>,
    Vec<PlannedFile<String, std::path::PathBuf>>,
)> {
    let mut dirs = vec![local_root.to_path_buf()];
    let mut files = Vec::new();
    let mut pending = vec![(remote_root.to_string(), local_root.to_path_buf())];

    while let Some((remote_dir, local_dir)) = pending.pop() {
        for entry in read_dir(sftp, &remote_dir).await? {
            let local_path = local_dir.join(safe_local_name(&entry.name));
            if entry.is_dir {
                dirs.push(local_path.clone());
                pending.push((entry.path, local_path));
            } else if !entry.is_symlink {
                files.push(PlannedFile {
                    source: entry.path,
                    destination: local_path,
                    size: entry.size,
                });
            }
        }
    }

    Ok((dirs, files))
}

async fn do_upload_dir(
    sftp: &SftpSession,
    local: &std::path::Path,
    remote: &str,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    let (dirs, files) = match plan_upload(local, remote).await {
        Ok(plan) => plan,
        Err(err) => {
            send_error(out_tx, format!("Couldn't read {}: {err}", local.display())).await;
            return;
        }
    };

    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label: transfer_label(local, remote),
            total: Some(files.iter().map(|file| file.size).sum()),
        })
        .await;

    let result = async {
        for dir in &dirs {
            let _ = sftp.create_dir(dir.clone()).await;
        }
        let mut done = 0u64;
        for file in &files {
            copy_up(sftp, &file.source, &file.destination, out_tx, &mut done).await?;
        }
        anyhow::Ok(())
    }
    .await;

    finish_transfer(out_tx, result).await;
}

async fn do_download_dir(
    sftp: &SftpSession,
    remote: &str,
    local: &std::path::Path,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    let (dirs, files) = match plan_download(sftp, remote, local).await {
        Ok(plan) => plan,
        Err(err) => {
            send_error(out_tx, format!("Couldn't read {remote}: {err}")).await;
            return;
        }
    };

    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label: transfer_label(local, remote),
            total: Some(files.iter().map(|file| file.size).sum()),
        })
        .await;

    let result = async {
        for dir in &dirs {
            tokio::fs::create_dir_all(dir).await?;
        }
        let mut done = 0u64;
        for file in &files {
            copy_down(sftp, &file.source, &file.destination, out_tx, &mut done).await?;
        }
        anyhow::Ok(())
    }
    .await;

    finish_transfer(out_tx, result).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }

    #[test]
    fn planning_an_upload_walks_the_whole_tree() {
        let base = std::env::temp_dir().join(format!("oxidal-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("nested").join("deeper")).expect("dirs");
        std::fs::write(base.join("top.txt"), b"12345").expect("file");
        std::fs::write(base.join("nested").join("mid.txt"), b"12").expect("file");
        std::fs::write(base.join("nested").join("deeper").join("leaf.bin"), b"123").expect("file");

        let (mut dirs, files) = block_on(plan_upload(&base, "/srv/dest")).expect("plan");
        dirs.sort();

        assert_eq!(
            dirs,
            vec![
                "/srv/dest".to_string(),
                "/srv/dest/nested".to_string(),
                "/srv/dest/nested/deeper".to_string(),
            ],
            "every directory should be created remotely, parents before children once sorted"
        );

        let mut destinations: Vec<&str> =
            files.iter().map(|file| file.destination.as_str()).collect();
        destinations.sort();
        assert_eq!(
            destinations,
            vec![
                "/srv/dest/nested/deeper/leaf.bin",
                "/srv/dest/nested/mid.txt",
                "/srv/dest/top.txt",
            ],
            "remote destinations stay slash separated whatever the local separator is"
        );

        assert_eq!(
            files.iter().map(|file| file.size).sum::<u64>(),
            10,
            "the progress total should cover every file in the tree"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn planning_an_upload_of_an_empty_directory_still_creates_it() {
        let base = std::env::temp_dir().join(format!("oxidal-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("dir");

        let (dirs, files) = block_on(plan_upload(&base, "/srv/empty")).expect("plan");

        assert_eq!(dirs, vec!["/srv/empty".to_string()]);
        assert!(files.is_empty());

        let _ = std::fs::remove_dir_all(&base);
    }
}
