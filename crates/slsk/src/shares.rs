use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use lofty::config::ParseOptions;
use lofty::file::AudioFile;
use lofty::probe::Probe;
use serde::{Deserialize, Serialize};

use crate::client::Event;
use crate::engine::{Engine, Input};
use crate::proto::peer::{PeerMessage, SharedFileList};
use crate::proto::types::{
    ATTR_BIT_DEPTH, ATTR_BITRATE, ATTR_DURATION, ATTR_SAMPLE_RATE, Directory, FileEntry,
};

const LOSSLESS: [&str; 7] = ["flac", "wav", "aif", "aiff", "ape", "wv", "alac"];
const AUDIO: [&str; 14] = [
    "mp3", "flac", "ogg", "opus", "m4a", "aac", "wav", "aif", "aiff", "ape", "wv", "wma", "mpc",
    "alac",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedFile {
    pub virtual_path: String,
    pub real_path: PathBuf,
    pub size: u64,
    pub ext: String,
    pub attrs: Vec<(u32, u32)>,
}

impl SharedFile {
    fn entry(&self, name: &str) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            size: self.size,
            ext: self.ext.clone(),
            attrs: self.attrs.clone(),
        }
    }

    fn dir_and_name(&self) -> (&str, &str) {
        match self.virtual_path.rfind('\\') {
            Some(ix) => (&self.virtual_path[..ix], &self.virtual_path[ix + 1..]),
            None => ("", &self.virtual_path),
        }
    }
}

/// Every shared file, with a word index so incoming searches never walk the whole share.
#[derive(Default)]
pub struct ShareIndex {
    files: Vec<SharedFile>,
    dirs: BTreeMap<String, Vec<usize>>,
    words: HashMap<String, Vec<u32>>,
    by_path: HashMap<String, usize>,
}

/// Splits text into lowercase words the way paths are indexed, so a query and a path agree on what a word is.
pub fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

impl ShareIndex {
    pub fn build(mut files: Vec<SharedFile>) -> Self {
        files.sort_by(|a, b| a.virtual_path.cmp(&b.virtual_path));
        let mut dirs: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut words_index: HashMap<String, Vec<u32>> = HashMap::new();
        let mut by_path = HashMap::with_capacity(files.len());
        for (ix, file) in files.iter().enumerate() {
            let (dir, _) = file.dir_and_name();
            dirs.entry(dir.to_string()).or_default().push(ix);
            by_path.insert(file.virtual_path.clone(), ix);
            let unique: HashSet<String> = words(&file.virtual_path).collect();
            for word in unique {
                words_index.entry(word).or_default().push(ix as u32);
            }
        }
        Self {
            files,
            dirs,
            words: words_index,
            by_path,
        }
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn dir_count(&self) -> usize {
        self.dirs.len()
    }

    pub fn get(&self, virtual_path: &str) -> Option<&SharedFile> {
        self.by_path.get(virtual_path).map(|ix| &self.files[*ix])
    }

    /// Files whose path holds every query word, none of the `-excluded` words, and words ending in any `*partial` term.
    pub fn search(
        &self,
        query: &str,
        excluded_phrases: &[String],
        limit: usize,
    ) -> Vec<&SharedFile> {
        let mut include: Vec<String> = Vec::new();
        let mut exclude: Vec<String> = Vec::new();
        let mut partial: Vec<String> = Vec::new();
        for term in query.split_whitespace() {
            if let Some(rest) = term.strip_prefix('-') {
                exclude.extend(words(rest));
            } else if let Some(rest) = term.strip_prefix('*') {
                partial.extend(words(rest));
            } else {
                include.extend(words(term));
            }
        }
        if include.iter().map(String::len).sum::<usize>() < 2 {
            return Vec::new();
        }
        let mut postings: Vec<&Vec<u32>> = Vec::with_capacity(include.len());
        for word in &include {
            match self.words.get(word) {
                Some(list) => postings.push(list),
                None => return Vec::new(),
            }
        }
        postings.sort_by_key(|list| list.len());
        let rest = &postings[1..];
        let mut found = Vec::new();
        for ix in postings[0] {
            if !rest.iter().all(|list| list.binary_search(ix).is_ok()) {
                continue;
            }
            let file = &self.files[*ix as usize];
            let lower = file.virtual_path.to_lowercase();
            let path_words: HashSet<String> = words(&lower).collect();
            if exclude.iter().any(|word| path_words.contains(word)) {
                continue;
            }
            if !partial
                .iter()
                .all(|end| path_words.iter().any(|word| word.ends_with(end.as_str())))
            {
                continue;
            }
            if excluded_phrases.iter().any(|phrase| lower.contains(phrase)) {
                continue;
            }
            found.push(file);
            if found.len() >= limit {
                break;
            }
        }
        found
    }

    pub fn browse(&self) -> Vec<Directory> {
        self.dirs
            .iter()
            .map(|(dir, files)| self.directory(dir, files))
            .collect()
    }

    /// The folder and every folder under it, as peers expect from a folder contents request.
    pub fn folder(&self, folder: &str) -> Vec<Directory> {
        let prefix = format!("{folder}\\");
        self.dirs
            .range(folder.to_string()..)
            .take_while(|(dir, _)| *dir == folder || dir.starts_with(&prefix))
            .map(|(dir, files)| self.directory(dir, files))
            .collect()
    }

    fn directory(&self, dir: &str, files: &[usize]) -> Directory {
        Directory {
            name: dir.to_string(),
            files: files
                .iter()
                .map(|ix| {
                    let file = &self.files[*ix];
                    file.entry(file.dir_and_name().1)
                })
                .collect(),
        }
    }

    /// A search reply lists files by their full virtual path.
    pub fn result_entry(file: &SharedFile) -> FileEntry {
        file.entry(&file.virtual_path)
    }
}

/// Audio attributes from earlier scans, keyed by real path, reused while size and modification time are unchanged.
#[derive(Default, Serialize, Deserialize)]
pub struct ScanCache {
    entries: HashMap<PathBuf, CachedFile>,
}

#[derive(Clone, Serialize, Deserialize)]
struct CachedFile {
    size: u64,
    modified: u64,
    attrs: Vec<(u32, u32)>,
}

impl ScanCache {
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec(self) {
            let _ = fs::write(path, json);
        }
    }
}

/// The name other users see for each shared folder: its own name, numbered when two share one. Missing folders get none.
pub fn virtual_roots(roots: &[PathBuf]) -> Vec<Option<String>> {
    let mut used: HashSet<String> = HashSet::new();
    roots
        .iter()
        .map(|root| {
            if !root.is_dir() {
                return None;
            }
            let base = root.file_name().map_or_else(
                || "shared".into(),
                |name| name.to_string_lossy().into_owned(),
            );
            let mut name = base.clone();
            let mut n = 2;
            while !used.insert(name.to_lowercase()) {
                name = format!("{base} ({n})");
                n += 1;
            }
            Some(name)
        })
        .collect()
}

/// Walks the shared folders, each under its virtual root name.
pub fn scan(roots: &[PathBuf], cache: &mut ScanCache) -> Vec<SharedFile> {
    let mut seen_cache = HashMap::with_capacity(cache.entries.len());
    let mut files = Vec::new();
    for (root, name) in roots.iter().zip(virtual_roots(roots)) {
        let Some(name) = name else {
            continue;
        };
        let mut stack = vec![(root.clone(), name)];
        while let Some((dir, virtual_dir)) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let file_name = entry.file_name().to_string_lossy().into_owned();
                let virtual_path = format!("{virtual_dir}\\{file_name}");
                if kind.is_symlink() || file_name.starts_with('.') {
                    continue;
                }
                if kind.is_dir() {
                    stack.push((entry.path(), virtual_path));
                    continue;
                }
                let ext = Path::new(&file_name)
                    .extension()
                    .map(|ext| ext.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if ext == "part" || ext == "tmp" {
                    continue;
                }
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let path = entry.path();
                let size = meta.len();
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or(0);
                let attrs = match cache.entries.get(&path) {
                    Some(cached) if cached.size == size && cached.modified == modified => {
                        cached.attrs.clone()
                    }
                    _ => audio_attrs(&path, &ext),
                };
                seen_cache.insert(
                    path.clone(),
                    CachedFile {
                        size,
                        modified,
                        attrs: attrs.clone(),
                    },
                );
                files.push(SharedFile {
                    virtual_path,
                    real_path: path,
                    size,
                    ext,
                    attrs,
                });
            }
        }
    }
    cache.entries = seen_cache;
    files
}

/// Attributes in the combinations SoulseekQt sends: bitrate and duration for lossy files, duration, sample rate and bit depth for lossless ones.
fn audio_attrs(path: &Path, ext: &str) -> Vec<(u32, u32)> {
    if !AUDIO.contains(&ext) {
        return Vec::new();
    }
    let Ok(probe) = Probe::open(path) else {
        return Vec::new();
    };
    let options = ParseOptions::new().read_tags(false).read_cover_art(false);
    let Ok(probe) = probe.options(options).guess_file_type() else {
        return Vec::new();
    };
    let Ok(file) = probe.read() else {
        return Vec::new();
    };
    let props = file.properties();
    let duration = props.duration().as_secs() as u32;
    if LOSSLESS.contains(&ext) {
        let mut attrs = vec![(ATTR_DURATION, duration)];
        if let Some(rate) = props.sample_rate() {
            attrs.push((ATTR_SAMPLE_RATE, rate));
        }
        if let Some(depth) = props.bit_depth() {
            attrs.push((ATTR_BIT_DEPTH, depth as u32));
        }
        attrs
    } else {
        let mut attrs = Vec::new();
        if let Some(bitrate) = props.audio_bitrate().or(props.overall_bitrate()) {
            attrs.push((ATTR_BITRATE, bitrate));
        }
        attrs.push((ATTR_DURATION, duration));
        attrs
    }
}

impl Engine {
    pub(crate) fn rescan(&mut self) {
        if self.scanning {
            self.rescan_again = true;
            return;
        }
        self.scanning = true;
        let roots = self.config.shared_dirs.clone();
        let cache_path = self.config.share_cache.clone();
        let inputs = self.inputs.clone();
        tokio::task::spawn_blocking(move || {
            let mut cache = cache_path
                .as_deref()
                .map(ScanCache::load)
                .unwrap_or_default();
            let files = scan(&roots, &mut cache);
            if let Some(path) = &cache_path {
                cache.save(path);
            }
            let index = ShareIndex::build(files);
            let _ = inputs.send(Input::Scanned(Arc::new(index)));
        });
    }

    pub(crate) fn on_scanned(&mut self, index: Arc<ShareIndex>) {
        self.scanning = false;
        self.shares = index;
        self.browse_frame = None;
        let (dirs, files) = self.share_counts();
        self.send_server(crate::proto::server::ServerRequest::SharedFoldersFiles { dirs, files });
        self.emit(Event::SharesScanned {
            dirs: dirs as usize,
            files: files as usize,
        });
        if std::mem::take(&mut self.rescan_again) {
            self.rescan();
        }
    }

    pub(crate) fn share_counts(&self) -> (u32, u32) {
        (
            self.shares.dir_count() as u32,
            self.shares.file_count() as u32,
        )
    }

    /// Answers a browse with a cached, already compressed reply; big shares are encoded off the engine task.
    pub(crate) fn answer_browse(&mut self, username: &str) {
        if let Some(frame) = &self.browse_frame {
            let frame = frame.clone();
            self.send_peer_frame(username, frame.to_vec());
            return;
        }
        let first = self.browse_waiting.is_empty();
        self.browse_waiting.push(username.to_string());
        if !first {
            return;
        }
        let index = self.shares.clone();
        let inputs = self.inputs.clone();
        tokio::task::spawn_blocking(move || {
            let list = SharedFileList {
                dirs: index.browse(),
                private_dirs: Vec::new(),
            };
            let frame = PeerMessage::SharedFileListResponse(list).encode();
            let _ = inputs.send(Input::BrowseFrame(Arc::new(frame)));
        });
    }

    pub(crate) fn on_browse_frame(&mut self, frame: Arc<Vec<u8>>) {
        self.browse_frame = Some(frame.clone());
        for username in std::mem::take(&mut self.browse_waiting) {
            self.send_peer_frame(&username, frame.to_vec());
        }
    }

    pub(crate) fn folder_contents(&self, _username: &str, folder: &str) -> Vec<Directory> {
        self.shares.folder(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, attrs: Vec<(u32, u32)>) -> SharedFile {
        SharedFile {
            virtual_path: path.into(),
            real_path: PathBuf::from(path),
            size: 10,
            ext: path.rsplit('.').next().unwrap().into(),
            attrs,
        }
    }

    fn index() -> ShareIndex {
        ShareIndex::build(vec![
            file(
                "Music\\Boards of Canada\\Geogaddi\\01 Ready Lets Go.flac",
                vec![],
            ),
            file(
                "Music\\Boards of Canada\\Geogaddi\\02 Music Is Math.flac",
                vec![],
            ),
            file("Music\\Boards of Canada\\Geogaddi\\cover.jpg", vec![]),
            file("Music\\Boards of Canada\\Live\\01 Intro.mp3", vec![]),
            file("Music\\Aphex Twin\\Syro\\01 minipops.mp3", vec![]),
        ])
    }

    #[test]
    fn matches_whole_words_in_any_order() {
        let index = index();
        let names = |query: &str| -> Vec<String> {
            index
                .search(query, &[], 100)
                .into_iter()
                .map(|file| file.virtual_path.clone())
                .collect()
        };
        assert_eq!(names("canada boards math").len(), 1);
        assert_eq!(names("boards canada").len(), 4);
        assert_eq!(names("boards canada -live").len(), 3);
        assert_eq!(names("canad").len(), 0);
        assert_eq!(names("boards *pops").len(), 0);
        assert_eq!(names("aphex *pops").len(), 1);
        assert_eq!(names("geogaddi").len(), 3);
        assert_eq!(names("x").len(), 0);
        assert_eq!(names("boards canada").len(), 4);
        assert_eq!(index.search("boards canada", &[], 2).len(), 2);
        assert_eq!(index.search("boards", &["geogaddi".into()], 100).len(), 1);
    }

    #[test]
    fn lists_folders_with_their_subfolders() {
        let index = index();
        assert_eq!(index.dir_count(), 3);
        let folder = index.folder("Music\\Boards of Canada");
        assert_eq!(folder.len(), 2);
        assert_eq!(folder[0].name, "Music\\Boards of Canada\\Geogaddi");
        assert_eq!(folder[0].files[0].name, "01 Ready Lets Go.flac");
        assert!(index.folder("Music\\Boards").is_empty());
        assert_eq!(index.browse().len(), 3);
    }

    #[test]
    fn scans_folders_and_reuses_cached_attributes() {
        let dir = std::env::temp_dir().join(format!("slsk-scan-{}", std::process::id()));
        let album = dir.join("Shared").join("Album");
        fs::create_dir_all(&album).unwrap();
        fs::write(album.join("a.txt"), b"hello").unwrap();
        fs::write(album.join("b.part"), b"partial").unwrap();
        let mut cache = ScanCache::default();
        let files = scan(&[dir.join("Shared"), dir.join("Shared")], &mut cache);
        assert_eq!(files.len(), 2);
        let mut paths: Vec<&str> = files.iter().map(|f| f.virtual_path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, ["Shared (2)\\Album\\a.txt", "Shared\\Album\\a.txt"]);
        assert_eq!(cache.entries.len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
