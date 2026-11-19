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
        b'h', b'i', IAC, DO, OPT_TTYPE, b' ', IAC, IAC, b'!', IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC,
        SE, b'?',
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

    let backend = spawn(addr.ip().to_string(), addr.port(), None, 24, 80);

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
