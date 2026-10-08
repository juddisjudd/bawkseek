use std::net::Ipv4Addr;

use md5::{Digest, Md5};

use super::types::{ConnectionType, Recommendation, UserStats, UserStatus};
use crate::wire::{Reader, WireResult, Writer};

pub mod code {
    pub const LOGIN: u32 = 1;
    pub const SET_WAIT_PORT: u32 = 2;
    pub const GET_PEER_ADDRESS: u32 = 3;
    pub const WATCH_USER: u32 = 5;
    pub const UNWATCH_USER: u32 = 6;
    pub const GET_USER_STATUS: u32 = 7;
    pub const SAY_CHATROOM: u32 = 13;
    pub const JOIN_ROOM: u32 = 14;
    pub const LEAVE_ROOM: u32 = 15;
    pub const USER_JOINED_ROOM: u32 = 16;
    pub const USER_LEFT_ROOM: u32 = 17;
    pub const CONNECT_TO_PEER: u32 = 18;
    pub const MESSAGE_USER: u32 = 22;
    pub const MESSAGE_ACKED: u32 = 23;
    pub const FILE_SEARCH: u32 = 26;
    pub const SET_STATUS: u32 = 28;
    pub const SERVER_PING: u32 = 32;
    pub const SHARED_FOLDERS_FILES: u32 = 35;
    pub const GET_USER_STATS: u32 = 36;
    pub const RELOGGED: u32 = 41;
    pub const USER_SEARCH: u32 = 42;
    pub const ADD_THING_I_LIKE: u32 = 51;
    pub const REMOVE_THING_I_LIKE: u32 = 52;
    pub const RECOMMENDATIONS: u32 = 54;
    pub const GLOBAL_RECOMMENDATIONS: u32 = 56;
    pub const USER_INTERESTS: u32 = 57;
    pub const ROOM_LIST: u32 = 64;
    pub const ADMIN_MESSAGE: u32 = 66;
    pub const PRIVILEGED_USERS: u32 = 69;
    pub const HAVE_NO_PARENT: u32 = 71;
    pub const PARENT_MIN_SPEED: u32 = 83;
    pub const PARENT_SPEED_RATIO: u32 = 84;
    pub const CHECK_PRIVILEGES: u32 = 92;
    pub const EMBEDDED_MESSAGE: u32 = 93;
    pub const ACCEPT_CHILDREN: u32 = 100;
    pub const POSSIBLE_PARENTS: u32 = 102;
    pub const WISHLIST_SEARCH: u32 = 103;
    pub const WISHLIST_INTERVAL: u32 = 104;
    pub const SIMILAR_USERS: u32 = 110;
    pub const ITEM_RECOMMENDATIONS: u32 = 111;
    pub const ITEM_SIMILAR_USERS: u32 = 112;
    pub const ROOM_TICKERS: u32 = 113;
    pub const ROOM_TICKER_ADDED: u32 = 114;
    pub const ROOM_TICKER_REMOVED: u32 = 115;
    pub const SET_ROOM_TICKER: u32 = 116;
    pub const ADD_THING_I_HATE: u32 = 117;
    pub const REMOVE_THING_I_HATE: u32 = 118;
    pub const ROOM_SEARCH: u32 = 120;
    pub const SEND_UPLOAD_SPEED: u32 = 121;
    pub const GIVE_PRIVILEGES: u32 = 123;
    pub const BRANCH_LEVEL: u32 = 126;
    pub const BRANCH_ROOT: u32 = 127;
    pub const RESET_DISTRIBUTED: u32 = 130;
    pub const ROOM_MEMBERS: u32 = 133;
    pub const ADD_ROOM_MEMBER: u32 = 134;
    pub const REMOVE_ROOM_MEMBER: u32 = 135;
    pub const CANCEL_ROOM_MEMBERSHIP: u32 = 136;
    pub const CANCEL_ROOM_OWNERSHIP: u32 = 137;
    pub const ROOM_MEMBERSHIP_GRANTED: u32 = 139;
    pub const ROOM_MEMBERSHIP_REVOKED: u32 = 140;
    pub const ENABLE_ROOM_INVITATIONS: u32 = 141;
    pub const CHANGE_PASSWORD: u32 = 142;
    pub const ADD_ROOM_OPERATOR: u32 = 143;
    pub const REMOVE_ROOM_OPERATOR: u32 = 144;
    pub const ROOM_OPERATORSHIP_GRANTED: u32 = 145;
    pub const ROOM_OPERATORSHIP_REVOKED: u32 = 146;
    pub const ROOM_OPERATORS: u32 = 148;
    pub const MESSAGE_USERS: u32 = 149;
    pub const JOIN_GLOBAL_ROOM: u32 = 150;
    pub const LEAVE_GLOBAL_ROOM: u32 = 151;
    pub const GLOBAL_ROOM_MESSAGE: u32 = 152;
    pub const EXCLUDED_SEARCH_PHRASES: u32 = 160;
    pub const CANT_CONNECT_TO_PEER: u32 = 1001;
    pub const CANT_CREATE_ROOM: u32 = 1003;
}

/// A message we send to the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerRequest {
    Login {
        username: String,
        password: String,
        major: u32,
        minor: u32,
    },
    SetWaitPort(u16),
    GetPeerAddress(String),
    WatchUser(String),
    UnwatchUser(String),
    GetUserStatus(String),
    SayChatroom {
        room: String,
        message: String,
    },
    JoinRoom {
        room: String,
        private: bool,
    },
    LeaveRoom(String),
    ConnectToPeer {
        token: u32,
        username: String,
        kind: ConnectionType,
    },
    MessageUser {
        username: String,
        message: String,
    },
    MessageAcked(u32),
    FileSearch {
        token: u32,
        query: String,
    },
    SetStatus(UserStatus),
    ServerPing,
    SharedFoldersFiles {
        dirs: u32,
        files: u32,
    },
    GetUserStats(String),
    UserSearch {
        username: String,
        token: u32,
        query: String,
    },
    AddThingILike(String),
    RemoveThingILike(String),
    AddThingIHate(String),
    RemoveThingIHate(String),
    Recommendations,
    GlobalRecommendations,
    UserInterests(String),
    RoomList,
    HaveNoParent(bool),
    CheckPrivileges,
    AcceptChildren(bool),
    WishlistSearch {
        token: u32,
        query: String,
    },
    SimilarUsers,
    ItemRecommendations(String),
    ItemSimilarUsers(String),
    SetRoomTicker {
        room: String,
        ticker: String,
    },
    RoomSearch {
        room: String,
        token: u32,
        query: String,
    },
    SendUploadSpeed(u32),
    GivePrivileges {
        username: String,
        days: u32,
    },
    BranchLevel(u32),
    BranchRoot(String),
    AddRoomMember {
        room: String,
        username: String,
    },
    RemoveRoomMember {
        room: String,
        username: String,
    },
    CancelRoomMembership(String),
    CancelRoomOwnership(String),
    EnableRoomInvitations(bool),
    ChangePassword(String),
    AddRoomOperator {
        room: String,
        username: String,
    },
    RemoveRoomOperator {
        room: String,
        username: String,
    },
    MessageUsers {
        usernames: Vec<String>,
        message: String,
    },
    JoinGlobalRoom,
    LeaveGlobalRoom,
    CantConnectToPeer {
        token: u32,
        username: String,
    },
}

pub fn md5_hex(text: &str) -> String {
    Md5::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl ServerRequest {
    pub fn code(&self) -> u32 {
        use ServerRequest::*;
        match self {
            Login { .. } => code::LOGIN,
            SetWaitPort(_) => code::SET_WAIT_PORT,
            GetPeerAddress(_) => code::GET_PEER_ADDRESS,
            WatchUser(_) => code::WATCH_USER,
            UnwatchUser(_) => code::UNWATCH_USER,
            GetUserStatus(_) => code::GET_USER_STATUS,
            SayChatroom { .. } => code::SAY_CHATROOM,
            JoinRoom { .. } => code::JOIN_ROOM,
            LeaveRoom(_) => code::LEAVE_ROOM,
            ConnectToPeer { .. } => code::CONNECT_TO_PEER,
            MessageUser { .. } => code::MESSAGE_USER,
            MessageAcked(_) => code::MESSAGE_ACKED,
            FileSearch { .. } => code::FILE_SEARCH,
            SetStatus(_) => code::SET_STATUS,
            ServerPing => code::SERVER_PING,
            SharedFoldersFiles { .. } => code::SHARED_FOLDERS_FILES,
            GetUserStats(_) => code::GET_USER_STATS,
            UserSearch { .. } => code::USER_SEARCH,
            AddThingILike(_) => code::ADD_THING_I_LIKE,
            RemoveThingILike(_) => code::REMOVE_THING_I_LIKE,
            AddThingIHate(_) => code::ADD_THING_I_HATE,
            RemoveThingIHate(_) => code::REMOVE_THING_I_HATE,
            Recommendations => code::RECOMMENDATIONS,
            GlobalRecommendations => code::GLOBAL_RECOMMENDATIONS,
            UserInterests(_) => code::USER_INTERESTS,
            RoomList => code::ROOM_LIST,
            HaveNoParent(_) => code::HAVE_NO_PARENT,
            CheckPrivileges => code::CHECK_PRIVILEGES,
            AcceptChildren(_) => code::ACCEPT_CHILDREN,
            WishlistSearch { .. } => code::WISHLIST_SEARCH,
            SimilarUsers => code::SIMILAR_USERS,
            ItemRecommendations(_) => code::ITEM_RECOMMENDATIONS,
            ItemSimilarUsers(_) => code::ITEM_SIMILAR_USERS,
            SetRoomTicker { .. } => code::SET_ROOM_TICKER,
            RoomSearch { .. } => code::ROOM_SEARCH,
            SendUploadSpeed(_) => code::SEND_UPLOAD_SPEED,
            GivePrivileges { .. } => code::GIVE_PRIVILEGES,
            BranchLevel(_) => code::BRANCH_LEVEL,
            BranchRoot(_) => code::BRANCH_ROOT,
            AddRoomMember { .. } => code::ADD_ROOM_MEMBER,
            RemoveRoomMember { .. } => code::REMOVE_ROOM_MEMBER,
            CancelRoomMembership(_) => code::CANCEL_ROOM_MEMBERSHIP,
            CancelRoomOwnership(_) => code::CANCEL_ROOM_OWNERSHIP,
            EnableRoomInvitations(_) => code::ENABLE_ROOM_INVITATIONS,
            ChangePassword(_) => code::CHANGE_PASSWORD,
            AddRoomOperator { .. } => code::ADD_ROOM_OPERATOR,
            RemoveRoomOperator { .. } => code::REMOVE_ROOM_OPERATOR,
            MessageUsers { .. } => code::MESSAGE_USERS,
            JoinGlobalRoom => code::JOIN_GLOBAL_ROOM,
            LeaveGlobalRoom => code::LEAVE_GLOBAL_ROOM,
            CantConnectToPeer { .. } => code::CANT_CONNECT_TO_PEER,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        use ServerRequest::*;
        let mut w = Writer::new();
        match self {
            Login {
                username,
                password,
                major,
                minor,
            } => {
                w.str(username)
                    .str(password)
                    .u32(*major)
                    .str(&md5_hex(&format!("{username}{password}")))
                    .u32(*minor);
            }
            SetWaitPort(port) => {
                w.u32(*port as u32);
            }
            GetPeerAddress(name)
            | WatchUser(name)
            | UnwatchUser(name)
            | GetUserStatus(name)
            | LeaveRoom(name)
            | GetUserStats(name)
            | AddThingILike(name)
            | RemoveThingILike(name)
            | AddThingIHate(name)
            | RemoveThingIHate(name)
            | UserInterests(name)
            | ItemRecommendations(name)
            | ItemSimilarUsers(name)
            | BranchRoot(name)
            | CancelRoomMembership(name)
            | CancelRoomOwnership(name)
            | ChangePassword(name) => {
                w.str(name);
            }
            SayChatroom { room, message } => {
                w.str(room).str(message);
            }
            JoinRoom { room, private } => {
                w.str(room).u32(*private as u32);
            }
            ConnectToPeer {
                token,
                username,
                kind,
            } => {
                w.u32(*token).str(username).str(kind.code());
            }
            MessageUser { username, message } => {
                w.str(username).str(message);
            }
            MessageAcked(id) => {
                w.u32(*id);
            }
            FileSearch { token, query } | WishlistSearch { token, query } => {
                w.u32(*token).str(query);
            }
            SetStatus(status) => {
                w.i32(status.code() as i32);
            }
            ServerPing
            | Recommendations
            | GlobalRecommendations
            | RoomList
            | CheckPrivileges
            | SimilarUsers
            | JoinGlobalRoom
            | LeaveGlobalRoom => {}
            SharedFoldersFiles { dirs, files } => {
                w.u32(*dirs).u32(*files);
            }
            UserSearch {
                username,
                token,
                query,
            } => {
                w.str(username).u32(*token).str(query);
            }
            HaveNoParent(flag) | AcceptChildren(flag) | EnableRoomInvitations(flag) => {
                w.bool(*flag);
            }
            SetRoomTicker { room, ticker } => {
                w.str(room).str(ticker);
            }
            RoomSearch { room, token, query } => {
                w.str(room).u32(*token).str(query);
            }
            SendUploadSpeed(value) | BranchLevel(value) => {
                w.u32(*value);
            }
            GivePrivileges { username, days } => {
                w.str(username).u32(*days);
            }
            AddRoomMember { room, username }
            | RemoveRoomMember { room, username }
            | AddRoomOperator { room, username }
            | RemoveRoomOperator { room, username } => {
                w.str(room).str(username);
            }
            MessageUsers { usernames, message } => {
                w.u32(usernames.len() as u32);
                for name in usernames {
                    w.str(name);
                }
                w.str(message);
            }
            CantConnectToPeer { token, username } => {
                w.u32(*token).str(username);
            }
        }
        Writer::frame_u32(self.code(), &w.into_inner())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginResponse {
    Success {
        greeting: String,
        own_ip: Ipv4Addr,
        supporter: bool,
    },
    Failure {
        reason: String,
        detail: Option<String>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomUser {
    pub username: String,
    pub status: UserStatus,
    pub stats: UserStats,
    pub slots_full: bool,
    pub country: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomJoined {
    pub room: String,
    pub users: Vec<RoomUser>,
    pub owner: Option<String>,
    pub operators: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomList {
    pub public: Vec<(String, u32)>,
    pub owned: Vec<(String, u32)>,
    pub private: Vec<(String, u32)>,
    pub operated: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PossibleParent {
    pub username: String,
    pub ip: Ipv4Addr,
    pub port: u16,
}

/// A message the server sends us.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerResponse {
    Login(LoginResponse),
    GetPeerAddress {
        username: String,
        ip: Ipv4Addr,
        port: u16,
    },
    WatchUser {
        username: String,
        exists: bool,
        status: UserStatus,
        stats: UserStats,
        country: Option<String>,
    },
    GetUserStatus {
        username: String,
        status: UserStatus,
        privileged: bool,
    },
    SayChatroom {
        room: String,
        username: String,
        message: String,
    },
    JoinRoom(RoomJoined),
    LeaveRoom(String),
    UserJoinedRoom {
        room: String,
        user: RoomUser,
    },
    UserLeftRoom {
        room: String,
        username: String,
    },
    ConnectToPeer {
        username: String,
        kind: ConnectionType,
        ip: Ipv4Addr,
        port: u16,
        token: u32,
        privileged: bool,
    },
    MessageUser {
        id: u32,
        timestamp: u32,
        username: String,
        message: String,
        new: bool,
    },
    FileSearch {
        username: String,
        token: u32,
        query: String,
    },
    GetUserStats {
        username: String,
        stats: UserStats,
    },
    Relogged,
    Recommendations {
        likes: Vec<Recommendation>,
        dislikes: Vec<Recommendation>,
    },
    GlobalRecommendations {
        likes: Vec<Recommendation>,
        dislikes: Vec<Recommendation>,
    },
    UserInterests {
        username: String,
        likes: Vec<String>,
        hates: Vec<String>,
    },
    RoomList(RoomList),
    AdminMessage(String),
    PrivilegedUsers(Vec<String>),
    ParentMinSpeed(u32),
    ParentSpeedRatio(u32),
    CheckPrivileges(u32),
    EmbeddedMessage {
        code: u8,
        payload: Vec<u8>,
    },
    PossibleParents(Vec<PossibleParent>),
    WishlistInterval(u32),
    SimilarUsers(Vec<(String, u32)>),
    ItemRecommendations {
        item: String,
        recommendations: Vec<Recommendation>,
    },
    ItemSimilarUsers {
        item: String,
        usernames: Vec<String>,
    },
    RoomTickers {
        room: String,
        tickers: Vec<(String, String)>,
    },
    RoomTickerAdded {
        room: String,
        username: String,
        ticker: String,
    },
    RoomTickerRemoved {
        room: String,
        username: String,
    },
    ResetDistributed,
    RoomMembers {
        room: String,
        members: Vec<String>,
    },
    AddRoomMember {
        room: String,
        username: String,
    },
    RemoveRoomMember {
        room: String,
        username: String,
    },
    RoomMembershipGranted(String),
    RoomMembershipRevoked(String),
    EnableRoomInvitations(bool),
    ChangePassword(String),
    AddRoomOperator {
        room: String,
        username: String,
    },
    RemoveRoomOperator {
        room: String,
        username: String,
    },
    RoomOperatorshipGranted(String),
    RoomOperatorshipRevoked(String),
    RoomOperators {
        room: String,
        operators: Vec<String>,
    },
    GlobalRoomMessage {
        room: String,
        username: String,
        message: String,
    },
    ExcludedSearchPhrases(Vec<String>),
    CantConnectToPeer(u32),
    CantCreateRoom(String),
    Unknown {
        code: u32,
        payload: Vec<u8>,
    },
}

fn port(r: &mut Reader) -> WireResult<u16> {
    Ok(r.u32()? as u16)
}

fn read_room_users(r: &mut Reader) -> WireResult<Vec<RoomUser>> {
    let names = r.strings()?;
    let mut users: Vec<RoomUser> = names
        .into_iter()
        .map(|username| RoomUser {
            username,
            ..RoomUser::default()
        })
        .collect();
    let statuses = r.count(4)?;
    for ix in 0..statuses {
        let status = UserStatus::from_code(r.u32()?);
        if let Some(user) = users.get_mut(ix) {
            user.status = status;
        }
    }
    let stats = r.count(20)?;
    for ix in 0..stats {
        let stats = UserStats::read(r)?;
        if let Some(user) = users.get_mut(ix) {
            user.stats = stats;
        }
    }
    let slots = r.count(4)?;
    for ix in 0..slots {
        let full = r.u32()? != 0;
        if let Some(user) = users.get_mut(ix) {
            user.slots_full = full;
        }
    }
    let countries = r.count(4)?;
    for ix in 0..countries {
        let country = r.string()?;
        if let Some(user) = users.get_mut(ix) {
            user.country = country;
        }
    }
    Ok(users)
}

fn read_counted_rooms(r: &mut Reader) -> WireResult<Vec<(String, u32)>> {
    let names = r.strings()?;
    let count = r.count(4)?;
    let sizes = (0..count)
        .map(|_| r.u32())
        .collect::<WireResult<Vec<_>>>()?;
    Ok(names
        .into_iter()
        .enumerate()
        .map(|(ix, name)| (name, sizes.get(ix).copied().unwrap_or(0)))
        .collect())
}

impl ServerResponse {
    pub fn decode(code: u32, body: &[u8]) -> WireResult<Self> {
        use ServerResponse::*;
        let mut r = Reader::new(body);
        let r = &mut r;
        let message = match code {
            code::LOGIN => {
                if r.bool()? {
                    let greeting = r.string()?;
                    let own_ip = r.ip()?;
                    let _hash = r.string().unwrap_or_default();
                    let supporter = r.bool().unwrap_or(false);
                    Login(LoginResponse::Success {
                        greeting,
                        own_ip,
                        supporter,
                    })
                } else {
                    let reason = r.string()?;
                    let detail = if r.is_done() { None } else { r.string().ok() };
                    Login(LoginResponse::Failure { reason, detail })
                }
            }
            code::GET_PEER_ADDRESS => GetPeerAddress {
                username: r.string()?,
                ip: r.ip()?,
                port: port(r)?,
            },
            code::WATCH_USER => {
                let username = r.string()?;
                let exists = r.bool()?;
                if !exists {
                    WatchUser {
                        username,
                        exists,
                        status: UserStatus::Offline,
                        stats: UserStats::default(),
                        country: None,
                    }
                } else {
                    let status = UserStatus::from_code(r.u32()?);
                    let stats = UserStats::read(r)?;
                    let country = if status != UserStatus::Offline && !r.is_done() {
                        r.string().ok()
                    } else {
                        None
                    };
                    WatchUser {
                        username,
                        exists,
                        status,
                        stats,
                        country,
                    }
                }
            }
            code::GET_USER_STATUS => GetUserStatus {
                username: r.string()?,
                status: UserStatus::from_code(r.u32()?),
                privileged: r.bool().unwrap_or(false),
            },
            code::SAY_CHATROOM => SayChatroom {
                room: r.string()?,
                username: r.string()?,
                message: r.string()?,
            },
            code::JOIN_ROOM => {
                let room = r.string()?;
                let users = read_room_users(r)?;
                let (owner, operators) = if r.is_done() {
                    (None, Vec::new())
                } else {
                    (Some(r.string()?), r.strings()?)
                };
                JoinRoom(RoomJoined {
                    room,
                    users,
                    owner,
                    operators,
                })
            }
            code::LEAVE_ROOM => LeaveRoom(r.string()?),
            code::USER_JOINED_ROOM => {
                let room = r.string()?;
                let username = r.string()?;
                let status = UserStatus::from_code(r.u32()?);
                let stats = UserStats::read(r)?;
                let slots_full = r.u32()? != 0;
                let country = r.string().unwrap_or_default();
                UserJoinedRoom {
                    room,
                    user: RoomUser {
                        username,
                        status,
                        stats,
                        slots_full,
                        country,
                    },
                }
            }
            code::USER_LEFT_ROOM => UserLeftRoom {
                room: r.string()?,
                username: r.string()?,
            },
            code::CONNECT_TO_PEER => ConnectToPeer {
                username: r.string()?,
                kind: ConnectionType::read(r)?,
                ip: r.ip()?,
                port: port(r)?,
                token: r.u32()?,
                privileged: r.bool().unwrap_or(false),
            },
            code::MESSAGE_USER => MessageUser {
                id: r.u32()?,
                timestamp: r.u32()?,
                username: r.string()?,
                message: r.string()?,
                new: r.bool().unwrap_or(true),
            },
            code::FILE_SEARCH => FileSearch {
                username: r.string()?,
                token: r.u32()?,
                query: r.string()?,
            },
            code::GET_USER_STATS => GetUserStats {
                username: r.string()?,
                stats: UserStats::read(r)?,
            },
            code::RELOGGED => Relogged,
            code::RECOMMENDATIONS => Recommendations {
                likes: Recommendation::read_list(r)?,
                dislikes: Recommendation::read_list(r).unwrap_or_default(),
            },
            code::GLOBAL_RECOMMENDATIONS => GlobalRecommendations {
                likes: Recommendation::read_list(r)?,
                dislikes: Recommendation::read_list(r).unwrap_or_default(),
            },
            code::USER_INTERESTS => UserInterests {
                username: r.string()?,
                likes: r.strings()?,
                hates: r.strings()?,
            },
            code::ROOM_LIST => {
                let public = read_counted_rooms(r)?;
                let owned = read_counted_rooms(r).unwrap_or_default();
                let private = read_counted_rooms(r).unwrap_or_default();
                let operated = r.strings().unwrap_or_default();
                RoomList(self::RoomList {
                    public,
                    owned,
                    private,
                    operated,
                })
            }
            code::ADMIN_MESSAGE => AdminMessage(r.string()?),
            code::PRIVILEGED_USERS => PrivilegedUsers(r.strings()?),
            code::PARENT_MIN_SPEED => ParentMinSpeed(r.u32()?),
            code::PARENT_SPEED_RATIO => ParentSpeedRatio(r.u32()?),
            code::CHECK_PRIVILEGES => CheckPrivileges(r.u32()?),
            code::EMBEDDED_MESSAGE => EmbeddedMessage {
                code: r.u8()?,
                payload: r.rest().to_vec(),
            },
            code::POSSIBLE_PARENTS => {
                let count = r.count(12)?;
                PossibleParents(
                    (0..count)
                        .map(|_| {
                            Ok(PossibleParent {
                                username: r.string()?,
                                ip: r.ip()?,
                                port: port(r)?,
                            })
                        })
                        .collect::<WireResult<_>>()?,
                )
            }
            code::WISHLIST_INTERVAL => WishlistInterval(r.u32()?),
            code::SIMILAR_USERS => {
                let count = r.count(8)?;
                SimilarUsers(
                    (0..count)
                        .map(|_| Ok((r.string()?, r.u32()?)))
                        .collect::<WireResult<_>>()?,
                )
            }
            code::ITEM_RECOMMENDATIONS => ItemRecommendations {
                item: r.string()?,
                recommendations: Recommendation::read_list(r)?,
            },
            code::ITEM_SIMILAR_USERS => ItemSimilarUsers {
                item: r.string()?,
                usernames: r.strings()?,
            },
            code::ROOM_TICKERS => {
                let room = r.string()?;
                let count = r.count(8)?;
                RoomTickers {
                    room,
                    tickers: (0..count)
                        .map(|_| Ok((r.string()?, r.string()?)))
                        .collect::<WireResult<_>>()?,
                }
            }
            code::ROOM_TICKER_ADDED => RoomTickerAdded {
                room: r.string()?,
                username: r.string()?,
                ticker: r.string()?,
            },
            code::ROOM_TICKER_REMOVED => RoomTickerRemoved {
                room: r.string()?,
                username: r.string()?,
            },
            code::RESET_DISTRIBUTED => ResetDistributed,
            code::ROOM_MEMBERS => RoomMembers {
                room: r.string()?,
                members: r.strings()?,
            },
            code::ADD_ROOM_MEMBER => AddRoomMember {
                room: r.string()?,
                username: r.string()?,
            },
            code::REMOVE_ROOM_MEMBER => RemoveRoomMember {
                room: r.string()?,
                username: r.string()?,
            },
            code::ROOM_MEMBERSHIP_GRANTED => RoomMembershipGranted(r.string()?),
            code::ROOM_MEMBERSHIP_REVOKED => RoomMembershipRevoked(r.string()?),
            code::ENABLE_ROOM_INVITATIONS => EnableRoomInvitations(r.bool()?),
            code::CHANGE_PASSWORD => ChangePassword(r.string()?),
            code::ADD_ROOM_OPERATOR => AddRoomOperator {
                room: r.string()?,
                username: r.string()?,
            },
            code::REMOVE_ROOM_OPERATOR => RemoveRoomOperator {
                room: r.string()?,
                username: r.string()?,
            },
            code::ROOM_OPERATORSHIP_GRANTED => RoomOperatorshipGranted(r.string()?),
            code::ROOM_OPERATORSHIP_REVOKED => RoomOperatorshipRevoked(r.string()?),
            code::ROOM_OPERATORS => RoomOperators {
                room: r.string()?,
                operators: r.strings()?,
            },
            code::GLOBAL_ROOM_MESSAGE => GlobalRoomMessage {
                room: r.string()?,
                username: r.string()?,
                message: r.string()?,
            },
            code::EXCLUDED_SEARCH_PHRASES => ExcludedSearchPhrases(r.strings()?),
            code::CANT_CONNECT_TO_PEER => CantConnectToPeer(r.u32()?),
            code::CANT_CREATE_ROOM => CantCreateRoom(r.string()?),
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

    fn split(frame: &[u8]) -> (u32, &[u8]) {
        let len = u32::from_le_bytes(frame[0..4].try_into().unwrap()) as usize;
        assert_eq!(len, frame.len() - 4);
        (
            u32::from_le_bytes(frame[4..8].try_into().unwrap()),
            &frame[8..],
        )
    }

    #[test]
    fn login_matches_the_documented_bytes() {
        let frame = ServerRequest::Login {
            username: "username".into(),
            password: "password".into(),
            major: 177,
            minor: 1,
        }
        .encode();
        let expected = "48 00 00 00 01 00 00 00 08 00 00 00 75 73 65 72 6e 61 6d 65 08 00 00 00 70 61 73 73 77 6f 72 64 b1 00 00 00 20 00 00 00 64 35 31 63 39 61 37 65 39 33 35 33 37 34 36 61 36 30 32 30 66 39 36 30 32 64 34 35 32 39 32 39 01 00 00 00";
        let expected: Vec<u8> = expected
            .split(' ')
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect();
        assert_eq!(frame, expected);
    }

    #[test]
    fn encodes_requests() {
        let frame = ServerRequest::ConnectToPeer {
            token: 7,
            username: "bob".into(),
            kind: ConnectionType::File,
        }
        .encode();
        let (code, body) = split(&frame);
        assert_eq!(code, 18);
        let mut r = Reader::new(body);
        assert_eq!(r.u32(), Ok(7));
        assert_eq!(r.string().as_deref(), Ok("bob"));
        assert_eq!(r.string().as_deref(), Ok("F"));

        let frame = ServerRequest::SetStatus(UserStatus::Away).encode();
        assert_eq!(split(&frame), (28, &[1, 0, 0, 0][..]));
        assert_eq!(split(&ServerRequest::ServerPing.encode()), (32, &[][..]));
    }

    #[test]
    fn decodes_login_replies() {
        let mut w = Writer::new();
        w.bool(true)
            .str("hello")
            .ip(Ipv4Addr::new(1, 2, 3, 4))
            .str("hash")
            .bool(false);
        assert_eq!(
            ServerResponse::decode(1, &w.into_inner()),
            Ok(ServerResponse::Login(LoginResponse::Success {
                greeting: "hello".into(),
                own_ip: Ipv4Addr::new(1, 2, 3, 4),
                supporter: false,
            }))
        );
        let mut w = Writer::new();
        w.bool(false).str("INVALIDUSERNAME").str("Nick empty.");
        assert_eq!(
            ServerResponse::decode(1, &w.into_inner()),
            Ok(ServerResponse::Login(LoginResponse::Failure {
                reason: "INVALIDUSERNAME".into(),
                detail: Some("Nick empty.".into()),
            }))
        );
    }

    #[test]
    fn decodes_a_private_room_join() {
        let mut w = Writer::new();
        w.str("room").u32(2).str("ann").str("bob");
        w.u32(2).u32(2).u32(1);
        w.u32(2);
        UserStats {
            avg_speed: 10,
            upload_num: 1,
            files: 2,
            dirs: 3,
        }
        .write(&mut w);
        UserStats::default().write(&mut w);
        w.u32(2).u32(0).u32(1);
        w.u32(2).str("NL").str("US");
        w.str("ann").u32(1).str("bob");
        let ServerResponse::JoinRoom(joined) = ServerResponse::decode(14, &w.into_inner()).unwrap()
        else {
            panic!("not a join");
        };
        assert_eq!(joined.users.len(), 2);
        assert_eq!(joined.users[0].stats.files, 2);
        assert_eq!(joined.users[1].status, UserStatus::Away);
        assert!(joined.users[1].slots_full);
        assert_eq!(joined.users[1].country, "US");
        assert_eq!(joined.owner.as_deref(), Some("ann"));
        assert_eq!(joined.operators, ["bob"]);
    }

    #[test]
    fn decodes_connect_to_peer() {
        let mut w = Writer::new();
        w.str("bob")
            .str("P")
            .ip(Ipv4Addr::new(10, 0, 0, 2))
            .u32(2234)
            .u32(99)
            .bool(true)
            .u32(0)
            .u32(0);
        assert_eq!(
            ServerResponse::decode(18, &w.into_inner()),
            Ok(ServerResponse::ConnectToPeer {
                username: "bob".into(),
                kind: ConnectionType::Peer,
                ip: Ipv4Addr::new(10, 0, 0, 2),
                port: 2234,
                token: 99,
                privileged: true,
            })
        );
    }

    #[test]
    fn keeps_unknown_codes() {
        assert_eq!(
            ServerResponse::decode(9999, &[1, 2]),
            Ok(ServerResponse::Unknown {
                code: 9999,
                payload: vec![1, 2]
            })
        );
    }
}
