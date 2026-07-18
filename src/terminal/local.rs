use std::io::{Read, Write};
use std::sync::Mutex;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use super::backend::{Backend, BackendEvent};

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

pub fn spawn(rows: u16, cols: u16) -> anyhow::Result<Backend> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = CommandBuilder::new(default_shell());
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    if std::env::var_os("LANG").is_none() {
        cmd.env("LANG", "en_US.UTF-8");
    }
    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let writer = pair.master.take_writer()?;
    let mut reader = pair.master.try_clone_reader()?;
    let master = Mutex::new(pair.master);
    let writer = Mutex::new(writer);

    let (out_tx, out_rx) = async_channel::unbounded::<BackendEvent>();
    let (in_tx, in_rx) = async_channel::unbounded::<Vec<u8>>();
    let (resize_tx, resize_rx) = async_channel::unbounded::<(u16, u16)>();

    {
        let out_tx = out_tx.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = out_tx.send_blocking(BackendEvent::Closed(None));
                        break;
                    }
                    Ok(n) => {
                        if out_tx
                            .send_blocking(BackendEvent::Data(buf[..n].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = out_tx.send_blocking(BackendEvent::Closed(Some(e.to_string())));
                        break;
                    }
                }
            }
        });
    }

    std::thread::spawn(move || {
        while let Ok(data) = in_rx.recv_blocking() {
            if writer.lock().unwrap().write_all(&data).is_err() {
                break;
            }
        }
    });

    std::thread::spawn(move || {
        while let Ok((rows, cols)) = resize_rx.recv_blocking() {
            let _ = master.lock().unwrap().resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    });

    std::thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(Backend::new(out_rx, in_tx, resize_tx))
}
