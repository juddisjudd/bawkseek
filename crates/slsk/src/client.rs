use std::io;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::engine::{Engine, Input};
use crate::proto::peer::{SearchReply, SharedFileList, UserInfo};
use crate::proto::server::{ServerRequest, ServerResponse};
use crate::proto::types::Directory;

/// Experimental clients use major version 177, per the protocol documentation.
pub const MAJOR_VERSION: u32 = 177;
pub const MINOR_VERSION: u32 = 1;
pub const DEFAULT_SERVER: &str = "server.slsknet.org:2242";
pub const DEFAULT_LISTEN_PORT: u16 = 2234;

#[derive(Clone, Debug)]
pub struct Config {
    pub server: String,
    pub username: String,
    pub password: String,
    pub listen_port: u16,
    pub reconnect: bool,
}

impl Config {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            server: DEFAULT_SERVER.into(),
            username: username.into(),
            password: password.into(),
            listen_port: DEFAULT_LISTEN_PORT,
            reconnect: true,
        }
    }
}

/// Where the server connection stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Session {
    Connecting {
        attempt: u32,
    },
    LoggedIn {
        greeting: String,
        own_ip: Ipv4Addr,
        supporter: bool,
    },
    Rejected {
        reason: String,
        detail: Option<String>,
    },
    /// Someone logged in with our name elsewhere; the server closed this session for good.
    Relogged,
    Lost {
        error: String,
        retry_in: Option<Duration>,
    },
    Disconnected,
}

/// A request to a peer that can fail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    UserInfo,
    Browse,
    FolderContents { token: u32, folder: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Session(Session),
    /// Server messages the library does not act on itself, such as room, chat and interest updates.
    Server(ServerResponse),
    Listening {
        port: u16,
    },
    ListenFailed {
        port: u16,
        error: String,
    },
    UserInfo {
        username: String,
        info: UserInfo,
    },
    Shares {
        username: String,
        list: SharedFileList,
    },
    FolderContents {
        username: String,
        token: u32,
        folder: String,
        dirs: Vec<Directory>,
    },
    RequestFailed {
        username: String,
        request: Request,
        reason: String,
    },
    SearchReply(SearchReply),
}

/// What we tell peers about ourselves when they ask.
#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub description: String,
    pub picture: Option<Vec<u8>>,
    pub total_uploads: u32,
}

pub(crate) enum Command {
    Server(ServerRequest),
    Disconnect,
    UserInfo(String),
    Browse(String),
    FolderContents {
        username: String,
        token: u32,
        folder: String,
    },
    SetIgnored(Vec<String>),
    SetProfile(Profile),
}

/// A running Soulseek session: it connects, logs in, reconnects, and reports what happens as events.
pub struct Client {
    inputs: UnboundedSender<Input>,
    events: UnboundedReceiver<Event>,
    tokens: Arc<AtomicU32>,
    runtime: Option<Runtime>,
}

impl Client {
    pub fn start(config: Config) -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("slsk")
            .enable_all()
            .build()?;
        let (inputs, input_rx) = unbounded_channel();
        let (events_tx, events) = unbounded_channel();
        let tokens = Arc::new(AtomicU32::new(seed_token()));
        let engine = Engine::new(config, events_tx, inputs.clone(), tokens.clone());
        runtime.spawn(engine.run(input_rx));
        Ok(Self {
            inputs,
            events,
            tokens,
            runtime: Some(runtime),
        })
    }

    pub fn send(&self, request: ServerRequest) {
        self.command(Command::Server(request));
    }

    pub(crate) fn command(&self, command: Command) {
        let _ = self.inputs.send(Input::Command(command));
    }

    /// A fresh token for searches and transfers, unique within this session.
    pub fn token(&self) -> u32 {
        next_token(&self.tokens)
    }

    pub fn try_event(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    pub fn wait_event(&mut self, timeout: Duration) -> Option<Event> {
        let runtime = self.runtime.as_ref()?;
        runtime.block_on(async {
            tokio::time::timeout(timeout, self.events.recv())
                .await
                .ok()?
        })
    }

    pub fn user_info(&self, username: &str) {
        self.command(Command::UserInfo(username.to_string()));
    }

    pub fn browse(&self, username: &str) {
        self.command(Command::Browse(username.to_string()));
    }

    /// Asks for one folder and everything under it; the reply carries the returned token.
    pub fn folder_contents(&self, username: &str, folder: &str) -> u32 {
        let token = self.token();
        self.command(Command::FolderContents {
            username: username.to_string(),
            token,
            folder: folder.to_string(),
        });
        token
    }

    /// Users whose requests and connections are refused.
    pub fn set_ignored(&self, usernames: Vec<String>) {
        self.command(Command::SetIgnored(usernames));
    }

    pub fn set_profile(&self, profile: Profile) {
        self.command(Command::SetProfile(profile));
    }

    pub fn disconnect(&self) {
        self.command(Command::Disconnect);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.command(Command::Disconnect);
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

pub(crate) fn next_token(tokens: &AtomicU32) -> u32 {
    loop {
        let token = tokens.fetch_add(1, Ordering::Relaxed);
        if token != 0 {
            return token;
        }
    }
}

/// Starting from a time-derived value keeps tokens from one session apart from the next.
fn seed_token() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos() ^ elapsed.as_secs() as u32)
        .unwrap_or(1);
    nanos.max(1)
}
