mod browse;
mod group;
mod sharing;
mod worker;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use gpui_kit::*;

use crate::chats::Chats;

pub use browse::{Listing, Node};
pub use group::{FileHit, FolderHit, SearchHits};
pub use sharing::{overlaps, virtual_roots};

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
    pub bitrate: Option<u32>,
    pub duration: Option<u32>,
}

impl Wanted {
    pub fn from_hit(username: &str, file: &FileHit) -> Self {
        Self {
            username: username.to_string(),
            filename: file.filename.clone(),
            size: file.size,
            bitrate: file.bitrate,
            duration: file.duration,
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
    pub size: u64,
    pub state: DlState,
}

pub enum Command {
    Login {
        username: String,
        password: String,
        listen_port: u16,
        download_dir: PathBuf,
        shares: Vec<PathBuf>,
        upload_slots: usize,
    },
    Logout,
    Reconnect,
    Search(String),
    ForgetSearch(String),
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
    CancelUpload {
        username: String,
        filename: String,
    },
    ClearUploads,
    Browse(String),
    SendMessage {
        username: String,
        text: String,
    },
    DownloadTree {
        root: String,
        files: Vec<(Wanted, String)>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
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
                let read = self.viewing_chat.as_deref() == Some(username.as_str());
                if new && !read {
                    cx.emit(Notice(
                        NoticeLevel::Info,
                        format!("{username}: {text}").into(),
                    ));
                }
                self.chats.receive(&username, text, at, read);
                self.chats.save();
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
        });
        self.send(Command::Search(query.to_string()));
        cx.notify();
        self.searches.len() - 1
    }

    pub fn close_search(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.searches.len() {
            let tab = self.searches.remove(ix);
            self.send(Command::ForgetSearch(tab.query.to_string()));
            cx.notify();
        }
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
