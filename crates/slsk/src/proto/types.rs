use crate::wire::{Reader, WireError, WireResult, Writer};

pub const ATTR_BITRATE: u32 = 0;
pub const ATTR_DURATION: u32 = 1;
pub const ATTR_VBR: u32 = 2;
pub const ATTR_SAMPLE_RATE: u32 = 4;
pub const ATTR_BIT_DEPTH: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConnectionType {
    Peer,
    File,
    Distributed,
}

impl ConnectionType {
    pub fn code(self) -> &'static str {
        match self {
            ConnectionType::Peer => "P",
            ConnectionType::File => "F",
            ConnectionType::Distributed => "D",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "P" => Some(ConnectionType::Peer),
            "F" => Some(ConnectionType::File),
            "D" => Some(ConnectionType::Distributed),
            _ => None,
        }
    }

    pub(crate) fn read(r: &mut Reader) -> WireResult<Self> {
        let code = r.string()?;
        Self::parse(&code).ok_or(WireError::Invalid(code.bytes().next().unwrap_or(0) as u32))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum UserStatus {
    #[default]
    Offline,
    Away,
    Online,
}

impl UserStatus {
    pub fn code(self) -> u32 {
        match self {
            UserStatus::Offline => 0,
            UserStatus::Away => 1,
            UserStatus::Online => 2,
        }
    }

    pub fn from_code(code: u32) -> Self {
        match code {
            1 => UserStatus::Away,
            2 => UserStatus::Online,
            _ => UserStatus::Offline,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UserStats {
    pub avg_speed: u32,
    pub upload_num: u32,
    pub files: u32,
    pub dirs: u32,
}

impl UserStats {
    pub(crate) fn read(r: &mut Reader) -> WireResult<Self> {
        let avg_speed = r.u32()?;
        let upload_num = r.u32()?;
        let _unknown = r.u32()?;
        Ok(Self {
            avg_speed,
            upload_num,
            files: r.u32()?,
            dirs: r.u32()?,
        })
    }

    #[cfg(test)]
    pub(crate) fn write(&self, w: &mut Writer) {
        w.u32(self.avg_speed)
            .u32(self.upload_num)
            .u32(0)
            .u32(self.files)
            .u32(self.dirs);
    }
}

/// One shared file as listed in search replies, browse replies and folder contents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
    pub ext: String,
    pub attrs: Vec<(u32, u32)>,
}

impl FileEntry {
    pub fn attr(&self, code: u32) -> Option<u32> {
        self.attrs
            .iter()
            .find(|(key, _)| *key == code)
            .map(|(_, value)| *value)
    }

    pub fn bitrate(&self) -> Option<u32> {
        self.attr(ATTR_BITRATE)
    }

    pub fn duration(&self) -> Option<u32> {
        self.attr(ATTR_DURATION)
    }

    pub fn vbr(&self) -> bool {
        self.attr(ATTR_VBR).is_some_and(|value| value != 0)
    }

    pub fn sample_rate(&self) -> Option<u32> {
        self.attr(ATTR_SAMPLE_RATE)
    }

    pub fn bit_depth(&self) -> Option<u32> {
        self.attr(ATTR_BIT_DEPTH)
    }

    pub(crate) fn read(r: &mut Reader) -> WireResult<Self> {
        let _code = r.u8()?;
        let name = r.string()?;
        let size = r.u64()?;
        let ext = r.string()?;
        let count = r.count(8)?;
        let attrs = (0..count)
            .map(|_| Ok((r.u32()?, r.u32()?)))
            .collect::<WireResult<_>>()?;
        Ok(Self {
            name,
            size,
            ext,
            attrs,
        })
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        w.u8(1)
            .str(&self.name)
            .u64(self.size)
            .str(&self.ext)
            .u32(self.attrs.len() as u32);
        for (code, value) in &self.attrs {
            w.u32(*code).u32(*value);
        }
    }

    pub(crate) fn read_list(r: &mut Reader) -> WireResult<Vec<Self>> {
        let count = r.count(21)?;
        (0..count).map(|_| Self::read(r)).collect()
    }

    pub(crate) fn write_list(files: &[Self], w: &mut Writer) {
        w.u32(files.len() as u32);
        for file in files {
            file.write(w);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Directory {
    pub name: String,
    pub files: Vec<FileEntry>,
}

impl Directory {
    pub(crate) fn read_list(r: &mut Reader) -> WireResult<Vec<Self>> {
        let count = r.count(8)?;
        (0..count)
            .map(|_| {
                Ok(Self {
                    name: r.string()?,
                    files: FileEntry::read_list(r)?,
                })
            })
            .collect()
    }

    pub(crate) fn write_list(dirs: &[Self], w: &mut Writer) {
        w.u32(dirs.len() as u32);
        for dir in dirs {
            w.str(&dir.name);
            FileEntry::write_list(&dir.files, w);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recommendation {
    pub item: String,
    pub score: i32,
}

impl Recommendation {
    pub(crate) fn read_list(r: &mut Reader) -> WireResult<Vec<Self>> {
        let count = r.count(8)?;
        (0..count)
            .map(|_| {
                Ok(Self {
                    item: r.string()?,
                    score: r.i32()?,
                })
            })
            .collect()
    }
}
