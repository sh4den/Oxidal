use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

use super::client::{
    CHUNK_SIZE, MAX_READ, PlannedFile, Progress, RECONNECT_ATTEMPTS, TRANSFER_BUFFER, create_new,
    finish_transfer, plan_upload, reconnect_delay, send_error, transfer_label,
};
use super::{SftpClient, SftpCommand, SftpEntry, SftpEvent, join_remote, parent_remote};
use crate::cert_trust::{self, CertificateFailure, PinningVerifier};
use crate::proxy::{self, ProxyConfig, Stream};
use crate::session::FtpSecurity;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
const DATA_TIMEOUT: Duration = Duration::from_secs(60);
const HEALTH_INTERVAL: Duration = Duration::from_secs(30);
const KEEPALIVE_AFTER: Duration = Duration::from_secs(60);
const IDLE_PROBE_AFTER: Duration = Duration::from_secs(20);
const QUIT_TIMEOUT: Duration = Duration::from_secs(2);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_LINE: u64 = 64 * 1024;
const MAX_LISTING: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct FtpTarget {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: SecretString,
    pub security: FtpSecurity,
    pub active_mode: bool,
    pub proxy: Option<ProxyConfig>,
}

impl FtpTarget {
    pub fn label(&self) -> String {
        if self.username.is_empty() {
            format!("{}:{}", self.host, self.port)
        } else {
            format!("{}@{}:{}", self.username, self.host, self.port)
        }
    }

    fn login_user(&self) -> &str {
        if self.username.is_empty() {
            "anonymous"
        } else {
            &self.username
        }
    }

    fn login_password(&self) -> SecretString {
        if self.username.is_empty() && self.password.expose_secret().is_empty() {
            SecretString::from("anonymous@")
        } else {
            self.password.clone()
        }
    }
}

pub fn spawn(target: FtpTarget) -> SftpClient {
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
        let result = runtime.block_on(run(target, out_tx.clone(), cmd_rx));
        let _ = out_tx.send_blocking(SftpEvent::Closed(result.err().map(|e| e.to_string())));
    });

    SftpClient {
        events: out_rx,
        commands: cmd_tx,
    }
}

pub fn check(target: FtpTarget) -> Result<String, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let connection = open(&target).await.map_err(|e| e.to_string())?;
        let secured = connection.tls.is_some();
        connection.close().await;
        Ok(format!(
            "Logged in to {}:{}{}",
            target.host,
            target.port,
            if secured { " over TLS" } else { "" }
        ))
    })
}

#[derive(Debug)]
struct UntrustedCertificate(CertificateFailure);

impl std::fmt::Display for UntrustedCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "The server's TLS certificate is not trusted. {}",
            self.0.reason
        )
    }
}

impl std::error::Error for UntrustedCertificate {}

#[derive(Clone)]
struct Tls {
    connector: TlsConnector,
    verifier: Arc<PinningVerifier>,
    name: ServerName<'static>,
}

impl Tls {
    fn new(target: &FtpTarget) -> anyhow::Result<Self> {
        let (config, verifier) = cert_trust::client_config(&target.host, target.port)?;
        let name = ServerName::try_from(target.host.clone())
            .map_err(|_| anyhow::anyhow!("{} is not a valid TLS server name", target.host))?;
        Ok(Self {
            connector: TlsConnector::from(config),
            verifier,
            name,
        })
    }

    async fn wrap(&self, stream: Box<dyn Stream>) -> anyhow::Result<Box<dyn Stream>> {
        let _ = self.verifier.take_failure();
        let handshake = self.connector.connect(self.name.clone(), stream);
        match tokio::time::timeout(CONNECT_TIMEOUT, handshake).await {
            Ok(Ok(stream)) => Ok(Box::new(stream)),
            Ok(Err(err)) => match self.verifier.take_failure() {
                Some(failure) => Err(UntrustedCertificate(failure).into()),
                None => Err(anyhow::anyhow!("TLS handshake failed: {err}")),
            },
            Err(_) => Err(anyhow::anyhow!("Timed out during the TLS handshake")),
        }
    }
}

struct Reply {
    code: u16,
    text: String,
}

impl Reply {
    fn class(&self) -> u16 {
        self.code / 100
    }

    fn message(&self) -> String {
        let last = self.text.lines().last().unwrap_or_default();
        let message = last.get(4..).unwrap_or_default().trim();
        if message.is_empty() {
            self.code.to_string()
        } else {
            format!("{message} ({})", self.code)
        }
    }

    fn expect(self, accepted: impl Fn(u16) -> bool) -> anyhow::Result<Reply> {
        if accepted(self.code) {
            Ok(self)
        } else {
            Err(anyhow::anyhow!(self.message()))
        }
    }

    fn completed(self) -> anyhow::Result<Reply> {
        self.expect(|code| code / 100 == 2)
    }
}

struct Control {
    reader: BufReader<Box<dyn Stream>>,
    broken: bool,
}

impl Control {
    fn new(stream: Box<dyn Stream>) -> Self {
        Self {
            reader: BufReader::new(stream),
            broken: false,
        }
    }

    async fn read_line(&mut self) -> anyhow::Result<String> {
        let mut line = Vec::new();
        let mut limited = (&mut self.reader).take(MAX_LINE);
        let read = limited.read_until(b'\n', &mut line);
        match tokio::time::timeout(REPLY_TIMEOUT, read).await {
            Ok(Ok(0)) => anyhow::bail!("The server closed the connection"),
            Ok(Ok(_)) => {}
            Ok(Err(err)) => return Err(err.into()),
            Err(_) => anyhow::bail!("The server stopped answering"),
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        Ok(String::from_utf8_lossy(&line).into_owned())
    }

    async fn read_reply_inner(&mut self) -> anyhow::Result<Reply> {
        let first = self.read_line().await?;
        let code = parse_code(&first)
            .ok_or_else(|| anyhow::anyhow!("Unexpected reply from the server: {first}"))?;
        let mut text = first.clone();
        if first.as_bytes().get(3) == Some(&b'-') {
            let terminator = format!("{code} ");
            loop {
                let line = self.read_line().await?;
                text.push('\n');
                text.push_str(&line);
                if line.starts_with(&terminator) || line == code.to_string() {
                    break;
                }
            }
        }
        Ok(Reply { code, text })
    }

    async fn read_reply(&mut self) -> anyhow::Result<Reply> {
        let reply = self.read_reply_inner().await;
        match &reply {
            Err(_) => self.broken = true,
            Ok(reply) if reply.code == 421 => self.broken = true,
            Ok(_) => {}
        }
        reply
    }

    async fn send(&mut self, command: &str) -> anyhow::Result<()> {
        if command.contains(['\r', '\n']) {
            anyhow::bail!("Names containing line breaks can't be used over FTP");
        }
        let line = format!("{command}\r\n");
        let write = async {
            let stream = self.reader.get_mut();
            stream.write_all(line.as_bytes()).await?;
            stream.flush().await
        };
        match tokio::time::timeout(REPLY_TIMEOUT, write).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => {
                self.broken = true;
                Err(err.into())
            }
            Err(_) => {
                self.broken = true;
                anyhow::bail!("The server stopped accepting commands")
            }
        }
    }

    async fn command(&mut self, command: &str) -> anyhow::Result<Reply> {
        self.send(command).await?;
        self.read_reply().await
    }

    async fn is_closed(&mut self) -> bool {
        if self.broken {
            return true;
        }
        match tokio::time::timeout(Duration::ZERO, self.reader.fill_buf()).await {
            Err(_) => false,
            Ok(Ok([])) | Ok(Err(_)) => {
                self.broken = true;
                true
            }
            Ok(Ok(_)) => {
                let _ = self.read_reply().await;
                self.broken = true;
                true
            }
        }
    }

    async fn upgrade(self, tls: &Tls) -> anyhow::Result<Control> {
        if !self.reader.buffer().is_empty() {
            anyhow::bail!("The server sent unexpected data before the TLS handshake");
        }
        let stream = tls.wrap(self.reader.into_inner()).await?;
        Ok(Control::new(stream))
    }
}

fn parse_code(line: &str) -> Option<u16> {
    let digits = line.get(..3)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match line.as_bytes().get(3) {
        None | Some(b' ' | b'-') => digits.parse().ok(),
        _ => None,
    }
}

#[derive(Default)]
struct Features {
    mlsd: bool,
    utf8: bool,
    size: bool,
    epsv_refused: bool,
    eprt_refused: bool,
    list_all_refused: bool,
}

fn parse_features(reply: &Reply) -> Features {
    let mut features = Features::default();
    for line in reply.text.lines().skip(1) {
        let feature = line.trim().to_ascii_uppercase();
        let name = feature.split_whitespace().next().unwrap_or_default();
        match name {
            "MLSD" | "MLST" => features.mlsd = true,
            "UTF8" => features.utf8 = true,
            "SIZE" => features.size = true,
            _ => {}
        }
    }
    features
}

enum PendingData {
    Connected(Box<dyn Stream>),
    Listening(TcpListener),
}

struct Connection {
    control: Control,
    tls: Option<Tls>,
    features: Features,
    host: String,
    proxy: Option<ProxyConfig>,
    active_mode: bool,
    local_ip: Option<IpAddr>,
}

async fn connect(
    host: &str,
    port: u16,
    proxy: Option<&ProxyConfig>,
) -> anyhow::Result<(Box<dyn Stream>, Option<IpAddr>)> {
    if proxy.is_some() {
        let stream = proxy::open_stream(host, port, proxy, CONNECT_TIMEOUT).await?;
        return Ok((stream, None));
    }
    let stream = match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port))).await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => anyhow::bail!("Could not reach {host}:{port}: {e}"),
        Err(_) => anyhow::bail!("Timed out connecting to {host}:{port}"),
    };
    let _ = stream.set_nodelay(true);
    let local_ip = stream.local_addr().ok().map(|addr| addr.ip());
    Ok((Box::new(stream), local_ip))
}

async fn open(target: &FtpTarget) -> anyhow::Result<Connection> {
    match open_once(target).await {
        Err(err) => match err.downcast_ref::<UntrustedCertificate>() {
            Some(UntrustedCertificate(failure)) => {
                cert_trust::confirm(&target.host, target.port, failure)
                    .await
                    .map_err(anyhow::Error::msg)?;
                open_once(target).await
            }
            None => Err(err),
        },
        ok => ok,
    }
}

async fn open_once(target: &FtpTarget) -> anyhow::Result<Connection> {
    if target.active_mode && target.proxy.is_some() {
        anyhow::bail!(
            "Active mode needs the server to connect back to this computer, which can't \
             happen through a SOCKS5 proxy. Switch the session to passive mode."
        );
    }

    let (stream, local_ip) = connect(&target.host, target.port, target.proxy.as_ref()).await?;
    let tls = match target.security {
        FtpSecurity::None => None,
        FtpSecurity::Explicit | FtpSecurity::Implicit => Some(Tls::new(target)?),
    };

    let mut control = match (&tls, target.security) {
        (Some(tls), FtpSecurity::Implicit) => Control::new(tls.wrap(stream).await?),
        _ => Control::new(stream),
    };

    let mut greeting = control.read_reply().await?;
    while greeting.code == 120 {
        greeting = control.read_reply().await?;
    }
    greeting.expect(|code| code == 220)?;

    if let (Some(tls), FtpSecurity::Explicit) = (&tls, target.security) {
        let reply = control.command("AUTH TLS").await?;
        if reply.code != 234 {
            anyhow::bail!(
                "{}:{} doesn't offer TLS ({}). Use \"No encryption\" only if you accept that \
                 the password and files travel in the clear.",
                target.host,
                target.port,
                reply.message()
            );
        }
        control = control.upgrade(tls).await?;
    }

    if tls.is_some() {
        control.command("PBSZ 0").await?.completed()?;
        control.command("PROT P").await?.completed()?;
    }

    let user = control
        .command(&format!("USER {}", target.login_user()))
        .await?;
    match user.code {
        230 | 202 => {}
        331 => {
            let password = target.login_password();
            let reply = control
                .command(&format!("PASS {}", password.expose_secret()))
                .await?;
            match reply.code {
                230 | 202 => {}
                332 => anyhow::bail!("The server asks for an account, which isn't supported"),
                _ => anyhow::bail!("Login refused: {}", reply.message()),
            }
        }
        332 => anyhow::bail!("The server asks for an account, which isn't supported"),
        _ => anyhow::bail!("Login refused: {}", user.message()),
    }

    let features = match control.command("FEAT").await? {
        reply if reply.code == 211 => parse_features(&reply),
        _ => Features::default(),
    };
    if features.utf8 {
        let _ = control.command("OPTS UTF8 ON").await?;
    }
    control.command("TYPE I").await?.completed()?;

    Ok(Connection {
        control,
        tls,
        features,
        host: target.host.clone(),
        proxy: target.proxy.clone(),
        active_mode: target.active_mode,
        local_ip,
    })
}

impl Connection {
    async fn close(mut self) {
        let _ = tokio::time::timeout(QUIT_TIMEOUT, self.control.command("QUIT")).await;
        let _ = tokio::time::timeout(QUIT_TIMEOUT, self.control.reader.get_mut().shutdown()).await;
    }

    async fn answers(&mut self) -> bool {
        !self.control.broken
            && matches!(self.control.command("NOOP").await, Ok(reply) if reply.class() == 2)
    }

    async fn pwd(&mut self) -> anyhow::Result<String> {
        let reply = self.control.command("PWD").await?.expect(|c| c == 257)?;
        parse_pwd(&reply.text).ok_or_else(|| anyhow::anyhow!("Couldn't read the current folder"))
    }

    async fn passive_port(&mut self) -> anyhow::Result<u16> {
        if !self.features.epsv_refused {
            let reply = self.control.command("EPSV").await?;
            if reply.code == 229 {
                return parse_epsv(&reply.text)
                    .ok_or_else(|| anyhow::anyhow!("Unexpected EPSV reply: {}", reply.text));
            }
            if self.control.broken {
                anyhow::bail!(reply.message());
            }
            self.features.epsv_refused = true;
        }
        let reply = self.control.command("PASV").await?.expect(|c| c == 227)?;
        parse_pasv(&reply.text)
            .ok_or_else(|| anyhow::anyhow!("Unexpected PASV reply: {}", reply.text))
    }

    async fn announce_port(&mut self, listener: &TcpListener) -> anyhow::Result<()> {
        let addr = listener.local_addr()?;
        if !self.features.eprt_refused {
            let family = if addr.is_ipv4() { 1 } else { 2 };
            let reply = self
                .control
                .command(&format!("EPRT |{family}|{}|{}|", addr.ip(), addr.port()))
                .await?;
            if reply.class() == 2 {
                return Ok(());
            }
            if addr.is_ipv6() || self.control.broken {
                anyhow::bail!("The server refused active mode: {}", reply.message());
            }
            self.features.eprt_refused = true;
        }
        let IpAddr::V4(ip) = addr.ip() else {
            anyhow::bail!("Active mode over IPv6 needs EPRT, which the server refused");
        };
        let [a, b, c, d] = ip.octets();
        let port = addr.port();
        self.control
            .command(&format!(
                "PORT {a},{b},{c},{d},{},{}",
                port >> 8,
                port & 0xff
            ))
            .await?
            .completed()
            .map_err(|e| anyhow::anyhow!("The server refused active mode: {e}"))?;
        Ok(())
    }

    async fn prepare_data(&mut self) -> anyhow::Result<PendingData> {
        if self.active_mode {
            let ip = self
                .local_ip
                .ok_or_else(|| anyhow::anyhow!("Couldn't tell which address to listen on"))?;
            let listener = TcpListener::bind((ip, 0)).await?;
            self.announce_port(&listener).await?;
            Ok(PendingData::Listening(listener))
        } else {
            let port = self.passive_port().await?;
            let (stream, _) = connect(&self.host, port, self.proxy.as_ref())
                .await
                .map_err(|e| anyhow::anyhow!("Couldn't open the data connection: {e}"))?;
            Ok(PendingData::Connected(stream))
        }
    }

    async fn start_transfer(&mut self, command: &str) -> anyhow::Result<DataStream> {
        let pending = self.prepare_data().await?;
        self.control.send(command).await?;

        let tls = self.tls.clone();
        let task = tokio::spawn(async move {
            let stream: Box<dyn Stream> = match pending {
                PendingData::Connected(stream) => stream,
                PendingData::Listening(listener) => {
                    match tokio::time::timeout(DATA_TIMEOUT, listener.accept()).await {
                        Ok(Ok((stream, _))) => Box::new(stream),
                        Ok(Err(err)) => return Err(anyhow::Error::from(err)),
                        Err(_) => anyhow::bail!(
                            "The server never connected back. Active mode doesn't work \
                             behind most routers and firewalls; try passive mode."
                        ),
                    }
                }
            };
            match tls {
                Some(tls) => tls.wrap(stream).await.map_err(|err| {
                    if err.is::<UntrustedCertificate>() {
                        anyhow::anyhow!("The data connection presented an untrusted certificate")
                    } else {
                        err
                    }
                }),
                None => Ok(stream),
            }
        });

        let reply = match self.control.read_reply().await {
            Ok(reply) => reply,
            Err(err) => {
                task.abort();
                return Err(err);
            }
        };
        let finished = match reply.class() {
            1 => false,
            2 => true,
            _ => {
                task.abort();
                return Err(anyhow::anyhow!(reply.message()));
            }
        };
        let stream = match task.await {
            Ok(result) => result?,
            Err(err) => anyhow::bail!("The data connection failed: {err}"),
        };
        Ok(DataStream { stream, finished })
    }

    async fn finish(&mut self, data: DataStream) -> anyhow::Result<()> {
        let DataStream {
            mut stream,
            finished,
        } = data;
        let _ = tokio::time::timeout(DATA_TIMEOUT, stream.shutdown()).await;
        let drain = async {
            let mut sink = [0u8; 4096];
            while matches!(stream.read(&mut sink).await, Ok(n) if n > 0) {}
        };
        let _ = tokio::time::timeout(DRAIN_TIMEOUT, drain).await;
        drop(stream);
        if !finished {
            self.control.read_reply().await?.completed()?;
        }
        Ok(())
    }

    async fn abandon(&mut self, data: DataStream) {
        drop(data.stream);
        if !data.finished {
            let _ = self.control.read_reply().await;
        }
    }

    async fn retrieve_all(&mut self, command: &str, limit: usize) -> anyhow::Result<Vec<u8>> {
        let mut data = self.start_transfer(command).await?;
        let mut bytes = Vec::new();
        let mut chunk = vec![0u8; CHUNK_SIZE];
        loop {
            match read_data(&mut data.stream, &mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    bytes.extend_from_slice(&chunk[..n]);
                    if bytes.len() > limit {
                        self.abandon(data).await;
                        anyhow::bail!("The listing is too large");
                    }
                }
                Err(err) => {
                    self.abandon(data).await;
                    return Err(err);
                }
            }
        }
        self.finish(data).await?;
        Ok(bytes)
    }

    async fn read_dir(&mut self, path: &str) -> anyhow::Result<(String, Vec<SftpEntry>)> {
        let (dir, mut entries) = self.read_dir_unsorted(path).await?;
        super::sort_entries(&mut entries);
        entries.shrink_to_fit();
        Ok((dir, entries))
    }

    async fn read_dir_unsorted(&mut self, path: &str) -> anyhow::Result<(String, Vec<SftpEntry>)> {
        let direct = if self.features.mlsd {
            plain_absolute(path)
        } else {
            None
        };
        let is_direct = direct.is_some();
        let dir = match direct {
            Some(dir) => dir,
            None => {
                self.control
                    .command(&format!("CWD {path}"))
                    .await?
                    .completed()?;
                self.pwd().await.unwrap_or_else(|_| path.to_string())
            }
        };

        let entries = if self.features.mlsd {
            let command = if is_direct {
                format!("MLSD {dir}")
            } else {
                "MLSD".to_string()
            };
            let raw = self.retrieve_all(&command, MAX_LISTING).await?;
            parse_listing(&raw, parse_mlsd_line)
        } else {
            let raw = if self.features.list_all_refused {
                self.retrieve_all("LIST", MAX_LISTING).await?
            } else {
                match self.retrieve_all("LIST -a", MAX_LISTING).await {
                    Ok(raw) => raw,
                    Err(_) if !self.control.broken => {
                        self.features.list_all_refused = true;
                        self.retrieve_all("LIST", MAX_LISTING).await?
                    }
                    Err(err) => return Err(err),
                }
            };
            let now = chrono::Utc::now();
            parse_listing(&raw, |line| parse_list_line(line, now))
        };

        let entries: Vec<SftpEntry> = entries
            .into_iter()
            .filter(|entry| entry.name != "." && entry.name != "..")
            .map(|mut entry| {
                entry.path = join_remote(&dir, &entry.name);
                entry
            })
            .collect();
        Ok((dir, entries))
    }

    async fn exists(&mut self, path: &str) -> anyhow::Result<bool> {
        if self.features.mlsd {
            let reply = self.control.command(&format!("MLST {path}")).await?;
            match reply.code {
                250 => return Ok(true),
                550 => return Ok(false),
                _ => {}
            }
        }
        let parent = parent_remote(path);
        let name = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(path);
        let (_, entries) = self.read_dir_unsorted(&parent).await?;
        Ok(entries.iter().any(|entry| entry.name == name))
    }

    async fn size(&mut self, path: &str) -> Option<u64> {
        if !self.features.size {
            return None;
        }
        let reply = self.control.command(&format!("SIZE {path}")).await.ok()?;
        if reply.code != 213 {
            return None;
        }
        reply.text.get(4..)?.trim().parse().ok()
    }

    async fn rename(&mut self, from: &str, to: &str) -> anyhow::Result<()> {
        self.control
            .command(&format!("RNFR {from}"))
            .await?
            .expect(|c| c == 350)?;
        self.control
            .command(&format!("RNTO {to}"))
            .await?
            .completed()?;
        Ok(())
    }

    async fn simple(&mut self, command: String) -> anyhow::Result<()> {
        self.control.command(&command).await?.completed()?;
        Ok(())
    }
}

struct DataStream {
    stream: Box<dyn Stream>,
    finished: bool,
}

async fn read_data(stream: &mut Box<dyn Stream>, buf: &mut [u8]) -> anyhow::Result<usize> {
    match tokio::time::timeout(DATA_TIMEOUT, stream.read(buf)).await {
        Ok(Ok(n)) => Ok(n),
        Ok(Err(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => Ok(0),
        Ok(Err(err)) => Err(err.into()),
        Err(_) => anyhow::bail!("The data connection stalled"),
    }
}

async fn write_data(stream: &mut Box<dyn Stream>, buf: &[u8]) -> anyhow::Result<()> {
    match tokio::time::timeout(DATA_TIMEOUT, stream.write_all(buf)).await {
        Ok(result) => Ok(result?),
        Err(_) => anyhow::bail!("The data connection stalled"),
    }
}

async fn reconnect(
    target: &FtpTarget,
    current_dir: &mut String,
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
        match open(target).await {
            Ok(mut connection) => {
                refresh(&mut connection, current_dir, out_tx).await;
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

async fn run(
    target: FtpTarget,
    out_tx: async_channel::Sender<SftpEvent>,
    cmd_rx: async_channel::Receiver<SftpCommand>,
) -> anyhow::Result<()> {
    let mut connection = open(&target).await?;
    let mut current_dir = connection.pwd().await.unwrap_or_else(|_| "/".to_string());
    refresh(&mut connection, &mut current_dir, &out_tx).await;

    let mut health = tokio::time::interval(HEALTH_INTERVAL);
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_used = Instant::now();

    loop {
        let cmd = tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Ok(cmd) => cmd,
                Err(_) => break,
            },
            _ = health.tick() => {
                if connection.control.is_closed().await {
                    connection = reconnect(&target, &mut current_dir, &out_tx, &cmd_rx).await?;
                    last_used = Instant::now();
                } else if last_used.elapsed() >= KEEPALIVE_AFTER {
                    let _ = connection.answers().await;
                    last_used = Instant::now();
                }
                continue;
            }
        };

        if connection.control.is_closed().await
            || (last_used.elapsed() >= IDLE_PROBE_AFTER && !connection.answers().await)
        {
            connection = reconnect(&target, &mut current_dir, &out_tx, &cmd_rx).await?;
        }

        execute(&mut connection, cmd, &mut current_dir, &out_tx).await;
        last_used = Instant::now();
    }

    connection.close().await;
    Ok(())
}

async fn list_and_send(
    connection: &mut Connection,
    current_dir: &mut String,
    path: String,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    match connection.read_dir(&path).await {
        Ok((dir, entries)) => {
            *current_dir = dir.clone();
            let _ = out_tx.send(SftpEvent::Listing { path: dir, entries }).await;
        }
        Err(err) => send_error(out_tx, format!("Couldn't list {path}: {err}")).await,
    }
}

async fn refresh(
    connection: &mut Connection,
    current_dir: &mut String,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    let dir = current_dir.clone();
    list_and_send(connection, current_dir, dir, out_tx).await;
}

async fn execute(
    connection: &mut Connection,
    cmd: SftpCommand,
    current_dir: &mut String,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    match cmd {
        SftpCommand::List { path } => {
            list_and_send(connection, current_dir, path, out_tx).await;
        }
        SftpCommand::CreateDir { name } => {
            let path = join_remote(current_dir, &name);
            if let Err(err) = connection.simple(format!("MKD {path}")).await {
                send_error(out_tx, format!("Couldn't create folder: {err}")).await;
            }
            refresh(connection, current_dir, out_tx).await;
        }
        SftpCommand::CreateFile { name } => {
            let path = join_remote(current_dir, &name);
            let result = async {
                if connection.exists(&path).await? {
                    anyhow::bail!("\"{}\" already exists", super::display_name(&name));
                }
                let data = connection.start_transfer(&format!("STOR {path}")).await?;
                connection.finish(data).await
            }
            .await;
            if let Err(err) = result {
                send_error(out_tx, format!("Couldn't create file: {err}")).await;
            }
            refresh(connection, current_dir, out_tx).await;
        }
        SftpCommand::Rename { from, to } => {
            if let Err(err) = connection.rename(&from, &to).await {
                send_error(out_tx, format!("Couldn't rename: {err}")).await;
            }
            refresh(connection, current_dir, out_tx).await;
        }
        SftpCommand::Move { from, to } => {
            match connection.exists(&to).await {
                Ok(true) => {
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
                Ok(false) => {}
                Err(err) => {
                    send_error(out_tx, format!("Couldn't move: {err}")).await;
                    return;
                }
            }
            match connection.rename(&from, &to).await {
                Ok(()) => refresh(connection, current_dir, out_tx).await,
                Err(err) => send_error(out_tx, format!("Couldn't move: {err}")).await,
            }
        }
        SftpCommand::RemoveFile { path } => {
            if let Err(err) = connection.simple(format!("DELE {path}")).await {
                send_error(out_tx, format!("Couldn't delete file: {err}")).await;
            }
            refresh(connection, current_dir, out_tx).await;
        }
        SftpCommand::RemoveDir { path } => {
            if let Err(err) = connection.simple(format!("RMD {path}")).await {
                send_error(out_tx, format!("Couldn't delete folder: {err}")).await;
            }
            refresh(connection, current_dir, out_tx).await;
        }
        SftpCommand::Upload { local, remote } => {
            do_upload(connection, &local, &remote, out_tx).await;
            refresh(connection, current_dir, out_tx).await;
        }
        SftpCommand::Download {
            remote,
            local,
            open_when_done,
        } => {
            if do_download(connection, &remote, &local, out_tx).await
                && open_when_done
                && let Err(err) = open::that_detached(&local)
            {
                send_error(out_tx, format!("Couldn't open {}: {err}", local.display())).await;
            }
        }
        SftpCommand::Read { remote } => {
            let result = do_read(connection, &remote, out_tx).await;
            let _ = out_tx.send(SftpEvent::FileLoaded { remote, result }).await;
        }
        SftpCommand::Write { remote, bytes, ack } => {
            do_write(connection, &remote, &bytes, &ack, out_tx).await;
            refresh(connection, current_dir, out_tx).await;
        }
        SftpCommand::UploadDir { local, remote } => {
            do_upload_dir(connection, &local, &remote, out_tx).await;
            refresh(connection, current_dir, out_tx).await;
        }
        SftpCommand::DownloadDir { remote, local } => {
            do_download_dir(connection, &remote, &local, out_tx).await;
        }
    }
}

async fn copy_up(
    connection: &mut Connection,
    local: &Path,
    remote: &str,
    out_tx: &async_channel::Sender<SftpEvent>,
    progress: &Progress,
) -> anyhow::Result<()> {
    let mut local_file = tokio::fs::File::open(local).await?;
    let mut data = connection.start_transfer(&format!("STOR {remote}")).await?;
    let mut buf = vec![0u8; TRANSFER_BUFFER];
    let copied = async {
        loop {
            let n = local_file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            write_data(&mut data.stream, &buf[..n]).await?;
            progress.advance(n as u64, out_tx).await;
        }
        anyhow::Ok(())
    }
    .await;
    if let Err(err) = copied {
        connection.abandon(data).await;
        return Err(err);
    }
    connection.finish(data).await
}

async fn copy_down(
    connection: &mut Connection,
    remote: &str,
    local: &Path,
    out_tx: &async_channel::Sender<SftpEvent>,
    progress: &Progress,
) -> anyhow::Result<()> {
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
    let mut data = match connection.start_transfer(&format!("RETR {remote}")).await {
        Ok(data) => data,
        Err(err) => {
            drop(local_file);
            let _ = tokio::fs::remove_file(local).await;
            return Err(err);
        }
    };
    let mut buf = vec![0u8; TRANSFER_BUFFER];
    let copied = async {
        loop {
            let n = read_data(&mut data.stream, &mut buf).await?;
            if n == 0 {
                break;
            }
            local_file.write_all(&buf[..n]).await?;
            progress.advance(n as u64, out_tx).await;
        }
        local_file.flush().await?;
        anyhow::Ok(())
    }
    .await;
    if let Err(err) = copied {
        connection.abandon(data).await;
        return Err(err);
    }
    connection.finish(data).await
}

async fn do_upload(
    connection: &mut Connection,
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
    let result = copy_up(connection, local, remote, out_tx, &Progress::new()).await;
    finish_transfer(out_tx, result, false).await
}

async fn do_download(
    connection: &mut Connection,
    remote: &str,
    local: &Path,
    out_tx: &async_channel::Sender<SftpEvent>,
) -> bool {
    let total = connection.size(remote).await;
    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label: transfer_label(local, remote),
            total,
        })
        .await;
    let result = copy_down(connection, remote, local, out_tx, &Progress::new()).await;
    finish_transfer(out_tx, result, true).await
}

async fn do_read(
    connection: &mut Connection,
    remote: &str,
    out_tx: &async_channel::Sender<SftpEvent>,
) -> Result<Vec<u8>, String> {
    let total = connection.size(remote).await;
    if let Some(total) = total
        && total > MAX_READ
    {
        return Err(format!(
            "{remote} is {}, too large to open here",
            super::format_size(total)
        ));
    }

    let _ = out_tx
        .send(SftpEvent::TransferStarted {
            label: transfer_label(Path::new(remote), remote),
            total,
        })
        .await;

    let result = async {
        let mut data = connection.start_transfer(&format!("RETR {remote}")).await?;
        let mut buf = Vec::with_capacity(total.unwrap_or(0) as usize);
        let mut chunk = vec![0u8; CHUNK_SIZE];
        let progress = Progress::new();
        let read = async {
            loop {
                let n = read_data(&mut data.stream, &mut chunk).await?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() as u64 > MAX_READ {
                    anyhow::bail!("the file is too large to open here");
                }
                progress.advance(n as u64, out_tx).await;
            }
            anyhow::Ok(())
        }
        .await;
        if let Err(err) = read {
            connection.abandon(data).await;
            return Err(err);
        }
        connection.finish(data).await?;
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
    connection: &mut Connection,
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

    let result = async {
        let mut data = connection.start_transfer(&format!("STOR {remote}")).await?;
        let progress = Progress::new();
        let written = async {
            for chunk in bytes.chunks(TRANSFER_BUFFER) {
                write_data(&mut data.stream, chunk).await?;
                progress.advance(chunk.len() as u64, out_tx).await;
            }
            anyhow::Ok(())
        }
        .await;
        if let Err(err) = written {
            connection.abandon(data).await;
            return Err(err);
        }
        connection.finish(data).await
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

async fn plan_download(
    connection: &mut Connection,
    remote_root: &str,
    local_root: &Path,
) -> anyhow::Result<(
    Vec<std::path::PathBuf>,
    Vec<PlannedFile<String, std::path::PathBuf>>,
)> {
    let mut dirs = vec![local_root.to_path_buf()];
    let mut files = Vec::new();
    let mut pending = vec![(remote_root.to_string(), local_root.to_path_buf())];

    while let Some((remote_dir, local_dir)) = pending.pop() {
        let (_, entries) = connection.read_dir_unsorted(&remote_dir).await?;
        for entry in entries {
            let local_path = local_dir.join(super::safe_local_name(&entry.name));
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
    connection: &mut Connection,
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
        for dir in &dirs {
            let _ = connection.simple(format!("MKD {dir}")).await;
            if connection.control.broken {
                anyhow::bail!("The connection was lost");
            }
        }
        let progress = Progress::new();
        for file in &files {
            copy_up(
                connection,
                &file.source,
                &file.destination,
                out_tx,
                &progress,
            )
            .await?;
        }
        anyhow::Ok(())
    }
    .await;

    finish_transfer(out_tx, result, false).await;
}

async fn do_download_dir(
    connection: &mut Connection,
    remote: &str,
    local: &Path,
    out_tx: &async_channel::Sender<SftpEvent>,
) {
    let (dirs, files) = match plan_download(connection, remote, local).await {
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
        for file in &files {
            copy_down(
                connection,
                &file.source,
                &file.destination,
                out_tx,
                &progress,
            )
            .await?;
        }
        anyhow::Ok(())
    }
    .await;

    finish_transfer(out_tx, result, true).await;
}

fn plain_absolute(path: &str) -> Option<String> {
    if !path.starts_with('/')
        || path
            .split('/')
            .any(|segment| segment == "." || segment == "..")
    {
        return None;
    }
    let trimmed = path.trim_end_matches('/');
    Some(if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    })
}

fn parse_pwd(text: &str) -> Option<String> {
    let line = text.lines().next()?;
    let start = line.find('"')? + 1;
    let mut path = String::new();
    let mut chars = line[start..].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                path.push('"');
            } else {
                return Some(path);
            }
        } else {
            path.push(c);
        }
    }
    None
}

fn parse_epsv(text: &str) -> Option<u16> {
    let start = text.find('(')? + 1;
    let end = start + text[start..].find(')')?;
    let inner = &text[start..end];
    let delimiter = inner.chars().next()?;
    inner
        .split(delimiter)
        .rfind(|part| !part.is_empty())?
        .parse()
        .ok()
}

fn parse_pasv(text: &str) -> Option<u16> {
    let body = match text.find('(') {
        Some(open) => &text[open + 1..],
        None => text.get(4..)?,
    };
    let numbers: Vec<u16> = body
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .take(6)
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    let [_, _, _, _, high, low] = numbers.as_slice() else {
        return None;
    };
    if *high > 255 || *low > 255 {
        return None;
    }
    Some(high * 256 + low)
}

fn parse_listing(raw: &[u8], parse: impl Fn(&str) -> Option<SftpEntry>) -> Vec<SftpEntry> {
    String::from_utf8_lossy(raw)
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.trim().is_empty())
        .filter_map(parse)
        .collect()
}

fn empty_entry(name: String) -> SftpEntry {
    SftpEntry {
        name,
        path: String::new(),
        is_dir: false,
        is_symlink: false,
        size: 0,
        modified: None,
        accessed: None,
        permissions: None,
        owner: None,
        group: None,
    }
}

fn parse_mlsd_line(line: &str) -> Option<SftpEntry> {
    let (facts, name) = line.split_once(' ')?;
    if name.is_empty() {
        return None;
    }
    let mut entry = empty_entry(name.to_string());
    let mut uid = None;
    let mut gid = None;
    for fact in facts.split(';').filter(|fact| !fact.is_empty()) {
        let (key, value) = fact.split_once('=')?;
        match key.to_ascii_lowercase().as_str() {
            "type" => {
                let value = value.to_ascii_lowercase();
                match value.as_str() {
                    "cdir" | "pdir" => return None,
                    "dir" => entry.is_dir = true,
                    _ if value.starts_with("os.unix=slink") || value == "os.unix=symlink" => {
                        entry.is_symlink = true
                    }
                    _ => {}
                }
            }
            "size" | "sizd" => entry.size = value.parse().unwrap_or(0),
            "modify" => entry.modified = parse_mlsd_time(value),
            "unix.mode" => entry.permissions = u32::from_str_radix(value, 8).ok(),
            "unix.owner" | "unix.ownername" | "unix.user" => entry.owner = Some(value.to_string()),
            "unix.group" | "unix.groupname" => entry.group = Some(value.to_string()),
            "unix.uid" => uid = Some(value.to_string()),
            "unix.gid" => gid = Some(value.to_string()),
            _ => {}
        }
    }
    entry.owner = entry.owner.or(uid);
    entry.group = entry.group.or(gid);
    Some(entry)
}

fn parse_mlsd_time(value: &str) -> Option<u64> {
    let stamp = value.get(..14)?;
    let parsed = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d%H%M%S").ok()?;
    u64::try_from(parsed.and_utc().timestamp()).ok()
}

fn tokens(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (ix, c) in line.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                out.push((s, &line[s..ix]));
                start = None;
            }
            (false, None) => start = Some(ix),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, &line[s..]));
    }
    out
}

fn month_number(token: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let lower = token.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|month| *month == lower)
        .map(|ix| ix as u32 + 1)
}

fn parse_list_line(line: &str, now: chrono::DateTime<chrono::Utc>) -> Option<SftpEntry> {
    parse_unix_line(line, now).or_else(|| parse_dos_line(line))
}

fn parse_mode(perms: &str) -> Option<u32> {
    let bits = perms.get(1..10)?;
    let mut mode = 0u32;
    for (ix, c) in bits.chars().enumerate() {
        let bit = 1 << (8 - ix);
        match (ix % 3, c) {
            (_, '-') => {}
            (0, 'r') | (1, 'w') | (2, 'x' | 's' | 't') => mode |= bit,
            (2, 'S' | 'T') => {}
            _ => return None,
        }
    }
    Some(mode)
}

fn parse_unix_line(line: &str, now: chrono::DateTime<chrono::Utc>) -> Option<SftpEntry> {
    use chrono::{Datelike as _, NaiveDate, NaiveTime};

    let parts = tokens(line);
    let perms = parts.first()?.1;
    let kind = perms.chars().next()?;
    if !"-dlbcps".contains(kind) {
        return None;
    }
    let mode = parse_mode(perms)?;

    let month_ix = (3..parts.len().saturating_sub(3)).find(|&ix| {
        month_number(parts[ix].1).is_some() && parts[ix - 1].1.bytes().all(|b| b.is_ascii_digit())
    })?;
    let month = month_number(parts[month_ix].1)?;
    let day: u32 = parts[month_ix + 1].1.parse().ok()?;
    let year_or_time = parts[month_ix + 2].1;
    let size: u64 = parts[month_ix - 1].1.parse().ok()?;
    let name_start = parts.get(month_ix + 3)?.0;
    let mut name = line[name_start..].to_string();

    let modified = if let Some((hour, minute)) = year_or_time.split_once(':') {
        let time = NaiveTime::from_hms_opt(hour.parse().ok()?, minute.parse().ok()?, 0)?;
        let mut date = NaiveDate::from_ymd_opt(now.year(), month, day)?;
        if date > now.date_naive() + chrono::Days::new(1) {
            date = NaiveDate::from_ymd_opt(now.year() - 1, month, day)?;
        }
        date.and_time(time)
    } else {
        NaiveDate::from_ymd_opt(year_or_time.parse().ok()?, month, day)?.and_time(NaiveTime::MIN)
    };

    let is_symlink = kind == 'l';
    if is_symlink && let Some((link, _)) = name.split_once(" -> ") {
        name = link.to_string();
    }

    let between = &parts[2..month_ix - 1];
    let mut entry = empty_entry(name);
    entry.is_dir = kind == 'd';
    entry.is_symlink = is_symlink;
    entry.size = size;
    entry.modified = u64::try_from(modified.and_utc().timestamp()).ok();
    entry.permissions = Some(mode);
    entry.owner = between.first().map(|(_, owner)| owner.to_string());
    entry.group = between.get(1).map(|(_, group)| group.to_string());
    Some(entry)
}

fn parse_dos_line(line: &str) -> Option<SftpEntry> {
    use chrono::{NaiveDate, NaiveTime};

    let parts = tokens(line);
    let (_, date) = *parts.first()?;
    let (_, time) = *parts.get(1)?;
    let (_, size_or_dir) = *parts.get(2)?;
    let name_start = parts.get(3)?.0;

    let mut date_parts = date.split('-');
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    let year: i32 = match date_parts.next()?.parse::<i32>().ok()? {
        year if year < 70 => 2000 + year,
        year if year < 100 => 1900 + year,
        year => year,
    };

    let upper = time.to_ascii_uppercase();
    let (clock, pm) = match upper.strip_suffix("PM") {
        Some(clock) => (clock, true),
        None => (upper.strip_suffix("AM").unwrap_or(&upper), false),
    };
    let (hour, minute) = clock.split_once(':')?;
    let mut hour: u32 = hour.parse().ok()?;
    if pm && hour < 12 {
        hour += 12;
    } else if !pm && hour == 12 && upper.ends_with("AM") {
        hour = 0;
    }
    let time = NaiveTime::from_hms_opt(hour, minute.parse().ok()?, 0)?;
    let modified = NaiveDate::from_ymd_opt(year, month, day)?.and_time(time);

    let mut entry = empty_entry(line[name_start..].to_string());
    if size_or_dir.eq_ignore_ascii_case("<DIR>") {
        entry.is_dir = true;
    } else {
        entry.size = size_or_dir.replace(',', "").parse().ok()?;
    }
    entry.modified = u64::try_from(modified.and_utc().timestamp()).ok();
    Some(entry)
}

#[cfg(test)]
#[path = "../../tests/sftp/ftp.rs"]
mod tests;
