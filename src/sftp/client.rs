use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::TryStreamExt as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use russh::client::Handle;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileType, OpenFlags};

use super::{SftpClient, SftpCommand, SftpEntry, SftpEvent, join_remote, safe_local_name};
use crate::proxy::ProxyConfig;
use crate::ssh_client::{self, SshCredentials};

pub(super) const CHUNK_SIZE: usize = 64 * 1024;
pub(super) const TRANSFER_BUFFER: usize = 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
// Bounds how long the transport thread lingers waiting for the disconnect to flush.
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const HEALTH_INTERVAL: Duration = Duration::from_secs(5);
const IDLE_PROBE_AFTER: Duration = Duration::from_secs(20);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const RECONNECT_ATTEMPTS: u32 = 8;
const RECONNECT_FIRST_DELAY: Duration = Duration::from_secs(1);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);
const WRITE_PACKET_LEN: u32 = 256 * 1024;
const MAX_IN_FLIGHT: usize = 64;
const PARALLEL_FILES: usize = 8;

pub(super) struct Progress {
    done: Cell<u64>,
    last: Cell<Instant>,
}

impl Progress {
    pub(super) fn new() -> Self {
        Self {
            done: Cell::new(0),
            last: Cell::new(Instant::now()),
        }
    }

    pub(super) async fn advance(&self, bytes: u64, out_tx: &async_channel::Sender<SftpEvent>) {
        let done = self.done.get() + bytes;
        self.done.set(done);
        let now = Instant::now();
        if now.duration_since(self.last.get()) >= PROGRESS_INTERVAL {
            self.last.set(now);
            let _ = out_tx
                .send(SftpEvent::TransferProgress { transferred: done })
                .await;
        }
    }
}

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

        let target = Target {
            host,
            port,
            credentials,
            proxy,
        };
        let result = runtime.block_on(run(target, initial_path, out_tx.clone(), cmd_rx));
        let _ = out_tx.send_blocking(SftpEvent::Closed(result.err().map(|e| e.to_string())));
    });

    SftpClient {
        events: out_rx,
        commands: cmd_tx,
    }
}

fn sftp_config() -> russh_sftp::client::Config {
    russh_sftp::client::Config {
        max_write_packet_len: WRITE_PACKET_LEN,
        max_concurrent_reads: MAX_IN_FLIGHT,
        max_concurrent_writes: MAX_IN_FLIGHT,
        ..Default::default()
    }
}

struct Target {
    host: String,
    port: u16,
    credentials: SshCredentials,
    proxy: Option<ProxyConfig>,
}

impl Target {
    async fn open(&self) -> anyhow::Result<Connection> {
        let session = ssh_client::connect(
            self.host.clone(),
            self.port,
            self.credentials.clone(),
            self.proxy.clone(),
        )
        .await?;

        let channel = session.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let sftp = SftpSession::new_with_config(channel.into_stream(), sftp_config()).await?;
        Ok(Connection {
            session,
            sftp: Rc::new(sftp),
        })
    }
}

struct Connection {
    session: Handle<ssh_client::Handler>,
    sftp: Rc<SftpSession>,
}

impl Connection {
    async fn answers(&self) -> bool {
        !self.session.is_closed()
            && matches!(
                tokio::time::timeout(PROBE_TIMEOUT, self.sftp.canonicalize(".")).await,
                Ok(Ok(_))
            )
    }

    async fn close(self) {
        drop(self.sftp);
        let _ = self
            .session
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
        let _ = tokio::time::timeout(DISCONNECT_TIMEOUT, self.session).await;
    }
}

pub(super) fn reconnect_delay(attempt: u32) -> Duration {
    (RECONNECT_FIRST_DELAY * 2u32.saturating_pow(attempt)).min(RECONNECT_MAX_DELAY)
}

async fn reconnect(
    target: &Target,
    current_dir: &RefCell<String>,
    out_tx: &async_channel::Sender<SftpEvent>,
    cmd_rx: &async_channel::Receiver<SftpCommand>,
) -> anyhow::Result<Connection> {
    let mut last_err = anyhow::anyhow!("the connection was lost");
    for attempt in 0..RECONNECT_ATTEMPTS {
        let _ = out_tx
            .send(SftpEvent::Reconnecting(format!(
                "Connection lost, reconnecting (attempt {} of {RECONNECT_ATTEMPTS})...",
                attempt + 1
            )))
            .await;
        match target.open().await {
            Ok(connection) => {
                let dir = current_dir.borrow().clone();
                list_and_send(&connection.sftp, dir, out_tx).await;
                return Ok(connection);
            }
            Err(err) => last_err = err,
        }
        if cmd_rx.is_closed() {
            break;
        }
        tokio::time::sleep(reconnect_delay(attempt)).await;
    }
    Err(anyhow::anyhow!(
        "Lost the connection to {}:{} and couldn't reconnect: {last_err}",
        target.host,
        target.port
    ))
}

struct Shared {
    current_dir: RefCell<String>,
    transferring: Cell<bool>,
    last_used: Cell<Instant>,
}

type Job = (Rc<SftpSession>, SftpCommand);

async fn run(
    target: Target,
    initial_path: String,
    out_tx: async_channel::Sender<SftpEvent>,
    cmd_rx: async_channel::Receiver<SftpCommand>,
) -> anyhow::Result<()> {
    let connection = target.open().await?;

    let current_dir = connection
        .sftp
        .canonicalize(initial_path.clone())
        .await
        .unwrap_or(initial_path);

    list_and_send(&connection.sftp, current_dir.clone(), &out_tx).await;

    let shared = Shared {
        current_dir: RefCell::new(current_dir),
        transferring: Cell::new(false),
        last_used: Cell::new(Instant::now()),
    };
    let (job_tx, job_rx) = async_channel::unbounded::<Job>();

    let commands = async {
        let result = command_loop(&target, connection, &shared, &out_tx, &cmd_rx, &job_tx).await;
        job_rx.close();
        result
    };
    let transfers = async {
        while let Ok((sftp, cmd)) = job_rx.recv().await {
            if job_rx.is_closed() {
                break;
            }
            shared.transferring.set(true);
            transfer(&sftp, cmd, &shared, &out_tx).await;
            shared.transferring.set(false);
            shared.last_used.set(Instant::now());
        }
    };
    let (result, ()) = tokio::join!(commands, transfers);
    result
}

async fn command_loop(
    target: &Target,
    mut connection: Connection,
    shared: &Shared,
    out_tx: &async_channel::Sender<SftpEvent>,
    cmd_rx: &async_channel::Receiver<SftpCommand>,
    job_tx: &async_channel::Sender<Job>,
) -> anyhow::Result<()> {
    let mut health = tokio::time::interval(HEALTH_INTERVAL);
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        let cmd = tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Ok(cmd) => cmd,
                Err(_) => break,
            },
            _ = health.tick() => {
                if connection.session.is_closed() {
                    connection = reconnect(target, &shared.current_dir, out_tx, cmd_rx).await?;
                    shared.last_used.set(Instant::now());
                }
                continue;
            }
        };

        let idle =
            !shared.transferring.get() && shared.last_used.get().elapsed() >= IDLE_PROBE_AFTER;
        if connection.session.is_closed() || (idle && !connection.answers().await) {
            connection = reconnect(target, &shared.current_dir, out_tx, cmd_rx).await?;
        }

        if is_transfer(&cmd) {
            let _ = job_tx.send((connection.sftp.clone(), cmd)).await;
        } else {
            execute(&connection.sftp, cmd, &shared.current_dir, out_tx).await;
        }
        shared.last_used.set(Instant::now());
    }

    // Panel is gone: close the subsystem and the session rather than leaving the
    // socket to die with this thread's runtime.
    connection.close().await;

    Ok(())
}

fn is_transfer(cmd: &SftpCommand) -> bool {
    matches!(
        cmd,
        SftpCommand::Upload { .. }
            | SftpCommand::Download { .. }
            | SftpCommand::Read { .. }
            | SftpCommand::Write { .. }
            | SftpCommand::UploadDir { .. }
            | SftpCommand::DownloadDir { .. }
    )
}

async fn execute(
    sftp: &SftpSession,
    cmd: SftpCommand,
    current_dir: &RefCell<String>,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    let dir = current_dir.borrow().clone();
    match cmd {
        SftpCommand::List { path } => {
            *current_dir.borrow_mut() = path.clone();
            list_and_send(sftp, path, out_tx).await;
        }
        SftpCommand::CreateDir { name } => {
            let path = join_remote(&dir, &name);
            if let Err(err) = sftp.create_dir(path).await {
                send_error(out_tx, format!("Couldn't create folder: {err}")).await;
            }
            list_and_send(sftp, dir, out_tx).await;
        }
        SftpCommand::CreateFile { name } => {
            let path = join_remote(&dir, &name);
            let flags = OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE;
            match sftp.open_with_flags(path, flags).await {
                Ok(mut file) => {
                    let _ = file.shutdown().await;
                }
                Err(err) => send_error(out_tx, format!("Couldn't create file: {err}")).await,
            }
            list_and_send(sftp, dir, out_tx).await;
        }
        SftpCommand::Rename { from, to } => {
            if let Err(err) = sftp.rename(from, to).await {
                send_error(out_tx, format!("Couldn't rename: {err}")).await;
            }
            list_and_send(sftp, dir, out_tx).await;
        }
        SftpCommand::Move { from, to } => {
            if sftp.symlink_metadata(to.clone()).await.is_ok() {
                let name = to.rsplit('/').next().unwrap_or(&to);
                send_error(
                    out_tx,
                    format!(
                        "Couldn't move: \"{}\" already exists there",
                        super::display_name(name)
                    ),
                )
                .await;
                return;
            }
            match sftp.rename(from, to).await {
                Ok(()) => list_and_send(sftp, dir, out_tx).await,
                Err(err) => send_error(out_tx, format!("Couldn't move: {err}")).await,
            }
        }
        SftpCommand::RemoveFile { path } => {
            if let Err(err) = sftp.remove_file(path).await {
                send_error(out_tx, format!("Couldn't delete file: {err}")).await;
            }
            list_and_send(sftp, dir, out_tx).await;
        }
        SftpCommand::RemoveDir { path } => {
            if let Err(err) = sftp.remove_dir(path).await {
                send_error(out_tx, format!("Couldn't delete folder: {err}")).await;
            }
            list_and_send(sftp, dir, out_tx).await;
        }
        _ => {}
    }
}

async fn transfer(
    sftp: &SftpSession,
    cmd: SftpCommand,
    shared: &Shared,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    match cmd {
        SftpCommand::Upload { local, remote } => {
            do_upload(sftp, &local, &remote, out_tx).await;
            refresh_current(sftp, &shared.current_dir, out_tx).await;
        }
        SftpCommand::Download {
            remote,
            local,
            open_when_done,
        } => {
            if do_download(sftp, &remote, &local, out_tx).await
                && open_when_done
                && let Err(err) = open::that_detached(&local)
            {
                send_error(out_tx, format!("Couldn't open {}: {err}", local.display())).await;
            }
        }
        SftpCommand::Read { remote } => {
            let result = do_read(sftp, &remote, out_tx).await;
            let _ = out_tx.send(SftpEvent::FileLoaded { remote, result }).await;
        }
        SftpCommand::Write { remote, bytes, ack } => {
            do_write(sftp, &remote, &bytes, &ack, out_tx).await;
            refresh_current(sftp, &shared.current_dir, out_tx).await;
        }
        SftpCommand::UploadDir { local, remote } => {
            do_upload_dir(sftp, &local, &remote, out_tx).await;
            refresh_current(sftp, &shared.current_dir, out_tx).await;
        }
        SftpCommand::DownloadDir { remote, local } => {
            do_download_dir(sftp, &remote, &local, out_tx).await;
        }
        _ => {}
    }
}

pub(super) async fn send_error(out_tx: &async_channel::Sender<SftpEvent>, message: String) {
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

async fn refresh_current(
    sftp: &SftpSession,
    current_dir: &RefCell<String>,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    let path = current_dir.borrow().clone();
    if let Ok(entries) = read_dir(sftp, &path).await
        && *current_dir.borrow() == path
    {
        let _ = out_tx.send(SftpEvent::Listing { path, entries }).await;
    }
}

async fn read_dir(sftp: &SftpSession, path: &str) -> anyhow::Result<Vec<SftpEntry>> {
    let mut entries = read_dir_unsorted(sftp, path).await?;
    super::sort_entries(&mut entries);
    entries.shrink_to_fit();
    Ok(entries)
}

async fn read_dir_unsorted(sftp: &SftpSession, path: &str) -> anyhow::Result<Vec<SftpEntry>> {
    let read_dir = sftp.read_dir(path).await?;
    Ok(read_dir
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
        .collect())
}

pub(super) fn transfer_label(path: &std::path::Path, fallback: &str) -> String {
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
    progress: &Progress,
) -> anyhow::Result<()> {
    let mut local_file = tokio::fs::File::open(local).await?;
    let mut remote_file = sftp.create(remote).await?;
    let mut buf = vec![0u8; TRANSFER_BUFFER];
    loop {
        let n = local_file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        remote_file.write_all(&buf[..n]).await?;
        progress.advance(n as u64, out_tx).await;
    }
    remote_file.shutdown().await?;
    Ok(())
}

pub(super) async fn create_new(local: &std::path::Path) -> std::io::Result<tokio::fs::File> {
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
    progress: &Progress,
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
    let mut buf = vec![0u8; TRANSFER_BUFFER];
    loop {
        let n = remote_file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        local_file.write_all(&buf[..n]).await?;
        progress.advance(n as u64, out_tx).await;
    }
    remote_file.shutdown().await?;
    local_file.flush().await?;
    Ok(())
}

pub(super) async fn finish_transfer(
    out_tx: &async_channel::Sender<SftpEvent>,
    result: anyhow::Result<()>,
    local_changed: bool,
) -> bool {
    let ok = result.is_ok();
    let _ = out_tx
        .send(SftpEvent::TransferFinished {
            error: result.err().map(|e| e.to_string()),
            local_changed,
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

    let result = copy_up(sftp, local, remote, out_tx, &Progress::new()).await;
    finish_transfer(out_tx, result, false).await
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

    let result = copy_down(sftp, remote, local, out_tx, &Progress::new()).await;
    finish_transfer(out_tx, result, true).await
}

pub(super) const MAX_READ: u64 = 10 * 1024 * 1024;

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
        let progress = Progress::new();
        loop {
            let n = file.read_buf(&mut buf).await?;
            if n == 0 {
                break;
            }
            if buf.len() as u64 > MAX_READ {
                anyhow::bail!("the file grew past the size limit while reading");
            }
            progress.advance(n as u64, out_tx).await;
        }
        file.shutdown().await?;
        anyhow::Ok(buf)
    }
    .await;

    let _ = out_tx
        .send(SftpEvent::TransferFinished {
            error: None,
            local_changed: false,
        })
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
        let progress = Progress::new();
        for chunk in bytes.chunks(TRANSFER_BUFFER) {
            file.write_all(chunk).await?;
            progress.advance(chunk.len() as u64, out_tx).await;
        }
        file.shutdown().await?;
        anyhow::Ok(())
    }
    .await;

    let error = result.err().map(|err| err.to_string());
    let _ = out_tx
        .send(SftpEvent::TransferFinished {
            error: error.clone(),
            local_changed: false,
        })
        .await;
    let _ = ack.send(error).await;
}

pub(super) struct PlannedFile<L, R> {
    pub(super) source: L,
    pub(super) destination: R,
    pub(super) size: u64,
}

pub(super) async fn plan_upload(
    local_root: &std::path::Path,
    remote_root: &str,
) -> anyhow::Result<(Vec<String>, Vec<PlannedFile<std::path::PathBuf, String>>)> {
    let local_root = local_root.to_path_buf();
    let remote_root = remote_root.to_string();
    tokio::task::spawn_blocking(move || plan_upload_blocking(&local_root, &remote_root)).await?
}

fn plan_upload_blocking(
    local_root: &std::path::Path,
    remote_root: &str,
) -> anyhow::Result<(Vec<String>, Vec<PlannedFile<std::path::PathBuf, String>>)> {
    let mut dirs = vec![remote_root.to_string()];
    let mut files = Vec::new();
    let mut pending = vec![(local_root.to_path_buf(), remote_root.to_string())];

    while let Some((local_dir, remote_dir)) = pending.pop() {
        for entry in std::fs::read_dir(&local_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let remote_path = join_remote(&remote_dir, &name);
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                dirs.push(remote_path.clone());
                pending.push((entry.path(), remote_path));
            } else if file_type.is_file() {
                files.push(PlannedFile {
                    size: entry.metadata().map(|m| m.len()).unwrap_or(0),
                    source: entry.path(),
                    destination: remote_path,
                });
            }
        }
    }

    Ok((dirs, files))
}

pub(super) fn dirs_by_depth(dirs: &[String]) -> Vec<Vec<&String>> {
    let mut levels: Vec<Vec<&String>> = Vec::new();
    for dir in dirs {
        let depth = dir.matches('/').count();
        if levels.len() <= depth {
            levels.resize_with(depth + 1, Vec::new);
        }
        levels[depth].push(dir);
    }
    levels.retain(|level| !level.is_empty());
    levels
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
        for entry in read_dir_unsorted(sftp, &remote_dir).await? {
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
        for level in dirs_by_depth(&dirs) {
            futures::future::join_all(level.into_iter().map(|dir| sftp.create_dir(dir.clone())))
                .await;
        }
        let progress = Progress::new();
        futures::stream::iter(files.iter().map(anyhow::Ok))
            .try_for_each_concurrent(PARALLEL_FILES, |file| {
                copy_up(sftp, &file.source, &file.destination, out_tx, &progress)
            })
            .await
    }
    .await;

    finish_transfer(out_tx, result, false).await;
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
        tokio::task::spawn_blocking(move || dirs.iter().try_for_each(std::fs::create_dir_all))
            .await??;
        let progress = Progress::new();
        futures::stream::iter(files.iter().map(anyhow::Ok))
            .try_for_each_concurrent(PARALLEL_FILES, |file| {
                copy_down(sftp, &file.source, &file.destination, out_tx, &progress)
            })
            .await
    }
    .await;

    finish_transfer(out_tx, result, true).await;
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
    fn reconnect_backoff_doubles_up_to_a_ceiling() {
        let delays: Vec<u64> = (0..RECONNECT_ATTEMPTS)
            .map(|attempt| reconnect_delay(attempt).as_secs())
            .collect();
        assert_eq!(delays, vec![1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(reconnect_delay(u32::MAX), RECONNECT_MAX_DELAY);
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
