use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use crate::client::{Event, next_token};
use crate::engine::{Engine, Input};
use crate::io::{connect, read_frame, spawn_writer, split_u8, split_u32};
use crate::proto::peer::{PeerInit, PeerMessage};
use crate::proto::server::ServerRequest;
use crate::proto::types::ConnectionType;

const DIRECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for either a direct connection or the peer piercing our firewall.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
const INIT_TIMEOUT: Duration = Duration::from_secs(15);
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) type ConnId = u64;

pub(crate) struct PeerConn {
    pub(crate) username: String,
    pub(crate) kind: ConnectionType,
    pub(crate) writer: UnboundedSender<Vec<u8>>,
    reader: AbortHandle,
    last_active: Instant,
}

/// What an outgoing connection is for, once it opens.
pub(crate) enum Purpose {
    Messages(Vec<Vec<u8>>),
    Parent,
    Upload(u64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direct {
    Waiting,
    Trying,
    Failed,
}

pub(crate) struct Attempt {
    username: String,
    kind: ConnectionType,
    purpose: Purpose,
    started: Instant,
    direct: Direct,
    indirect_failed: bool,
}

#[derive(Default)]
pub(crate) struct Peers {
    next_conn: ConnId,
    pub(crate) conns: HashMap<ConnId, PeerConn>,
    by_user: HashMap<(String, ConnectionType), ConnId>,
    attempts: HashMap<u32, Attempt>,
}

impl Engine {
    pub(crate) fn start_listener(&mut self) {
        let port = self.config.listen_port;
        let inputs = self.inputs.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let listener =
                match TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)).await {
                    Ok(listener) => listener,
                    Err(err) => {
                        let _ = events.send(Event::ListenFailed {
                            port,
                            error: err.to_string(),
                        });
                        return;
                    }
                };
            let _ = events.send(Event::Listening { port });
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    continue;
                };
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    let _ = stream.set_nodelay(true);
                    let Ok(Ok(frame)) =
                        tokio::time::timeout(INIT_TIMEOUT, read_frame(&mut stream)).await
                    else {
                        return;
                    };
                    let Some((code, body)) = split_u8(&frame) else {
                        return;
                    };
                    if let Ok(init) = PeerInit::decode(code, body) {
                        let _ = inputs.send(Input::Incoming { stream, init });
                    }
                });
            }
        });
    }

    /// Sends a peer message over the open connection to `username`, opening one first if needed.
    pub(crate) fn send_peer(&mut self, username: &str, message: PeerMessage) {
        self.send_peer_frame(username, message.encode());
    }

    pub(crate) fn send_peer_frame(&mut self, username: &str, frame: Vec<u8>) {
        let key = (username.to_string(), ConnectionType::Peer);
        if let Some(conn) = self.peers.by_user.get(&key).copied()
            && let Some(peer) = self.peers.conns.get_mut(&conn)
        {
            if peer.writer.send(frame.clone()).is_ok() {
                peer.last_active = Instant::now();
                return;
            }
            self.drop_conn(conn);
        }
        let pending = self.peers.attempts.values_mut().find(|attempt| {
            attempt.username == username
                && attempt.kind == ConnectionType::Peer
                && matches!(attempt.purpose, Purpose::Messages(_))
        });
        if let Some(Attempt {
            purpose: Purpose::Messages(queue),
            ..
        }) = pending
        {
            queue.push(frame);
            return;
        }
        self.open_peer(
            username,
            ConnectionType::Peer,
            Purpose::Messages(vec![frame]),
        );
    }

    /// Asks for a connection both ways at once: the server relays our request so the peer can pierce our firewall, while we try their address directly.
    pub(crate) fn open_peer(
        &mut self,
        username: &str,
        kind: ConnectionType,
        purpose: Purpose,
    ) -> u32 {
        let token = next_token(&self.tokens);
        self.peers.attempts.insert(
            token,
            Attempt {
                username: username.to_string(),
                kind,
                purpose,
                started: Instant::now(),
                direct: Direct::Waiting,
                indirect_failed: false,
            },
        );
        self.send_server(ServerRequest::ConnectToPeer {
            token,
            username: username.to_string(),
            kind,
        });
        self.send_server(ServerRequest::GetPeerAddress(username.to_string()));
        token
    }

    pub(crate) fn on_peer_address(&mut self, username: &str, ip: Ipv4Addr, port: u16) {
        let tokens: Vec<u32> = self
            .peers
            .attempts
            .iter()
            .filter(|(_, attempt)| {
                attempt.username == username && attempt.direct == Direct::Waiting
            })
            .map(|(token, _)| *token)
            .collect();
        for token in tokens {
            self.dial(token, ip, port);
        }
    }

    /// Like `open_peer`, for a peer whose address we already have.
    pub(crate) fn open_peer_at(
        &mut self,
        username: &str,
        kind: ConnectionType,
        purpose: Purpose,
        ip: Ipv4Addr,
        port: u16,
    ) -> u32 {
        let token = next_token(&self.tokens);
        self.peers.attempts.insert(
            token,
            Attempt {
                username: username.to_string(),
                kind,
                purpose,
                started: Instant::now(),
                direct: Direct::Waiting,
                indirect_failed: false,
            },
        );
        self.send_server(ServerRequest::ConnectToPeer {
            token,
            username: username.to_string(),
            kind,
        });
        self.dial(token, ip, port);
        token
    }

    fn dial(&mut self, token: u32, ip: Ipv4Addr, port: u16) {
        let Some(attempt) = self.peers.attempts.get_mut(&token) else {
            return;
        };
        if ip.is_unspecified() || port == 0 {
            attempt.direct = Direct::Failed;
            self.maybe_fail(token);
            return;
        }
        attempt.direct = Direct::Trying;
        let init = PeerInit::PeerInit {
            username: self.config.username.clone(),
            kind: attempt.kind,
            token: 0,
        }
        .encode();
        let inputs = self.inputs.clone();
        tokio::spawn(async move {
            let addr = format!("{ip}:{port}");
            let result = async {
                let mut stream = connect(&addr, DIRECT_TIMEOUT).await?;
                stream.write_all(&init).await?;
                Ok::<_, std::io::Error>(stream)
            }
            .await;
            let _ = inputs.send(match result {
                Ok(stream) => Input::Outbound { token, stream },
                Err(_) => Input::OutboundFailed { token },
            });
        });
    }

    pub(crate) fn on_outbound(&mut self, token: u32, stream: TcpStream) {
        if let Some(attempt) = self.peers.attempts.remove(&token) {
            self.adopt(attempt.username, attempt.kind, stream, attempt.purpose);
        }
    }

    pub(crate) fn on_outbound_failed(&mut self, token: u32) {
        if let Some(attempt) = self.peers.attempts.get_mut(&token) {
            attempt.direct = Direct::Failed;
            self.maybe_fail(token);
        }
    }

    pub(crate) fn on_cant_connect(&mut self, token: u32) {
        if let Some(attempt) = self.peers.attempts.get_mut(&token) {
            attempt.indirect_failed = true;
            self.maybe_fail(token);
        }
    }

    fn maybe_fail(&mut self, token: u32) {
        let failed = self
            .peers
            .attempts
            .get(&token)
            .is_some_and(|attempt| attempt.direct == Direct::Failed && attempt.indirect_failed);
        if failed {
            self.fail_attempt(token);
        }
    }

    fn fail_attempt(&mut self, token: u32) {
        let Some(attempt) = self.peers.attempts.remove(&token) else {
            return;
        };
        log::debug!("cannot reach {} ({:?})", attempt.username, attempt.kind);
        match attempt.purpose {
            Purpose::Messages(_) => self.peer_unreachable(&attempt.username),
            Purpose::Parent => self.parent_failed(&attempt.username),
            Purpose::Upload(id) => self.upload_unreachable(id),
        }
    }

    pub(crate) fn on_incoming(&mut self, stream: TcpStream, init: PeerInit) {
        match init {
            PeerInit::PierceFirewall(token) => {
                if let Some(attempt) = self.peers.attempts.remove(&token) {
                    self.adopt(attempt.username, attempt.kind, stream, attempt.purpose);
                }
            }
            PeerInit::PeerInit { username, kind, .. } => {
                if self.is_ignored(&username) {
                    return;
                }
                self.accept(username, kind, stream);
            }
        }
    }

    /// The server relays a peer who could not reach us; we connect to them and pierce with their token.
    pub(crate) fn on_connect_to_peer(
        &mut self,
        username: String,
        kind: ConnectionType,
        ip: Ipv4Addr,
        port: u16,
        token: u32,
    ) {
        if self.is_ignored(&username) {
            return;
        }
        let inputs = self.inputs.clone();
        tokio::spawn(async move {
            let addr = format!("{ip}:{port}");
            let result = async {
                let mut stream = connect(&addr, DIRECT_TIMEOUT).await?;
                stream
                    .write_all(&PeerInit::PierceFirewall(token).encode())
                    .await?;
                Ok::<_, std::io::Error>(stream)
            }
            .await;
            let _ = inputs.send(match result {
                Ok(stream) => Input::Pierced {
                    username,
                    kind,
                    stream,
                },
                Err(_) => Input::PierceFailed { username, token },
            });
        });
    }

    pub(crate) fn on_pierce_failed(&mut self, username: String, token: u32) {
        self.send_server(ServerRequest::CantConnectToPeer { token, username });
    }

    /// A connection the peer asked for, either directly or through the server.
    pub(crate) fn accept(&mut self, username: String, kind: ConnectionType, mut stream: TcpStream) {
        match kind {
            ConnectionType::Peer => {
                self.register(username, kind, stream);
            }
            ConnectionType::Distributed => self.child_connected(username, stream),
            ConnectionType::File => {
                let inputs = self.inputs.clone();
                tokio::spawn(async move {
                    let Ok(Ok(token)) =
                        tokio::time::timeout(INIT_TIMEOUT, stream.read_u32_le()).await
                    else {
                        return;
                    };
                    let _ = inputs.send(Input::FileIncoming {
                        username,
                        token,
                        stream,
                    });
                });
            }
        }
    }

    fn adopt(
        &mut self,
        username: String,
        kind: ConnectionType,
        stream: TcpStream,
        purpose: Purpose,
    ) {
        match purpose {
            Purpose::Messages(queue) => {
                let conn = self.register(username, kind, stream);
                if let Some(peer) = self.peers.conns.get(&conn) {
                    for frame in queue {
                        let _ = peer.writer.send(frame);
                    }
                }
            }
            Purpose::Parent => self.parent_connected(username, stream),
            Purpose::Upload(id) => self.upload_connected(id, stream),
        }
    }

    /// Starts reading framed messages from a `P` or `D` connection and makes it the one used for that peer.
    pub(crate) fn register(
        &mut self,
        username: String,
        kind: ConnectionType,
        stream: TcpStream,
    ) -> ConnId {
        self.peers.next_conn += 1;
        let conn = self.peers.next_conn;
        let (mut read, write) = stream.into_split();
        let writer = spawn_writer(write);
        let inputs = self.inputs.clone();
        let reader = tokio::spawn(async move {
            while let Ok(frame) = read_frame(&mut read).await {
                if inputs.send(Input::PeerFrame { conn, frame }).is_err() {
                    return;
                }
            }
            let _ = inputs.send(Input::PeerClosed { conn });
        })
        .abort_handle();
        self.peers.conns.insert(
            conn,
            PeerConn {
                username: username.clone(),
                kind,
                writer,
                reader,
                last_active: Instant::now(),
            },
        );
        self.peers.by_user.insert((username, kind), conn);
        conn
    }

    pub(crate) fn on_peer_frame(&mut self, conn: ConnId, frame: Vec<u8>) {
        let Some(peer) = self.peers.conns.get_mut(&conn) else {
            return;
        };
        peer.last_active = Instant::now();
        let username = peer.username.clone();
        match peer.kind {
            ConnectionType::Peer => {
                let Some((code, body)) = split_u32(&frame) else {
                    return;
                };
                match PeerMessage::decode(code, body) {
                    Ok(message) => self.on_peer_message(&username, message),
                    Err(err) => log::debug!("peer message {code} from {username}: {err}"),
                }
            }
            ConnectionType::Distributed => {
                if let Some((code, body)) = split_u8(&frame) {
                    self.on_distrib_frame(conn, code, body.to_vec());
                }
            }
            ConnectionType::File => {}
        }
    }

    pub(crate) fn on_peer_closed(&mut self, conn: ConnId) {
        if let Some(peer) = self.peers.conns.remove(&conn) {
            peer.reader.abort();
            let key = (peer.username.clone(), peer.kind);
            if self.peers.by_user.get(&key) == Some(&conn) {
                self.peers.by_user.remove(&key);
            }
            if peer.kind == ConnectionType::Distributed {
                self.distrib_closed(conn, &peer.username);
            }
        }
    }

    pub(crate) fn drop_conn(&mut self, conn: ConnId) {
        self.on_peer_closed(conn);
    }

    pub(crate) fn peer_writer(&self, conn: ConnId) -> Option<&UnboundedSender<Vec<u8>>> {
        self.peers.conns.get(&conn).map(|peer| &peer.writer)
    }

    pub(crate) fn tick_peers(&mut self) {
        let expired: Vec<u32> = self
            .peers
            .attempts
            .iter()
            .filter(|(_, attempt)| attempt.started.elapsed() > ATTEMPT_TIMEOUT)
            .map(|(token, _)| *token)
            .collect();
        for token in expired {
            self.fail_attempt(token);
        }
        let idle: Vec<ConnId> = self
            .peers
            .conns
            .iter()
            .filter(|(_, peer)| {
                peer.kind == ConnectionType::Peer
                    && peer.last_active.elapsed() > IDLE_TIMEOUT
                    && !self.waiting.involves(&peer.username)
            })
            .map(|(conn, _)| *conn)
            .collect();
        for conn in idle {
            self.drop_conn(conn);
        }
    }
}
