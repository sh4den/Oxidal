use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

use super::backend::{Backend, BackendEvent};

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

pub fn spawn(host: String, port: u16, rows: u16, cols: u16) -> Backend {
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
    rows: u16,
    cols: u16,
    out_tx: async_channel::Sender<BackendEvent>,
    in_rx: async_channel::Receiver<Vec<u8>>,
    resize_rx: async_channel::Receiver<(u16, u16)>,
) -> anyhow::Result<()> {
    let mut stream = match tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => anyhow::bail!("Could not reach {host}:{port}: {e}"),
        Err(_) => anyhow::bail!("Timed out connecting to {host}:{port}"),
    };
    let _ = stream.set_nodelay(true);

    let (mut reader, mut writer) = stream.split();
    let mut telnet = Telnet::new(rows, cols);
    writer.write_all(&telnet.initial_offers()).await?;

    let mut buf = [0u8; 4096];
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
mod tests {
    use super::*;

    fn connected() -> Telnet {
        let mut telnet = Telnet::new(24, 80);
        telnet.initial_offers();
        telnet
    }

    #[test]
    fn offers_are_answered_without_a_second_round() {
        let mut telnet = connected();

        let (display, reply) = telnet.receive(&[IAC, DO, OPT_TTYPE]);
        assert!(display.is_empty());
        assert_eq!(reply, Vec::<u8>::new());
        assert!(telnet.local[OPT_TTYPE as usize]);

        let (_, reply) = telnet.receive(&[IAC, WILL, OPT_ECHO]);
        assert_eq!(reply, Vec::<u8>::new());
        assert!(telnet.server_echoes());
    }

    #[test]
    fn unsolicited_negotiation_gets_a_single_answer() {
        let mut telnet = connected();

        let (_, reply) = telnet.receive(&[IAC, DO, OPT_BINARY]);
        assert_eq!(reply, vec![IAC, WILL, OPT_BINARY]);

        let (_, reply) = telnet.receive(&[IAC, DO, OPT_BINARY]);
        assert!(reply.is_empty());
    }

    #[test]
    fn unsupported_options_are_refused() {
        let mut telnet = connected();
        const OPT_NEW_ENVIRON: u8 = 39;

        let (_, reply) = telnet.receive(&[IAC, DO, OPT_NEW_ENVIRON]);
        assert_eq!(reply, vec![IAC, WONT, OPT_NEW_ENVIRON]);

        let (_, reply) = telnet.receive(&[IAC, WILL, OPT_NEW_ENVIRON]);
        assert_eq!(reply, vec![IAC, DONT, OPT_NEW_ENVIRON]);
    }

    #[test]
    fn window_size_is_sent_when_naws_turns_on_and_when_it_changes() {
        let mut telnet = connected();

        let (_, reply) = telnet.receive(&[IAC, DO, OPT_NAWS]);
        assert_eq!(reply, vec![IAC, SB, OPT_NAWS, 0, 80, 0, 24, IAC, SE]);

        assert_eq!(
            telnet.resize(50, 132),
            vec![IAC, SB, OPT_NAWS, 0, 132, 0, 50, IAC, SE]
        );
    }

    #[test]
    fn window_size_escapes_a_dimension_that_looks_like_a_command() {
        let mut telnet = connected();
        telnet.receive(&[IAC, DO, OPT_NAWS]);

        assert_eq!(
            telnet.resize(24, 255),
            vec![IAC, SB, OPT_NAWS, 0, IAC, IAC, 0, 24, IAC, SE]
        );
    }

    #[test]
    fn terminal_type_is_reported_on_request() {
        let mut telnet = connected();
        telnet.receive(&[IAC, DO, OPT_TTYPE]);

        let (_, reply) = telnet.receive(&[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
        let mut expected = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
        expected.extend_from_slice(TERM_NAME);
        expected.extend_from_slice(&[IAC, SE]);
        assert_eq!(reply, expected);
    }

    #[test]
    fn commands_never_reach_the_grid() {
        let mut telnet = connected();

        let (display, _) = telnet.receive(&[
            b'h', b'i', IAC, DO, OPT_TTYPE, b' ', IAC, IAC, b'!', IAC, SB, OPT_TTYPE, TTYPE_SEND,
            IAC, SE, b'?',
        ]);
        assert_eq!(display, vec![b'h', b'i', b' ', IAC, b'!', b'?']);
    }

    #[test]
    fn negotiation_split_across_reads_is_still_understood() {
        let mut telnet = connected();

        assert!(telnet.receive(&[b'a', IAC]).0 == vec![b'a']);
        assert!(telnet.receive(&[DO]).1.is_empty());
        let (display, reply) = telnet.receive(&[OPT_BINARY, b'b']);
        assert_eq!(display, vec![b'b']);
        assert_eq!(reply, vec![IAC, WILL, OPT_BINARY]);
    }

    #[test]
    fn carriage_return_padding_is_stripped_but_newlines_survive() {
        let mut telnet = connected();

        let (display, _) = telnet.receive(b"one\r\0two\r\nthree");
        assert_eq!(display, b"one\rtwo\r\nthree");
    }

    #[test]
    fn carriage_return_padding_is_stripped_across_a_read_boundary() {
        let mut telnet = connected();

        assert_eq!(telnet.receive(b"one\r").0, b"one\r");
        assert_eq!(telnet.receive(&[0, b'x']).0, b"x");
    }

    #[test]
    fn binary_mode_leaves_the_stream_alone() {
        let mut telnet = connected();
        telnet.receive(&[IAC, WILL, OPT_BINARY]);

        let (display, _) = telnet.receive(b"one\r\0two");
        assert_eq!(display, b"one\r\0two");
    }

    #[test]
    fn outgoing_data_is_escaped_for_an_nvt() {
        let telnet = connected();

        assert_eq!(telnet.encode(&[IAC]), vec![IAC, IAC]);
        assert_eq!(telnet.encode(b"ls\r"), b"ls\r\0".to_vec());
        assert_eq!(telnet.encode(b"ls\r\n"), b"ls\r\n".to_vec());
    }

    #[test]
    fn outgoing_data_keeps_bare_returns_in_binary_mode() {
        let mut telnet = connected();
        telnet.receive(&[IAC, DO, OPT_BINARY]);

        assert_eq!(telnet.encode(b"ls\r"), b"ls\r".to_vec());
    }

    #[test]
    fn local_echo_covers_typing_and_drops_escape_sequences() {
        let telnet = connected();

        assert_eq!(telnet.echo(b"ls"), b"ls".to_vec());
        assert_eq!(telnet.echo(b"\r"), b"\r\n".to_vec());
        assert_eq!(telnet.echo(&[0x7f]), b"\x08 \x08".to_vec());
        assert!(telnet.echo(b"\x1b[A").is_empty());
    }

    #[test]
    fn a_live_session_negotiates_and_carries_traffic_both_ways() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("port");

        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout");

            let mut offers = [0u8; 64];
            let n = socket.read(&mut offers).expect("client offers");
            let offers = offers[..n].to_vec();

            socket
                .write_all(&[
                    IAC, DO, OPT_NAWS, IAC, DO, OPT_TTYPE, IAC, WILL, OPT_ECHO, IAC, SB, OPT_TTYPE,
                    TTYPE_SEND, IAC, SE,
                ])
                .expect("server negotiation");
            socket.write_all(b"login: ").expect("banner");

            let mut replies = Vec::new();
            let mut buf = [0u8; 256];
            while !replies.windows(6).any(|w| w == b"root\r\0") {
                match socket.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => replies.extend_from_slice(&buf[..n]),
                    Err(_) => break,
                }
            }
            (offers, replies)
        });

        let backend = spawn(addr.ip().to_string(), addr.port(), 24, 80);

        let mut display = Vec::new();
        while !display.ends_with(b"login: ") {
            match backend.events.recv_blocking() {
                Ok(BackendEvent::Data(bytes)) => display.extend_from_slice(&bytes),
                _ => break,
            }
        }
        assert_eq!(display, b"login: ", "banner should reach the grid intact");

        backend.write_input(b"root\r");
        let (offers, replies) = server.join().expect("server thread");

        for offer in [
            [IAC, WILL, OPT_TTYPE],
            [IAC, WILL, OPT_NAWS],
            [IAC, DO, OPT_ECHO],
        ] {
            assert!(
                offers.windows(3).any(|w| w == offer),
                "client should open with {offer:?}, got {offers:?}"
            );
        }

        let mut ttype = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
        ttype.extend_from_slice(TERM_NAME);
        ttype.extend_from_slice(&[IAC, SE]);
        assert!(
            replies.windows(ttype.len()).any(|w| w == ttype),
            "terminal type should be reported"
        );
        assert!(
            replies
                .windows(9)
                .any(|w| w == [IAC, SB, OPT_NAWS, 0, 80, 0, 24, IAC, SE]),
            "window size should be reported"
        );
        assert_eq!(
            replies.windows(6).filter(|w| *w == b"root\r\0").count(),
            1,
            "typed line should arrive once, NVT padded"
        );
    }

    #[test]
    fn a_hostile_subnegotiation_cannot_grow_without_bound() {
        let mut telnet = connected();

        telnet.receive(&[IAC, SB, OPT_TTYPE]);
        for _ in 0..64 {
            telnet.receive(&[b'x'; 64]);
        }
        assert!(telnet.subneg.len() <= MAX_SUBNEG);
    }
}
