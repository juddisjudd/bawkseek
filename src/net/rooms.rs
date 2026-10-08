use std::collections::HashSet;

use soulseek_rs::{RoomEvent, RoomInfo, RoomTicker};

const MAX_LINES: usize = 500;

#[derive(Clone, Debug, PartialEq)]
pub struct RoomLine {
    pub at: i64,
    pub username: String,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Room {
    pub name: String,
    pub users: Vec<String>,
    pub lines: Vec<RoomLine>,
    pub tickers: Vec<RoomTicker>,
    pub unread: usize,
    pub private: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeedLine {
    pub at: i64,
    pub room: String,
    pub username: String,
    pub text: String,
}

/// Joined rooms in join order, the server's public room list, and the public feed.
#[derive(Debug, Default)]
pub struct Rooms {
    pub joined: Vec<Room>,
    pub list: Vec<RoomInfo>,
    pub viewing: Option<String>,
    pub feed: Vec<FeedLine>,
    pub feed_on: bool,
    private: HashSet<String>,
}

impl Rooms {
    pub fn get(&self, name: &str) -> Option<&Room> {
        self.joined.iter().find(|room| room.name == name)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut Room> {
        self.joined.iter_mut().find(|room| room.name == name)
    }

    /// Applies one server event; returns a message for the user when something went wrong.
    pub fn apply(&mut self, at: i64, event: RoomEvent) -> Option<String> {
        match event {
            RoomEvent::List(mut rooms) => {
                rooms.sort_by(|a, b| {
                    b.user_count
                        .cmp(&a.user_count)
                        .then_with(|| a.name.cmp(&b.name))
                });
                self.list = rooms;
            }
            RoomEvent::Joined { room, mut users } => {
                users.sort_by_cached_key(|user| user.to_lowercase());
                users.dedup();
                let private = self.private.contains(&room);
                match self.get_mut(&room) {
                    Some(existing) => {
                        existing.users = users;
                        existing.private |= private;
                    }
                    None => self.joined.push(Room {
                        name: room,
                        users,
                        private,
                        ..Default::default()
                    }),
                }
            }
            RoomEvent::Left { room } => self.joined.retain(|joined| joined.name != room),
            RoomEvent::Message {
                room,
                username,
                message,
            } => {
                let viewing = self.viewing.as_deref() == Some(room.as_str());
                if let Some(joined) = self.get_mut(&room) {
                    joined.lines.push(RoomLine {
                        at,
                        username,
                        text: message,
                    });
                    let excess = joined.lines.len().saturating_sub(MAX_LINES);
                    joined.lines.drain(..excess);
                    if !viewing {
                        joined.unread += 1;
                    }
                }
            }
            RoomEvent::UserJoined { room, username } => {
                if let Some(joined) = self.get_mut(&room) {
                    let key = username.to_lowercase();
                    if let Err(ix) = joined
                        .users
                        .binary_search_by(|user| user.to_lowercase().cmp(&key))
                    {
                        joined.users.insert(ix, username);
                    }
                }
            }
            RoomEvent::UserLeft { room, username } => {
                if let Some(joined) = self.get_mut(&room) {
                    joined.users.retain(|user| *user != username);
                }
            }
            RoomEvent::Tickers { room, tickers } => {
                if let Some(joined) = self.get_mut(&room) {
                    joined.tickers = tickers;
                }
            }
            RoomEvent::TickerAdded {
                room,
                username,
                ticker,
            } => {
                if let Some(joined) = self.get_mut(&room) {
                    joined
                        .tickers
                        .retain(|existing| existing.username != username);
                    joined.tickers.push(RoomTicker { username, ticker });
                }
            }
            RoomEvent::TickerRemoved { room, username } => {
                if let Some(joined) = self.get_mut(&room) {
                    joined
                        .tickers
                        .retain(|existing| existing.username != username);
                }
            }
            RoomEvent::PrivateMembers { room, .. } => {
                if let Some(joined) = self.get_mut(&room) {
                    joined.private = true;
                }
                self.private.insert(room);
            }
            RoomEvent::GlobalMessage {
                room,
                username,
                message,
            } => {
                self.feed.push(FeedLine {
                    at,
                    room,
                    username,
                    text: message,
                });
                let excess = self.feed.len().saturating_sub(MAX_LINES);
                self.feed.drain(..excess);
            }
            RoomEvent::CantCreate { room } => {
                return Some(format!(
                    "{room} is taken, or it is a public room. pick another name."
                ));
            }
            _ => {}
        }
        None
    }

    pub fn mark_read(&mut self, name: &str) -> bool {
        match self.get_mut(name) {
            Some(room) if room.unread > 0 => {
                room.unread = 0;
                true
            }
            _ => false,
        }
    }

    pub fn unread(&self) -> usize {
        self.joined.iter().map(|room| room.unread).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(room: &str, user: &str, text: &str) -> RoomEvent {
        RoomEvent::Message {
            room: room.into(),
            username: user.into(),
            message: text.into(),
        }
    }

    #[test]
    fn tracks_members_messages_and_unread() {
        let mut rooms = Rooms::default();
        rooms.apply(
            1,
            RoomEvent::Joined {
                room: "lobby".into(),
                users: vec!["zed".into(), "Ann".into(), "bob".into()],
            },
        );
        rooms.apply(
            2,
            RoomEvent::UserJoined {
                room: "lobby".into(),
                username: "carl".into(),
            },
        );
        rooms.apply(
            3,
            RoomEvent::UserLeft {
                room: "lobby".into(),
                username: "zed".into(),
            },
        );
        rooms.apply(4, message("lobby", "ann", "hi"));
        rooms.apply(5, message("nowhere", "ann", "ignored"));

        let lobby = rooms.get("lobby").unwrap();
        assert_eq!(lobby.users, vec!["Ann", "bob", "carl"]);
        assert_eq!(lobby.lines.len(), 1);
        assert_eq!(lobby.lines[0].at, 4);
        assert_eq!(rooms.unread(), 1);

        rooms.viewing = Some("lobby".into());
        rooms.apply(6, message("lobby", "bob", "yo"));
        assert_eq!(rooms.unread(), 1);
        assert!(rooms.mark_read("lobby"));
        assert_eq!(rooms.unread(), 0);
    }

    #[test]
    fn replaces_a_users_ticker_and_leaves() {
        let mut rooms = Rooms::default();
        rooms.apply(
            1,
            RoomEvent::Joined {
                room: "r".into(),
                users: vec![],
            },
        );
        for ticker in ["one", "two"] {
            rooms.apply(
                2,
                RoomEvent::TickerAdded {
                    room: "r".into(),
                    username: "ann".into(),
                    ticker: ticker.into(),
                },
            );
        }
        assert_eq!(rooms.get("r").unwrap().tickers.len(), 1);
        assert_eq!(rooms.get("r").unwrap().tickers[0].ticker, "two");
        rooms.apply(3, RoomEvent::Left { room: "r".into() });
        assert!(rooms.get("r").is_none());
    }

    #[test]
    fn remembers_private_rooms_announced_before_the_join() {
        let mut rooms = Rooms::default();
        rooms.apply(
            1,
            RoomEvent::PrivateMembers {
                room: "club".into(),
                users: vec!["ann".into()],
            },
        );
        rooms.apply(
            2,
            RoomEvent::Joined {
                room: "club".into(),
                users: vec!["ann".into()],
            },
        );
        assert!(rooms.get("club").unwrap().private);
    }

    #[test]
    fn sorts_the_room_list_by_size() {
        let mut rooms = Rooms::default();
        rooms.apply(
            1,
            RoomEvent::List(vec![
                RoomInfo {
                    name: "small".into(),
                    user_count: 2,
                },
                RoomInfo {
                    name: "big".into(),
                    user_count: 90,
                },
            ]),
        );
        assert_eq!(rooms.list[0].name, "big");
        assert!(
            rooms
                .apply(2, RoomEvent::CantCreate { room: "x".into() })
                .is_some()
        );
    }
}
