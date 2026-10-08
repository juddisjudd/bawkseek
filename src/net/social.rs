use std::time::{Duration, Instant};

use soulseek_rs::message::peer::PeerInfo;
use soulseek_rs::{Client, UserInfo, UserStats, UserStatus};

const BUDDY_REFRESH: Duration = Duration::from_secs(2);
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Presence {
    #[default]
    Unknown,
    Offline,
    Away,
    Online,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserCard {
    pub username: String,
    pub presence: Presence,
    pub privileged: bool,
    pub stats: Option<UserStats>,
    pub peer: Option<PeerInfo>,
    pub loading: bool,
}

fn card(username: &str, info: Option<UserInfo>, peer: Option<PeerInfo>, loading: bool) -> UserCard {
    let presence = info.as_ref().and_then(|info| info.presence);
    UserCard {
        username: username.to_string(),
        presence: match presence.map(|presence| presence.status) {
            None => Presence::Unknown,
            Some(UserStatus::Offline) => Presence::Offline,
            Some(UserStatus::Away) => Presence::Away,
            Some(UserStatus::Online) => Presence::Online,
        },
        privileged: presence.is_some_and(|presence| presence.privileged),
        stats: info.and_then(|info| info.stats),
        peer,
        loading,
    }
}

/// Watched users; the server pushes their status changes, which only show up by polling.
#[derive(Default)]
pub struct Buddies {
    names: Vec<String>,
    sent: Vec<UserCard>,
    checked: Option<Instant>,
}

impl Buddies {
    pub fn set(&mut self, names: Vec<String>) {
        self.names = names;
        self.checked = None;
    }

    pub fn add(&mut self, client: Option<&Client>, name: String) {
        if self.names.contains(&name) {
            return;
        }
        if let Some(client) = client {
            let _ = client.watch_user(&name);
            let _ = client.request_user_info(&name);
        }
        self.names.push(name);
        self.checked = None;
    }

    pub fn remove(&mut self, client: Option<&Client>, name: &str) {
        self.names.retain(|existing| existing != name);
        if let Some(client) = client {
            let _ = client.unwatch_user(name);
        }
        self.checked = None;
    }

    /// The library drops watches on re-login, so this runs after every login.
    pub fn watch_all(&self, client: &Client) {
        for name in &self.names {
            let _ = client.watch_user(name);
            let _ = client.request_user_info(name);
        }
    }

    pub fn poll(&mut self, client: &Client) -> Option<Vec<UserCard>> {
        if self.checked.is_some_and(|at| at.elapsed() < BUDDY_REFRESH) {
            return None;
        }
        self.checked = Some(Instant::now());
        let cards: Vec<UserCard> = self
            .names
            .iter()
            .map(|name| card(name, client.user_info(name), None, false))
            .collect();
        if cards == self.sent {
            return None;
        }
        self.sent = cards.clone();
        Some(cards)
    }
}

/// One user's status, stats and self-description, gathered from two separate replies.
#[derive(Default)]
pub struct Lookup {
    user: Option<(String, Instant)>,
    sent: Option<UserCard>,
}

impl Lookup {
    pub fn start(&mut self, client: &Client, username: String) {
        let _ = client.request_user_info(&username);
        let _ = client.request_peer_info(&username);
        self.sent = None;
        self.user = Some((username, Instant::now() + LOOKUP_TIMEOUT));
    }

    pub fn poll(&mut self, client: &Client) -> Option<UserCard> {
        let (username, deadline) = self.user.as_ref()?;
        let info = client.user_info(username);
        let peer = client.peer_info(username);
        let complete = info.as_ref().is_some_and(UserInfo::is_complete) && peer.is_some();
        let offline = info
            .as_ref()
            .and_then(|info| info.presence)
            .is_some_and(|presence| presence.status == UserStatus::Offline);
        let done = complete || offline || Instant::now() >= *deadline;
        let result = card(username, info, peer, !done);
        if done {
            self.user = None;
        }
        if self.sent.as_ref() == Some(&result) {
            return None;
        }
        self.sent = Some(result.clone());
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use soulseek_rs::UserPresence;

    use super::*;

    #[test]
    fn maps_presence_and_privileges() {
        let mut info = UserInfo::pending("ann".into());
        info.presence = Some(UserPresence {
            status: UserStatus::Away,
            privileged: true,
        });
        let result = card("ann", Some(info), None, false);
        assert_eq!(result.presence, Presence::Away);
        assert!(result.privileged);
        assert!(result.stats.is_none());
        assert_eq!(card("bob", None, None, true).presence, Presence::Unknown);
    }
}
