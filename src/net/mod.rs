mod group;
mod worker;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use gpui_kit::*;

pub use group::{FileHit, FolderHit, SearchHits};

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
    Notice(NoticeLevel, String),
}

pub struct SearchTab {
    pub query: SharedString,
    pub hits: Arc<SearchHits>,
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
                    self.searches.clear();
                    self.downloads = Arc::default();
                }
                self.status = status;
            }
            Event::LoggedIn(username) => self.username = username.into(),
            Event::Search { query, hits } => {
                if let Some(tab) = self.searches.iter_mut().find(|tab| tab.query == query) {
                    tab.hits = hits;
                }
            }
            Event::Downloads(rows) => self.downloads = rows,
            Event::Notice(level, text) => cx.emit(Notice(level, text.into())),
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn login(
        &mut self,
        username: String,
        password: String,
        listen_port: u16,
        download_dir: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.status = Status::Connecting;
        self.send(Command::Login {
            username,
            password,
            listen_port,
            download_dir,
        });
        cx.notify();
    }

    pub fn logout(&mut self, cx: &mut Context<Self>) {
        self.send(Command::Logout);
        self.status = Status::Offline;
        self.searches.clear();
        self.downloads = Arc::default();
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

    pub fn active_downloads(&self) -> usize {
        self.downloads
            .iter()
            .filter(|row| row.state.is_live())
            .count()
    }
}
