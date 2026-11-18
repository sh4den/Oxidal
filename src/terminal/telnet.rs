use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::backend::{Backend, BackendEvent, EVENT_CAPACITY};
use crate::proxy::ProxyConfig;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

const SE: u8 = 240;
const SB: u8 = 250;
const WILL: u8 = 251;
const WONT: u8 = 252;
const DO: u8 = 253;
const DONT: u8 = 254;
const IAC: u8 = 255;

const OPT_BINARY: u8 = 0;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;

const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;

const TERM_NAME: &[u8] = b"xterm-256color";

const MAX_SUBNEG: usize = 512;

pub fn spawn(host: String, port: u16, proxy: Option<ProxyConfig>, rows: u16, cols: u16) -> Backend {
    let (out_tx, out_rx) = async_channel::bounded::<BackendEvent>(EVENT_CAPACITY);
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
            proxy,
            rows,
            cols,
            out_tx.clone(),
            in_rx,
            resize_rx,
        ));
        let _ = out_tx.send_blocking(BackendEvent::Closed(result.err().map(|e| e.to_string())));
    });

    Backend::new(out_rx, in_tx, Some(resize_tx))
}

async fn run(
    host: String,
    port: u16,
    proxy: Option<ProxyConfig>,
    rows: u16,
    cols: u16,
    out_tx: async_channel::Sender<BackendEvent>,
    in_rx: async_channel::Receiver<Vec<u8>>,
    resize_rx: async_channel::Receiver<(u16, u16)>,
) -> anyhow::Result<()> {
    let stream = crate::proxy::open_stream(&host, port, proxy.as_ref(), CONNECT_TIMEOUT).await?;

    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut telnet = Telnet::new(rows, cols);
    writer.write_all(&telnet.initial_offers()).await?;

    let mut buf = vec![0u8; 64 * 1024];
    loop {
        tokio::select! {
            input = in_rx.recv() => {
                match input {
                    Ok(bytes) => {
                        writer.write_all(&telnet.encode(&bytes)).await?;
                        if !telnet.server_echoes() {
                            let echo = telnet.echo(&bytes);
                            if !echo.is_empty()
                                && out_tx.send(BackendEvent::Data(echo)).await.is_err()
                            {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            resize = resize_rx.recv() => {
                if let Ok((rows, cols)) = resize {
                    let update = telnet.resize(rows, cols);
                    if !update.is_empty() {
                        writer.write_all(&update).await?;
                    }
                }
            }
            read = reader.read(&mut buf) => {
                match read {
                    Ok(0) => break,
                    Ok(n) => {
                        let (display, reply) = telnet.receive(&buf[..n]);
                        if !reply.is_empty() {
                            writer.write_all(&reply).await?;
                        }
                        if !display.is_empty()
                            && out_tx.send(BackendEvent::Data(display)).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }

    let _ = writer.shutdown().await;
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Data,
    Iac,
    Command(u8),
    Subneg,
    SubnegIac,
}

struct Telnet {
    state: State,
    subneg: Vec<u8>,
    saw_cr: bool,
    local: [bool; 256],
    local_pending: [bool; 256],
    remote: [bool; 256],
    remote_pending: [bool; 256],
    rows: u16,
    cols: u16,
}

impl Telnet {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            state: State::Data,
            subneg: Vec::new(),
            saw_cr: false,
            local: [false; 256],
            local_pending: [false; 256],
            remote: [false; 256],
            remote_pending: [false; 256],
            rows,
            cols,
        }
    }

    fn supports_local(option: u8) -> bool {
        matches!(option, OPT_BINARY | OPT_SGA | OPT_TTYPE | OPT_NAWS)
    }

    fn supports_remote(option: u8) -> bool {
        matches!(option, OPT_BINARY | OPT_ECHO | OPT_SGA)
    }

    fn initial_offers(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for option in [OPT_TTYPE, OPT_NAWS, OPT_SGA] {
            self.local_pending[option as usize] = true;
            out.extend_from_slice(&[IAC, WILL, option]);
        }
        for option in [OPT_ECHO, OPT_SGA] {
            self.remote_pending[option as usize] = true;
            out.extend_from_slice(&[IAC, DO, option]);
        }
        out
    }

    fn server_echoes(&self) -> bool {
        self.remote[OPT_ECHO as usize]
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Vec<u8> {
        self.rows = rows;
        self.cols = cols;
        if self.local[OPT_NAWS as usize] {
            self.naws()
        } else {
            Vec::new()
        }
    }

    fn receive(&mut self, input: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut display = Vec::with_capacity(input.len());
        let mut reply = Vec::new();

        for &byte in input {
            match self.state {
                State::Data => {
                    if byte == IAC {
                        self.state = State::Iac;
                    } else {
                        self.push_data(byte, &mut display);
                    }
                }
                State::Iac => match byte {
                    IAC => {
                        self.state = State::Data;
                        self.push_data(IAC, &mut display);
                    }
                    WILL | WONT | DO | DONT => self.state = State::Command(byte),
                    SB => {
                        self.subneg.clear();
                        self.state = State::Subneg;
                    }
                    _ => self.state = State::Data,
                },
                State::Command(command) => {
                    self.negotiate(command, byte, &mut reply);
                    self.state = State::Data;
                }
                State::Subneg => {
                    if byte == IAC {
                        self.state = State::SubnegIac;
                    } else if self.subneg.len() < MAX_SUBNEG {
                        self.subneg.push(byte);
                    }
                }
                State::SubnegIac => match byte {
                    IAC => {
                        if self.subneg.len() < MAX_SUBNEG {
                            self.subneg.push(IAC);
                        }
                        self.state = State::Subneg;
                    }
                    SE => {
                        let subneg = std::mem::take(&mut self.subneg);
                        self.subnegotiate(&subneg, &mut reply);
                        self.state = State::Data;
                    }
                    _ => self.state = State::Subneg,
                },
            }
        }

        (display, reply)
    }

    fn push_data(&mut self, byte: u8, display: &mut Vec<u8>) {
        let after_cr = std::mem::take(&mut self.saw_cr);
        if after_cr && byte == 0 && !self.remote[OPT_BINARY as usize] {
            return;
        }
        self.saw_cr = byte == b'\r';
        display.push(byte);
    }

    fn negotiate(&mut self, command: u8, option: u8, reply: &mut Vec<u8>) {
        let index = option as usize;
        match command {
            DO => {
                if self.local_pending[index] {
                    self.local_pending[index] = false;
                    if !self.local[index] {
                        self.local[index] = true;
                        self.on_local_enabled(option, reply);
                    }
                } else if Self::supports_local(option) {
                    if !self.local[index] {
                        self.local[index] = true;
                        reply.extend_from_slice(&[IAC, WILL, option]);
                        self.on_local_enabled(option, reply);
                    }
                } else {
                    reply.extend_from_slice(&[IAC, WONT, option]);
                }
            }
            DONT => {
                self.local_pending[index] = false;
                if self.local[index] {
                    self.local[index] = false;
                    reply.extend_from_slice(&[IAC, WONT, option]);
                }
            }
            WILL => {
                if self.remote_pending[index] {
                    self.remote_pending[index] = false;
                    self.remote[index] = true;
                } else if Self::supports_remote(option) {
                    if !self.remote[index] {
                        self.remote[index] = true;
                        reply.extend_from_slice(&[IAC, DO, option]);
                    }
                } else {
                    reply.extend_from_slice(&[IAC, DONT, option]);
                }
            }
            WONT => {
                self.remote_pending[index] = false;
                if self.remote[index] {
                    self.remote[index] = false;
                    reply.extend_from_slice(&[IAC, DONT, option]);
                }
            }
            _ => {}
        }
    }

    fn on_local_enabled(&mut self, option: u8, reply: &mut Vec<u8>) {
        if option == OPT_NAWS {
            reply.extend_from_slice(&self.naws());
        }
    }

    fn subnegotiate(&mut self, subneg: &[u8], reply: &mut Vec<u8>) {
        if let [OPT_TTYPE, TTYPE_SEND, ..] = subneg {
            reply.extend_from_slice(&[IAC, SB, OPT_TTYPE, TTYPE_IS]);
            reply.extend_from_slice(TERM_NAME);
            reply.extend_from_slice(&[IAC, SE]);
        }
    }

    fn naws(&self) -> Vec<u8> {
        let mut out = vec![IAC, SB, OPT_NAWS];
        let [cols_hi, cols_lo] = self.cols.to_be_bytes();
        let [rows_hi, rows_lo] = self.rows.to_be_bytes();
        for byte in [cols_hi, cols_lo, rows_hi, rows_lo] {
            if byte == IAC {
                out.push(IAC);
            }
            out.push(byte);
        }
        out.extend_from_slice(&[IAC, SE]);
        out
    }

    fn encode(&self, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len());
        let binary = self.local[OPT_BINARY as usize];
        let mut bytes = data.iter().peekable();
        while let Some(&byte) = bytes.next() {
            match byte {
                IAC => out.extend_from_slice(&[IAC, IAC]),
                b'\r' if !binary => {
                    out.push(b'\r');
                    if bytes.peek() != Some(&&b'\n') {
                        out.push(0);
                    }
                }
                _ => out.push(byte),
            }
        }
        out
    }

    fn echo(&self, data: &[u8]) -> Vec<u8> {
        if data.first() == Some(&0x1b) {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(data.len());
        for &byte in data {
            match byte {
                b'\r' => out.extend_from_slice(b"\r\n"),
                0x08 | 0x7f => out.extend_from_slice(b"\x08 \x08"),
                b'\t' | 0x20..=0x7e | 0x80..=0xff => out.push(byte),
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "../../tests/terminal/telnet.rs"]
mod tests;
