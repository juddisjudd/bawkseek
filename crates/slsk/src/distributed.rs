use std::collections::{HashMap, HashSet};

use tokio::net::TcpStream;

use crate::engine::Engine;
use crate::peers::{ConnId, Purpose};
use crate::proto::distributed::{DistribMessage, code};
use crate::proto::server::{PossibleParent, ServerRequest};
use crate::proto::types::ConnectionType;
use crate::wire::Writer;

const MAX_CANDIDATES: usize = 10;
const MAX_CHILDREN: usize = 10;

struct Candidate {
    username: String,
    level: Option<i32>,
    root: Option<String>,
}

/// Our place in the distributed search tree: who feeds us searches, and whom we pass them on to.
pub(crate) struct Distributed {
    parent: Option<ConnId>,
    level: i32,
    root: String,
    candidates: HashMap<ConnId, Candidate>,
    dialing: HashSet<String>,
    children: HashMap<ConnId, String>,
    min_speed: u32,
    speed_ratio: u32,
    pub(crate) own_speed: u32,
    pub(crate) accept_children: bool,
}

impl Distributed {
    pub(crate) fn new(accept_children: bool) -> Self {
        Self {
            parent: None,
            level: 0,
            root: String::new(),
            candidates: HashMap::new(),
            dialing: HashSet::new(),
            children: HashMap::new(),
            min_speed: 0,
            speed_ratio: 0,
            own_speed: 0,
            accept_children,
        }
    }

    /// Children we can feed, from our upload speed and the ratio the server hands out.
    fn capacity(&self) -> usize {
        if !self.accept_children || self.speed_ratio == 0 || self.own_speed < self.min_speed {
            return 0;
        }
        ((self.own_speed / self.speed_ratio / 100) as usize).min(MAX_CHILDREN)
    }
}

impl Engine {
    /// Tells the server where we stand: after login, and whenever our parent changes.
    pub(crate) fn announce_branch(&mut self) {
        let has_parent = self.distributed.parent.is_some();
        if !has_parent {
            self.distributed.level = 0;
            self.distributed.root = self.config.username.clone();
        }
        let level = self.distributed.level;
        let root = self.distributed.root.clone();
        let accept = has_parent && self.distributed.capacity() > 0;
        self.send_server(ServerRequest::HaveNoParent(!has_parent));
        self.send_server(ServerRequest::BranchRoot(root.clone()));
        self.send_server(ServerRequest::BranchLevel(level.max(0) as u32));
        self.send_server(ServerRequest::AcceptChildren(accept));
        let level_frame = DistribMessage::BranchLevel(level).encode();
        let root_frame = DistribMessage::BranchRoot(root).encode();
        for conn in self.distributed.children.keys() {
            if let Some(writer) = self.peer_writer(*conn) {
                let _ = writer.send(level_frame.clone());
                let _ = writer.send(root_frame.clone());
            }
        }
    }

    pub(crate) fn on_possible_parents(&mut self, parents: Vec<PossibleParent>) {
        if self.distributed.parent.is_some() {
            return;
        }
        for parent in parents {
            if self.distributed.dialing.len() >= MAX_CANDIDATES {
                break;
            }
            if parent.username == self.config.username
                || self.distributed.dialing.contains(&parent.username)
            {
                continue;
            }
            self.distributed.dialing.insert(parent.username.clone());
            self.open_peer_at(
                &parent.username,
                ConnectionType::Distributed,
                Purpose::Parent,
                parent.ip,
                parent.port,
            );
        }
    }

    pub(crate) fn parent_connected(&mut self, username: String, stream: TcpStream) {
        if self.distributed.parent.is_some() {
            self.distributed.dialing.remove(&username);
            return;
        }
        let conn = self.register(username.clone(), ConnectionType::Distributed, stream);
        self.distributed.candidates.insert(
            conn,
            Candidate {
                username,
                level: None,
                root: None,
            },
        );
    }

    pub(crate) fn parent_failed(&mut self, username: &str) {
        self.distributed.dialing.remove(username);
    }

    pub(crate) fn child_connected(&mut self, username: String, stream: TcpStream) {
        let room = self.distributed.capacity() > self.distributed.children.len();
        let fed = self.distributed.parent.is_some();
        if !room || !fed || username == self.config.username {
            return;
        }
        if self
            .distributed
            .children
            .values()
            .any(|child| *child == username)
        {
            return;
        }
        let conn = self.register(username.clone(), ConnectionType::Distributed, stream);
        self.distributed.children.insert(conn, username);
        if let Some(writer) = self.peer_writer(conn) {
            let _ = writer.send(DistribMessage::BranchLevel(self.distributed.level).encode());
            let _ = writer.send(DistribMessage::BranchRoot(self.distributed.root.clone()).encode());
        }
        if self.distributed.children.len() >= self.distributed.capacity() {
            self.send_server(ServerRequest::AcceptChildren(false));
        }
    }

    pub(crate) fn on_distrib_frame(&mut self, conn: ConnId, code: u8, body: Vec<u8>) {
        if self.distributed.children.contains_key(&conn) {
            return;
        }
        let message = match DistribMessage::decode(code, &body) {
            Ok(message) => message,
            Err(err) => {
                log::debug!("distributed message {code}: {err}");
                return;
            }
        };
        let from_parent = self.distributed.parent == Some(conn);
        match message {
            DistribMessage::BranchLevel(level) => {
                if from_parent {
                    self.distributed.level = level + 1;
                    self.announce_branch();
                } else if let Some(candidate) = self.distributed.candidates.get_mut(&conn) {
                    candidate.level = Some(level);
                    if level == 0 {
                        candidate.root = Some(candidate.username.clone());
                    }
                }
            }
            DistribMessage::BranchRoot(root) => {
                if from_parent {
                    self.distributed.root = root;
                    self.announce_branch();
                } else if let Some(candidate) = self.distributed.candidates.get_mut(&conn) {
                    candidate.root = Some(root);
                }
            }
            DistribMessage::Search {
                username,
                token,
                query,
            } => {
                if !from_parent && !self.try_adopt(conn) {
                    return;
                }
                log::trace!("distributed search from {username}: {query}");
                self.pass_to_children(Writer::frame_u8(code, &body));
                self.answer_search(&username, token, &query);
            }
            DistribMessage::EmbeddedMessage { code, payload } if from_parent => {
                self.on_embedded(code, payload);
            }
            _ => {}
        }
    }

    /// The first candidate that has told us its branch and then sends a search becomes our parent.
    fn try_adopt(&mut self, conn: ConnId) -> bool {
        if self.distributed.parent.is_some() {
            return false;
        }
        let Some(candidate) = self.distributed.candidates.get(&conn) else {
            return false;
        };
        let (Some(level), Some(root)) = (candidate.level, candidate.root.clone()) else {
            return false;
        };
        if level < 0 {
            return false;
        }
        let others: Vec<ConnId> = self
            .distributed
            .candidates
            .keys()
            .copied()
            .filter(|other| *other != conn)
            .collect();
        for other in others {
            self.drop_conn(other);
        }
        if let Some(candidate) = self.distributed.candidates.remove(&conn) {
            log::info!("distributed parent: {} (level {level})", candidate.username);
        }
        self.distributed.candidates.clear();
        self.distributed.dialing.clear();
        self.distributed.parent = Some(conn);
        self.distributed.level = level + 1;
        self.distributed.root = root;
        self.announce_branch();
        true
    }

    /// A search the server hands us directly, which means we are a branch root.
    pub(crate) fn on_embedded(&mut self, code: u8, payload: Vec<u8>) {
        if code != code::SEARCH {
            return;
        }
        self.pass_to_children(Writer::frame_u8(code, &payload));
        if let Ok(DistribMessage::Search {
            username,
            token,
            query,
        }) = DistribMessage::decode(code, &payload)
        {
            self.answer_search(&username, token, &query);
        }
    }

    fn pass_to_children(&mut self, frame: Vec<u8>) {
        let children: Vec<ConnId> = self.distributed.children.keys().copied().collect();
        for conn in children {
            if let Some(writer) = self.peer_writer(conn)
                && writer.send(frame.clone()).is_err()
            {
                self.drop_conn(conn);
            }
        }
    }

    pub(crate) fn distrib_closed(&mut self, conn: ConnId, username: &str) {
        if self.distributed.parent == Some(conn) {
            log::info!("lost distributed parent {username}");
            self.lose_parent();
        } else if self.distributed.candidates.remove(&conn).is_some() {
            self.distributed.dialing.remove(username);
        } else if self.distributed.children.remove(&conn).is_some()
            && self.distributed.parent.is_some()
            && self.distributed.capacity() > self.distributed.children.len()
        {
            self.send_server(ServerRequest::AcceptChildren(true));
        }
    }

    /// Without a parent we cannot feed children, so they are let go to find another.
    pub(crate) fn lose_parent(&mut self) {
        self.distributed.parent = None;
        let conns: Vec<ConnId> = self
            .distributed
            .children
            .keys()
            .chain(self.distributed.candidates.keys())
            .copied()
            .collect();
        self.distributed.children.clear();
        self.distributed.candidates.clear();
        self.distributed.dialing.clear();
        for conn in conns {
            self.drop_conn(conn);
        }
        self.announce_branch();
    }

    pub(crate) fn reset_distributed(&mut self) {
        if let Some(parent) = self.distributed.parent.take() {
            self.drop_conn(parent);
        }
        self.lose_parent();
    }

    pub(crate) fn set_parent_min_speed(&mut self, speed: u32) {
        self.distributed.min_speed = speed;
    }

    pub(crate) fn set_parent_speed_ratio(&mut self, ratio: u32) {
        self.distributed.speed_ratio = ratio;
    }

    pub(crate) fn set_own_speed(&mut self, speed: u32) {
        self.distributed.own_speed = speed;
    }
}
