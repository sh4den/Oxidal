use std::io::{Read, Write};
use std::sync::Mutex;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use super::backend::{Backend, BackendEvent};

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

/// Spawn the user's default shell inside a local pseudo-terminal.
pub fn spawn(rows: u16, cols: u16) -> anyhow::Result<Backend> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let cmd = CommandBuilder::new(default_shell());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::grid::Grid;
    use std::time::{Duration, Instant};

    #[test]
    fn local_shell_echoes_command() {
        let backend = spawn(10, 80).expect("failed to spawn local shell");
        let mut grid = Grid::new(10, 80);

        // Let the shell print its startup prompt first.
        drain_events(&backend, &mut grid, Duration::from_millis(1000));

        backend.write_input(b"echo OXIDAL_TEST_MARKER\r\n");
        drain_events(&backend, &mut grid, Duration::from_millis(2000));

        let text = grid_text(&grid);
        assert!(
            text.contains("OXIDAL_TEST_MARKER"),
            "shell output did not contain the echoed marker:\n{text}"
        );
    }

    /// End-to-end check that a real full-screen TUI (vim, via Git for
    /// Windows) drives the alternate-screen switch correctly through the
    /// actual PTY, and that the shell is usable again after it exits.
    #[test]
    fn real_vim_enters_and_exits_alt_screen() {
        let backend = spawn(15, 80).expect("failed to spawn local shell");
        let mut grid = Grid::new(15, 80);

        drain_events(&backend, &mut grid, Duration::from_millis(1000));
        assert!(!grid.is_alt_screen());

        backend.write_input(b"\"C:\\Program Files\\Git\\usr\\bin\\vim.exe\"\r\n");
        drain_events(&backend, &mut grid, Duration::from_millis(3000));

        assert!(
            grid.is_alt_screen(),
            "vim did not switch to the alternate screen:\n{}",
            grid_text(&grid)
        );
        assert!(
            grid_text(&grid).contains('~'),
            "vim's empty-line tildes were not rendered:\n{}",
            grid_text(&grid)
        );

        // Quit without saving: Escape (in case of a stray mode) then :q!
        backend.write_input(b"\x1b:q!\r");
        drain_events(&backend, &mut grid, Duration::from_millis(2000));

        assert!(
            !grid.is_alt_screen(),
            "grid did not return to the primary screen after vim quit:\n{}",
            grid_text(&grid)
        );

        // The shell must still be alive and responsive afterwards.
        backend.write_input(b"echo OXIDAL_AFTER_VIM\r\n");
        drain_events(&backend, &mut grid, Duration::from_millis(1500));
        assert!(
            grid_text(&grid).contains("OXIDAL_AFTER_VIM"),
            "shell did not respond after vim exited:\n{}",
            grid_text(&grid)
        );
    }

    /// Confirms `Backend::resize` actually reaches the real PTY/console —
    /// not just our own `Grid` model — by asking `cmd.exe` to report its
    /// console size (`mode con`) before and after resizing.
    #[test]
    fn backend_resize_reaches_the_real_pty() {
        let backend = spawn(20, 60).expect("failed to spawn local shell");
        let mut grid = Grid::new(20, 60);
        drain_events(&backend, &mut grid, Duration::from_millis(1000));

        backend.write_input(b"mode con\r\n");
        drain_events(&backend, &mut grid, Duration::from_millis(1000));
        let (before_lines, before_cols) =
            parse_mode_con(&grid_text(&grid)).expect("could not parse initial `mode con` output");
        assert_eq!((before_lines, before_cols), (20, 60));

        backend.resize(30, 100);
        // Give the console a moment to pick up the resize, then ask again.
        std::thread::sleep(Duration::from_millis(200));
        backend.write_input(b"mode con\r\n");
        drain_events(&backend, &mut grid, Duration::from_millis(1000));
        let (after_lines, after_cols) =
            parse_mode_con(&grid_text(&grid)).expect("could not parse post-resize `mode con` output");

        assert_eq!(
            (after_lines, after_cols),
            (30, 100),
            "PTY did not report the new size after Backend::resize"
        );
    }

    /// Parses the last `Lines:` / `Columns:` pair out of `mode con` output.
    fn parse_mode_con(text: &str) -> Option<(u32, u32)> {
        let mut lines = None;
        let mut cols = None;
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("Lines:") {
                lines = rest.trim().parse().ok();
            } else if let Some(rest) = line.strip_prefix("Columns:") {
                cols = rest.trim().parse().ok();
            }
        }
        Some((lines?, cols?))
    }

    fn drain_events(backend: &Backend, grid: &mut Grid, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match backend.events.try_recv() {
                Ok(BackendEvent::Data(bytes)) => grid.feed(&bytes),
                Ok(BackendEvent::Closed(_)) => break,
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    fn grid_text(grid: &Grid) -> String {
        let mut s = String::new();
        for row in 0..grid.rows {
            for col in 0..grid.cols {
                s.push(grid.cell(row, col).ch);
            }
            s.push('\n');
        }
        s
    }
}
