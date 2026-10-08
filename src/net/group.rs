use std::collections::{HashMap, HashSet};

use soulseek_rs::SearchResult;

use crate::format;

const ATTR_BITRATE: u32 = 0;
const ATTR_DURATION: u32 = 1;
const ATTR_VBR: u32 = 2;
const ATTR_SAMPLE_RATE: u32 = 4;
const ATTR_BIT_DEPTH: u32 = 5;

const LOSSLESS: [&str; 6] = ["flac", "wav", "alac", "ape", "aiff", "wv"];
const LOSSY: [&str; 6] = ["mp3", "ogg", "opus", "m4a", "aac", "wma"];

pub fn is_lossless(ext: &str) -> bool {
    LOSSLESS.contains(&ext)
}

fn is_audio(ext: &str) -> bool {
    is_lossless(ext) || LOSSY.contains(&ext)
}

#[derive(Clone, Debug)]
pub struct FileHit {
    pub filename: String,
    pub name: String,
    pub ext: String,
    pub size: u64,
    pub bitrate: Option<u32>,
    pub duration: Option<u32>,
    pub vbr: bool,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u32>,
}

impl FileHit {
    pub fn quality(&self) -> String {
        match (self.bit_depth, self.sample_rate) {
            (Some(depth), Some(rate)) => format!("{depth}/{}", khz(rate)),
            _ => match self.bitrate {
                Some(rate) if self.vbr => format!("~{rate}"),
                Some(rate) => rate.to_string(),
                None => String::new(),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct FolderHit {
    pub username: String,
    pub folder: String,
    pub name: String,
    pub files: Vec<FileHit>,
    pub size: u64,
    pub speed: u32,
    pub free: bool,
    pub format: String,
    pub quality: String,
}

impl FolderHit {
    pub fn summary(&self) -> String {
        if self.quality.is_empty() {
            self.format.clone()
        } else {
            format!("{} {}", self.format, self.quality)
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SearchHits {
    pub folders: Vec<FolderHit>,
    /// Audio formats in the results, most files first.
    pub formats: Vec<(String, usize)>,
    pub files: usize,
    pub users: usize,
}

pub fn group(results: &[SearchResult]) -> SearchHits {
    let mut index: HashMap<(&str, &str), usize> = HashMap::new();
    let mut folders: Vec<FolderHit> = Vec::new();
    let mut users = HashSet::new();
    let mut files = 0;
    let mut formats: HashMap<String, usize> = HashMap::new();

    for result in results {
        users.insert(result.username.as_str());
        for file in &result.files {
            files += 1;
            let (folder, name) = format::split_path(&file.name);
            let key = (result.username.as_str(), folder);
            let ix = *index.entry(key).or_insert_with(|| {
                folders.push(FolderHit {
                    username: result.username.clone(),
                    folder: folder.to_string(),
                    name: format::split_path(folder).1.to_string(),
                    files: Vec::new(),
                    size: 0,
                    speed: result.speed,
                    free: result.slots > 0,
                    format: String::new(),
                    quality: String::new(),
                });
                folders.len() - 1
            });
            let attr = |code| file.attribs.get(&code).copied().filter(|v| *v > 0);
            let hit = FileHit {
                filename: file.name.clone(),
                name: name.to_string(),
                ext: format::extension(name),
                size: file.size,
                bitrate: attr(ATTR_BITRATE),
                duration: attr(ATTR_DURATION),
                vbr: attr(ATTR_VBR).is_some(),
                sample_rate: attr(ATTR_SAMPLE_RATE),
                bit_depth: attr(ATTR_BIT_DEPTH),
            };
            if is_audio(&hit.ext) {
                *formats.entry(hit.ext.clone()).or_default() += 1;
            }
            let entry = &mut folders[ix];
            entry.size += hit.size;
            entry.files.push(hit);
        }
    }

    for folder in &mut folders {
        folder.files.sort_by(|a, b| a.name.cmp(&b.name));
        (folder.format, folder.quality) = summarize(&folder.files);
    }

    let mut formats: Vec<(String, usize)> = formats.into_iter().collect();
    formats.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    SearchHits {
        folders,
        formats,
        files,
        users: users.len(),
    }
}

fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        (rate / 1000).to_string()
    } else {
        format!("{:.1}", rate as f64 / 1000.0)
    }
}

/// A folder's dominant audio format and one short quality label for it.
fn summarize(files: &[FileHit]) -> (String, String) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for file in files {
        if is_audio(&file.ext) {
            *counts.entry(file.ext.as_str()).or_default() += 1;
        }
    }
    let Some((ext, _)) = counts
        .iter()
        .max_by_key(|(ext, count)| (**count, std::cmp::Reverse(**ext)))
    else {
        let ext = files.first().map(|file| file.ext.clone());
        return (ext.unwrap_or_default(), String::new());
    };
    let audio: Vec<&FileHit> = files.iter().filter(|file| file.ext == *ext).collect();

    let quality = if is_lossless(ext) {
        let depth = uniform(audio.iter().map(|file| file.bit_depth));
        let rate = uniform(audio.iter().map(|file| file.sample_rate));
        match (depth, rate) {
            (Some(depth), Some(rate)) => format!("{depth}/{}", khz(rate)),
            _ => String::new(),
        }
    } else {
        let rates: Vec<u32> = audio.iter().filter_map(|file| file.bitrate).collect();
        let vbr = audio.iter().any(|file| file.vbr);
        match uniform(audio.iter().map(|file| file.bitrate)) {
            Some(rate) if !vbr => rate.to_string(),
            _ if !rates.is_empty() => {
                let average = rates.iter().sum::<u32>() / rates.len() as u32;
                format!("~{average}")
            }
            _ => String::new(),
        }
    };

    let mixed = if counts.len() > 1 { "+" } else { "" };
    let quality = [quality.as_str(), mixed]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (ext.to_string(), quality)
}

fn uniform(mut values: impl Iterator<Item = Option<u32>>) -> Option<u32> {
    let first = values.next()??;
    values.all(|value| value == Some(first)).then_some(first)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use soulseek_rs::File;

    use super::*;

    fn file(user: &str, name: &str, size: u64, attribs: &[(u32, u32)]) -> File {
        File {
            username: user.into(),
            name: name.into(),
            size,
            attribs: attribs.iter().copied().collect::<HashMap<_, _>>(),
        }
    }

    fn result(user: &str, speed: u32, slots: u8, files: Vec<File>) -> SearchResult {
        SearchResult {
            token: 1,
            files,
            slots,
            speed,
            username: user.into(),
        }
    }

    #[test]
    fn groups_files_by_user_and_folder() {
        let hits = group(&[
            result(
                "ann",
                900,
                1,
                vec![
                    file(
                        "ann",
                        "@@m\\Artist\\Album\\02 b.flac",
                        30,
                        &[(4, 44100), (5, 16)],
                    ),
                    file(
                        "ann",
                        "@@m\\Artist\\Album\\01 a.flac",
                        20,
                        &[(4, 44100), (5, 16)],
                    ),
                    file("ann", "@@m\\Artist\\Other\\x.mp3", 5, &[(0, 320)]),
                ],
            ),
            result(
                "bob",
                100,
                0,
                vec![file(
                    "bob",
                    "music\\Album\\01 a.mp3",
                    7,
                    &[(0, 192), (2, 1)],
                )],
            ),
        ]);

        assert_eq!(hits.files, 4);
        assert_eq!(hits.formats, [("flac".into(), 2), ("mp3".into(), 2)]);
        assert_eq!(hits.users, 2);
        assert_eq!(hits.folders.len(), 3);

        let album = &hits.folders[0];
        assert_eq!(album.name, "Album");
        assert_eq!(album.size, 50);
        assert!(album.free);
        assert_eq!(album.files[0].name, "01 a.flac");
        assert_eq!(album.summary(), "flac 16/44.1");

        assert_eq!(hits.folders[1].summary(), "mp3 320");
        assert_eq!(hits.folders[2].summary(), "mp3 ~192");
        assert!(!hits.folders[2].free);
    }

    #[test]
    fn describes_file_quality() {
        let hits = group(&[result(
            "ann",
            1,
            1,
            vec![
                file("ann", "a\\x.flac", 1, &[(4, 96000), (5, 24)]),
                file("ann", "a\\y.mp3", 1, &[(0, 245), (2, 1)]),
                file("ann", "a\\z.jpg", 1, &[]),
            ],
        )]);
        let files = &hits.folders[0].files;
        assert_eq!(files[0].quality(), "24/96");
        assert_eq!(files[1].quality(), "~245");
        assert_eq!(files[2].quality(), "");
        assert_eq!(hits.folders[0].format, "flac");
        assert_eq!(hits.folders[0].quality, "24/96 +");
    }
}
