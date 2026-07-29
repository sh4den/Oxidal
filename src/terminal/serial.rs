use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::backend::{Backend, BackendEvent};

pub fn spawn(port_name: String, baud_rate: u32) -> anyhow::Result<Backend> {
    let mut writer = serialport::new(&port_name, baud_rate)
        .timeout(Duration::from_millis(50))
        .open()?;
    let mut reader = writer.try_clone()?;

    let (out_tx, out_rx) = async_channel::unbounded::<BackendEvent>();
    let (in_tx, in_rx) = async_channel::unbounded::<Vec<u8>>();

    // The port stays open until both handles are dropped, and a quiet port only
    // ever yields read timeouts, so the reader needs an explicit stop signal.
    let stop = Arc::new(AtomicBool::new(false));

    let reader_stop = stop.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while !reader_stop.load(Ordering::Relaxed) {
            match reader.read(&mut buf) {
                Ok(0) => continue,
                Ok(n) => {
                    if out_tx
                        .send_blocking(BackendEvent::Data(buf[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) => {
                    let _ = out_tx.send_blocking(BackendEvent::Closed(Some(e.to_string())));
                    break;
                }
            }
        }
    });

    let writer_stop = stop.clone();
    std::thread::spawn(move || {
        while let Ok(data) = in_rx.recv_blocking() {
            if writer_stop.load(Ordering::Relaxed) || writer.write_all(&data).is_err() {
                break;
            }
        }
    });

    Ok(Backend::new(out_rx, in_tx, None).on_shutdown(move || stop.store(true, Ordering::Relaxed)))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serialport::SerialPort as _;

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
}
