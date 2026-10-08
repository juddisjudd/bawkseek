use crate::wire::{Reader, WireError, WireResult, Writer};

pub mod code {
    pub const PING: u8 = 0;
    pub const SEARCH: u8 = 3;
    pub const BRANCH_LEVEL: u8 = 4;
    pub const BRANCH_ROOT: u8 = 5;
    pub const CHILD_DEPTH: u8 = 7;
    pub const EMBEDDED_MESSAGE: u8 = 93;
}

/// The value every distributed search carries in its first field.
const SEARCH_IDENTIFIER: u32 = 49;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DistribMessage {
    Ping,
    Search {
        username: String,
        token: u32,
        query: String,
    },
    BranchLevel(i32),
    BranchRoot(String),
    ChildDepth(u32),
    EmbeddedMessage {
        code: u8,
        payload: Vec<u8>,
    },
    Unknown {
        code: u8,
        payload: Vec<u8>,
    },
}

impl DistribMessage {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        let code = match self {
            DistribMessage::Ping => code::PING,
            DistribMessage::Search {
                username,
                token,
                query,
            } => {
                w.u32(SEARCH_IDENTIFIER)
                    .str(username)
                    .u32(*token)
                    .str(query);
                code::SEARCH
            }
            DistribMessage::BranchLevel(level) => {
                w.i32(*level);
                code::BRANCH_LEVEL
            }
            DistribMessage::BranchRoot(root) => {
                w.str(root);
                code::BRANCH_ROOT
            }
            DistribMessage::ChildDepth(depth) => {
                w.u32(*depth);
                code::CHILD_DEPTH
            }
            DistribMessage::EmbeddedMessage { code, payload } => {
                w.u8(*code).raw(payload);
                code::EMBEDDED_MESSAGE
            }
            DistribMessage::Unknown { code, payload } => {
                w.raw(payload);
                *code
            }
        };
        Writer::frame_u8(code, &w.into_inner())
    }

    pub fn decode(code: u8, body: &[u8]) -> WireResult<Self> {
        let mut r = Reader::new(body);
        let message = match code {
            code::PING => DistribMessage::Ping,
            code::SEARCH => {
                let identifier = r.u32()?;
                if identifier != SEARCH_IDENTIFIER {
                    return Err(WireError::Invalid(identifier));
                }
                DistribMessage::Search {
                    username: r.string()?,
                    token: r.u32()?,
                    query: r.string()?,
                }
            }
            code::BRANCH_LEVEL => DistribMessage::BranchLevel(r.i32()?),
            code::BRANCH_ROOT => DistribMessage::BranchRoot(r.string()?),
            code::CHILD_DEPTH => DistribMessage::ChildDepth(r.u32()?),
            code::EMBEDDED_MESSAGE => DistribMessage::EmbeddedMessage {
                code: r.u8()?,
                payload: r.rest().to_vec(),
            },
            _ => DistribMessage::Unknown {
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

    #[test]
    fn roundtrips() {
        for message in [
            DistribMessage::Ping,
            DistribMessage::Search {
                username: "ann".into(),
                token: 9,
                query: "boards of canada".into(),
            },
            DistribMessage::BranchLevel(2),
            DistribMessage::BranchRoot("root".into()),
            DistribMessage::ChildDepth(0),
            DistribMessage::EmbeddedMessage {
                code: 3,
                payload: vec![1, 2, 3],
            },
        ] {
            let frame = message.encode();
            assert_eq!(DistribMessage::decode(frame[4], &frame[5..]), Ok(message));
        }
    }

    #[test]
    fn rejects_searches_with_a_bad_identifier() {
        let mut w = Writer::new();
        w.u32(1).str("ann").u32(1).str("x");
        assert_eq!(
            DistribMessage::decode(3, &w.into_inner()),
            Err(WireError::Invalid(1))
        );
    }
}
