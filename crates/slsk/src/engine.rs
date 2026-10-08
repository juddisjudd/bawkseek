use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::{Instant, MissedTickBehavior, interval};

use tokio::net::TcpStream;

use crate::client::{Command, Config, Event, MAJOR_VERSION, MINOR_VERSION, Profile, Session};
use crate::io::{connect, read_frame, spawn_writer, split_u32};
use crate::peers::{ConnId, Peers};
use crate::proto::peer::PeerInit;
use crate::proto::server::{LoginResponse, ServerRequest, ServerResponse};
use crate::proto::types::ConnectionType;
use crate::proto::types::UserStatus;
use crate::requests::Waiting;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// The server omits its login reply to banned users, so a silent server counts as a lost connection.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(30);
const PING_EVERY: Duration = Duration::from_secs(5 * 60);
const BACKOFF: [u64; 6] = [2, 5, 10, 20, 40, 60];

pub(crate) enum Input {
    Command(Command),
    ServerConnected {
        generation: u64,
        writer: UnboundedSender<Vec<u8>>,
    },
    ServerFrame {
        generation: u64,
        frame: Vec<u8>,
    },
    ServerClosed {
        generation: u64,
        error: String,
    },
    Incoming {
        stream: TcpStream,
        init: PeerInit,
    },
    Outbound {
        token: u32,
        stream: TcpStream,
    },
    OutboundFailed {
        token: u32,
    },
    Pierced {
        username: String,
        kind: ConnectionType,
        stream: TcpStream,
    },
    PierceFailed {
        username: String,
        token: u32,
    },
    FileIncoming {
        username: String,
        token: u32,
        stream: TcpStream,
    },
    PeerFrame {
        conn: ConnId,
        frame: Vec<u8>,
    },
    PeerClosed {
        conn: ConnId,
    },
}

struct ServerLink {
    writer: UnboundedSender<Vec<u8>>,
    logged_in: bool,
    connected_at: Instant,
}

pub(crate) struct Engine {
    pub(crate) config: Config,
    pub(crate) events: UnboundedSender<Event>,
    pub(crate) inputs: UnboundedSender<Input>,
    pub(crate) tokens: Arc<AtomicU32>,
    pub(crate) peers: Peers,
    pub(crate) waiting: Waiting,
    pub(crate) profile: Profile,
    ignored: HashSet<String>,
    server: Option<ServerLink>,
    generation: u64,
    attempt: u32,
    reconnect_at: Option<Instant>,
    finished: bool,
    last_ping: Instant,
    status: UserStatus,
    rooms: BTreeSet<String>,
    watched: BTreeSet<String>,
    public_feed: bool,
}

impl Engine {
    pub(crate) fn new(
        config: Config,
        events: UnboundedSender<Event>,
        inputs: UnboundedSender<Input>,
        tokens: Arc<AtomicU32>,
    ) -> Self {
        Self {
            config,
            events,
            inputs,
            tokens,
            peers: Peers::default(),
            waiting: Waiting::default(),
            profile: Profile::default(),
            ignored: HashSet::new(),
            server: None,
            generation: 0,
            attempt: 0,
            reconnect_at: None,
            finished: false,
            last_ping: Instant::now(),
            status: UserStatus::Online,
            rooms: BTreeSet::new(),
            watched: BTreeSet::new(),
            public_feed: false,
        }
    }

    pub(crate) async fn run(mut self, mut inputs: UnboundedReceiver<Input>) {
        self.start_listener();
        self.connect_server();
        let mut tick = interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        while !self.finished {
            tokio::select! {
                input = inputs.recv() => match input {
                    Some(input) => self.handle(input),
                    None => break,
                },
                _ = tick.tick() => self.on_tick(),
            }
        }
    }

    pub(crate) fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn session(&self, session: Session) {
        self.emit(Event::Session(session));
    }

    fn handle(&mut self, input: Input) {
        match input {
            Input::Command(command) => self.on_command(command),
            Input::ServerConnected { generation, writer } if generation == self.generation => {
                let login = ServerRequest::Login {
                    username: self.config.username.clone(),
                    password: self.config.password.clone(),
                    major: MAJOR_VERSION,
                    minor: MINOR_VERSION,
                };
                let _ = writer.send(login.encode());
                self.server = Some(ServerLink {
                    writer,
                    logged_in: false,
                    connected_at: Instant::now(),
                });
            }
            Input::ServerFrame { generation, frame } if generation == self.generation => {
                let Some((code, body)) = split_u32(&frame) else {
                    return;
                };
                match ServerResponse::decode(code, body) {
                    Ok(message) => self.on_server(message),
                    Err(err) => log::warn!("server message {code} unreadable: {err}"),
                }
            }
            Input::ServerClosed { generation, error } if generation == self.generation => {
                self.server_lost(error);
            }
            Input::ServerConnected { .. }
            | Input::ServerFrame { .. }
            | Input::ServerClosed { .. } => {}
            Input::Incoming { stream, init } => self.on_incoming(stream, init),
            Input::Outbound { token, stream } => self.on_outbound(token, stream),
            Input::OutboundFailed { token } => self.on_outbound_failed(token),
            Input::Pierced {
                username,
                kind,
                stream,
            } => self.accept(username, kind, stream),
            Input::PierceFailed { username, token } => self.on_pierce_failed(username, token),
            Input::FileIncoming {
                username,
                token,
                stream,
            } => self.on_file_incoming(username, token, stream),
            Input::PeerFrame { conn, frame } => self.on_peer_frame(conn, frame),
            Input::PeerClosed { conn } => self.on_peer_closed(conn),
        }
    }

    fn on_command(&mut self, command: Command) {
        match command {
            Command::Server(request) => {
                self.remember(&request);
                self.send_server(request);
            }
            Command::Disconnect => {
                self.finished = true;
                self.server = None;
                self.session(Session::Disconnected);
            }
            Command::UserInfo(username) => self.request_user_info(username),
            Command::Browse(username) => self.request_browse(username),
            Command::FolderContents {
                username,
                token,
                folder,
            } => self.request_folder(username, token, folder),
            Command::SetIgnored(usernames) => self.ignored = usernames.into_iter().collect(),
            Command::SetProfile(profile) => self.profile = profile,
        }
    }

    /// Tracks the requests that must be repeated after a reconnect.
    fn remember(&mut self, request: &ServerRequest) {
        match request {
            ServerRequest::JoinRoom { room, .. } => {
                self.rooms.insert(room.clone());
            }
            ServerRequest::LeaveRoom(room) => {
                self.rooms.remove(room);
            }
            ServerRequest::WatchUser(user) => {
                self.watched.insert(user.clone());
            }
            ServerRequest::UnwatchUser(user) => {
                self.watched.remove(user);
            }
            ServerRequest::SetStatus(status) => self.status = *status,
            ServerRequest::JoinGlobalRoom => self.public_feed = true,
            ServerRequest::LeaveGlobalRoom => self.public_feed = false,
            _ => {}
        }
    }

    pub(crate) fn is_ignored(&self, username: &str) -> bool {
        self.ignored.contains(username)
    }

    pub(crate) fn send_server(&mut self, request: ServerRequest) {
        if let Some(server) = self.server.as_ref().filter(|server| server.logged_in) {
            let _ = server.writer.send(request.encode());
        }
    }

    fn connect_server(&mut self) {
        self.generation += 1;
        self.attempt += 1;
        self.reconnect_at = None;
        self.session(Session::Connecting {
            attempt: self.attempt,
        });
        let generation = self.generation;
        let inputs = self.inputs.clone();
        let addr = self.config.server.clone();
        tokio::spawn(async move {
            let stream = match connect(&addr, CONNECT_TIMEOUT).await {
                Ok(stream) => stream,
                Err(err) => {
                    let _ = inputs.send(Input::ServerClosed {
                        generation,
                        error: err.to_string(),
                    });
                    return;
                }
            };
            let (mut read, write) = stream.into_split();
            let writer = spawn_writer(write);
            let _ = inputs.send(Input::ServerConnected { generation, writer });
            let error = loop {
                match read_frame(&mut read).await {
                    Ok(frame) => {
                        if inputs
                            .send(Input::ServerFrame { generation, frame })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(err) => break err.to_string(),
                }
            };
            let _ = inputs.send(Input::ServerClosed { generation, error });
        });
    }

    fn server_lost(&mut self, error: String) {
        self.server = None;
        if self.finished {
            return;
        }
        if !self.config.reconnect {
            self.finished = true;
            self.session(Session::Lost {
                error,
                retry_in: None,
            });
            return;
        }
        let step = (self.attempt as usize)
            .saturating_sub(1)
            .min(BACKOFF.len() - 1);
        let delay = Duration::from_secs(BACKOFF[step]);
        self.reconnect_at = Some(Instant::now() + delay);
        self.session(Session::Lost {
            error,
            retry_in: Some(delay),
        });
    }

    fn on_tick(&mut self) {
        self.tick_peers();
        self.tick_requests();
        if self.reconnect_at.is_some_and(|at| Instant::now() >= at) {
            self.connect_server();
            return;
        }
        let Some(server) = &self.server else {
            return;
        };
        if !server.logged_in && server.connected_at.elapsed() > LOGIN_TIMEOUT {
            self.generation += 1;
            self.server_lost("the server did not answer the login".into());
            return;
        }
        if server.logged_in && self.last_ping.elapsed() > PING_EVERY {
            self.last_ping = Instant::now();
            self.send_server(ServerRequest::ServerPing);
        }
    }

    fn on_server(&mut self, message: ServerResponse) {
        match message {
            ServerResponse::Login(LoginResponse::Success {
                greeting,
                own_ip,
                supporter,
            }) => {
                if let Some(server) = &mut self.server {
                    server.logged_in = true;
                }
                self.attempt = 0;
                self.last_ping = Instant::now();
                self.after_login();
                self.session(Session::LoggedIn {
                    greeting,
                    own_ip,
                    supporter,
                });
            }
            ServerResponse::Login(LoginResponse::Failure { reason, detail }) => {
                self.finished = true;
                self.server = None;
                self.session(Session::Rejected { reason, detail });
            }
            ServerResponse::Relogged => {
                self.finished = true;
                self.server = None;
                self.session(Session::Relogged);
            }
            ServerResponse::GetPeerAddress { username, ip, port } => {
                self.on_peer_address(&username, ip, port);
            }
            ServerResponse::ConnectToPeer {
                username,
                kind,
                ip,
                port,
                token,
                ..
            } => self.on_connect_to_peer(username, kind, ip, port, token),
            ServerResponse::CantConnectToPeer(token) => self.on_cant_connect(token),
            message => {
                if let ServerResponse::MessageUser { id, .. } = &message {
                    self.send_server(ServerRequest::MessageAcked(*id));
                }
                if let ServerResponse::JoinRoom(joined) = &message {
                    self.rooms.insert(joined.room.clone());
                }
                if let ServerResponse::LeaveRoom(room) = &message {
                    self.rooms.remove(room);
                }
                self.emit(Event::Server(message));
            }
        }
    }

    /// What the server expects right after login, then whatever the last session had set up.
    fn after_login(&mut self) {
        self.send_server(ServerRequest::SetWaitPort(self.config.listen_port));
        self.send_server(ServerRequest::SharedFoldersFiles { dirs: 0, files: 0 });
        self.send_server(ServerRequest::HaveNoParent(true));
        self.send_server(ServerRequest::SetStatus(self.status));
        self.send_server(ServerRequest::CheckPrivileges);
        if self.public_feed {
            self.send_server(ServerRequest::JoinGlobalRoom);
        }
        for room in self.rooms.clone() {
            self.send_server(ServerRequest::JoinRoom {
                room,
                private: false,
            });
        }
        for user in self.watched.clone() {
            self.send_server(ServerRequest::WatchUser(user));
        }
    }
}
