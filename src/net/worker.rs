use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use slsk::proto::peer::SearchReply;
use slsk::proto::server::{ServerRequest, ServerResponse};
use slsk::proto::types::{Directory, UserStatus};
use slsk::{
    Client, Config, Event as Net, Request, SearchScope, Session, TransferState, TransferUpdate,
};

use super::browse::Listing;
use super::discover::Discover;
use super::group::group;
use super::portmap::{PortMap, PortMapper};
use super::rooms::RoomEvent;
use super::scope::{Scope, parse_scope};
use super::sharing::{Uploads, tidy_reason};
use super::social::{Buddies, Lookup};
use super::wishlist::Wishlist;
use super::{
    Command, DlState, DownloadRow, Event, LoginSettings, NoticeLevel, ShareState, Status, Wanted,
    unix_now,
};
use crate::format;

const TICK: Duration = Duration::from_millis(50);
const SEARCH_REFRESH: Duration = Duration::from_millis(700);
const DOWNLOADS_REFRESH: Duration = Duration::from_millis(250);
const UPLOADS_REFRESH: Duration = Duration::from_millis(500);
const FOLDER_TIMEOUT: Duration = Duration::from_secs(25);
/// One search keeps at most this many replies and files; past that it costs memory without helping anyone choose.
const MAX_REPLIES: usize = 1_000;
const MAX_FILES: usize = 20_000;

pub fn spawn() -> (Sender<Command>, Receiver<Event>) {
    let (command_tx, command_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    thread::Builder::new()
        .name("soulseek".into())
        .spawn(move || Worker::new(event_tx).run(command_rx))
        .expect("spawn network thread");
    (command_tx, event_rx)
}

#[derive(Clone)]
struct Credentials {
    username: String,
    password: String,
    listen_port: u16,
}

/// One search tab and every reply it has collected, across wishlist reruns.
#[derive(Default)]
struct Tab {
    replies: Vec<SearchReply>,
    seen: HashSet<(String, String)>,
    dirty: bool,
    sent: Option<Instant>,
}

impl Tab {
    fn is_full(&self) -> bool {
        self.replies.len() >= MAX_REPLIES || self.seen.len() >= MAX_FILES
    }

    /// Keeps only files this tab has not shown yet, up to the limits; returns how many were new.
    fn merge(&mut self, mut reply: SearchReply) -> usize {
        if self.is_full() {
            return 0;
        }
        let username = reply.username.clone();
        reply
            .files
            .retain(|file| self.seen.insert((username.clone(), file.name.clone())));
        let room = MAX_FILES.saturating_sub(self.seen.len() - reply.files.len());
        reply.files.truncate(room);
        let added = reply.files.len();
        if added > 0 {
            self.replies.push(reply);
            self.dirty = true;
        }
        added
    }
}

struct FolderRequest {
    username: String,
    fallback: Vec<Wanted>,
    deadline: Instant,
}

struct Row {
    view: DownloadRow,
    wanted: Wanted,
}

struct Worker {
    events: Sender<Event>,
    client: Option<Client>,
    credentials: Option<Credentials>,
    logged_in_once: bool,
    attempt: u32,
    download_dir: PathBuf,
    tabs: HashMap<String, Tab>,
    tokens: HashMap<u32, String>,
    folders: HashMap<u32, FolderRequest>,
    rows: Vec<Row>,
    rows_dirty: bool,
    rows_sent: Instant,
    shares: Vec<PathBuf>,
    upload_slots: usize,
    uploads: Uploads,
    uploads_sent: Instant,
    browses: HashSet<String>,
    buddies: Buddies,
    lookup: Lookup,
    wishlist: Wishlist,
    discover: Discover,
    upnp: bool,
    portmap: Option<PortMapper>,
    download_limit: u64,
    ignored: Vec<String>,
}

impl Worker {
    fn new(events: Sender<Event>) -> Self {
        let now = Instant::now();
        Self {
            events,
            client: None,
            credentials: None,
            logged_in_once: false,
            attempt: 0,
            download_dir: PathBuf::new(),
            tabs: HashMap::new(),
            tokens: HashMap::new(),
            folders: HashMap::new(),
            rows: Vec::new(),
            rows_dirty: false,
            rows_sent: now,
            shares: Vec::new(),
            upload_slots: crate::config::DEFAULT_UPLOAD_SLOTS,
            uploads: Uploads::default(),
            uploads_sent: now,
            browses: HashSet::new(),
            buddies: Buddies::default(),
            lookup: Lookup::default(),
            wishlist: Wishlist::default(),
            discover: Discover::default(),
            upnp: false,
            portmap: None,
            download_limit: 0,
            ignored: Vec::new(),
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
            while let Some(event) = self.client.as_mut().and_then(Client::try_event) {
                self.on_net(event);
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

    fn send(&self, request: ServerRequest) {
        if let Some(client) = &self.client {
            client.send(request);
        }
    }

    fn send_all(&self, requests: Vec<ServerRequest>) {
        for request in requests {
            self.send(request);
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Login(settings) => self.login(settings),
            Command::SetUpnp(on) => {
                self.upnp = on;
                if on {
                    self.map_port();
                } else {
                    self.portmap = None;
                    self.emit(Event::PortMap(PortMap::Off));
                }
            }
            Command::SetDownloadLimit(kilobytes) => {
                self.download_limit = kilobytes;
                if let Some(client) = &self.client {
                    client.set_download_limit(kilobytes * 1024);
                }
            }
            Command::ChangePassword(password) => {
                if self.client.is_some() {
                    self.send(ServerRequest::ChangePassword(password));
                    self.notice(NoticeLevel::Info, "password change sent to the server");
                } else {
                    self.notice(
                        NoticeLevel::Warning,
                        "could not change the password: not connected to the server",
                    );
                }
            }
            Command::Logout => self.logout(),
            Command::Reconnect => self.reconnect(),
            Command::Search(tab) => self.search(tab),
            Command::SetWishes(wishes) => {
                for wish in &wishes {
                    self.tabs.entry(wish.clone()).or_default();
                }
                self.wishlist.set(wishes);
            }
            Command::AddWish(query) => {
                self.tabs.entry(query.clone()).or_default();
                self.wishlist.add(query);
            }
            Command::RemoveWish(query) => self.wishlist.remove(&query),
            Command::ForgetSearch(tab) => {
                self.tabs.remove(&tab);
                self.forget_tokens(&tab);
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
            Command::Pause(id) => {
                if let Some(client) = &self.client {
                    client.pause_download(id);
                }
            }
            Command::Resume(id) | Command::Retry(id) => {
                if let Some(client) = &self.client {
                    client.resume_download(id);
                }
            }
            Command::Cancel(id) => {
                if let Some(client) = &self.client {
                    client.cancel_download(id);
                }
            }
            Command::Remove(id) => {
                if let Some(ix) = self.rows.iter().position(|row| row.view.id == id) {
                    self.rows.remove(ix);
                    if let Some(client) = &self.client {
                        client.remove_download(id);
                    }
                    self.rows_dirty = true;
                }
            }
            Command::ClearFinished => {
                let client = self.client.as_ref();
                self.rows.retain(|row| {
                    let finished =
                        matches!(row.view.state, DlState::Completed | DlState::Cancelled);
                    if finished && let Some(client) = client {
                        client.remove_download(row.view.id);
                    }
                    !finished
                });
                self.rows_dirty = true;
            }
            Command::SetDownloadDir(dir) => self.download_dir = dir,
            Command::SetShares(dirs) => {
                self.shares = dirs.clone();
                if let Some(client) = &self.client {
                    client.set_shares(dirs);
                    self.scanning();
                }
            }
            Command::Rescan => {
                if let Some(client) = &self.client {
                    client.rescan();
                    self.scanning();
                }
            }
            Command::SetUploadSlots(slots) => {
                self.upload_slots = slots;
                if let Some(client) = &self.client {
                    client.set_upload_slots(slots);
                }
            }
            Command::CancelUpload(id) => {
                if let Some(client) = &self.client {
                    client.cancel_upload(id);
                }
            }
            Command::ClearUploads => self.uploads.clear_finished(),
            Command::Browse(username) => {
                if let Some(client) = &self.client {
                    client.browse(&username);
                    self.browses.insert(username);
                } else {
                    self.emit(Event::Browse {
                        username,
                        result: Err("not connected to the server".into()),
                    });
                }
            }
            Command::Watch(name) => {
                let requests = self.buddies.add(name);
                self.send_all(requests);
                self.emit(Event::Buddies(self.buddies.cards()));
            }
            Command::Unwatch(name) => {
                let requests = self.buddies.remove(&name);
                self.send_all(requests);
                self.emit(Event::Buddies(self.buddies.cards()));
            }
            Command::LookUp(name) => {
                let requests = self.lookup.start(name.clone());
                self.send_all(requests);
                if let Some(client) = &self.client {
                    client.user_info(&name);
                }
            }
            Command::SetAway(away) => self.send(ServerRequest::SetStatus(if away {
                UserStatus::Away
            } else {
                UserStatus::Online
            })),
            Command::Discover => self.send_all(self.discover.refresh()),
            Command::DiscoverItem(item) => {
                let requests = self.discover.open_item(item);
                self.send_all(requests);
            }
            Command::SetInterest { item, like, add } => {
                let requests = self.discover.set_interest(item, like, add);
                self.send_all(requests);
            }
            Command::RoomList => self.send(ServerRequest::RoomList),
            Command::JoinRoom { room, private } => {
                self.send(ServerRequest::JoinRoom { room, private })
            }
            Command::LeaveRoom(room) => self.send(ServerRequest::LeaveRoom(room)),
            Command::Say { room, text } => self.send(ServerRequest::SayChatroom {
                room,
                message: text,
            }),
            Command::PublicFeed(on) => self.send(if on {
                ServerRequest::JoinGlobalRoom
            } else {
                ServerRequest::LeaveGlobalRoom
            }),
            Command::GivePrivileges { username, days } => {
                self.send(ServerRequest::GivePrivileges {
                    username: username.clone(),
                    days,
                });
                self.notice(
                    NoticeLevel::Info,
                    format!(
                        "asked the server to give {username} {}",
                        format::plural(days as usize, "day", "days")
                    ),
                );
            }
            Command::SetTicker { room, ticker } => {
                self.send(ServerRequest::SetRoomTicker { room, ticker })
            }
            Command::SendMessage { username, text } => {
                if self.client.is_some() {
                    self.send(ServerRequest::MessageUser {
                        username,
                        message: text,
                    });
                } else {
                    self.notice(
                        NoticeLevel::Warning,
                        format!("could not send to {username}: not connected to the server"),
                    );
                }
            }
            Command::DownloadTree { root, files } => self.enqueue_tree(&root, files),
            Command::SetIgnored(usernames) => {
                self.ignored = usernames.clone();
                if let Some(client) = &self.client {
                    client.set_ignored(usernames);
                }
            }
        }
    }

    fn login(&mut self, settings: LoginSettings) {
        let LoginSettings {
            username,
            password,
            listen_port,
            download_dir,
            shares,
            upload_slots,
            buddies,
            likes,
            dislikes,
            upnp,
            download_limit,
        } = settings;
        self.logout_quietly();
        self.buddies.set(buddies);
        self.discover.set_interests(likes, dislikes);
        self.download_dir = download_dir;
        self.shares = shares;
        self.upload_slots = upload_slots;
        self.upnp = upnp;
        self.download_limit = download_limit;
        self.credentials = Some(Credentials {
            username,
            password,
            listen_port,
        });
        self.logged_in_once = false;
        self.start_client();
    }

    fn start_client(&mut self) {
        let Some(credentials) = self.credentials.clone() else {
            return;
        };
        let mut config = Config::new(credentials.username, credentials.password);
        config.listen_port = credentials.listen_port;
        config.shared_dirs = self.shares.clone();
        config.share_cache = crate::config::data_dir().map(|dir| dir.join("share-cache.json"));
        config.upload_slots = self.upload_slots;
        match Client::start(config) {
            Ok(client) => {
                client.set_download_limit(self.download_limit * 1024);
                client.set_ignored(self.ignored.clone());
                self.client = Some(client);
                self.attempt = 0;
                self.emit(Event::Status(Status::Connecting));
                if !self.shares.is_empty() {
                    self.scanning();
                }
            }
            Err(err) => self.emit(Event::Status(Status::Failed(format!(
                "could not start the network: {err}"
            )))),
        }
    }

    fn scanning(&self) {
        self.emit(Event::Shares(ShareState {
            scanning: true,
            ..ShareState::default()
        }));
    }

    /// Logs in again after another session took over, putting unfinished downloads back in line.
    fn reconnect(&mut self) {
        self.client = None;
        self.portmap = None;
        self.start_client();
        let Some(client) = &self.client else {
            return;
        };
        for row in &mut self.rows {
            if row.view.state.is_live() {
                let dest = row.view.local_dir.join(sanitize(&row.view.name));
                row.view.id = client.download(
                    &row.wanted.username,
                    &row.wanted.filename,
                    row.wanted.size,
                    dest,
                );
                row.view.state = DlState::Queued { position: None };
            }
        }
        self.rows_dirty = true;
    }

    fn logout_quietly(&mut self) {
        self.client = None;
        self.portmap = None;
        self.credentials = None;
        self.tabs.clear();
        self.tokens.clear();
        self.folders.clear();
        self.rows.clear();
        self.rows_dirty = false;
        self.uploads.reset();
        self.browses.clear();
        self.lookup = Lookup::default();
    }

    fn logout(&mut self) {
        self.logout_quietly();
        self.emit(Event::Status(Status::Offline));
    }

    fn map_port(&mut self) {
        if self.client.is_some()
            && let Some(credentials) = &self.credentials
        {
            self.portmap = Some(PortMapper::start(
                credentials.listen_port,
                self.events.clone(),
            ));
        }
    }

    fn on_net(&mut self, event: Net) {
        match event {
            Net::Session(session) => self.on_session(session),
            Net::Server(message) => self.on_server(message),
            Net::Listening { .. } => {
                if self.upnp && self.portmap.is_none() {
                    self.map_port();
                }
            }
            Net::ListenFailed { port, error } => self.notice(
                NoticeLevel::Warning,
                format!(
                    "could not listen on port {port}: {error}. other users cannot connect to you."
                ),
            ),
            Net::SearchReply(reply) => self.on_search_reply(reply),
            Net::Shares { username, list } => {
                if self.browses.remove(&username) {
                    let events = self.events.clone();
                    thread::spawn(move || {
                        let mut dirs: Vec<Directory> = list.dirs;
                        dirs.extend(list.private_dirs);
                        let listing = Listing::build(dirs);
                        let _ = events.send(Event::Browse {
                            username,
                            result: Ok(Arc::new(listing)),
                        });
                    });
                }
            }
            Net::FolderContents {
                username,
                token,
                folder,
                dirs,
            } => {
                if self.folders.remove(&token).is_some() {
                    self.folder_arrived(&username, &folder, dirs);
                }
            }
            Net::RequestFailed {
                username,
                request,
                reason,
            } => match request {
                Request::Browse => {
                    if self.browses.remove(&username) {
                        self.emit(Event::Browse {
                            username: username.clone(),
                            result: Err(format!(
                                "no answer from {username} ({reason}). they may be offline or unreachable."
                            )),
                        });
                    }
                }
                Request::FolderContents { token, .. } => {
                    if let Some(request) = self.folders.remove(&token) {
                        self.folder_fallback(request);
                    }
                }
                Request::UserInfo => {
                    if let Some(card) = self.lookup.info_failed(&username) {
                        self.emit(Event::Card(card));
                    }
                }
            },
            Net::UserInfo { username, info } => {
                if let Some(card) = self.lookup.apply_info(&username, info) {
                    self.emit(Event::Card(card));
                }
            }
            Net::SharesScanned { dirs, files } => self.emit(Event::Shares(ShareState {
                scanning: false,
                folders: dirs as u32,
                files: files as u32,
            })),
            Net::Download(update) => self.on_download(update),
            Net::Upload(update) => self.uploads.apply(update),
        }
    }

    fn on_session(&mut self, session: Session) {
        match session {
            Session::Connecting { attempt } => {
                self.attempt = attempt;
                if self.logged_in_once {
                    self.emit(Event::Status(Status::Reconnecting { attempt }));
                }
            }
            Session::LoggedIn { .. } => {
                if !self.logged_in_once {
                    self.logged_in_once = true;
                    if let Some(credentials) = &self.credentials {
                        self.emit(Event::LoggedIn(credentials.username.clone()));
                    }
                    self.send_all(self.buddies.watch_all());
                    self.send_all(self.discover.announce());
                    self.send_all(self.discover.refresh());
                    self.send(ServerRequest::RoomList);
                    self.emit(Event::Buddies(self.buddies.cards()));
                }
                self.emit(Event::Status(Status::Online));
            }
            Session::Rejected { reason, detail } => {
                self.client = None;
                self.emit(Event::Status(Status::Failed(describe_rejection(
                    &reason,
                    detail.as_deref(),
                ))));
            }
            Session::Relogged => self.emit(Event::Status(Status::Displaced)),
            Session::Lost { error, .. } => {
                if self.logged_in_once {
                    self.emit(Event::Status(Status::Reconnecting {
                        attempt: self.attempt.max(1),
                    }));
                } else {
                    self.client = None;
                    self.emit(Event::Status(Status::Failed(format!(
                        "could not reach the server: {error}"
                    ))));
                }
            }
            Session::Disconnected => {}
        }
    }

    fn on_server(&mut self, message: ServerResponse) {
        let room_events = RoomEvent::from_server(&message);
        if !room_events.is_empty() {
            self.emit(Event::Rooms {
                at: unix_now(),
                events: room_events,
            });
            return;
        }
        if let Some(cards) = self.buddies.apply(&message) {
            self.emit(Event::Buddies(cards));
        }
        if let Some(card) = self.lookup.apply(&message) {
            self.emit(Event::Card(card));
        }
        if let Some(discovery) = self.discover.apply(&message) {
            self.emit(Event::Discovery(discovery));
        }
        match message {
            ServerResponse::MessageUser {
                timestamp,
                username,
                message,
                new,
                ..
            } => self.emit(Event::Message {
                username,
                text: message,
                at: i64::from(timestamp),
                new,
            }),
            ServerResponse::CheckPrivileges(seconds) => self.emit(Event::Privileges(seconds)),
            ServerResponse::WishlistInterval(seconds) => self.wishlist.set_interval(seconds),
            ServerResponse::ChangePassword(_) => {
                self.notice(NoticeLevel::Info, "the server changed your password")
            }
            ServerResponse::AdminMessage(text) => self.notice(
                NoticeLevel::Alert,
                format!("message from the server: {text}"),
            ),
            _ => {}
        }
    }

    fn search(&mut self, tab: String) {
        let Some(client) = &self.client else {
            return;
        };
        let (scope, query) = parse_scope(&tab);
        let scope = match scope {
            Scope::Everyone => SearchScope::Network,
            Scope::User(user) => SearchScope::User(user),
            Scope::Room(room) => SearchScope::Room(room),
        };
        let token = client.search(scope, &query);
        self.tokens.insert(token, tab.clone());
        self.tabs.entry(tab).or_default();
    }

    fn on_search_reply(&mut self, reply: SearchReply) {
        let Some(name) = self.tokens.get(&reply.token).cloned() else {
            return;
        };
        if self.ignored.contains(&reply.username) {
            return;
        }
        let Some(tab) = self.tabs.get_mut(&name) else {
            return;
        };
        let added = tab.merge(reply);
        if tab.is_full() {
            self.forget_tokens(&name);
        }
        if self.wishlist.is_wish(&name) {
            let fresh = self.wishlist.fresh(&name, added);
            if fresh > 0 {
                self.notice(
                    NoticeLevel::Alert,
                    format!(
                        "wishlist: {} for {name}",
                        format::plural(fresh, "new result", "new results")
                    ),
                );
            }
        }
    }

    /// Stops the crate from passing on more replies for this tab.
    fn forget_tokens(&mut self, tab: &str) {
        let tokens: Vec<u32> = self
            .tokens
            .iter()
            .filter(|(_, owner)| *owner == tab)
            .map(|(token, _)| *token)
            .collect();
        for token in tokens {
            self.tokens.remove(&token);
            if let Some(client) = &self.client {
                client.forget_search(token);
            }
        }
    }

    fn request_folder(&mut self, username: String, folder: String, fallback: Vec<Wanted>) {
        let Some(client) = &self.client else {
            return;
        };
        let token = client.folder_contents(&username, &folder);
        let label = format::split_path(&folder).1.to_string();
        self.notice(NoticeLevel::Info, format!("asking {username} for {label}"));
        self.folders.insert(
            token,
            FolderRequest {
                username,
                fallback,
                deadline: Instant::now() + FOLDER_TIMEOUT,
            },
        );
    }

    fn folder_arrived(&mut self, username: &str, folder: &str, dirs: Vec<Directory>) {
        let mut files = Vec::new();
        for dir in dirs {
            let relative = dir.name.strip_prefix(folder).unwrap_or("").to_string();
            for entry in dir.files {
                let wanted = Wanted {
                    username: username.to_string(),
                    filename: format!("{}\\{}", dir.name, entry.name),
                    size: entry.size,
                };
                files.push((wanted, relative.clone()));
            }
        }
        self.enqueue_tree(folder, files);
    }

    fn folder_fallback(&mut self, request: FolderRequest) {
        let count = request.fallback.len();
        for wanted in request.fallback {
            self.enqueue(wanted);
        }
        self.notice(
            NoticeLevel::Warning,
            format!(
                "{} did not list the folder, queued the {count} matching files",
                request.username
            ),
        );
    }

    fn enqueue(&mut self, wanted: Wanted) -> bool {
        self.enqueue_to(wanted, None)
    }

    /// Adds a download row and starts it. Returns false when it is already queued.
    fn enqueue_to(&mut self, wanted: Wanted, dir: Option<PathBuf>) -> bool {
        let Some(client) = &self.client else {
            return false;
        };
        if let Some(row) = self.rows.iter().find(|row| {
            row.view.username == wanted.username && row.view.filename == wanted.filename
        }) {
            let state = &row.view.state;
            if state.is_live() || *state == DlState::Completed {
                return false;
            }
            client.resume_download(row.view.id);
            return true;
        }

        let (folder, name) = format::split_path(&wanted.filename);
        let folder = format::split_path(folder).1.to_string();
        let local_dir = dir.unwrap_or_else(|| self.local_dir(&wanted.username, &folder, name));
        let dest = local_dir.join(sanitize(name));
        let id = client.download(&wanted.username, &wanted.filename, wanted.size, dest);
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
        });
        self.rows_dirty = true;
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

    fn on_download(&mut self, update: TransferUpdate) {
        let Some(row) = self.rows.iter_mut().find(|row| row.view.id == update.id) else {
            return;
        };
        let state = download_state(&update);
        if row.view.state == state {
            return;
        }
        let finished = state == DlState::Completed;
        row.view.state = state;
        self.rows_dirty = true;
        if finished {
            let dir = row.view.local_dir.clone();
            self.announce_finished(&dir);
        }
    }

    /// One alert per folder, once every file in it has arrived.
    fn announce_finished(&self, dir: &std::path::Path) {
        let rows: Vec<&Row> = self
            .rows
            .iter()
            .filter(|row| row.view.local_dir == *dir)
            .collect();
        if rows.iter().any(|row| row.view.state != DlState::Completed) {
            return;
        }
        let label = match rows.as_slice() {
            [only] => only.view.name.clone(),
            [first, ..] if !first.view.folder.is_empty() => format!(
                "{} ({})",
                first.view.folder,
                format::plural(rows.len(), "file", "files")
            ),
            _ => format::plural(rows.len(), "file", "files"),
        };
        self.notice(NoticeLevel::Alert, format!("downloaded {label}"));
    }

    fn tick(&mut self) {
        for wish in self.wishlist.due() {
            if let Some(client) = &self.client {
                let token = client.search(SearchScope::Wishlist, &wish);
                self.tokens.insert(token, wish.clone());
                self.tabs.entry(wish).or_default();
            }
        }
        for (name, tab) in &mut self.tabs {
            if tab.dirty && tab.sent.is_none_or(|at| at.elapsed() >= SEARCH_REFRESH) {
                tab.dirty = false;
                tab.sent = Some(Instant::now());
                let mut hits = group(&tab.replies);
                hits.capped = tab.is_full();
                let _ = self.events.send(Event::Search {
                    query: name.clone(),
                    hits: Arc::new(hits),
                });
            }
        }
        let now = Instant::now();
        let late: Vec<u32> = self
            .folders
            .iter()
            .filter(|(_, request)| now >= request.deadline)
            .map(|(token, _)| *token)
            .collect();
        for token in late {
            if let Some(request) = self.folders.remove(&token) {
                self.folder_fallback(request);
            }
        }
        if let Some(card) = self.lookup.tick() {
            self.emit(Event::Card(card));
        }
        if self.rows_dirty && self.rows_sent.elapsed() >= DOWNLOADS_REFRESH {
            self.rows_dirty = false;
            self.rows_sent = Instant::now();
            let rows = self.rows.iter().map(|row| row.view.clone()).collect();
            self.emit(Event::Downloads(Arc::new(rows)));
        }
        if self.uploads_sent.elapsed() >= UPLOADS_REFRESH {
            self.uploads_sent = Instant::now();
            if let Some(rows) = self.uploads.take_changed() {
                self.emit(Event::Uploads(rows));
            }
        }
    }
}

fn download_state(update: &TransferUpdate) -> DlState {
    match &update.state {
        TransferState::Queued { place } => DlState::Queued { position: *place },
        TransferState::Connecting => DlState::Active {
            done: 0,
            total: update.size,
            speed: 0,
        },
        TransferState::Transferring { bytes, speed } => DlState::Active {
            done: *bytes,
            total: update.size,
            speed: *speed,
        },
        TransferState::Paused { bytes } => DlState::Paused {
            done: *bytes,
            total: update.size,
        },
        TransferState::Done => DlState::Completed,
        TransferState::Cancelled => DlState::Cancelled,
        TransferState::Failed(reason) => DlState::Failed(tidy_reason(reason)),
    }
}

fn describe_rejection(reason: &str, detail: Option<&str>) -> String {
    match reason {
        "INVALIDPASS" => "wrong password, or that name belongs to someone else".into(),
        "INVALIDUSERNAME" => detail.map_or_else(
            || "the server does not allow that name".into(),
            |detail| {
                format!(
                    "the server does not allow that name: {}",
                    tidy_reason(detail)
                )
            },
        ),
        "EMPTYPASSWORD" => "enter a password".into(),
        "INVALIDVERSION" => "the server no longer accepts this version of bawkseek".into(),
        "SVRFULL" => "the server is full. try again later.".into(),
        "SVRPRIVATE" => "the server is not taking new accounts right now".into(),
        other => format!("the server refused the login: {}", other.to_lowercase()),
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
    use slsk::proto::types::FileEntry;

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
    fn maps_download_states() {
        let update = |state| TransferUpdate {
            id: 1,
            username: "ann".into(),
            filename: "a.mp3".into(),
            size: 100,
            state,
            path: None,
        };
        assert_eq!(
            download_state(&update(TransferState::Failed("File not shared.".into()))),
            DlState::Failed("file not shared".into())
        );
        assert_eq!(
            download_state(&update(TransferState::Transferring { bytes: 5, speed: 9 })),
            DlState::Active {
                done: 5,
                total: 100,
                speed: 9
            }
        );
    }

    #[test]
    fn tabs_keep_each_file_once() {
        let mut tab = Tab::default();
        let reply = |files: &[&str]| SearchReply {
            username: "ann".into(),
            files: files
                .iter()
                .map(|name| FileEntry {
                    name: (*name).into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        assert_eq!(tab.merge(reply(&["a", "b"])), 2);
        assert_eq!(tab.merge(reply(&["b", "c"])), 1);
        assert_eq!(tab.merge(reply(&["a"])), 0);
        assert_eq!(tab.replies.len(), 2);
    }

    #[test]
    fn tabs_stop_at_the_file_limit() {
        let mut tab = Tab::default();
        let names: Vec<String> = (0..MAX_FILES + 10).map(|n| n.to_string()).collect();
        let reply = SearchReply {
            username: "ann".into(),
            files: names
                .iter()
                .map(|name| FileEntry {
                    name: name.clone(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        assert_eq!(tab.merge(reply.clone()), MAX_FILES);
        assert!(tab.is_full());
        assert_eq!(tab.merge(reply), 0);
    }
}
