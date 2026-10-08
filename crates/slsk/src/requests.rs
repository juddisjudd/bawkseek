use std::collections::HashMap;
use std::time::Duration;

use tokio::time::Instant;

use crate::client::{Event, Request};
use crate::engine::Engine;
use crate::proto::peer::{PeerMessage, UserInfo};

const INFO_TIMEOUT: Duration = Duration::from_secs(60);
/// Big shares take a while to compress and send.
const BROWSE_TIMEOUT: Duration = Duration::from_secs(180);
const FOLDER_TIMEOUT: Duration = Duration::from_secs(60);

/// Requests to peers that are still waiting for an answer.
#[derive(Default)]
pub(crate) struct Waiting {
    info: HashMap<String, Instant>,
    browse: HashMap<String, Instant>,
    folders: HashMap<u32, (String, String, Instant)>,
}

impl Waiting {
    pub(crate) fn involves(&self, username: &str) -> bool {
        self.info.contains_key(username)
            || self.browse.contains_key(username)
            || self.folders.values().any(|(user, _, _)| user == username)
    }
}

impl Engine {
    pub(crate) fn request_user_info(&mut self, username: String) {
        self.waiting.info.insert(username.clone(), Instant::now());
        self.send_peer(&username, PeerMessage::UserInfoRequest);
    }

    pub(crate) fn request_browse(&mut self, username: String) {
        self.waiting.browse.insert(username.clone(), Instant::now());
        self.send_peer(&username, PeerMessage::SharedFileListRequest);
    }

    pub(crate) fn request_folder(&mut self, username: String, token: u32, folder: String) {
        self.waiting
            .folders
            .insert(token, (username.clone(), folder.clone(), Instant::now()));
        self.send_peer(
            &username,
            PeerMessage::FolderContentsRequest { token, folder },
        );
    }

    pub(crate) fn on_peer_message(&mut self, username: &str, message: PeerMessage) {
        if self.is_ignored(username) {
            return;
        }
        match message {
            PeerMessage::UserInfoRequest => {
                let info = self.own_user_info();
                self.send_peer(username, PeerMessage::UserInfoResponse(info));
            }
            PeerMessage::SharedFileListRequest => {
                let list = self.shared_file_list(username);
                self.send_peer(username, PeerMessage::SharedFileListResponse(list));
            }
            PeerMessage::FolderContentsRequest { token, folder } => {
                let dirs = self.folder_contents(username, &folder);
                self.send_peer(
                    username,
                    PeerMessage::FolderContentsResponse {
                        token,
                        folder,
                        dirs,
                    },
                );
            }
            PeerMessage::UserInfoResponse(info) => {
                self.waiting.info.remove(username);
                self.emit(Event::UserInfo {
                    username: username.to_string(),
                    info,
                });
            }
            PeerMessage::SharedFileListResponse(list) => {
                self.waiting.browse.remove(username);
                self.emit(Event::Shares {
                    username: username.to_string(),
                    list,
                });
            }
            PeerMessage::FolderContentsResponse {
                token,
                folder,
                dirs,
            } => {
                self.waiting.folders.remove(&token);
                self.emit(Event::FolderContents {
                    username: username.to_string(),
                    token,
                    folder,
                    dirs,
                });
            }
            PeerMessage::FileSearchResponse(reply) => self.on_search_reply(username, reply),
            other => self.on_transfer_message(username, other),
        }
    }

    pub(crate) fn peer_unreachable(&mut self, username: &str) {
        self.fail_requests(username, "could not connect");
        self.transfers_unreachable(username);
    }

    fn fail_requests(&mut self, username: &str, reason: &str) {
        let mut failed = Vec::new();
        if self.waiting.info.remove(username).is_some() {
            failed.push(Request::UserInfo);
        }
        if self.waiting.browse.remove(username).is_some() {
            failed.push(Request::Browse);
        }
        let tokens: Vec<u32> = self
            .waiting
            .folders
            .iter()
            .filter(|(_, (user, _, _))| user == username)
            .map(|(token, _)| *token)
            .collect();
        for token in tokens {
            if let Some((_, folder, _)) = self.waiting.folders.remove(&token) {
                failed.push(Request::FolderContents { token, folder });
            }
        }
        for request in failed {
            self.emit(Event::RequestFailed {
                username: username.to_string(),
                request,
                reason: reason.to_string(),
            });
        }
    }

    pub(crate) fn tick_requests(&mut self) {
        let mut late: Vec<(String, Request)> = Vec::new();
        self.waiting.info.retain(|user, at| {
            let keep = at.elapsed() < INFO_TIMEOUT;
            if !keep {
                late.push((user.clone(), Request::UserInfo));
            }
            keep
        });
        self.waiting.browse.retain(|user, at| {
            let keep = at.elapsed() < BROWSE_TIMEOUT;
            if !keep {
                late.push((user.clone(), Request::Browse));
            }
            keep
        });
        self.waiting.folders.retain(|token, (user, folder, at)| {
            let keep = at.elapsed() < FOLDER_TIMEOUT;
            if !keep {
                late.push((
                    user.clone(),
                    Request::FolderContents {
                        token: *token,
                        folder: folder.clone(),
                    },
                ));
            }
            keep
        });
        for (username, request) in late {
            self.emit(Event::RequestFailed {
                username,
                request,
                reason: "no answer".into(),
            });
        }
    }

    pub(crate) fn own_user_info(&self) -> UserInfo {
        UserInfo {
            description: self.profile.description.clone(),
            picture: self.profile.picture.clone(),
            total_uploads: self.profile.total_uploads,
            queue_size: self.queued_uploads() as u32,
            slots_free: self.free_upload_slots() > 0,
            upload_permitted: Some(1),
        }
    }
}
