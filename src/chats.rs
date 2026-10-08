use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config;

const MAX_LINES: usize = 1000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub at: i64,
    pub mine: bool,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub username: String,
    pub lines: Vec<Line>,
    pub unread: usize,
}

/// Private conversations for one account, most recent first, saved to disk because the server forgets them.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Chats {
    pub conversations: Vec<Conversation>,
    #[serde(skip)]
    owner: String,
}

impl Chats {
    pub fn load(owner: &str) -> Self {
        let mut chats: Chats = path(owner)
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        chats.owner = owner.to_string();
        chats
    }

    pub fn save(&self) {
        if self.owner.is_empty() {
            return;
        }
        let Some(path) = path(&self.owner) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec(self) {
            let _ = fs::write(path, json);
        }
    }

    pub fn get(&self, username: &str) -> Option<&Conversation> {
        self.conversations
            .iter()
            .find(|conversation| conversation.username == username)
    }

    /// Returns the conversation, creating it at the top when it is new.
    pub fn open(&mut self, username: &str) -> &mut Conversation {
        let ix = match self
            .conversations
            .iter()
            .position(|conversation| conversation.username == username)
        {
            Some(ix) => ix,
            None => {
                self.conversations.insert(
                    0,
                    Conversation {
                        username: username.to_string(),
                        ..Default::default()
                    },
                );
                0
            }
        };
        &mut self.conversations[ix]
    }

    pub fn receive(&mut self, username: &str, text: String, at: i64, read: bool) {
        self.push(
            username,
            Line {
                at,
                mine: false,
                text,
            },
        );
        if !read {
            self.conversations[0].unread += 1;
        }
    }

    pub fn sent(&mut self, username: &str, text: String, at: i64) {
        self.push(
            username,
            Line {
                at,
                mine: true,
                text,
            },
        );
    }

    fn push(&mut self, username: &str, line: Line) {
        let conversation = self.open(username);
        conversation.lines.push(line);
        let excess = conversation.lines.len().saturating_sub(MAX_LINES);
        conversation.lines.drain(..excess);
        let ix = self
            .conversations
            .iter()
            .position(|conversation| conversation.username == username)
            .unwrap_or(0);
        let conversation = self.conversations.remove(ix);
        self.conversations.insert(0, conversation);
    }

    pub fn mark_read(&mut self, username: &str) -> bool {
        match self
            .conversations
            .iter_mut()
            .find(|conversation| conversation.username == username)
        {
            Some(conversation) if conversation.unread > 0 => {
                conversation.unread = 0;
                true
            }
            _ => false,
        }
    }

    pub fn remove(&mut self, username: &str) {
        self.conversations
            .retain(|conversation| conversation.username != username);
    }

    pub fn unread(&self) -> usize {
        self.conversations
            .iter()
            .map(|conversation| conversation.unread)
            .sum()
    }
}

fn path(owner: &str) -> Option<PathBuf> {
    config::account_file("messages", owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_conversation_moves_to_the_top() {
        let mut chats = Chats::default();
        chats.receive("ann", "hi".into(), 1, false);
        chats.receive("bob", "yo".into(), 2, false);
        chats.sent("ann", "hello".into(), 3);

        let order: Vec<&str> = chats
            .conversations
            .iter()
            .map(|conversation| conversation.username.as_str())
            .collect();
        assert_eq!(order, vec!["ann", "bob"]);
        assert_eq!(chats.get("ann").unwrap().lines.len(), 2);
        assert!(chats.get("ann").unwrap().lines[1].mine);
    }

    #[test]
    fn counts_unread_until_marked() {
        let mut chats = Chats::default();
        chats.receive("ann", "1".into(), 1, false);
        chats.receive("ann", "2".into(), 2, false);
        chats.receive("bob", "3".into(), 3, true);
        assert_eq!(chats.unread(), 2);
        assert!(chats.mark_read("ann"));
        assert!(!chats.mark_read("ann"));
        assert_eq!(chats.unread(), 0);
    }

    #[test]
    fn keeps_only_the_latest_lines() {
        let mut chats = Chats::default();
        for at in 0..(MAX_LINES as i64 + 5) {
            chats.receive("ann", at.to_string(), at, true);
        }
        let lines = &chats.get("ann").unwrap().lines;
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(lines[0].at, 5);
    }
}
