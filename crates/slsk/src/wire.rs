use std::io::{Read, Write as _};
use std::net::Ipv4Addr;

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WireError {
    #[error("message ended early")]
    Short,
    #[error("message is too large")]
    TooLarge,
    #[error("bad compressed data")]
    Inflate,
    #[error("unexpected value {0}")]
    Invalid(u32),
}

pub type WireResult<T> = Result<T, WireError>;

/// Builds a message body in Soulseek's little-endian packing.
#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn u8(&mut self, value: u8) -> &mut Self {
        self.buf.push(value);
        self
    }

    pub fn u16(&mut self, value: u16) -> &mut Self {
        self.buf.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn u32(&mut self, value: u32) -> &mut Self {
        self.buf.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn i32(&mut self, value: i32) -> &mut Self {
        self.buf.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn u64(&mut self, value: u64) -> &mut Self {
        self.buf.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn bool(&mut self, value: bool) -> &mut Self {
        self.u8(value as u8)
    }

    pub fn str(&mut self, value: &str) -> &mut Self {
        self.bytes(value.as_bytes())
    }

    pub fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.u32(value.len() as u32);
        self.buf.extend_from_slice(value);
        self
    }

    pub fn raw(&mut self, value: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(value);
        self
    }

    /// The address as the protocol packs it: the first octet is the most significant byte.
    pub fn ip(&mut self, ip: Ipv4Addr) -> &mut Self {
        self.u32(u32::from(ip))
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    /// Frames the body behind a length and a 4-byte code, as server and peer messages are.
    pub fn frame_u32(code: u32, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(body.len() + 8);
        out.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
        out.extend_from_slice(&code.to_le_bytes());
        out.extend_from_slice(body);
        out
    }

    /// Frames the body behind a length and a 1-byte code, as peer init and distributed messages are.
    pub fn frame_u8(code: u8, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(body.len() + 5);
        out.extend_from_slice(&(body.len() as u32 + 1).to_le_bytes());
        out.push(code);
        out.extend_from_slice(body);
        out
    }
}

/// Reads a message body, failing cleanly on truncated or hostile input.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn is_done(&self) -> bool {
        self.remaining() == 0
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let rest = &self.buf[self.pos..];
        self.pos = self.buf.len();
        rest
    }

    fn take(&mut self, len: usize) -> WireResult<&'a [u8]> {
        if self.remaining() < len {
            return Err(WireError::Short);
        }
        let slice = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> WireResult<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("take returns N bytes"))
    }

    pub fn u8(&mut self) -> WireResult<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u16(&mut self) -> WireResult<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> WireResult<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> WireResult<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> WireResult<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub fn bool(&mut self) -> WireResult<bool> {
        Ok(self.u8()? != 0)
    }

    pub fn ip(&mut self) -> WireResult<Ipv4Addr> {
        Ok(Ipv4Addr::from(self.u32()?))
    }

    pub fn bytes(&mut self) -> WireResult<&'a [u8]> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    /// Text is UTF-8 from modern clients, but older ones send Latin-1.
    pub fn string(&mut self) -> WireResult<String> {
        Ok(decode_text(self.bytes()?))
    }

    /// A list count, capped by what the remaining bytes could hold so a bogus count cannot exhaust memory.
    pub fn count(&mut self, min_item: usize) -> WireResult<usize> {
        let count = self.u32()? as usize;
        if count > self.remaining() / min_item.max(1) {
            return Err(WireError::Short);
        }
        Ok(count)
    }

    pub fn strings(&mut self) -> WireResult<Vec<String>> {
        let count = self.count(4)?;
        (0..count).map(|_| self.string()).collect()
    }
}

pub fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().map(|byte| *byte as char).collect(),
    }
}

pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .expect("writing to a Vec cannot fail");
    encoder.finish().expect("writing to a Vec cannot fail")
}

/// Inflates zlib data, refusing output beyond `limit` bytes.
pub fn inflate(data: &[u8], limit: usize) -> WireResult<Vec<u8>> {
    let mut out = Vec::new();
    ZlibDecoder::new(data)
        .take(limit as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|_| WireError::Inflate)?;
    if out.len() > limit {
        return Err(WireError::TooLarge);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_little_endian() {
        let mut w = Writer::new();
        w.u8(1).u16(2).u32(3).i32(-1).u64(5).bool(true).str("hi");
        assert_eq!(
            w.into_inner(),
            [
                1, 2, 0, 3, 0, 0, 0, 255, 255, 255, 255, 5, 0, 0, 0, 0, 0, 0, 0, 1, 2, 0, 0, 0,
                b'h', b'i'
            ]
        );
    }

    #[test]
    fn reads_what_it_wrote() {
        let mut w = Writer::new();
        w.u8(9)
            .u16(300)
            .u32(70_000)
            .i32(-5)
            .u64(1 << 40)
            .bool(false)
            .str("héllo");
        w.ip(Ipv4Addr::new(10, 0, 0, 5));
        let bytes = w.into_inner();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8(), Ok(9));
        assert_eq!(r.u16(), Ok(300));
        assert_eq!(r.u32(), Ok(70_000));
        assert_eq!(r.i32(), Ok(-5));
        assert_eq!(r.u64(), Ok(1 << 40));
        assert_eq!(r.bool(), Ok(false));
        assert_eq!(r.string().as_deref(), Ok("héllo"));
        assert_eq!(r.ip(), Ok(Ipv4Addr::new(10, 0, 0, 5)));
        assert!(r.is_done());
        assert_eq!(r.u8(), Err(WireError::Short));
    }

    #[test]
    fn falls_back_to_latin1() {
        assert_eq!(decode_text(&[b'c', 0xe9]), "cé");
    }

    #[test]
    fn rejects_counts_larger_than_the_message() {
        let mut w = Writer::new();
        w.u32(1_000_000).u32(1);
        let bytes = w.into_inner();
        assert_eq!(Reader::new(&bytes).count(4), Err(WireError::Short));
    }

    #[test]
    fn frames_with_length_and_code() {
        assert_eq!(Writer::frame_u32(32, &[]), [4, 0, 0, 0, 32, 0, 0, 0]);
        assert_eq!(Writer::frame_u8(1, &[7]), [2, 0, 0, 0, 1, 7]);
    }

    #[test]
    fn inflates_with_a_limit() {
        let packed = deflate(&[7; 1000]);
        assert_eq!(inflate(&packed, 1000).unwrap().len(), 1000);
        assert_eq!(inflate(&packed, 999), Err(WireError::TooLarge));
        assert_eq!(inflate(b"nonsense", 10), Err(WireError::Inflate));
    }
}
