use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use soulseek_rs::types::DownloadMetadata;
use soulseek_rs::utils::logger::{self, LogLevel};
use soulseek_rs::{Client, ClientSettings, DownloadStatus, SessionLoss, SoulseekRs};

use super::browse::Listing;
use super::group::group;
use super::sharing::{Scanner, Uploads};
use super::{Command, DlState, DownloadRow, Event, NoticeLevel, Status, Wanted};
use crate::format;

const TICK: Duration = Duration::from_millis(150);
const SEARCH_REFRESH: Duration = Duration::from_millis(700);
const DOWNLOADS_REFRESH: Duration = Duration::from_millis(250);
const QUEUE_REFRESH: Duration = Duration::from_secs(3);
const PING_EVERY: Duration = Duration::from_secs(60);
const FOLDER_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const BROWSE_TIMEOUT: Duration = Duration::from_secs(90);

pub fn spawn() -> (Sender<Command>, Receiver<Event>) {
    let (command_tx, command_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    thread::Builder::new()
        .name("soulseek".into())
        .spawn(move || Worker::new(event_tx).run(command_rx))
        .expect("spawn network thread");
    (command_tx, event_rx)
}

struct Credentials {
    username: String,
    password: String,
    listen_port: u16,
}

struct SearchPoll {
    responses: usize,
    fetched: Option<Instant>,
}

struct FolderRequest {
    username: String,
    folder: String,
    fallback: Vec<Wanted>,
    deadline: Instant,
}

struct Row {
    view: DownloadRow,
    wanted: Wanted,
    status: Option<Receiver<DownloadStatus>>,
}

struct Reconnect {
    attempt: u32,
    at: Instant,
}

struct Worker {
    events: Sender<Event>,
    client: Option<Arc<Client>>,
    download_dir: PathBuf,
    searches: HashMap<String, SearchPoll>,
    folders: Vec<FolderRequest>,
    rows: Vec<Row>,
    next_id: u64,
    rows_dirty: bool,
    rows_sent: Instant,
    queue_checked: Instant,
    pinged: Instant,
    reconnect: Option<Reconnect>,
    displaced: bool,
    shares: Vec<PathBuf>,
    upload_slots: usize,
    scanner: Scanner,
    uploads: Uploads,
    browses: Vec<(String, Instant)>,
}

impl Worker {
    fn new(events: Sender<Event>) -> Self {
        logger::init();
        if std::env::var_os("LOG_LEVEL").is_none() && std::env::var_os("RUST_LOG").is_none() {
            logger::set_log_level(LogLevel::Error);
        }
        let now = Instant::now();
        Self {
            scanner: Scanner::spawn(events.clone()),
            events,
            client: None,
            download_dir: PathBuf::new(),
            searches: HashMap::new(),
            folders: Vec::new(),
            rows: Vec::new(),
            next_id: 1,
            rows_dirty: false,
            rows_sent: now,
            queue_checked: now,
            pinged: now,
            reconnect: None,
            displaced: false,
            shares: Vec::new(),
            upload_slots: crate::config::DEFAULT_UPLOAD_SLOTS,
            uploads: Uploads::default(),
            browses: Vec::new(),
        }
    }

    fn run(mut self, commands: Receiver<Command>) {
        loop {
            match commands.recv_timeout(TICK) {
                Ok(command) => self.handle(command),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            while let Ok(command) = commands.try_recv() {
                self.handle(command);
            }
            self.tick();
        }
        self.client = None;
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn notice(&self, level: NoticeLevel, text: impl Into<String>) {
        self.emit(Event::Notice(level, text.into()));
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Login {
                username,
                password,
                listen_port,
                download_dir,
                shares,
                upload_slots,
            } => {
                self.download_dir = download_dir;
                self.shares = shares;
                self.upload_slots = upload_slots;
                self.login(Credentials {
                    username,
                    password,
                    listen_port,
                });
            }
            Command::Logout => self.logout(),
            Command::Reconnect => self.reconnect_now(),
            Command::Search(query) => self.search(query),
            Command::ForgetSearch(query) => {
                self.searches.remove(&query);
                if let Some(client) = &self.client {
                    let _ = client.forget_search(&query);
                }
            }
            Command::Download(files) => {
                let added = files
                    .into_iter()
                    .filter(|file| self.enqueue(file.clone()))
                    .count();
                if added > 1 {
                    self.notice(NoticeLevel::Info, format!("queued {added} files"));
                }
            }
            Command::DownloadFolder {
                username,
                folder,
                fallback,
            } => self.request_folder(username, folder, fallback),
            Command::Pause(id) => self.with_row(id, |client, row| {
                let _ = client.pause_download(&row.view.username, &row.view.filename);
            }),
            Command::Resume(id) => self.with_row(id, |client, row| {
                let _ = client.resume_download(&row.view.username, &row.view.filename);
            }),
            Command::Retry(id) => {
                if let Some(ix) = self.rows.iter().position(|row| row.view.id == id) {
                    self.start(ix);
                }
            }
            Command::Cancel(id) => self.with_row(id, |client, row| {
                let _ = client.cancel_download(&row.view.username, &row.view.filename);
                row.view.state = DlState::Cancelled;
                row.status = None;
            }),
            Command::Remove(id) => {
                if let Some(ix) = self.rows.iter().position(|row| row.view.id == id) {
                    let row = self.rows.remove(ix);
                    if let Some(client) = &self.client {
                        let _ = client.remove_download(&row.view.username, &row.view.filename);
                    }
                    self.rows_dirty = true;
                }
            }
            Command::ClearFinished => {
                let client = self.client.clone();
                self.rows.retain(|row| {
                    let finished =
                        matches!(row.view.state, DlState::Completed | DlState::Cancelled);
                    if finished && let Some(client) = &client {
                        let _ = client.remove_download(&row.view.username, &row.view.filename);
                    }
                    !finished
                });
                self.rows_dirty = true;
            }
            Command::SetDownloadDir(dir) => self.download_dir = dir,
            Command::SetShares(dirs) => {
                self.shares = dirs;
                self.rescan();
            }
            Command::Rescan => self.rescan(),
            Command::SetUploadSlots(slots) => {
                self.upload_slots = slots;
                if let Some(client) = &self.client {
                    client.set_upload_slots(slots);
                }
            }
            Command::CancelUpload { username, filename } => {
                if let Some(client) = &self.client {
                    let _ = client.cancel_upload(&username, &filename);
                }
            }
            Command::Browse(username) => self.browse(username),
            Command::SendMessage { username, text } => {
                let result = match &self.client {
                    Some(client) => client.send_private_message(&username, &text),
                    None => Err(SoulseekRs::NotConnected),
                };
                if let Err(err) = result {
                    self.notice(
                        NoticeLevel::Warning,
                        format!("could not send to {username}: {}", describe(&err)),
                    );
                }
            }
            Command::DownloadTree { root, files } => self.enqueue_tree(&root, files),
            Command::ClearUploads => {
                if let Some(client) = &self.client {
                    self.uploads.clear_finished(client);
                }
            }
        }
    }

    fn with_row(&mut self, id: u64, f: impl FnOnce(&Client, &mut Row)) {
        let Some(client) = self.client.clone() else {
            return;
        };
        if let Some(row) = self.rows.iter_mut().find(|row| row.view.id == id) {
            f(&client, row);
            self.rows_dirty = true;
        }
    }

    fn login(&mut self, credentials: Credentials) {
        self.emit(Event::Status(Status::Connecting));
        self.drop_client();
        self.reconnect = None;
        self.displaced = false;

        let mut settings = ClientSettings::new(&credentials.username, &credentials.password);
        settings.listen_port = credentials.listen_port;
        let mut client = Client::with_settings(settings);
        if let Err(err) = client.connect() {
            self.emit(Event::Status(Status::Failed(describe(&err))));
            return;
        }
        match client.login() {
            Ok(true) => {
                self.emit(Event::LoggedIn(credentials.username.clone()));
                self.emit(Event::Status(Status::Online));
                if client.listen_port() != Some(credentials.listen_port) {
                    self.notice(
                        NoticeLevel::Warning,
                        format!(
                            "port {} was busy, listening on {}",
                            credentials.listen_port,
                            client
                                .listen_port()
                                .map_or("none".into(), |port| port.to_string())
                        ),
                    );
                }
                client.set_upload_slots(self.upload_slots);
                self.client = Some(Arc::new(client));
                self.pinged = Instant::now();
                self.rescan();
            }
            Ok(false) => self.emit(Event::Status(Status::Failed(
                "the server refused the login".into(),
            ))),
            Err(err) => self.emit(Event::Status(Status::Failed(describe(&err)))),
        }
    }

    /// Shares are scanned after login, so a slow disk never delays it, and again after a reconnect resets the server's counts.
    fn rescan(&self) {
        if let Some(client) = &self.client {
            self.scanner.scan(client.clone(), &self.shares);
        }
    }

    fn browse(&mut self, username: String) {
        let Some(client) = &self.client else {
            return;
        };
        match client.browse_user(&username) {
            Ok(()) => {
                self.browses.retain(|(user, _)| *user != username);
                self.browses
                    .push((username, Instant::now() + BROWSE_TIMEOUT));
            }
            Err(err) => self.emit(Event::Browse {
                username,
                result: Err(describe(&err)),
            }),
        }
    }

    /// Big listings take a moment to parse, so the tree is built off the worker thread.
    fn poll_browses(&mut self, client: &Client) {
        let now = Instant::now();
        let events = self.events.clone();
        self.browses.retain(|(username, deadline)| {
            if let Some(dirs) = client.take_browse_result(username) {
                let (events, username) = (events.clone(), username.clone());
                thread::spawn(move || {
                    let listing = Listing::build(dirs);
                    let _ = events.send(Event::Browse {
                        username,
                        result: Ok(Arc::new(listing)),
                    });
                });
                false
            } else if now >= *deadline {
                let _ = events.send(Event::Browse {
                    username: username.clone(),
                    result: Err(format!(
                        "no answer from {username}. they may be offline or unreachable."
                    )),
                });
                false
            } else {
                true
            }
        });
    }

    fn drop_client(&mut self) {
        self.scanner.invalidate();
        self.uploads.reset();
        self.browses.clear();
        self.client = None;
    }

    fn logout(&mut self) {
        self.drop_client();
        self.reconnect = None;
        self.searches.clear();
        self.folders.clear();
        self.rows.clear();
        self.rows_dirty = false;
        self.emit(Event::Status(Status::Offline));
    }

    fn search(&mut self, query: String) {
        let Some(client) = &self.client else {
            return;
        };
        match client.search(&query, Duration::ZERO) {
            Ok(_) => {
                self.searches.insert(
                    query,
                    SearchPoll {
                        responses: 0,
                        fetched: None,
                    },
                );
            }
            Err(err) => self.notice(
                NoticeLevel::Warning,
                format!("search failed: {}", describe(&err)),
            ),
        }
    }

    fn request_folder(&mut self, username: String, folder: String, fallback: Vec<Wanted>) {
        let Some(client) = &self.client else {
            return;
        };
        if client.request_folder_contents(&username, &folder).is_err() {
            for file in fallback {
                self.enqueue(file);
            }
            return;
        }
        self.notice(
            NoticeLevel::Info,
            format!("asking {username} for {}", format::split_path(&folder).1),
        );
        self.folders.push(FolderRequest {
            username,
            folder,
            fallback,
            deadline: Instant::now() + FOLDER_TIMEOUT,
        });
    }

    fn enqueue(&mut self, wanted: Wanted) -> bool {
        self.enqueue_to(wanted, None)
    }

    /// Adds a download row and starts it. Returns false when it is already queued.
    fn enqueue_to(&mut self, wanted: Wanted, dir: Option<PathBuf>) -> bool {
        if let Some(ix) = self.rows.iter().position(|row| {
            row.view.username == wanted.username && row.view.filename == wanted.filename
        }) {
            let state = &self.rows[ix].view.state;
            if state.is_live() || *state == DlState::Completed {
                return false;
            }
            self.start(ix);
            return true;
        }

        let (folder, name) = format::split_path(&wanted.filename);
        let folder = format::split_path(folder).1.to_string();
        let local_dir = dir.unwrap_or_else(|| self.local_dir(&wanted.username, &folder, name));
        let id = self.next_id;
        self.next_id += 1;
        self.rows.push(Row {
            view: DownloadRow {
                id,
                username: wanted.username.clone(),
                filename: wanted.filename.clone(),
                name: name.to_string(),
                folder,
                local_dir,
                size: wanted.size,
                state: DlState::Queued { position: None },
            },
            wanted,
            status: None,
        });
        self.start(self.rows.len() - 1);
        true
    }

    /// Picks a folder under the download directory, so two users' files with one name never share a `.part`.
    fn local_dir(&self, username: &str, folder: &str, name: &str) -> PathBuf {
        let base = if folder.is_empty() {
            self.download_dir.clone()
        } else {
            self.download_dir.join(sanitize(folder))
        };
        let clash = self.rows.iter().any(|row| {
            row.view.local_dir == base && row.view.name == name && row.view.username != username
        });
        if clash {
            self.download_dir
                .join(sanitize(&format!("{folder} ({username})")))
        } else {
            base
        }
    }

    fn start(&mut self, ix: usize) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let row = &mut self.rows[ix];
        let dir = row.view.local_dir.to_string_lossy().into_owned();
        let metadata = DownloadMetadata {
            bitrate: row.wanted.bitrate,
            length_seconds: row.wanted.duration,
            ..Default::default()
        };
        match client.download_with_metadata(
            row.wanted.filename.clone(),
            row.wanted.username.clone(),
            row.wanted.size,
            dir,
            metadata,
        ) {
            Ok((_, status)) => {
                row.status = Some(status);
                row.view.state = DlState::Queued { position: None };
            }
            Err(err) => {
                row.status = None;
                row.view.state = DlState::Failed(describe(&err));
            }
        }
        self.rows_dirty = true;
    }

    fn tick(&mut self) {
        self.watch_session();
        let Some(client) = self.client.clone() else {
            return;
        };
        if !self.displaced && self.reconnect.is_none() && self.pinged.elapsed() >= PING_EVERY {
            self.pinged = Instant::now();
            let _ = client.ping_server();
        }
        self.poll_searches(&client);
        self.poll_folders(&client);
        self.poll_downloads(&client);
        self.poll_browses(&client);
        for message in client.take_private_messages() {
            self.emit(Event::Message {
                username: message.username().to_string(),
                text: message.message().to_string(),
                at: i64::from(message.timestamp()),
                new: message.is_new(),
            });
        }
        if let Some(rows) = self.uploads.poll(&client) {
            self.emit(Event::Uploads(rows));
        }
    }

    fn watch_session(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        if self.displaced {
            return;
        }
        match client.session_loss() {
            None => {
                if self.reconnect.take().is_some() {
                    self.emit(Event::Status(Status::Online));
                }
            }
            Some(SessionLoss::Displaced) => {
                self.displaced = true;
                self.reconnect = None;
                self.emit(Event::Status(Status::Displaced));
            }
            Some(SessionLoss::Disconnected) => {
                let reconnect = self.reconnect.get_or_insert_with(|| Reconnect {
                    attempt: 0,
                    at: Instant::now(),
                });
                if Instant::now() < reconnect.at {
                    return;
                }
                reconnect.attempt += 1;
                let attempt = reconnect.attempt;
                self.emit(Event::Status(Status::Reconnecting { attempt }));
                if matches!(client.login(), Ok(true)) {
                    self.reconnect = None;
                    self.emit(Event::Status(Status::Online));
                    self.requeue_peers(&client);
                    self.rescan();
                } else {
                    let backoff =
                        Duration::from_secs(5 * 2u64.pow(attempt.min(4))).min(MAX_BACKOFF);
                    if let Some(reconnect) = &mut self.reconnect {
                        reconnect.at = Instant::now() + backoff;
                    }
                }
            }
        }
    }

    /// Queued rows are only re-sent once a connection to their peer exists again.
    fn requeue_peers(&self, client: &Client) {
        let mut users: Vec<&str> = self
            .rows
            .iter()
            .filter(|row| matches!(row.view.state, DlState::Queued { .. }))
            .map(|row| row.view.username.as_str())
            .collect();
        users.sort_unstable();
        users.dedup();
        for user in users {
            let _ = client.connect_peer(user);
        }
    }

    /// Logs the existing client in again, so running transfers keep their channels.
    fn reconnect_now(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.emit(Event::Status(Status::Reconnecting { attempt: 1 }));
        match client.login() {
            Ok(true) => {
                self.displaced = false;
                self.reconnect = None;
                self.emit(Event::Status(Status::Online));
                self.requeue_peers(&client);
                self.rescan();
            }
            Ok(false) => self.reconnect_failed("the server refused the login".into()),
            Err(err) => self.reconnect_failed(describe(&err)),
        }
    }

    fn reconnect_failed(&mut self, reason: String) {
        let status = if self.displaced {
            Status::Displaced
        } else {
            Status::Reconnecting { attempt: 1 }
        };
        self.emit(Event::Status(status));
        self.notice(
            NoticeLevel::Warning,
            format!("could not log in again: {reason}"),
        );
    }

    fn poll_searches(&mut self, client: &Client) {
        for (query, poll) in &mut self.searches {
            if poll.fetched.is_some_and(|at| at.elapsed() < SEARCH_REFRESH) {
                continue;
            }
            let responses = client.get_search_results_count(query);
            if responses == poll.responses {
                continue;
            }
            poll.responses = responses;
            poll.fetched = Some(Instant::now());
            let hits = group(&client.get_search_results(query));
            let _ = self.events.send(Event::Search {
                query: query.clone(),
                hits: Arc::new(hits),
            });
        }
    }

    /// Downloads a remote folder into one local folder named after it, keeping its subfolders.
    fn enqueue_tree(&mut self, root: &str, files: Vec<(Wanted, String)>) {
        let name = format::split_path(root).1;
        let base = self.download_dir.join(sanitize(name));
        let mut count = 0;
        for (wanted, relative) in files {
            let mut local = base.clone();
            for part in relative.split(['\\', '/']).filter(|part| !part.is_empty()) {
                local.push(sanitize(part));
            }
            if self.enqueue_to(wanted, Some(local)) {
                count += 1;
            }
        }
        self.notice(
            NoticeLevel::Info,
            format!(
                "queued {} from {name}",
                format::plural(count, "file", "files")
            ),
        );
    }

    fn poll_folders(&mut self, client: &Client) {
        let mut ready = Vec::new();
        let mut expired = Vec::new();
        self.folders.retain_mut(|request| {
            if let Some(dirs) = client.take_folder_contents(&request.username, &request.folder) {
                ready.push((request.username.clone(), request.folder.clone(), dirs));
                false
            } else if Instant::now() >= request.deadline {
                expired.push((
                    request.username.clone(),
                    std::mem::take(&mut request.fallback),
                ));
                false
            } else {
                true
            }
        });

        for (username, folder, dirs) in ready {
            let mut files = Vec::new();
            for dir in dirs {
                let relative = dir
                    .name
                    .strip_prefix(folder.as_str())
                    .unwrap_or("")
                    .to_string();
                for entry in dir.files {
                    let wanted = Wanted {
                        username: username.clone(),
                        filename: format!("{}\\{}", dir.name, entry.name),
                        size: entry.size,
                        bitrate: entry.attribute(0),
                        duration: entry.attribute(1),
                    };
                    files.push((wanted, relative.clone()));
                }
            }
            self.enqueue_tree(&folder, files);
        }

        for (username, fallback) in expired {
            let count = fallback.len();
            for wanted in fallback {
                self.enqueue(wanted);
            }
            self.notice(
                NoticeLevel::Warning,
                format!("{username} did not list the folder, queued the {count} matching files"),
            );
        }
    }

    fn poll_downloads(&mut self, client: &Client) {
        for row in &mut self.rows {
            let Some(status) = &row.status else {
                continue;
            };
            let mut terminal = false;
            while let Ok(update) = status.try_recv() {
                let state = map_status(update);
                terminal = !state.is_live();
                if row.view.state != state {
                    row.view.state = state;
                    self.rows_dirty = true;
                }
            }
            if terminal {
                row.status = None;
            }
        }

        if self.queue_checked.elapsed() >= QUEUE_REFRESH {
            self.queue_checked = Instant::now();
            for download in client.get_all_downloads() {
                let Some(row) = self.rows.iter_mut().find(|row| {
                    row.view.username == download.username && row.view.filename == download.filename
                }) else {
                    continue;
                };
                if let DlState::Queued { position } = &mut row.view.state
                    && *position != download.queue_position
                {
                    *position = download.queue_position;
                    self.rows_dirty = true;
                }
            }
        }

        if self.rows_dirty && self.rows_sent.elapsed() >= DOWNLOADS_REFRESH {
            self.rows_dirty = false;
            self.rows_sent = Instant::now();
            let rows = self.rows.iter().map(|row| row.view.clone()).collect();
            self.emit(Event::Downloads(Arc::new(rows)));
        }
    }
}

fn map_status(status: DownloadStatus) -> DlState {
    match status {
        DownloadStatus::Queued => DlState::Queued { position: None },
        DownloadStatus::InProgress {
            bytes_downloaded,
            total_bytes,
            speed_bytes_per_sec,
        } => DlState::Active {
            done: bytes_downloaded,
            total: total_bytes,
            speed: speed_bytes_per_sec.max(0.0) as u64,
        },
        DownloadStatus::Paused {
            bytes_downloaded,
            total_bytes,
        } => DlState::Paused {
            done: bytes_downloaded,
            total: total_bytes,
        },
        DownloadStatus::Completed => DlState::Completed,
        DownloadStatus::Cancelled => DlState::Cancelled,
        DownloadStatus::Failed(reason) => DlState::Failed(
            reason
                .map(|reason| reason.trim_end_matches('.').to_lowercase())
                .unwrap_or_else(|| "failed".into()),
        ),
        DownloadStatus::TimedOut => DlState::Failed("timed out".into()),
    }
}

fn describe(err: &SoulseekRs) -> String {
    match err {
        SoulseekRs::AuthenticationFailed => {
            "wrong password, or that name belongs to someone else".into()
        }
        SoulseekRs::Timeout => "the server did not answer".into(),
        SoulseekRs::NotConnected => "not connected to the server".into(),
        SoulseekRs::NetworkError(err) => format!("network error: {err}"),
        other => other.to_string().to_lowercase(),
    }
}

/// Makes a remote folder name safe as a Windows path component.
pub fn sanitize(name: &str) -> String {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim_end().to_string();
    let stem = cleaned.split('.').next().unwrap_or_default();
    if cleaned.is_empty() {
        "_".into()
    } else if RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
    {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_windows_names() {
        assert_eq!(sanitize("Album: Live?"), "Album_ Live_");
        assert_eq!(sanitize("  trailing dots... "), "trailing dots");
        assert_eq!(sanitize("con"), "_con");
        assert_eq!(sanitize("CON.flac"), "_CON.flac");
        assert_eq!(sanitize(""), "_");
        assert_eq!(sanitize("Normal Album (2001)"), "Normal Album (2001)");
    }

    #[test]
    fn maps_failure_reasons() {
        assert_eq!(
            map_status(DownloadStatus::Failed(Some("File not shared.".into()))),
            DlState::Failed("file not shared".into())
        );
        assert_eq!(
            map_status(DownloadStatus::Failed(None)),
            DlState::Failed("failed".into())
        );
    }
}
