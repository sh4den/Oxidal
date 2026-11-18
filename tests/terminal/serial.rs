use std::io::Write as _;

use super::*;

// macOS rejects baud-rate changes on a pty, and 0 means "leave it alone".
#[cfg(any(target_os = "macos", target_os = "ios"))]
const TEST_BAUD: u32 = 0;
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
const TEST_BAUD: u32 = 9600;

fn reopen(path: &str) -> serialport::Result<Box<dyn serialport::SerialPort>> {
    serialport::new(path, TEST_BAUD)
        .timeout(Duration::from_millis(50))
        .open()
}

fn next_event(backend: &Backend, within: Duration) -> Option<BackendEvent> {
    let deadline = std::time::Instant::now() + within;
    loop {
        match backend.events.try_recv() {
            Ok(event) => return Some(event),
            Err(async_channel::TryRecvError::Closed) => return None,
            Err(async_channel::TryRecvError::Empty) => {
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[test]
fn closing_a_session_releases_the_port() {
    let (_master, slave) = serialport::TTYPort::pair().expect("pty pair");
    let path = slave.name().expect("slave port name");
    // Hand the port over untouched, the way a real device would be found.
    drop(slave);

    let backend = spawn(path.clone(), TEST_BAUD).expect("open serial port");
    assert!(
        reopen(&path).is_err(),
        "port should be held while the session is open"
    );

    drop(backend);

    // The reader only notices the shutdown between read timeouts.
    let released = (0..40).any(|_| {
        std::thread::sleep(Duration::from_millis(25));
        reopen(&path).is_ok()
    });
    assert!(
        released,
        "port should be released once the session is closed"
    );
}

#[test]
fn data_flows_both_ways() {
    let (mut master, slave) = serialport::TTYPort::pair().expect("pty pair");
    let path = slave.name().expect("slave port name");
    drop(slave);
    master.set_timeout(Duration::from_secs(2)).expect("timeout");

    let backend = spawn(path, TEST_BAUD).expect("open serial port");

    master.write_all(b"hello").expect("write to device side");
    let mut received = Vec::new();
    while received.len() < 5 {
        match next_event(&backend, Duration::from_secs(2)) {
            Some(BackendEvent::Data(bytes)) => received.extend(bytes),
            other => panic!("expected data, got {:?}", other.map(describe)),
        }
    }
    assert_eq!(received, b"hello");

    backend.write_input(b"world");
    let mut echoed = [0u8; 5];
    master
        .read_exact(&mut echoed)
        .expect("read from device side");
    assert_eq!(&echoed, b"world");
}

#[test]
fn losing_the_device_ends_the_session() {
    let (master, slave) = serialport::TTYPort::pair().expect("pty pair");
    let path = slave.name().expect("slave port name");
    drop(slave);

    let backend = spawn(path, TEST_BAUD).expect("open serial port");
    drop(master);

    let closed = loop {
        match next_event(&backend, Duration::from_secs(2)) {
            Some(BackendEvent::Closed(message)) => break message,
            Some(BackendEvent::Data(_)) => continue,
            None => panic!("the session should report the lost device instead of idling"),
        }
    };
    assert!(
        closed.is_some(),
        "a lost device should come with an explanation"
    );
}

fn describe(event: BackendEvent) -> String {
    match event {
        BackendEvent::Data(bytes) => format!("Data({bytes:?})"),
        BackendEvent::Closed(message) => format!("Closed({message:?})"),
    }
}
