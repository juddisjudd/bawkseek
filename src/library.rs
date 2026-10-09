use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag};
use serde::{Deserialize, Serialize};

/// The formats the player can decode.
pub const PLAYABLE: [&str; 10] = [
    "mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "wav", "aif", "aiff",
];
/// Covers are kept at twice the size they are drawn, so they stay sharp on high-density screens.
const COVER_SIZE: u32 = 320;
const COVER_NAMES: [&str; 4] = ["cover", "folder", "front", "album"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub year: Option<u32>,
    pub duration: u32,
    size: u64,
    modified: u64,
}

#[derive(Clone, Debug)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub year: Option<u32>,
    pub tracks: Vec<Track>,
    pub cover: Option<PathBuf>,
    pub duration: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Library {
    pub albums: Vec<Album>,
    pub tracks: usize,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    tracks: Vec<Track>,
}

fn hash_of(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn modified(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Roots inside another root would list their files twice, so only the outermost are walked.
fn outermost(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = Vec::new();
    for root in roots {
        if roots
            .iter()
            .any(|other| other != root && root.starts_with(other))
            || kept.contains(root)
        {
            continue;
        }
        kept.push(root.clone());
    }
    kept
}

fn walk(root: &Path, found: &mut Vec<(PathBuf, fs::Metadata)>) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file()
                && PLAYABLE.contains(&extension(&path).as_str())
                && let Ok(meta) = entry.metadata()
            {
                found.push((path, meta));
            }
        }
    }
}

fn tag_text(tag: &Tag, key: ItemKey) -> Option<String> {
    tag.get_string(key)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Tags where the file has them; the file and folder names stand in where it does not.
fn read_track(path: &Path, size: u64, modified: u64) -> Track {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut track = Track {
        path: path.to_path_buf(),
        title: stem,
        artist: String::new(),
        album: folder,
        album_artist: String::new(),
        track: None,
        disc: None,
        year: None,
        duration: 0,
        size,
        modified,
    };
    let options = ParseOptions::new().read_cover_art(false);
    let Some(file) = Probe::open(path)
        .ok()
        .and_then(|probe| probe.options(options).guess_file_type().ok())
        .and_then(|probe| probe.read().ok())
    else {
        return track;
    };
    track.duration = file.properties().duration().as_secs() as u32;
    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else {
        return track;
    };
    if let Some(title) = tag.title().filter(|title| !title.trim().is_empty()) {
        track.title = title.trim().to_string();
    }
    if let Some(artist) = tag.artist() {
        track.artist = artist.trim().to_string();
    }
    if let Some(album) = tag.album().filter(|album| !album.trim().is_empty()) {
        track.album = album.trim().to_string();
    }
    track.album_artist = tag_text(tag, ItemKey::AlbumArtist).unwrap_or_default();
    track.track = tag.track();
    track.disc = tag.disk();
    track.year = tag.date().map(|date| u32::from(date.year));
    track
}

/// Tracks of one album share an album artist and title; untagged files group by folder instead.
fn album_key(track: &Track) -> (String, String) {
    let artist = if track.album_artist.is_empty() {
        &track.artist
    } else {
        &track.album_artist
    };
    let folder = track
        .path
        .parent()
        .map(|dir| dir.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if track.album.is_empty() {
        (String::new(), folder)
    } else {
        (artist.to_lowercase(), track.album.to_lowercase())
    }
}

fn folder_image(dir: &Path) -> Option<PathBuf> {
    let mut images: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| matches!(extension(path).as_str(), "jpg" | "jpeg" | "png"))
        .collect();
    images.sort();
    let named = images.iter().find(|path| {
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        COVER_NAMES.iter().any(|name| stem.contains(name))
    });
    named.or(images.first()).cloned()
}

fn embedded_image(path: &Path) -> Option<Vec<u8>> {
    let options = ParseOptions::new().read_properties(false);
    let file = Probe::open(path)
        .ok()?
        .options(options)
        .guess_file_type()
        .ok()?
        .read()
        .ok()?;
    let tag = file.primary_tag().or_else(|| file.first_tag())?;
    let pictures = tag.pictures();
    pictures
        .iter()
        .find(|picture| picture.pic_type() == PictureType::CoverFront)
        .or(pictures.first())
        .map(|picture| picture.data().to_vec())
}

fn write_thumbnail(bytes: &[u8], out: &Path) -> bool {
    let Ok(image) = image::load_from_memory(bytes) else {
        return false;
    };
    let thumb = image.thumbnail(COVER_SIZE, COVER_SIZE).to_rgb8();
    if let Some(dir) = out.parent() {
        let _ = fs::create_dir_all(dir);
    }
    thumb
        .save_with_format(out, image::ImageFormat::Jpeg)
        .is_ok()
}

/// A small cover for the album, from an image in its folder or the art inside its first track.
fn cover_for(album: &Album, covers: &Path) -> Option<PathBuf> {
    let first = album.tracks.first()?;
    let out = covers.join(format!("{:016x}.jpg", hash_of(&album_key(first))));
    if out.exists() {
        return Some(out);
    }
    let from_folder = first
        .path
        .parent()
        .and_then(folder_image)
        .and_then(|image| fs::read(image).ok());
    let bytes = from_folder.or_else(|| {
        album
            .tracks
            .iter()
            .take(3)
            .find_map(|t| embedded_image(&t.path))
    })?;
    write_thumbnail(&bytes, &out).then_some(out)
}

fn album_of(tracks: Vec<Track>) -> Album {
    let first = &tracks[0];
    let artist = [&first.album_artist, &first.artist]
        .into_iter()
        .find(|name| !name.is_empty())
        .cloned()
        .unwrap_or_else(|| "unknown artist".into());
    Album {
        title: first.album.clone(),
        artist,
        year: tracks.iter().find_map(|track| track.year),
        duration: tracks.iter().map(|track| track.duration).sum(),
        cover: None,
        tracks,
    }
}

/// Files played straight from the transfer list, in the order given, which the library may not have scanned yet.
pub fn album_from(paths: &[PathBuf], data_dir: &Path) -> Option<Album> {
    let tracks: Vec<Track> = paths
        .iter()
        .filter_map(|path| {
            let meta = fs::metadata(path).ok()?;
            Some(read_track(path, meta.len(), modified(&meta)))
        })
        .collect();
    if tracks.is_empty() {
        return None;
    }
    let mut album = album_of(tracks);
    album.cover = cover_for(&album, &data_dir.join("covers"));
    Some(album)
}

pub fn is_playable(path: &Path) -> bool {
    PLAYABLE.contains(&extension(path).as_str())
}

fn group(tracks: Vec<Track>) -> Vec<Album> {
    let mut albums: BTreeMap<(String, String), Vec<Track>> = BTreeMap::new();
    for track in tracks {
        albums.entry(album_key(&track)).or_default().push(track);
    }
    let mut albums: Vec<Album> = albums
        .into_values()
        .map(|mut tracks| {
            tracks.sort_by(|a, b| {
                (a.disc.unwrap_or(1), a.track.unwrap_or(u32::MAX), &a.path).cmp(&(
                    b.disc.unwrap_or(1),
                    b.track.unwrap_or(u32::MAX),
                    &b.path,
                ))
            });
            album_of(tracks)
        })
        .collect();
    albums.sort_by_cached_key(|album| {
        (
            album.artist.to_lowercase(),
            album.year.unwrap_or(0),
            album.title.to_lowercase(),
        )
    });
    albums
}

/// Walks the folders, reusing what the last scan read for files that did not change.
pub fn scan(roots: &[PathBuf], data_dir: &Path) -> Library {
    let cache_path = data_dir.join("library.json");
    let cached: HashMap<PathBuf, Track> = fs::read(&cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Cache>(&bytes).ok())
        .map(|cache| {
            cache
                .tracks
                .into_iter()
                .map(|track| (track.path.clone(), track))
                .collect()
        })
        .unwrap_or_default();

    let mut found = Vec::new();
    for root in outermost(roots) {
        walk(&root, &mut found);
    }
    let mut seen = HashSet::new();
    let tracks: Vec<Track> = found
        .into_iter()
        .filter(|(path, _)| seen.insert(path.clone()))
        .map(|(path, meta)| {
            let (size, modified) = (meta.len(), modified(&meta));
            match cached.get(&path) {
                Some(track) if track.size == size && track.modified == modified => track.clone(),
                _ => read_track(&path, size, modified),
            }
        })
        .collect();

    if let Ok(json) = serde_json::to_vec(&Cache {
        tracks: tracks.clone(),
    }) {
        let _ = fs::create_dir_all(data_dir);
        let _ = fs::write(&cache_path, json);
    }

    let count = tracks.len();
    let covers = data_dir.join("covers");
    let mut albums = group(tracks);
    for album in &mut albums {
        album.cover = cover_for(album, &covers);
    }
    Library {
        albums,
        tracks: count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(path: &str, album: &str, artist: &str, number: Option<u32>) -> Track {
        Track {
            path: PathBuf::from(path),
            title: path.into(),
            artist: artist.into(),
            album: album.into(),
            album_artist: String::new(),
            track: number,
            disc: None,
            year: None,
            duration: 60,
            size: 1,
            modified: 1,
        }
    }

    #[test]
    fn groups_by_album_and_orders_tracks() {
        let albums = group(vec![
            track(r"m\b\2.mp3", "Geogaddi", "Boards of Canada", Some(2)),
            track(r"m\b\1.mp3", "Geogaddi", "Boards of Canada", Some(1)),
            track(r"m\a\x.mp3", "Syro", "Aphex Twin", None),
            track(r"m\c\y.mp3", "", "", None),
        ]);
        assert_eq!(albums.len(), 3);
        let titles: Vec<&str> = albums.iter().map(|album| album.title.as_str()).collect();
        assert_eq!(titles, ["Syro", "Geogaddi", ""]);
        let geogaddi = &albums[1];
        assert_eq!(geogaddi.tracks[0].track, Some(1));
        assert_eq!(geogaddi.duration, 120);
        assert_eq!(albums[2].artist, "unknown artist");
    }

    #[test]
    fn skips_roots_inside_other_roots() {
        let roots = vec![
            PathBuf::from(r"D:\Music"),
            PathBuf::from(r"D:\Music\Rock"),
            PathBuf::from(r"E:\Downloads"),
            PathBuf::from(r"E:\Downloads"),
        ];
        assert_eq!(
            outermost(&roots),
            vec![PathBuf::from(r"D:\Music"), PathBuf::from(r"E:\Downloads")]
        );
    }

    #[test]
    fn scans_files_and_names_untagged_ones_after_their_folder() {
        let dir = std::env::temp_dir().join(format!("bawkseek-library-{}", std::process::id()));
        let album = dir.join("Music").join("Some Album");
        fs::create_dir_all(&album).unwrap();
        fs::write(album.join("01 intro.mp3"), b"not really audio").unwrap();
        fs::write(album.join("notes.txt"), b"skip me").unwrap();
        let data = dir.join("data");
        let library = scan(&[dir.join("Music")], &data);
        assert_eq!(library.tracks, 1);
        assert_eq!(library.albums[0].title, "Some Album");
        assert_eq!(library.albums[0].tracks[0].title, "01 intro");
        assert!(data.join("library.json").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
