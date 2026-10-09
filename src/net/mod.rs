mod browse;
mod discover;
mod filter;
mod group;
mod portmap;
mod rooms;
mod scope;
mod sharing;
mod social;
mod wishlist;
mod worker;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use gpui_kit::*;

use crate::chats::Chats;

pub use browse::{Listing, Node};
pub use discover::Discovery;
pub use filter::{Filter, Quality, toggle_format};
pub use group::{FileHit, FolderHit, SearchHits};
pub use portmap::PortMap;
pub use rooms::{RoomEvent, RoomLine, Rooms};
pub use scope::{Scope, parse_scope};
pub use sharing::{overlaps, virtual_roots};
pub use social::{Presence, UserCard};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Offline,
    Connecting,
    Online,
    Reconnecting { attempt: u32 },
    Displaced,
    Failed(String),
}

impl Status {
    pub fn is_online(&self) -> bool {
        matches!(self, Status::Online)
    }

    pub fn has_session(&self) -> bool {
        matches!(
            self,
            Status::Online | Status::Reconnecting { .. } | Status::Displaced
        )
    }
}

#[derive(Clone, Debug)]
pub struct Wanted {
    pub username: String,
    pub filename: String,
    pub size: u64,
}

impl Wanted {
    pub fn from_hit(username: &str, file: &FileHit) -> Self {
        Self {
            username: username.to_string(),
            filename: file.filename.clone(),
            size: file.size,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DlState {
    Queued { position: Option<u32> },
    Active { done: u64, total: u64, speed: u64 },
    Paused { done: u64, total: u64 },
    Completed,
    Cancelled,
    Failed(String),
}

impl DlState {
    pub fn is_live(&self) -> bool {
        matches!(
            self,
            DlState::Queued { .. } | DlState::Active { .. } | DlState::Paused { .. }
        )
    }

    pub fn progress(&self, size: u64) -> f32 {
        match self {
            DlState::Active { done, total, .. } | DlState::Paused { done, total } => {
                let total = (*total).max(size).max(1);
                (*done as f32 / total as f32).clamp(0.0, 1.0)
            }
            DlState::Completed => 1.0,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UlState {
    Queued { place: u32 },
    Active { speed: u64 },
    Completed,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct UploadRow {
    pub id: u64,
    pub username: String,
    pub filename: String,
    pub name: String,
    pub folder: String,
    pub size: u64,
    pub sent: u64,
    pub state: UlState,
}

impl UploadRow {
    pub fn progress(&self) -> f32 {
        match self.state {
            UlState::Completed => 1.0,
            _ => (self.sent as f32 / self.size.max(1) as f32).clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShareState {
    pub scanning: bool,
    pub folders: u32,
    pub files: u32,
}

#[derive(Clone, Debug)]
pub struct DownloadRow {
    pub id: u64,
    pub username: String,
    pub filename: String,
    pub name: String,
    pub folder: String,
    pub local_dir: PathBuf,
    /// Where the finished file was saved, which can differ from the planned name when one was taken.
    pub saved: Option<PathBuf>,
    pub size: u64,
    pub state: DlState,
}

/// Everything the worker needs from the saved settings to start a session.
pub struct LoginSettings {
    pub username: String,
    pub password: String,
    pub listen_port: u16,
    pub download_dir: PathBuf,
    pub shares: Vec<PathBuf>,
    pub upload_slots: usize,
    pub buddies: Vec<String>,
    pub likes: Vec<String>,
    pub dislikes: Vec<String>,
    pub upnp: bool,
    pub download_limit: u64,
}

pub enum Command {
    Login(LoginSettings),
    SetUpnp(bool),
    SetDownloadLimit(u64),
    ChangePassword(String),
    Logout,
    Reconnect,
    Search(String),
    ForgetSearch(String),
    SetWishes(Vec<String>),
    AddWish(String),
    RemoveWish(String),
    Download(Vec<Wanted>),
    DownloadFolder {
        username: String,
        folder: String,
        fallback: Vec<Wanted>,
    },
    Pause(u64),
    Resume(u64),
    Retry(u64),
    Cancel(u64),
    Remove(u64),
    ClearFinished,
    SetDownloadDir(PathBuf),
    SetShares(Vec<PathBuf>),
    Rescan,
    SetUploadSlots(usize),
    CancelUpload(u64),
    ClearUploads,
    Browse(String),
    SendMessage {
        username: String,
        text: String,
    },
    RoomList,
    Discover,
    DiscoverItem(String),
    SetInterest {
        item: String,
        like: bool,
        add: bool,
    },
    Watch(String),
    Unwatch(String),
    LookUp(String),
    SetAway(bool),
    JoinRoom {
        room: String,
        private: bool,
    },
    LeaveRoom(String),
    Say {
        room: String,
        text: String,
    },
    SetTicker {
        room: String,
        ticker: String,
    },
    PublicFeed(bool),
    GivePrivileges {
        username: String,
        days: u32,
    },
    DownloadTree {
        root: String,
        files: Vec<(Wanted, String)>,
    },
    SetIgnored(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
    Alert,
}

pub enum Event {
    Status(Status),
    LoggedIn(String),
    Search {
        query: String,
        hits: Arc<SearchHits>,
    },
    Downloads(Arc<Vec<DownloadRow>>),
    Uploads(Arc<Vec<UploadRow>>),
    Shares(ShareState),
    Browse {
        username: String,
        result: Result<Arc<Listing>, String>,
    },
    Message {
        username: String,
        text: String,
        at: i64,
        new: bool,
    },
    Rooms {
        at: i64,
        events: Vec<RoomEvent>,
    },
    Buddies(Vec<UserCard>),
    PortMap(PortMap),
    Privileges(u32),
    Discovery(Discovery),
    Card(UserCard),
    Notice(NoticeLevel, String),
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

pub struct SearchTab {
    pub query: SharedString,
    pub hits: Arc<SearchHits>,
    pub wish: bool,
}

pub enum BrowseState {
    Loading,
    Ready(Arc<Listing>),
    Failed(String),
}

pub struct BrowseTab {
    pub username: SharedString,
    pub state: BrowseState,
}

#[derive(Clone)]
pub struct Notice(pub NoticeLevel, pub SharedString);

/// The UI's view of the network, fed by the worker thread that owns the client.
pub struct Session {
    commands: Sender<Command>,
    pub status: Status,
    pub username: SharedString,
    pub searches: Vec<SearchTab>,
    pub downloads: Arc<Vec<DownloadRow>>,
    pub uploads: Arc<Vec<UploadRow>>,
    pub shares: ShareState,
    pub browses: Vec<BrowseTab>,
    pub chats: Chats,
    pub viewing_chat: Option<String>,
    pub rooms: Rooms,
    pub buddies: Vec<UserCard>,
    pub card: Option<UserCard>,
    pub ignored: std::collections::HashSet<String>,
    pub away: bool,
    pub discovery: Discovery,
    pub likes: Vec<String>,
    pub dislikes: Vec<String>,
    pub portmap: PortMap,
    pub privileges: Option<u32>,
    _pump: Task<()>,
}

impl EventEmitter<Notice> for Session {}

impl Session {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (commands, events) = worker::spawn();
        let pump = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let alive = this
                    .update(cx, |this, cx| this.drain(&events, cx))
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        });
        Self {
            commands,
            status: Status::Offline,
            username: SharedString::default(),
            searches: Vec::new(),
            downloads: Arc::default(),
            uploads: Arc::default(),
            shares: ShareState::default(),
            browses: Vec::new(),
            chats: Chats::default(),
            viewing_chat: None,
            rooms: Rooms::default(),
            buddies: Vec::new(),
            card: None,
            ignored: Default::default(),
            away: false,
            discovery: Discovery::default(),
            likes: Vec::new(),
            dislikes: Vec::new(),
            portmap: PortMap::Off,
            privileges: None,
            _pump: pump,
        }
    }

    fn drain(&mut self, events: &Receiver<Event>, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        loop {
            match events.try_recv() {
                Ok(event) => {
                    changed = true;
                    self.apply(event, cx);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
        if changed {
            cx.notify();
        }
        true
    }

    fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Status(status) => {
                if status == Status::Offline {
                    self.clear();
                }
                self.status = status;
            }
            Event::LoggedIn(username) => {
                self.chats = Chats::load(&username);
                let wishes = crate::config::load_wishlist(&username);
                self.searches = wishes
                    .iter()
                    .map(|wish| SearchTab {
                        query: wish.clone().into(),
                        hits: Arc::default(),
                        wish: true,
                    })
                    .collect();
                self.send(Command::SetWishes(wishes));
                self.username = username.into();
            }
            Event::Search { query, hits } => {
                if let Some(tab) = self.searches.iter_mut().find(|tab| tab.query == query) {
                    tab.hits = hits;
                }
            }
            Event::Downloads(rows) => self.downloads = rows,
            Event::Uploads(rows) => self.uploads = rows,
            Event::Shares(shares) => self.shares = shares,
            Event::Browse { username, result } => {
                if let Some(tab) = self.browses.iter_mut().find(|tab| tab.username == username) {
                    tab.state = match result {
                        Ok(listing) => BrowseState::Ready(listing),
                        Err(reason) => BrowseState::Failed(reason),
                    };
                }
            }
            Event::Message {
                username,
                text,
                at,
                new,
            } => {
                if self.ignored.contains(&username) {
                    return;
                }
                let read = self.viewing_chat.as_deref() == Some(username.as_str());
                if new && !read {
                    cx.emit(Notice(
                        NoticeLevel::Alert,
                        format!("{username}: {text}").into(),
                    ));
                }
                self.chats.receive(&username, text, at, read);
                self.chats.save();
            }
            Event::Rooms { at, events } => {
                for event in events {
                    if let RoomEvent::Message { username, .. }
                    | RoomEvent::GlobalMessage { username, .. } = &event
                        && self.ignored.contains(username)
                    {
                        continue;
                    }
                    if let Some(problem) = self.rooms.apply(at, event) {
                        cx.emit(Notice(NoticeLevel::Warning, problem.into()));
                    }
                }
            }
            Event::Buddies(cards) => {
                if let Some(card) = &mut self.card
                    && let Some(buddy) = cards.iter().find(|buddy| buddy.username == card.username)
                    && buddy.presence != Presence::Unknown
                {
                    card.presence = buddy.presence;
                }
                self.buddies = cards;
            }
            Event::Discovery(discovery) => self.discovery = discovery,
            Event::PortMap(state) => self.portmap = state,
            Event::Privileges(seconds) => self.privileges = Some(seconds),
            Event::Card(card) => {
                if self
                    .card
                    .as_ref()
                    .is_some_and(|current| current.username == card.username)
                {
                    self.card = Some(card);
                }
            }
            Event::Notice(level, text) => cx.emit(Notice(level, text.into())),
        }
    }

    fn clear(&mut self) {
        self.searches.clear();
        self.downloads = Arc::default();
        self.uploads = Arc::default();
        self.shares = ShareState::default();
        self.browses.clear();
        self.chats = Chats::default();
        self.viewing_chat = None;
        self.rooms = Rooms::default();
        self.buddies.clear();
        self.card = None;
        self.away = false;
        self.discovery = Discovery::default();
        self.portmap = PortMap::Off;
        self.privileges = None;
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn login(&mut self, command: Command, cx: &mut Context<Self>) {
        self.status = Status::Connecting;
        self.send(command);
        cx.notify();
    }

    pub fn logout(&mut self, cx: &mut Context<Self>) {
        self.send(Command::Logout);
        self.status = Status::Offline;
        self.clear();
        cx.notify();
    }

    /// Returns the tab index for `query`, starting a search unless one is already open.
    pub fn search(&mut self, query: &str, cx: &mut Context<Self>) -> usize {
        if let Some(ix) = self
            .searches
            .iter()
            .position(|tab| tab.query.as_ref() == query)
        {
            return ix;
        }
        self.searches.push(SearchTab {
            query: query.to_string().into(),
            hits: Arc::default(),
            wish: false,
        });
        self.send(Command::Search(query.to_string()));
        cx.notify();
        self.searches.len() - 1
    }

    pub fn close_search(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.searches.len() {
            let tab = self.searches.remove(ix);
            if tab.wish {
                self.send(Command::RemoveWish(tab.query.to_string()));
                self.save_wishlist();
            }
            self.send(Command::ForgetSearch(tab.query.to_string()));
            cx.notify();
        }
    }

    pub fn toggle_wish(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.searches.get_mut(ix) else {
            return;
        };
        tab.wish = !tab.wish;
        let (wish, query) = (tab.wish, tab.query.to_string());
        self.send(if wish {
            Command::AddWish(query)
        } else {
            Command::RemoveWish(query)
        });
        self.save_wishlist();
        cx.notify();
    }

    fn save_wishlist(&self) {
        let wishes: Vec<String> = self
            .searches
            .iter()
            .filter(|tab| tab.wish)
            .map(|tab| tab.query.to_string())
            .collect();
        crate::config::save_wishlist(&self.username, &wishes);
    }

    /// Returns the tab index for `username`, asking for their shares unless a listing is open or on its way.
    pub fn browse(&mut self, username: &str, cx: &mut Context<Self>) -> usize {
        let ix = match self
            .browses
            .iter()
            .position(|tab| tab.username.as_ref() == username)
        {
            Some(ix) => ix,
            None => {
                self.browses.push(BrowseTab {
                    username: username.to_string().into(),
                    state: BrowseState::Failed(String::new()),
                });
                self.browses.len() - 1
            }
        };
        if matches!(self.browses[ix].state, BrowseState::Failed(_)) {
            self.refresh_browse(ix, cx);
        }
        ix
    }

    pub fn refresh_browse(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self.browses.get_mut(ix) {
            tab.state = BrowseState::Loading;
            let username = tab.username.to_string();
            self.send(Command::Browse(username));
            cx.notify();
        }
    }

    pub fn close_browse(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.browses.len() {
            self.browses.remove(ix);
            cx.notify();
        }
    }

    pub fn send_message(&mut self, username: &str, text: String, cx: &mut Context<Self>) {
        self.chats.sent(username, text.clone(), unix_now());
        self.chats.save();
        self.send(Command::SendMessage {
            username: username.to_string(),
            text,
        });
        cx.notify();
    }

    /// Tells the session which conversation is on screen, so its messages arrive already read.
    pub fn view_chat(&mut self, username: Option<String>, cx: &mut Context<Self>) {
        if let Some(username) = &username
            && self.chats.mark_read(username)
        {
            self.chats.save();
            cx.notify();
        }
        self.viewing_chat = username;
    }

    /// Tells the session which room is on screen, so its lines arrive already read.
    pub fn view_room(&mut self, room: Option<String>, cx: &mut Context<Self>) {
        if let Some(room) = &room
            && self.rooms.mark_read(room)
        {
            cx.notify();
        }
        self.rooms.viewing = room;
    }

    pub fn look_up(&mut self, username: &str, cx: &mut Context<Self>) {
        self.card = Some(UserCard {
            username: username.to_string(),
            loading: true,
            ..Default::default()
        });
        self.send(Command::LookUp(username.to_string()));
        cx.notify();
    }

    pub fn set_public_feed(&mut self, on: bool, cx: &mut Context<Self>) {
        self.rooms.feed_on = on;
        self.send(Command::PublicFeed(on));
        cx.notify();
    }

    pub fn set_away(&mut self, away: bool, cx: &mut Context<Self>) {
        self.away = away;
        self.send(Command::SetAway(away));
        cx.notify();
    }

    pub fn active_uploads(&self) -> usize {
        self.uploads
            .iter()
            .filter(|row| matches!(row.state, UlState::Active { .. }))
            .count()
    }

    pub fn active_downloads(&self) -> usize {
        self.downloads
            .iter()
            .filter(|row| row.state.is_live())
            .count()
    }
}
