use super::types::{ConnectionType, Directory, FileEntry};
use crate::wire::{Reader, WireError, WireResult, Writer, deflate, inflate};

/// Browse replies of very large shares inflate to tens of megabytes; anything past this is refused.
pub const MAX_INFLATED: usize = 512 * 1024 * 1024;

pub mod code {
    pub const SHARED_FILE_LIST_REQUEST: u32 = 4;
    pub const SHARED_FILE_LIST_RESPONSE: u32 = 5;
    pub const FILE_SEARCH_RESPONSE: u32 = 9;
    pub const USER_INFO_REQUEST: u32 = 15;
    pub const USER_INFO_RESPONSE: u32 = 16;
    pub const FOLDER_CONTENTS_REQUEST: u32 = 36;
    pub const FOLDER_CONTENTS_RESPONSE: u32 = 37;
    pub const TRANSFER_REQUEST: u32 = 40;
    pub const TRANSFER_RESPONSE: u32 = 41;
    pub const QUEUE_UPLOAD: u32 = 43;
    pub const PLACE_IN_QUEUE_RESPONSE: u32 = 44;
    pub const UPLOAD_FAILED: u32 = 46;
    pub const UPLOAD_DENIED: u32 = 50;
    pub const PLACE_IN_QUEUE_REQUEST: u32 = 51;
}

/// The first message on a new peer connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerInit {
    PierceFirewall(u32),
    PeerInit {
        username: String,
        kind: ConnectionType,
        token: u32,
    },
}

impl PeerInit {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            PeerInit::PierceFirewall(token) => {
                w.u32(*token);
                Writer::frame_u8(0, &w.into_inner())
            }
            PeerInit::PeerInit {
                username,
                kind,
                token,
            } => {
                w.str(username).str(kind.code()).u32(*token);
                Writer::frame_u8(1, &w.into_inner())
            }
        }
    }

    pub fn decode(code: u8, body: &[u8]) -> WireResult<Self> {
        let mut r = Reader::new(body);
        match code {
            0 => Ok(PeerInit::PierceFirewall(r.u32()?)),
            1 => Ok(PeerInit::PeerInit {
                username: r.string()?,
                kind: ConnectionType::read(&mut r)?,
                token: r.u32().unwrap_or(0),
            }),
            other => Err(WireError::Invalid(other as u32)),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SharedFileList {
    pub dirs: Vec<Directory>,
    pub private_dirs: Vec<Directory>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchReply {
    pub username: String,
    pub token: u32,
    pub files: Vec<FileEntry>,
    pub slot_free: bool,
    pub avg_speed: u32,
    pub queue_len: u32,
    pub private_files: Vec<FileEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserInfo {
    pub description: String,
    pub picture: Option<Vec<u8>>,
    pub upload_slots: u32,
    pub queue_size: u32,
    pub slots_free: bool,
    pub upload_permitted: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Download,
    Upload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerMessage {
    SharedFileListRequest,
    SharedFileListResponse(SharedFileList),
    FileSearchResponse(SearchReply),
    UserInfoRequest,
    UserInfoResponse(UserInfo),
    FolderContentsRequest {
        token: u32,
        folder: String,
    },
    FolderContentsResponse {
        token: u32,
        folder: String,
        dirs: Vec<Directory>,
    },
    TransferRequest {
        direction: Direction,
        token: u32,
        filename: String,
        size: Option<u64>,
    },
    TransferResponse {
        token: u32,
        allowed: bool,
        size: Option<u64>,
        reason: Option<String>,
    },
    QueueUpload(String),
    PlaceInQueueResponse {
        filename: String,
        place: u32,
    },
    UploadFailed(String),
    UploadDenied {
        filename: String,
        reason: String,
    },
    PlaceInQueueRequest(String),
    Unknown {
        code: u32,
        payload: Vec<u8>,
    },
}

impl PeerMessage {
    pub fn code(&self) -> u32 {
        use PeerMessage::*;
        match self {
            SharedFileListRequest => code::SHARED_FILE_LIST_REQUEST,
            SharedFileListResponse(_) => code::SHARED_FILE_LIST_RESPONSE,
            FileSearchResponse(_) => code::FILE_SEARCH_RESPONSE,
            UserInfoRequest => code::USER_INFO_REQUEST,
            UserInfoResponse(_) => code::USER_INFO_RESPONSE,
            FolderContentsRequest { .. } => code::FOLDER_CONTENTS_REQUEST,
            FolderContentsResponse { .. } => code::FOLDER_CONTENTS_RESPONSE,
            TransferRequest { .. } => code::TRANSFER_REQUEST,
            TransferResponse { .. } => code::TRANSFER_RESPONSE,
            QueueUpload(_) => code::QUEUE_UPLOAD,
            PlaceInQueueResponse { .. } => code::PLACE_IN_QUEUE_RESPONSE,
            UploadFailed(_) => code::UPLOAD_FAILED,
            UploadDenied { .. } => code::UPLOAD_DENIED,
            PlaceInQueueRequest(_) => code::PLACE_IN_QUEUE_REQUEST,
            Unknown { code, .. } => *code,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        use PeerMessage::*;
        let mut w = Writer::new();
        match self {
            SharedFileListRequest | UserInfoRequest => {}
            SharedFileListResponse(list) => {
                let mut inner = Writer::new();
                Directory::write_list(&list.dirs, &mut inner);
                inner.u32(0);
                Directory::write_list(&list.private_dirs, &mut inner);
                w.raw(&deflate(&inner.into_inner()));
            }
            FileSearchResponse(reply) => {
                let mut inner = Writer::new();
                inner.str(&reply.username).u32(reply.token);
                FileEntry::write_list(&reply.files, &mut inner);
                inner
                    .bool(reply.slot_free)
                    .u32(reply.avg_speed)
                    .u32(reply.queue_len)
                    .u32(0);
                FileEntry::write_list(&reply.private_files, &mut inner);
                w.raw(&deflate(&inner.into_inner()));
            }
            UserInfoResponse(info) => {
                w.str(&info.description);
                match &info.picture {
                    Some(picture) if !picture.is_empty() => {
                        w.bool(true).bytes(picture);
                    }
                    _ => {
                        w.bool(false);
                    }
                }
                w.u32(info.upload_slots)
                    .u32(info.queue_size)
                    .bool(info.slots_free);
                if let Some(permitted) = info.upload_permitted {
                    w.u32(permitted);
                }
            }
            FolderContentsRequest { token, folder } => {
                w.u32(*token).str(folder);
            }
            FolderContentsResponse {
                token,
                folder,
                dirs,
            } => {
                let mut inner = Writer::new();
                inner.u32(*token).str(folder);
                Directory::write_list(dirs, &mut inner);
                w.raw(&deflate(&inner.into_inner()));
            }
            TransferRequest {
                direction,
                token,
                filename,
                size,
            } => {
                w.u32(match direction {
                    Direction::Download => 0,
                    Direction::Upload => 1,
                })
                .u32(*token)
                .str(filename);
                if *direction == Direction::Upload {
                    w.u64(size.unwrap_or(0));
                }
            }
            TransferResponse {
                token,
                allowed,
                size,
                reason,
            } => {
                w.u32(*token).bool(*allowed);
                if *allowed {
                    if let Some(size) = size {
                        w.u64(*size);
                    }
                } else {
                    w.str(reason.as_deref().unwrap_or("Cancelled"));
                }
            }
            QueueUpload(filename) | UploadFailed(filename) | PlaceInQueueRequest(filename) => {
                w.str(filename);
            }
            PlaceInQueueResponse { filename, place } => {
                w.str(filename).u32(*place);
            }
            UploadDenied { filename, reason } => {
                w.str(filename).str(reason);
            }
            Unknown { payload, .. } => {
                w.raw(payload);
            }
        }
        Writer::frame_u32(self.code(), &w.into_inner())
    }

    pub fn decode(code: u32, body: &[u8]) -> WireResult<Self> {
        use PeerMessage::*;
        let message = match code {
            code::SHARED_FILE_LIST_REQUEST => SharedFileListRequest,
            code::SHARED_FILE_LIST_RESPONSE => {
                let data = inflate(body, MAX_INFLATED)?;
                let mut r = Reader::new(&data);
                let dirs = Directory::read_list(&mut r)?;
                let private_dirs = if r.remaining() >= 8 {
                    let _unknown = r.u32()?;
                    Directory::read_list(&mut r).unwrap_or_default()
                } else {
                    Vec::new()
                };
                SharedFileListResponse(SharedFileList { dirs, private_dirs })
            }
            code::FILE_SEARCH_RESPONSE => {
                let data = inflate(body, MAX_INFLATED)?;
                let mut r = Reader::new(&data);
                let username = r.string()?;
                let token = r.u32()?;
                let files = FileEntry::read_list(&mut r)?;
                let slot_free = r.bool()?;
                let avg_speed = r.u32()?;
                let queue_len = r.u32()?;
                let private_files = if r.remaining() >= 8 {
                    let _unknown = r.u32()?;
                    FileEntry::read_list(&mut r).unwrap_or_default()
                } else {
                    Vec::new()
                };
                FileSearchResponse(SearchReply {
                    username,
                    token,
                    files,
                    slot_free,
                    avg_speed,
                    queue_len,
                    private_files,
                })
            }
            code::USER_INFO_REQUEST => UserInfoRequest,
            code::USER_INFO_RESPONSE => {
                let mut r = Reader::new(body);
                let description = r.string()?;
                let picture = if r.bool()? {
                    Some(r.bytes()?.to_vec())
                } else {
                    None
                };
                let upload_slots = r.u32()?;
                let queue_size = r.u32()?;
                let slots_free = r.bool()?;
                let upload_permitted = r.u32().ok();
                UserInfoResponse(UserInfo {
                    description,
                    picture,
                    upload_slots,
                    queue_size,
                    slots_free,
                    upload_permitted,
                })
            }
            code::FOLDER_CONTENTS_REQUEST => {
                let mut r = Reader::new(body);
                FolderContentsRequest {
                    token: r.u32()?,
                    folder: r.string()?,
                }
            }
            code::FOLDER_CONTENTS_RESPONSE => {
                let data = inflate(body, MAX_INFLATED)?;
                let mut r = Reader::new(&data);
                FolderContentsResponse {
                    token: r.u32()?,
                    folder: r.string()?,
                    dirs: Directory::read_list(&mut r)?,
                }
            }
            code::TRANSFER_REQUEST => {
                let mut r = Reader::new(body);
                let direction = match r.u32()? {
                    0 => Direction::Download,
                    1 => Direction::Upload,
                    other => return Err(WireError::Invalid(other)),
                };
                let token = r.u32()?;
                let filename = r.string()?;
                let size = if direction == Direction::Upload {
                    Some(r.u64()?)
                } else {
                    None
                };
                TransferRequest {
                    direction,
                    token,
                    filename,
                    size,
                }
            }
            code::TRANSFER_RESPONSE => {
                let mut r = Reader::new(body);
                let token = r.u32()?;
                let allowed = r.bool()?;
                let (size, reason) = if allowed {
                    (r.u64().ok(), None)
                } else {
                    (None, r.string().ok())
                };
                TransferResponse {
                    token,
                    allowed,
                    size,
                    reason,
                }
            }
            code::QUEUE_UPLOAD => QueueUpload(Reader::new(body).string()?),
            code::PLACE_IN_QUEUE_RESPONSE => {
                let mut r = Reader::new(body);
                PlaceInQueueResponse {
                    filename: r.string()?,
                    place: r.u32()?,
                }
            }
            code::UPLOAD_FAILED => UploadFailed(Reader::new(body).string()?),
            code::UPLOAD_DENIED => {
                let mut r = Reader::new(body);
                UploadDenied {
                    filename: r.string()?,
                    reason: r.string()?,
                }
            }
            code::PLACE_IN_QUEUE_REQUEST => PlaceInQueueRequest(Reader::new(body).string()?),
            _ => Unknown {
                code,
                payload: body.to_vec(),
            },
        };
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::types::{ATTR_BITRATE, ATTR_DURATION};

    fn roundtrip(message: PeerMessage) {
        let frame = message.encode();
        let len = u32::from_le_bytes(frame[0..4].try_into().unwrap()) as usize;
        assert_eq!(len, frame.len() - 4);
        let code = u32::from_le_bytes(frame[4..8].try_into().unwrap());
        assert_eq!(PeerMessage::decode(code, &frame[8..]), Ok(message));
    }

    fn file(name: &str) -> FileEntry {
        FileEntry {
            name: name.into(),
            size: 1234,
            ext: "mp3".into(),
            attrs: vec![(ATTR_BITRATE, 320), (ATTR_DURATION, 200)],
        }
    }

    #[test]
    fn roundtrips_every_message() {
        roundtrip(PeerMessage::SharedFileListRequest);
        roundtrip(PeerMessage::SharedFileListResponse(SharedFileList {
            dirs: vec![Directory {
                name: "music\\a".into(),
                files: vec![file("1.mp3"), file("2.mp3")],
            }],
            private_dirs: vec![Directory {
                name: "music\\b".into(),
                files: vec![file("3.mp3")],
            }],
        }));
        roundtrip(PeerMessage::FileSearchResponse(SearchReply {
            username: "ann".into(),
            token: 5,
            files: vec![file("music\\a\\1.mp3")],
            slot_free: true,
            avg_speed: 1000,
            queue_len: 2,
            private_files: vec![],
        }));
        roundtrip(PeerMessage::UserInfoRequest);
        roundtrip(PeerMessage::UserInfoResponse(UserInfo {
            description: "hi".into(),
            picture: Some(vec![1, 2, 3]),
            upload_slots: 4,
            queue_size: 5,
            slots_free: true,
            upload_permitted: Some(1),
        }));
        roundtrip(PeerMessage::FolderContentsRequest {
            token: 3,
            folder: "music\\a".into(),
        });
        roundtrip(PeerMessage::FolderContentsResponse {
            token: 3,
            folder: "music\\a".into(),
            dirs: vec![Directory {
                name: "music\\a".into(),
                files: vec![file("1.mp3")],
            }],
        });
        roundtrip(PeerMessage::TransferRequest {
            direction: Direction::Upload,
            token: 8,
            filename: "music\\a\\1.mp3".into(),
            size: Some(99),
        });
        roundtrip(PeerMessage::TransferRequest {
            direction: Direction::Download,
            token: 8,
            filename: "music\\a\\1.mp3".into(),
            size: None,
        });
        roundtrip(PeerMessage::TransferResponse {
            token: 8,
            allowed: true,
            size: None,
            reason: None,
        });
        roundtrip(PeerMessage::TransferResponse {
            token: 8,
            allowed: false,
            size: None,
            reason: Some("Queued".into()),
        });
        roundtrip(PeerMessage::QueueUpload("x".into()));
        roundtrip(PeerMessage::PlaceInQueueResponse {
            filename: "x".into(),
            place: 4,
        });
        roundtrip(PeerMessage::UploadFailed("x".into()));
        roundtrip(PeerMessage::UploadDenied {
            filename: "x".into(),
            reason: "File not shared.".into(),
        });
        roundtrip(PeerMessage::PlaceInQueueRequest("x".into()));
    }

    #[test]
    fn reads_search_replies_without_private_files() {
        let mut inner = Writer::new();
        inner.str("ann").u32(5);
        FileEntry::write_list(&[file("a.mp3")], &mut inner);
        inner.bool(false).u32(10).u32(0);
        let body = deflate(&inner.into_inner());
        let PeerMessage::FileSearchResponse(reply) = PeerMessage::decode(9, &body).unwrap() else {
            panic!("not a search reply");
        };
        assert_eq!(reply.files.len(), 1);
        assert_eq!(reply.files[0].bitrate(), Some(320));
        assert!(reply.private_files.is_empty());
    }

    #[test]
    fn roundtrips_init_messages() {
        for init in [
            PeerInit::PierceFirewall(77),
            PeerInit::PeerInit {
                username: "ann".into(),
                kind: ConnectionType::Distributed,
                token: 0,
            },
        ] {
            let frame = init.encode();
            assert_eq!(PeerInit::decode(frame[4], &frame[5..]), Ok(init));
        }
    }
}
