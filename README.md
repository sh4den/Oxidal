<p align="center">
  <a href="https://github.com/sh4den/Oxidal/releases/latest" target="_blank">
    <img src="https://img.shields.io/badge/Download%20Latest-Oxidal-e43717?style=for-the-badge&logo=github" alt="Download Oxidal" style="margin-bottom: 20px;"/>
  </a>
</p>

<div align="center">
  <h1>Oxidal</h1>
  <p><b>Oxidal</b>: every session in one window. SSH, SFTP, serial and local shells in a single native app.</p>
  <p>
    <img src="https://img.shields.io/badge/Built%20with-Rust-e43717?style=flat-square&logo=rust&logoColor=white" alt="Built with Rust"/>
    <img src="https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-informational?style=flat-square" alt="Platforms"/>
    <img src="https://img.shields.io/badge/status-early%20days-yellow?style=flat-square" alt="Status"/>
  </p>
</div>

<hr/>

## Why Oxidal?

MobaXterm nailed the idea of one window for every remote session, but it only runs on Windows and you cannot read a line of its source. Oxidal takes the same layout and rebuilds it as an open, native app that behaves the same on Windows, macOS and Linux.

Your sessions live in a plain JSON file you can read, diff and back up yourself. Passwords go to your operating system's credential vault, not to that file. There is no Electron runtime underneath, just a Rust binary drawing through the GPU.

## Features

**Tabbed sessions.** SSH, SFTP, serial and local shells, open side by side in one window, organized into folders in the sidebar.

**SSH with a file browser attached.** Opening an SSH session docks an SFTP panel next to the terminal, the way MobaXterm does it. Each side runs over its own connection, so a slow file transfer never blocks your shell. The panel is resizable, shows size, modified date and permissions columns, has an editable path bar, and can show or hide dotfiles per session. Double clicking a remote file downloads it and opens it with whatever your OS uses for that extension.

**A real terminal emulator.** Built directly on `vte` rather than shelling out to one. It handles 256 color and truecolor, scroll regions, line and character insert or delete, and the alternate screen buffer, so vim, htop and less all behave.

**Live remote monitoring.** SSH sessions get a status strip with CPU, memory, network and disk usage sampled over the same connection. Hovering the disk segment breaks it down per filesystem.

**A session dialog that helps.** Session kinds as tiles across the top, a Test Connection button that actually authenticates, a private key picker that finds the keys in your `~/.ssh`, and a dropdown of the serial ports currently plugged in, with the device names next to them.

**SFTP that does the boring parts.** Browse, upload, download, rename, delete and create folders, with live transfer progress.

**Serial console.** Pick a detected port from the list, any baud rate, defaulting to 115200.

**Keys or passwords.** Point a session at a private key and it gets tried first, with password auth as the fallback. Connections send keepalives, so an idle shell or file browser stays up.

**Make it yours.** Light and dark themes, a searchable picker over every font your system has, and a window opacity slider if you like your terminal glassy.

**Small in memory.** Every terminal cell is packed into 12 bytes, so screen buffers stay cheap even with a stack of tabs open.

## Screenshots

COMING SOON

## Getting Started

### Requirements

Rust 1.85 or newer, since the crate is on edition 2024. Oxidal pulls GPUI straight from the Zed repository, so the first build compiles a large dependency tree and takes a while. Grab a coffee. If you are on Linux and the build complains about missing system libraries, Zed's [Linux dependency list](https://github.com/zed-industries/zed/blob/main/docs/src/development/linux.md) covers what GPUI needs; serial port support wants `libudev` and password storage wants the D-Bus development headers (`libdbus-1-dev` on Debian-likes) on top of that.

### Build and run

```sh
git clone https://github.com/sh4den/Oxidal.git
cd Oxidal
cargo run --release
```

The debug profile works too, but the terminal feels noticeably better on `--release`.

### Usage

1. Hit **New Session** in the sidebar and pick a kind from the tiles: SSH, SFTP, RDP, Serial or Local.
2. Fill in what that kind needs. Host, username and port for SSH and SFTP, or a port and baud rate for serial. Local needs nothing. **Test Connection** tells you whether the details work before you save them.
3. Double click the session to connect, or use the connect button on its row. Single click just selects it.
4. SSH sessions open with the file browser on the left and the shell on the right. Drag the divider to taste.

Font, size, light or dark mode and window opacity live in the Settings tab.

## Configuration

Sessions and preferences are written as JSON under a per user config directory:

| Platform | Location |
| --- | --- |
| Windows | `%APPDATA%\Oxidal\` |
| macOS | `~/Library/Application Support/Oxidal/` |
| Linux | `~/.config/Oxidal/` |

`sessions.json` holds your saved connections, `folders.json` the sidebar folders and `settings.json` the appearance settings. All plain text, so version them or sync them however you like. Passwords are deliberately absent from all three; see below.

## Security

Worth being straight with you about where this stands.

**Passwords go to your OS credential vault.** Windows Credential Manager, the macOS Keychain, or the Secret Service on Linux, one entry per session, keyed by the session's id. They are never written to `sessions.json`, and deleting a session deletes its entry. Private keys are read from the path you configure and never copied anywhere.

**Host key verification is not implemented yet.** Oxidal currently accepts whatever host key a server presents instead of checking it against a known_hosts store. That leaves SSH and SFTP sessions open to a machine in the middle on a network you do not trust. It is the next thing on the list, but until it lands, plan accordingly.

## Status

SSH, SFTP, serial and local shells all work. RDP shows up in the session list because the groundwork is there, but it has no backend behind it yet and will tell you so.

This is early software. Expect rough edges.

## Contributing

Issues and pull requests are welcome. Bug reports with the escape sequences or the session kind that triggered them are especially useful, since terminal emulation has a long tail of edge cases and the fastest way to fix one is to reproduce it.

## Acknowledgements

* [GPUI](https://github.com/zed-industries/zed) for the GPU accelerated UI framework
* [gpui-component](https://github.com/longbridge/gpui-component) for the widget set
* [russh](https://github.com/Eugeny/russh) and [russh-sftp](https://github.com/AspectUnk/russh-sftp) for SSH and SFTP
* [vte](https://github.com/alacritty/vte) for the escape sequence parser
* [portable-pty](https://github.com/wez/wezterm) for local shells across platforms
* [serialport-rs](https://github.com/serialport/serialport-rs) for the serial backend
* [keyring-rs](https://github.com/open-source-cooperative/keyring-rs) for talking to the platform credential stores
* [Rust](https://www.rust-lang.org/), for making all of the above worth writing

---

<div align="center">
  <p>Built for people who don't want to run a whole browser for a terminal</p>
</div>
