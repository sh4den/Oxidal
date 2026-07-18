use std::io::{Read, Write};
use std::time::Duration;

use super::backend::{Backend, BackendEvent};

pub fn spawn(port_name: String, baud_rate: u32) -> anyhow::Result<Backend> {
    let mut writer = serialport::new(&port_name, baud_rate)
        .timeout(Duration::from_millis(50))
        .open()?;
    let mut reader = writer.try_clone()?;

    let (out_tx, out_rx) = async_channel::unbounded::<BackendEvent>();
    let (in_tx, in_rx) = async_channel::unbounded::<Vec<u8>>();
    let (resize_tx, _resize_rx) = async_channel::unbounded::<(u16, u16)>();

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
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

    std::thread::spawn(move || {
        while let Ok(data) = in_rx.recv_blocking() {
            if writer.write_all(&data).is_err() {
                break;
            }
        }
    });

    Ok(Backend::new(out_rx, in_tx, resize_tx))
}
