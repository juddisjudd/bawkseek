# bawkseek

A Soulseek desktop client for Windows, built in Rust with [GPUI Kit](https://gpui-kit.com). It looks and feels like [bawkterm](https://bawkterm.com).

> Status: early. Search, downloads, sharing, browsing, messages, chat rooms and buddies work. Wishlist and recommendations come next.

## What works

- **Log in**: any free name and a password. The server makes the account on first login. "Remember me" keeps the password in Windows Credential Manager.
- **Search**: results arrive live and group by folder, with the format and quality of each folder (flac 24/96, mp3 320). Sort by speed, folder, user, file count or size. Filter by text (`-word` excludes), or show free slots only. Each search stays open in its own tab.
- **Download**: a single file, or a whole folder. A folder download asks the user for the complete folder listing and keeps subfolders such as `CD1` or `Scans`.
- **Browse**: open any user's shared folders as a tree, from the browse page or by clicking a username in search, transfers or uploads. Filter by folder or file name, download single files or a whole folder with its subfolders. Folders with more than 200 files ask before downloading.
- **Messages**: private conversations with any user, even offline ones. Unread counts in the sidebar, a toast for new messages, and logs saved on this computer per account (`%APPDATA%\bawkseek\messages`).
- **Rooms**: the public room list, sorted by size and filterable. Join several rooms at once, each in its own tab with members, tickers and unread counts. Set your own ticker, create private rooms, and rejoin automatically after a reconnect.
- **Users**: look anyone up for their status, shares, speed, upload slots and queue. Keep buddies with live online, away and offline status. Ignore a user to hide their messages and room lines (the network gives no way to refuse their downloads). Set yourself away from settings.
- **User menu**: right-click any username to browse their shares, send a message, see their info, add or remove them as a buddy, or ignore them.
- **Transfers**: progress, speed and queue place per file, grouped by folder. Pause, resume, retry, cancel, open the folder.
- **Sharing**: share any number of folders from the uploads page. They are scanned in the background after login, so a big library never delays it. Other users see each folder by its name: `D:\Music` appears as `Music`. A folder inside one you already share is refused. Everything in a shared folder is shared, hidden files included.
- **Uploads**: who downloads from you, with progress, speed and queue place. Cancel a running upload, clear finished ones, and set how many people can download at once (upload slots, 1 to 50, default 10).
- **Reconnects**: a dropped server connection reconnects by itself. Queued downloads resume when their peer is back.
- **Settings**: download folder, listening port, dark or light theme.

Not built yet: wishlist, interests and recommendations, UPnP port mapping, an upload speed limit, banning users.

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
| Settings and shared folders | `%APPDATA%\bawkseek\config.json` |
| Saved password | Windows Credential Manager, service `bawkseek` |
| Downloads | `Downloads\bawkseek`, one folder per album (change it under settings) |

## Network

bawkseek listens on port 2234 for other users. When that port is busy, it picks a free one and tells you. Windows asks once whether to allow it through the firewall.

Forward TCP port 2234 on your router to this PC. There is no UPnP yet, so bawkseek cannot open the port for you. Without the forward, downloads from reachable users still work. But a user who cannot be reached either gets none of your search results and cannot download from you.

To see the protocol traffic, set two environment variables before you start it:

```powershell
$env:LOG_LEVEL = "DEBUG"; $env:LOG_FILE = "$env:TEMP\bawkseek.log"; cargo run
```

## Credits

- [soulseek-rs-lib](https://github.com/michel/soulseek-rs) (MIT) speaks the Soulseek protocol.
- [GPUI Kit](https://github.com/longbridge/gpui-kit) (Apache-2.0) and Zed's GPUI draw the interface.
- [IBM Plex Mono](https://github.com/IBM/plex) (SIL Open Font License, `assets/fonts/OFL.txt`) is the typeface.
- [Lucide](https://lucide.dev) (ISC) icons.

Please share back what you download.

## License

[AGPL-3.0](LICENSE).
