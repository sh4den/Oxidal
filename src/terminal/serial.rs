use std::io::{ErrorKind, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serialport::SerialPort;

use super::backend::{Backend, BackendEvent, EVENT_CAPACITY};

const IO_TIMEOUT: Duration = Duration::from_millis(250);

pub fn spawn(port_name: String, baud_rate: u32) -> anyhow::Result<Backend> {
    let mut writer = serialport::new(&port_name, baud_rate)
        .timeout(IO_TIMEOUT)
        .open()?;
    let mut reader = writer.try_clone()?;

    let (out_tx, out_rx) = async_channel::bounded::<BackendEvent>(EVENT_CAPACITY);
    let (in_tx, in_rx) = async_channel::unbounded::<Vec<u8>>();

    // The port stays open until both handles are dropped, and a quiet port only
    // ever yields read timeouts, so the reader needs an explicit stop signal.
    let stop = Arc::new(AtomicBool::new(false));

    let reader_stop = stop.clone();
    let reader_tx = out_tx.clone();
    let reader_port = port_name.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while !reader_stop.load(Ordering::Relaxed) {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = reader_tx.send_blocking(BackendEvent::Closed(Some(format!(
                        "{reader_port} was disconnected"
                    ))));
                    break;
                }
                Ok(n) => {
                    if reader_tx
                        .send_blocking(BackendEvent::Data(buf[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(e) if is_retryable(&e) => continue,
                Err(e) => {
                    let _ = reader_tx
                        .send_blocking(BackendEvent::Closed(Some(format!("{reader_port}: {e}"))));
                    break;
                }
            }
        }
    });

    let writer_stop = stop.clone();
    std::thread::spawn(move || {
        while let Ok(data) = in_rx.recv_blocking() {
            if writer_stop.load(Ordering::Relaxed) {
                break;
            }
            if let Err(e) = write_all_retrying(writer.as_mut(), &data, &writer_stop) {
                let _ = out_tx.send_blocking(BackendEvent::Closed(Some(format!(
                    "Couldn't write to {port_name}: {e}"
                ))));
                break;
            }
        }
    });

    Ok(Backend::new(out_rx, in_tx, None).on_shutdown(move || stop.store(true, Ordering::Relaxed)))
}

fn is_retryable(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::TimedOut | ErrorKind::Interrupted | ErrorKind::WouldBlock
    )
}

fn write_all_retrying(
    port: &mut dyn SerialPort,
    mut data: &[u8],
    stop: &AtomicBool,
) -> std::io::Result<()> {
    while !data.is_empty() {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        match port.write(data) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::WriteZero,
                    "the port accepted no data",
                ));
            }
            Ok(n) => data = &data[n..],
            Err(e) if is_retryable(&e) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "../../tests/terminal/serial.rs"]
mod tests;
