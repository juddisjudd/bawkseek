use std::collections::HashSet;
use std::time::Duration;

use tokio::time::Instant;

use crate::client::{Event, SearchScope};
use crate::engine::Engine;
use crate::proto::peer::{PeerMessage, SearchReply};
use crate::proto::server::ServerRequest;
use crate::shares::ShareIndex;

/// The most files one reply of ours lists.
const MAX_REPLY_FILES: usize = 300;
/// Incoming searches answered per second; the rest are still passed on to distributed children.
const ANSWER_BUDGET: u32 = 100;

#[derive(Default)]
pub(crate) struct Searches {
    active: HashSet<u32>,
    window: Option<Instant>,
    answered: u32,
}

impl Engine {
    pub(crate) fn start_search(&mut self, token: u32, scope: SearchScope, query: String) {
        self.searches.active.insert(token);
        let request = match scope {
            SearchScope::Network => ServerRequest::FileSearch { token, query },
            SearchScope::User(username) => ServerRequest::UserSearch {
                username,
                token,
                query,
            },
            SearchScope::Room(room) => ServerRequest::RoomSearch { room, token, query },
            SearchScope::Wishlist => ServerRequest::WishlistSearch { token, query },
        };
        self.send_server(request);
    }

    pub(crate) fn forget_search(&mut self, token: u32) {
        self.searches.active.remove(&token);
    }

    /// Replies are matched by token and attributed to the connection they came on, not to the name inside them.
    pub(crate) fn on_search_reply(&mut self, username: &str, mut reply: SearchReply) {
        if !self.searches.active.contains(&reply.token) {
            return;
        }
        reply.username = username.to_string();
        self.emit(Event::SearchReply(reply));
    }

    pub(crate) fn answer_search(&mut self, username: &str, token: u32, query: &str) {
        if username == self.config.username || self.is_ignored(username) {
            return;
        }
        let now = Instant::now();
        if self
            .searches
            .window
            .is_none_or(|start| now.duration_since(start) >= Duration::from_secs(1))
        {
            self.searches.window = Some(now);
            self.searches.answered = 0;
        }
        if self.searches.answered >= ANSWER_BUDGET {
            return;
        }
        self.searches.answered += 1;
        let files: Vec<_> = self
            .shares
            .search(query, &self.excluded_phrases, MAX_REPLY_FILES)
            .into_iter()
            .map(ShareIndex::result_entry)
            .collect();
        if files.is_empty() {
            return;
        }
        let reply = SearchReply {
            username: self.config.username.clone(),
            token,
            files,
            slot_free: self.free_upload_slots() > 0,
            avg_speed: self.distributed.own_speed,
            queue_len: self.queued_uploads() as u32,
            private_files: Vec::new(),
        };
        self.send_peer(username, PeerMessage::FileSearchResponse(reply));
    }
}
