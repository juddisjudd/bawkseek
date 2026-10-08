# bawkseek

A Soulseek desktop client for Windows, built in Rust with [GPUI Kit](https://gpui-kit.com). It looks and feels like [bawkterm](https://bawkterm.com).

> Status: early. Search and downloads work. Sharing, browsing, chat and messages come later.

## What works

- **Log in**: any free name and a password. The server makes the account on first login. "Remember me" keeps the password in Windows Credential Manager.
- **Search**: results arrive live and group by folder, with the format and quality of each folder (flac 24/96, mp3 320). Sort by speed, folder, user, file count or size. Filter by text (`-word` excludes), or show free slots only. Each search stays open in its own tab.
- **Download**: a single file, or a whole folder. A folder download asks the user for the complete folder listing and keeps subfolders such as `CD1` or `Scans`.
- **Transfers**: progress, speed and queue place per file, grouped by folder. Pause, resume, retry, cancel, open the folder.
- **Reconnects**: a dropped server connection reconnects by itself. Queued downloads resume when their peer is back.
- **Settings**: download folder, listening port, dark or light theme.

Not built yet: sharing and uploads, browsing a user's files, chat rooms, private messages, wishlist, UPnP port mapping.

## Build

You need:

1. Rust 1.92 or later, with the MSVC toolchain.
2. Visual Studio 2022 Build Tools with the **Desktop development with C++** workload.
3. CMake on `PATH`.

Then:

```sh
cargo run              # debug build
cargo build --release  # target/release/bawkseek.exe
cargo test
```

The first build compiles GPUI and takes a few minutes.

## Where things live

| What | Where |
| --- | --- |
| Settings | `%APPDATA%\bawkseek\config.json` |
| Saved password | Windows Credential Manager, service `bawkseek` |
| Downloads | `Downloads\bawkseek`, one folder per album (change it under settings) |

## Network

bawkseek listens on port 2234 for other users. When that port is busy, it picks a free one and tells you. Windows asks once whether to allow it through the firewall.

There is no UPnP yet. If your router does not forward the port, most transfers still work, because peers fall back to connecting through the server. Forwarding the port makes them faster to start.

To see the protocol traffic, set two environment variables before you start it:

```powershell
$env:LOG_LEVEL = "DEBUG"; $env:LOG_FILE = "$env:TEMP\bawkseek.log"; cargo run
```

## Credits

- [soulseek-rs-lib](https://github.com/michel/soulseek-rs) (MIT) speaks the Soulseek protocol.
- [GPUI Kit](https://github.com/longbridge/gpui-kit) (Apache-2.0) and Zed's GPUI draw the interface.
- [IBM Plex Mono](https://github.com/IBM/plex) (SIL Open Font License, `assets/fonts/OFL.txt`) is the typeface.
- [Lucide](https://lucide.dev) (ISC) icons.

Please share back what you download, once sharing lands here or with another client.

## License

[AGPL-3.0](LICENSE).
