use std::time::{Duration, Instant};

use slsk::proto::peer::UserInfo;
use slsk::proto::server::{ServerRequest, ServerResponse};
use slsk::proto::types::{UserStats, UserStatus};

const LOOKUP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Presence {
    #[default]
    Unknown,
    Offline,
    Away,
    Online,
}

impl From<UserStatus> for Presence {
    fn from(status: UserStatus) -> Self {
        match status {
            UserStatus::Offline => Presence::Offline,
            UserStatus::Away => Presence::Away,
            UserStatus::Online => Presence::Online,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserCard {
    pub username: String,
    pub presence: Presence,
    pub privileged: bool,
    pub stats: Option<UserStats>,
    pub peer: Option<UserInfo>,
    pub loading: bool,
}

impl UserCard {
    /// Takes in a status or stats reply about this user; returns true when something changed.
    fn absorb(&mut self, message: &ServerResponse) -> bool {
        let before = self.clone();
        match message {
            ServerResponse::WatchUser {
                username,
                exists,
                status,
                stats,
                ..
            } if *username == self.username => {
                self.presence = if *exists {
                    (*status).into()
                } else {
                    Presence::Offline
                };
                if *exists {
                    self.stats = Some(*stats);
                }
            }
            ServerResponse::GetUserStatus {
                username,
                status,
                privileged,
            } if *username == self.username => {
                self.presence = (*status).into();
                self.privileged = *privileged;
            }
            ServerResponse::GetUserStats { username, stats } if *username == self.username => {
                self.stats = Some(*stats);
            }
            _ => return false,
        }
        *self != before
    }
}

/// Watched users, whose status changes the server pushes to us.
#[derive(Default)]
pub struct Buddies {
    cards: Vec<UserCard>,
}

impl Buddies {
    pub fn set(&mut self, names: Vec<String>) {
        self.cards = names
            .into_iter()
            .map(|username| UserCard {
                username,
                ..Default::default()
            })
            .collect();
    }

    pub fn cards(&self) -> Vec<UserCard> {
        self.cards.clone()
    }

    pub fn add(&mut self, name: String) -> Vec<ServerRequest> {
        if self.cards.iter().any(|card| card.username == name) {
            return Vec::new();
        }
        self.cards.push(UserCard {
            username: name.clone(),
            ..Default::default()
        });
        watch(name)
    }

    pub fn remove(&mut self, name: &str) -> Vec<ServerRequest> {
        self.cards.retain(|card| card.username != name);
        vec![ServerRequest::UnwatchUser(name.to_string())]
    }

    pub fn watch_all(&self) -> Vec<ServerRequest> {
        self.cards
            .iter()
            .flat_map(|card| watch(card.username.clone()))
            .collect()
    }

    /// Returns the buddy list when a reply changed it.
    pub fn apply(&mut self, message: &ServerResponse) -> Option<Vec<UserCard>> {
        let mut changed = false;
        for card in &mut self.cards {
            changed |= card.absorb(message);
        }
        changed.then(|| self.cards.clone())
    }
}

fn watch(name: String) -> Vec<ServerRequest> {
    vec![
        ServerRequest::WatchUser(name.clone()),
        ServerRequest::GetUserStatus(name),
    ]
}

/// One user's status, stats and self-description, gathered from three separate replies.
#[derive(Default)]
pub struct Lookup {
    card: Option<UserCard>,
    deadline: Option<Instant>,
    has_status: bool,
    has_stats: bool,
}

impl Lookup {
    pub fn start(&mut self, username: String) -> Vec<ServerRequest> {
        self.card = Some(UserCard {
            username: username.clone(),
            loading: true,
            ..Default::default()
        });
        self.deadline = Some(Instant::now() + LOOKUP_TIMEOUT);
        self.has_status = false;
        self.has_stats = false;
        vec![
            ServerRequest::GetUserStatus(username.clone()),
            ServerRequest::GetUserStats(username),
        ]
    }

    pub fn apply(&mut self, message: &ServerResponse) -> Option<UserCard> {
        let card = self.card.as_mut()?;
        if !card.absorb(message) {
            return None;
        }
        match message {
            ServerResponse::GetUserStatus { .. } => self.has_status = true,
            ServerResponse::GetUserStats { .. } => self.has_stats = true,
            _ => {}
        }
        self.finish_if_done()
    }

    pub fn apply_info(&mut self, username: &str, info: UserInfo) -> Option<UserCard> {
        let card = self
            .card
            .as_mut()
            .filter(|card| card.username == username)?;
        card.peer = Some(info);
        self.finish_if_done()
    }

    /// The peer could not be reached for its description; what the server said is all there is.
    pub fn info_failed(&mut self, username: &str) -> Option<UserCard> {
        self.card
            .as_ref()
            .filter(|card| card.username == username)?;
        self.finish()
    }

    pub fn tick(&mut self) -> Option<UserCard> {
        if self.deadline.is_some_and(|at| Instant::now() >= at) {
            return self.finish();
        }
        None
    }

    fn finish_if_done(&mut self) -> Option<UserCard> {
        let card = self.card.as_ref()?;
        let complete = self.has_status && self.has_stats && card.peer.is_some();
        if complete || (self.has_status && card.presence == Presence::Offline) {
            return self.finish();
        }
        Some(card.clone())
    }

    fn finish(&mut self) -> Option<UserCard> {
        self.deadline = None;
        let mut card = self.card.take()?;
        card.loading = false;
        Some(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buddies_follow_status_pushes() {
        let mut buddies = Buddies::default();
        buddies.set(vec!["ann".into()]);
        let cards = buddies
            .apply(&ServerResponse::GetUserStatus {
                username: "ann".into(),
                status: UserStatus::Away,
                privileged: true,
            })
            .unwrap();
        assert_eq!(cards[0].presence, Presence::Away);
        assert!(cards[0].privileged);
        assert!(
            buddies
                .apply(&ServerResponse::GetUserStatus {
                    username: "bob".into(),
                    status: UserStatus::Online,
                    privileged: false,
                })
                .is_none()
        );
    }

    #[test]
    fn a_lookup_finishes_when_all_replies_are_in() {
        let mut lookup = Lookup::default();
        lookup.start("ann".into());
        let card = lookup
            .apply(&ServerResponse::GetUserStatus {
                username: "ann".into(),
                status: UserStatus::Online,
                privileged: false,
            })
            .unwrap();
        assert!(card.loading);
        lookup.apply(&ServerResponse::GetUserStats {
            username: "ann".into(),
            stats: UserStats::default(),
        });
        let card = lookup.apply_info("ann", UserInfo::default()).unwrap();
        assert!(!card.loading);
        assert!(lookup.tick().is_none());
    }
}
