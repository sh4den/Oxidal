use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::TryStreamExt as _;
use russh::ChannelMsg;
use russh::client::Handle;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use super::client::{
    DISCONNECT_TIMEOUT, HEALTH_INTERVAL, IDLE_PROBE_AFTER, MAX_READ, PROBE_TIMEOUT, PlannedFile,
    Progress, RECONNECT_ATTEMPTS, Shared, TRANSFER_BUFFER, Target, create_new, finish_transfer,
    plan_upload, reconnect_delay, send_error, transfer_label,
};
use super::{SftpCommand, SftpEntry, SftpEvent, join_remote, parent_remote, safe_local_name};
use crate::ssh_client::Handler;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_OUTPUT: usize = 64 * 1024 * 1024;
const MAX_HEADER: usize = 4096;
const MAX_SCRIPT: usize = 64 * 1024;
// OpenSSH allows 10 sessions per connection by default; leave room for listings.
const PARALLEL_FILES: usize = 4;
const EXISTS_STATUS: u32 = 3;
const NO_SCP: &str = "the server closed the transfer without answering. Is scp installed there?";

type Session = Rc<Handle<Handler>>;
type Job = (Session, SftpCommand);

pub(super) async fn run(
    target: Target,
    session: Handle<Handler>,
    initial_path: String,
    out_tx: async_channel::Sender<SftpEvent>,
    cmd_rx: async_channel::Receiver<SftpCommand>,
) -> anyhow::Result<()> {
    let session = Rc::new(session);
    let current_dir = match list(&session, &initial_path).await {
        Ok((path, entries)) => {
            let _ = out_tx
                .send(SftpEvent::Listing {
                    path: path.clone(),
                    entries,
                })
                .await;
            path
        }
        Err(err) => {
            send_error(&out_tx, format!("Couldn't list {initial_path}: {err}")).await;
            initial_path
        }
    };

    let shared = Shared {
        current_dir: current_dir.into(),
        transferring: false.into(),
        last_used: Instant::now().into(),
    };
    let (job_tx, job_rx) = async_channel::unbounded::<Job>();

    let commands = async {
        let result = command_loop(&target, session, &shared, &out_tx, &cmd_rx, &job_tx).await;
        job_rx.close();
        result
    };
    let transfers = async {
        while let Ok((session, cmd)) = job_rx.recv().await {
            if job_rx.is_closed() {
                break;
            }
            shared.transferring.set(true);
            transfer(&session, cmd, &shared, &out_tx).await;
            shared.transferring.set(false);
            shared.last_used.set(Instant::now());
        }
    };
    let (result, ()) = tokio::join!(commands, transfers);
    result
}

async fn command_loop(
    target: &Target,
    mut session: Session,
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
                if session.is_closed() {
                    session = reconnect(target, shared, out_tx, cmd_rx).await?;
                    shared.last_used.set(Instant::now());
                }
                continue;
            }
        };

        let idle =
            !shared.transferring.get() && shared.last_used.get().elapsed() >= IDLE_PROBE_AFTER;
        if session.is_closed() || (idle && !answers(&session).await) {
            session = reconnect(target, shared, out_tx, cmd_rx).await?;
        }

        if is_transfer(&cmd) {
            let _ = job_tx.send((session.clone(), cmd)).await;
        } else {
            execute(&session, cmd, shared, out_tx).await;
        }
        shared.last_used.set(Instant::now());
    }

    let _ = session
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    if let Ok(handle) = Rc::try_unwrap(session) {
        let _ = tokio::time::timeout(DISCONNECT_TIMEOUT, handle).await;
    }
    Ok(())
}

async fn answers(session: &Handle<Handler>) -> bool {
    matches!(
        tokio::time::timeout(PROBE_TIMEOUT, run_script(session, "true")).await,
        Ok(Ok(_))
    )
}

async fn reconnect(
    target: &Target,
    shared: &Shared,
    out_tx: &async_channel::Sender<SftpEvent>,
    cmd_rx: &async_channel::Receiver<SftpCommand>,
) -> anyhow::Result<Session> {
    let mut last_err = anyhow::anyhow!("the connection was lost");
    for attempt in 0..RECONNECT_ATTEMPTS {
        let _ = out_tx
            .send(SftpEvent::Reconnecting(format!(
                "Connection lost, reconnecting (attempt {} of {RECONNECT_ATTEMPTS})...",
                attempt + 1
            )))
            .await;
        match target.ssh().await {
            Ok(session) => {
                let session = Rc::new(session);
                let dir = shared.current_dir.borrow().clone();
                list_and_send(&session, dir, out_tx).await;
                return Ok(session);
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
    session: &Handle<Handler>,
    cmd: SftpCommand,
    shared: &Shared,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    let dir = shared.current_dir.borrow().clone();
    let (script, failure) = match cmd {
        SftpCommand::List { path } => {
            *shared.current_dir.borrow_mut() = path.clone();
            list_and_send(session, path, out_tx).await;
            return;
        }
        SftpCommand::CreateDir { name } => (
            format!("mkdir -- {}", quote(&join_remote(&dir, &name))),
            "Couldn't create folder",
        ),
        SftpCommand::CreateFile { name } => (
            format!("set -C && : > {}", quote(&join_remote(&dir, &name))),
            "Couldn't create file",
        ),
        SftpCommand::Rename { from, to } => (move_script(&from, &to), "Couldn't rename"),
        SftpCommand::Move { from, to } => {
            match exec(session, &move_script(&from, &to)).await {
                Ok(output) if output.status == Some(EXISTS_STATUS) => {
                    let name = to.rsplit('/').next().unwrap_or(&to);
                    send_error(
                        out_tx,
                        format!(
                            "Couldn't move: \"{}\" already exists there",
                            super::display_name(name)
                        ),
                    )
                    .await;
                }
                Ok(output) if output.status == Some(0) => {
                    list_and_send(session, dir, out_tx).await;
                }
                Ok(output) => {
                    send_error(
                        out_tx,
                        format!("Couldn't move: {}", failure_message(&output)),
                    )
                    .await;
                }
                Err(err) => send_error(out_tx, format!("Couldn't move: {err}")).await,
            }
            return;
        }
        SftpCommand::RemoveFile { path } => {
            (format!("rm -- {}", quote(&path)), "Couldn't delete file")
        }
        SftpCommand::RemoveDir { path } => (
            format!("rmdir -- {}", quote(&path)),
            "Couldn't delete folder",
        ),
        _ => return,
    };
    if let Err(err) = run_script(session, &script).await {
        send_error(out_tx, format!("{failure}: {err}")).await;
    }
    list_and_send(session, dir, out_tx).await;
}

fn move_script(from: &str, to: &str) -> String {
    let to = quote(to);
    format!(
        "if [ -e {to} ] || [ -L {to} ]; then echo 'it already exists' >&2; exit {EXISTS_STATUS}; \
         fi; mv -- {} {to}",
        quote(from)
    )
}

async fn transfer(
    session: &Handle<Handler>,
    cmd: SftpCommand,
    shared: &Shared,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    match cmd {
        SftpCommand::Upload { local, remote } => {
            do_upload(session, &local, &remote, out_tx).await;
            refresh_current(session, shared, out_tx).await;
        }
        SftpCommand::Download {
            remote,
            local,
            open_when_done,
        } => {
            if do_download(session, &remote, &local, out_tx).await
                && open_when_done
                && let Err(err) = open::that_detached(&local)
            {
                send_error(out_tx, format!("Couldn't open {}: {err}", local.display())).await;
            }
        }
        SftpCommand::Read { remote } => {
            let result = do_read(session, &remote, out_tx).await;
            let _ = out_tx.send(SftpEvent::FileLoaded { remote, result }).await;
        }
        SftpCommand::Write { remote, bytes, ack } => {
            do_write(session, &remote, &bytes, &ack, out_tx).await;
            refresh_current(session, shared, out_tx).await;
        }
        SftpCommand::UploadDir { local, remote } => {
            do_upload_dir(session, &local, &remote, out_tx).await;
            refresh_current(session, shared, out_tx).await;
        }
        SftpCommand::DownloadDir { remote, local } => {
            do_download_dir(session, &remote, &local, out_tx).await;
        }
        _ => {}
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[derive(Default)]
struct Output {
    status: Option<u32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn append(buffer: &mut Vec<u8>, data: &[u8]) -> anyhow::Result<()> {
    if buffer.len() + data.len() > MAX_OUTPUT {
        anyhow::bail!("the server sent more output than expected");
    }
    buffer.extend_from_slice(data);
    Ok(())
}

async fn exec(session: &Handle<Handler>, script: &str) -> anyhow::Result<Output> {
    let command = format!("sh -c {}", quote(script));
    let run = async {
        let mut channel = session.channel_open_session().await?;
        channel.exec(true, command).await?;
        let mut output = Output::default();
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => append(&mut output.stdout, &data)?,
                ChannelMsg::ExtendedData { data, ext: 1 } => append(&mut output.stderr, &data)?,
                ChannelMsg::ExitStatus { exit_status } => output.status = Some(exit_status),
                ChannelMsg::Failure => anyhow::bail!("the server refused to run a command"),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        anyhow::Ok(output)
    };
    tokio::time::timeout(COMMAND_TIMEOUT, run)
        .await
        .map_err(|_| anyhow::anyhow!("the server took too long to answer"))?
}

fn failure_message(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    match stderr.lines().map(str::trim).rfind(|line| !line.is_empty()) {
        Some(line) => line.to_string(),
        None => match output.status {
            Some(status) => format!("the command failed with status {status}"),
            None => "the command ended without reporting a status".to_string(),
        },
    }
}

async fn run_script(session: &Handle<Handler>, script: &str) -> anyhow::Result<Vec<u8>> {
    let output = exec(session, script).await?;
    if output.status == Some(0) {
        Ok(output.stdout)
    } else {
        anyhow::bail!(failure_message(&output))
    }
}

async fn list(session: &Handle<Handler>, path: &str) -> anyhow::Result<(String, Vec<SftpEntry>)> {
    let script = format!("cd -- {} && pwd && TZ=UTC0 LC_ALL=C ls -lA", quote(path));
    let stdout = run_script(session, &script).await?;
    let (dir, mut entries) = parse_listing(&stdout, chrono::Utc::now())?;
    super::sort_entries(&mut entries);
    entries.shrink_to_fit();
    Ok((dir, entries))
}

fn parse_listing(
    raw: &[u8],
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<(String, Vec<SftpEntry>)> {
    let text = String::from_utf8_lossy(raw);
    let mut lines = text.lines().map(|line| line.trim_end_matches('\r'));
    let dir = lines
        .next()
        .map(str::to_string)
        .filter(|dir| dir.starts_with('/'))
        .ok_or_else(|| anyhow::anyhow!("the server didn't report the folder it listed"))?;
    let entries = lines
        .filter_map(|line| super::ftp::parse_list_line(line, now))
        .filter(|entry| entry.name != "." && entry.name != "..")
        .map(|mut entry| {
            entry.path = join_remote(&dir, &entry.name);
            entry
        })
        .collect();
    Ok((dir, entries))
}

async fn list_and_send(
    session: &Handle<Handler>,
    path: String,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    match list(session, &path).await {
        Ok((path, entries)) => {
            let _ = out_tx.send(SftpEvent::Listing { path, entries }).await;
        }
        Err(err) => send_error(out_tx, format!("Couldn't list {path}: {err}")).await,
    }
}

async fn refresh_current(
    session: &Handle<Handler>,
    shared: &Shared,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    let path = shared.current_dir.borrow().clone();
    if let Ok((listed, entries)) = list(session, &path).await
        && *shared.current_dir.borrow() == path
    {
        let _ = out_tx
            .send(SftpEvent::Listing {
                path: listed,
                entries,
            })
            .await;
    }
}

fn split_target(remote: &str) -> anyhow::Result<(String, &str)> {
    let name = remote
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    if name.is_empty() || name == "." || name == ".." || name.contains('\n') {
        anyhow::bail!("\"{remote}\" isn't a file name scp can write");
    }
    Ok((parent_remote(remote), name))
}

async fn read_line<R: AsyncRead + Unpin>(reader: &mut R) -> anyhow::Result<String> {
    let mut line = Vec::new();
    loop {
        let byte = reader
            .read_u8()
            .await
            .map_err(|_| anyhow::anyhow!(NO_SCP))?;
        if byte == b'\n' {
            break;
        }
        if line.len() >= MAX_HEADER {
            anyhow::bail!("scp sent a header that is too long");
        }
        line.push(byte);
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

async fn read_ack<R: AsyncRead + Unpin>(reader: &mut R) -> anyhow::Result<()> {
    let byte = reader
        .read_u8()
        .await
        .map_err(|_| anyhow::anyhow!(NO_SCP))?;
    match byte {
        0 => Ok(()),
        1 | 2 => anyhow::bail!(remote_message(&read_line(reader).await?)),
        other => anyhow::bail!("scp sent an unexpected reply ({other:#04x})"),
    }
}

fn remote_message(line: &str) -> String {
    let message = line.trim().trim_start_matches("scp: ").trim();
    if message.is_empty() {
        "scp reported an error".to_string()
    } else {
        message.to_string()
    }
}

fn parse_file_header(line: &str) -> anyhow::Result<u64> {
    let invalid = || anyhow::anyhow!("scp sent a header Oxidal doesn't understand");
    let mut parts = line.splitn(3, ' ');
    let mode = parts.next().ok_or_else(invalid)?;
    let size = parts.next().ok_or_else(invalid)?;
    let name = parts.next().ok_or_else(invalid)?;
    if mode.len() != 4 || u32::from_str_radix(mode, 8).is_err() || name.is_empty() {
        return Err(invalid());
    }
    size.parse().map_err(|_| invalid())
}

async fn pump<R, W>(
    reader: &mut R,
    writer: &mut W,
    size: u64,
    out_tx: &async_channel::Sender<SftpEvent>,
    progress: &Progress,
) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![
        0u8;
        TRANSFER_BUFFER
            .min(usize::try_from(size).unwrap_or(usize::MAX))
            .max(1)
    ];
    let mut remaining = size;
    while remaining > 0 {
        let want = buf
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        let n = reader.read(&mut buf[..want]).await?;
        if n == 0 {
            anyhow::bail!("the data ended before the expected size");
        }
        writer.write_all(&buf[..n]).await?;
        remaining -= n as u64;
        progress.advance(n as u64, out_tx).await;
    }
    Ok(())
}

async fn send_file<R: AsyncRead + Unpin>(
    session: &Handle<Handler>,
    remote: &str,
    source: &mut R,
    size: u64,
    out_tx: &async_channel::Sender<SftpEvent>,
    progress: &Progress,
) -> anyhow::Result<()> {
    let (dir, name) = split_target(remote)?;
    let channel = session.channel_open_session().await?;
    channel
        .exec(true, format!("scp -t -- {}", quote(&dir)))
        .await?;
    let mut stream = channel.into_stream();
    read_ack(&mut stream).await?;
    stream
        .write_all(format!("C0644 {size} {name}\n").as_bytes())
        .await?;
    stream.flush().await?;
    read_ack(&mut stream).await?;
    pump(source, &mut stream, size, out_tx, progress).await?;
    stream.write_all(&[0]).await?;
    stream.flush().await?;
    read_ack(&mut stream).await?;
    let _ = stream.shutdown().await;
    Ok(())
}

struct Incoming<S> {
    stream: S,
    size: u64,
}

async fn open_download(
    session: &Handle<Handler>,
    remote: &str,
) -> anyhow::Result<Incoming<impl AsyncRead + AsyncWrite + Unpin>> {
    let channel = session.channel_open_session().await?;
    channel
        .exec(true, format!("scp -f -- {}", quote(remote)))
        .await?;
    let mut stream = channel.into_stream();
    stream.write_all(&[0]).await?;
    stream.flush().await?;
    loop {
        let kind = stream
            .read_u8()
            .await
            .map_err(|_| anyhow::anyhow!(NO_SCP))?;
        match kind {
            b'C' => {
                let size = parse_file_header(&read_line(&mut stream).await?)?;
                stream.write_all(&[0]).await?;
                stream.flush().await?;
                return Ok(Incoming { stream, size });
            }
            b'T' => {
                read_line(&mut stream).await?;
                stream.write_all(&[0]).await?;
                stream.flush().await?;
            }
            b'D' => anyhow::bail!("{remote} is a folder"),
            1 | 2 => anyhow::bail!(remote_message(&read_line(&mut stream).await?)),
            other => anyhow::bail!("scp sent an unexpected reply ({other:#04x})"),
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> Incoming<S> {
    async fn copy_to<W: AsyncWrite + Unpin>(
        mut self,
        writer: &mut W,
        out_tx: &async_channel::Sender<SftpEvent>,
        progress: &Progress,
    ) -> anyhow::Result<()> {
        pump(&mut self.stream, writer, self.size, out_tx, progress).await?;
        read_ack(&mut self.stream).await?;
        self.stream.write_all(&[0]).await?;
        self.stream.flush().await?;
        let _ = self.stream.shutdown().await;
        Ok(())
    }
}

async fn upload_file(
    session: &Handle<Handler>,
    local: &Path,
    remote: &str,
    size: u64,
    out_tx: &async_channel::Sender<SftpEvent>,
    progress: &Progress,
) -> anyhow::Result<()> {
    let mut file = tokio::fs::File::open(local).await?;
    send_file(session, remote, &mut file, size, out_tx, progress).await
}

async fn download_file(
    incoming: Incoming<impl AsyncRead + AsyncWrite + Unpin>,
    local: &Path,
    out_tx: &async_channel::Sender<SftpEvent>,
    progress: &Progress,
) -> anyhow::Result<()> {
    let mut file = create_new(local).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow::anyhow!(
                "{} already exists, so nothing was written over it",
                local.display()
            )
        } else {
            anyhow::anyhow!("couldn't create {}: {err}", local.display())
        }
    })?;
    incoming.copy_to(&mut file, out_tx, progress).await?;
    file.flush().await?;
    Ok(())
}

async fn do_upload(
    session: &Handle<Handler>,
    local: &Path,
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

    let result = upload_file(session, local, remote, total, out_tx, &Progress::new()).await;
    finish_transfer(out_tx, result, false).await
}

async fn do_download(
    session: &Handle<Handler>,
    remote: &str,
    local: &Path,
    out_tx: &async_channel::Sender<SftpEvent>,
) -> bool {
    let incoming = match open_download(session, remote).await {
        Ok(incoming) => incoming,
        Err(err) => {
            send_error(out_tx, format!("Couldn't read {remote}: {err}")).await;
            return false;
        }
    };

    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label: transfer_label(local, remote),
            total: Some(incoming.size),
        })
        .await;

    let result = download_file(incoming, local, out_tx, &Progress::new()).await;
    finish_transfer(out_tx, result, true).await
}

async fn do_read(
    session: &Handle<Handler>,
    remote: &str,
    out_tx: &async_channel::Sender<SftpEvent>,
) -> Result<Vec<u8>, String> {
    let incoming = open_download(session, remote)
        .await
        .map_err(|err| format!("Couldn't read {remote}: {err}"))?;
    if incoming.size > MAX_READ {
        return Err(format!(
            "{remote} is {}, too large to open here",
            super::format_size(incoming.size)
        ));
    }

    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label: transfer_label(Path::new(remote), remote),
            total: Some(incoming.size),
        })
        .await;

    let mut buf = Vec::with_capacity(incoming.size as usize);
    let result = incoming.copy_to(&mut buf, out_tx, &Progress::new()).await;

    let _ = out_tx
        .send(SftpEvent::TransferFinished {
            error: None,
            local_changed: false,
        })
        .await;
    result
        .map(|()| buf)
        .map_err(|err| format!("Couldn't read {remote}: {err}"))
}

async fn do_write(
    session: &Handle<Handler>,
    remote: &str,
    bytes: &[u8],
    ack: &async_channel::Sender<Option<String>>,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label: transfer_label(Path::new(remote), remote),
            total: Some(bytes.len() as u64),
        })
        .await;

    let mut source = bytes;
    let result = send_file(
        session,
        remote,
        &mut source,
        bytes.len() as u64,
        out_tx,
        &Progress::new(),
    )
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

fn mkdir_scripts(dirs: &[String]) -> Vec<String> {
    let mut scripts = Vec::new();
    let mut script = String::new();
    for dir in dirs {
        let quoted = quote(dir);
        if !script.is_empty() && script.len() + quoted.len() + 1 > MAX_SCRIPT {
            scripts.push(std::mem::take(&mut script));
        }
        if script.is_empty() {
            script.push_str("mkdir -p --");
        }
        script.push(' ');
        script.push_str(&quoted);
    }
    if !script.is_empty() {
        scripts.push(script);
    }
    scripts
}

async fn do_upload_dir(
    session: &Handle<Handler>,
    local: &Path,
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
        for script in mkdir_scripts(&dirs) {
            run_script(session, &script).await?;
        }
        let progress = Progress::new();
        futures::stream::iter(files.iter().map(anyhow::Ok))
            .try_for_each_concurrent(PARALLEL_FILES, |file| {
                upload_file(
                    session,
                    &file.source,
                    &file.destination,
                    file.size,
                    out_tx,
                    &progress,
                )
            })
            .await
    }
    .await;

    finish_transfer(out_tx, result, false).await;
}

async fn plan_download(
    session: &Handle<Handler>,
    remote_root: &str,
    local_root: &Path,
) -> anyhow::Result<(Vec<PathBuf>, Vec<PlannedFile<String, PathBuf>>)> {
    let mut dirs = vec![local_root.to_path_buf()];
    let mut files = Vec::new();
    let mut pending = vec![(remote_root.to_string(), local_root.to_path_buf())];

    while let Some((remote_dir, local_dir)) = pending.pop() {
        let (_, entries) = list(session, &remote_dir).await?;
        for entry in entries {
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

async fn do_download_dir(
    session: &Handle<Handler>,
    remote: &str,
    local: &Path,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    let (dirs, files) = match plan_download(session, remote, local).await {
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
                let progress = &progress;
                async move {
                    let incoming = open_download(session, &file.source).await?;
                    download_file(incoming, &file.destination, out_tx, progress).await
                }
            })
            .await
    }
    .await;

    finish_transfer(out_tx, result, true).await;
}

#[cfg(test)]
#[path = "../../tests/sftp/scp.rs"]
mod tests;
