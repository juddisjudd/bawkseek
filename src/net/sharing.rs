use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use soulseek_rs::{Client, UploadInfo, UploadStatus};

use super::{Event, NoticeLevel, ShareState, UlState, UploadRow};
use crate::format;

const UPLOADS_REFRESH: Duration = Duration::from_millis(500);

struct ScanRequest {
    client: Arc<Client>,
    dirs: Vec<String>,
    generation: u64,
}

/// Runs share scans one at a time on their own thread, so a big library never stalls the worker.
pub struct Scanner {
    requests: Sender<ScanRequest>,
    generation: Arc<AtomicU64>,
}

impl Scanner {
    pub fn spawn(events: Sender<Event>) -> Self {
        let (requests, queue) = mpsc::channel::<ScanRequest>();
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        thread::Builder::new()
            .name("soulseek-shares".into())
            .spawn(move || {
                let mut last = ShareState::default();
                while let Ok(mut request) = queue.recv() {
                    while let Ok(newer) = queue.try_recv() {
                        request = newer;
                    }
                    let stale = || request.generation != current.load(Ordering::Relaxed);
                    if stale() {
                        continue;
                    }
                    last.scanning = true;
                    let _ = events.send(Event::Shares(last));
                    let result = request.client.set_shared_directories(request.dirs.clone());
                    let (folders, files) = request.client.shared_counts();
                    if stale() {
                        continue;
                    }
                    last = ShareState {
                        scanning: false,
                        folders,
                        files,
                    };
                    let _ = events.send(Event::Shares(last));
                    if let Err(err) = result {
                        let _ = events.send(Event::Notice(
                            NoticeLevel::Warning,
                            format!("scanned your shares, but the server was not told: {err}"),
                        ));
                    }
                }
            })
            .expect("spawn share scanner");
        Self {
            requests,
            generation,
        }
    }

    pub fn scan(&self, client: Arc<Client>, dirs: &[PathBuf]) {
        let _ = self.requests.send(ScanRequest {
            client,
            dirs: dirs
                .iter()
                .map(|dir| dir.to_string_lossy().into_owned())
                .collect(),
            generation: self.generation.load(Ordering::Relaxed),
        });
    }

    /// Drops scans queued for a client that has gone away.
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

type Key = (String, String);

/// Turns the library's upload snapshots, which have no ids and never forget finished rows, into UI rows.
#[derive(Default)]
pub struct Uploads {
    sent: Arc<Vec<UploadRow>>,
    checked: Option<Instant>,
    cleared: HashMap<Key, usize>,
}

impl Uploads {
    pub fn poll(&mut self, client: &Client) -> Option<Arc<Vec<UploadRow>>> {
        if self
            .checked
            .is_some_and(|at| at.elapsed() < UPLOADS_REFRESH)
        {
            return None;
        }
        self.checked = Some(Instant::now());
        let rows = build_rows(client.uploads(), &self.cleared);
        if rows == *self.sent {
            return None;
        }
        self.sent = Arc::new(rows);
        Some(self.sent.clone())
    }

    pub fn clear_finished(&mut self, client: &Client) {
        self.cleared = finished_counts(&client.uploads());
        self.checked = None;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

fn is_finished(status: &UploadStatus) -> bool {
    matches!(
        status,
        UploadStatus::Completed | UploadStatus::Cancelled | UploadStatus::Failed(_)
    )
}

fn finished_counts(infos: &[UploadInfo]) -> HashMap<Key, usize> {
    let mut counts = HashMap::new();
    for info in infos.iter().filter(|info| is_finished(&info.status)) {
        *counts
            .entry((info.username.clone(), info.filename.clone()))
            .or_default() += 1;
    }
    counts
}

/// Finished rows only ever accumulate, so hiding the first `cleared[key]` of them hides exactly the cleared ones.
fn build_rows(infos: Vec<UploadInfo>, cleared: &HashMap<Key, usize>) -> Vec<UploadRow> {
    let mut seen: HashMap<Key, (usize, usize)> = HashMap::new();
    infos
        .into_iter()
        .filter_map(|info| {
            let key = (info.username.clone(), info.filename.clone());
            let finished = is_finished(&info.status);
            let (all, done) = seen.entry(key.clone()).or_default();
            *all += 1;
            if finished {
                *done += 1;
                if *done <= cleared.get(&key).copied().unwrap_or(0) {
                    return None;
                }
            }
            let mut hasher = DefaultHasher::new();
            (&key, *all).hash(&mut hasher);
            Some(to_row(info, hasher.finish()))
        })
        .collect()
}

fn to_row(info: UploadInfo, id: u64) -> UploadRow {
    let (folder, name) = format::split_path(&info.filename);
    let state = match info.status {
        UploadStatus::Queued(place) => UlState::Queued { place },
        UploadStatus::InProgress => UlState::Active {
            speed: info.speed_bytes_per_sec.max(0.0) as u64,
        },
        UploadStatus::Completed => UlState::Completed,
        UploadStatus::Cancelled => UlState::Cancelled,
        UploadStatus::Failed(reason) => {
            UlState::Failed(reason.trim_end_matches('.').to_lowercase())
        }
    };
    UploadRow {
        id,
        folder: folder.to_string(),
        name: name.to_string(),
        username: info.username,
        filename: info.filename,
        size: info.size,
        sent: info.bytes_sent,
        state,
    }
}

/// The names other users see for each shared folder, following the library's rules.
pub fn virtual_roots(dirs: &[PathBuf]) -> Vec<Option<String>> {
    let mut uses: HashMap<String, usize> = HashMap::new();
    dirs.iter()
        .map(|dir| {
            if !dir.is_dir() {
                return None;
            }
            let base = root_name(dir);
            let count = uses.entry(base.clone()).or_default();
            *count += 1;
            Some(if *count == 1 {
                base
            } else {
                format!("{base} ({count})")
            })
        })
        .collect()
}

fn root_name(dir: &Path) -> String {
    dir.file_name().map_or_else(
        || "shared".into(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Shared roots must not nest, or every file inside is indexed twice.
pub fn overlaps(dirs: &[PathBuf], candidate: &Path) -> bool {
    dirs.iter()
        .any(|dir| dir.starts_with(candidate) || candidate.starts_with(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(user: &str, file: &str, status: UploadStatus) -> UploadInfo {
        UploadInfo {
            username: user.into(),
            filename: file.into(),
            size: 100,
            bytes_sent: 40,
            status,
            speed_bytes_per_sec: 0.0,
        }
    }

    #[test]
    fn hides_only_cleared_finished_rows() {
        let before = vec![
            info("ann", "Music\\a.flac", UploadStatus::Completed),
            info("bob", "Music\\b.flac", UploadStatus::InProgress),
        ];
        let cleared = finished_counts(&before);

        let mut after = before.clone();
        after.push(info("ann", "Music\\a.flac", UploadStatus::Completed));
        after.push(info("cat", "Music\\c.flac", UploadStatus::Queued(1)));

        let rows = build_rows(after, &cleared);
        let names: Vec<_> = rows
            .iter()
            .map(|row| (row.username.as_str(), &row.state))
            .collect();
        assert_eq!(
            names,
            vec![
                ("bob", &UlState::Active { speed: 0 }),
                ("ann", &UlState::Completed),
                ("cat", &UlState::Queued { place: 1 }),
            ]
        );
    }

    #[test]
    fn gives_repeated_files_distinct_ids() {
        let rows = build_rows(
            vec![
                info("ann", "Music\\a.flac", UploadStatus::Completed),
                info("ann", "Music\\a.flac", UploadStatus::InProgress),
            ],
            &HashMap::new(),
        );
        assert_ne!(rows[0].id, rows[1].id);
        assert_eq!(rows[0].name, "a.flac");
        assert_eq!(rows[0].folder, "Music");
        assert!((rows[1].progress() - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn maps_failure_reasons() {
        let rows = build_rows(
            vec![info(
                "ann",
                "x.mp3",
                UploadStatus::Failed("Peer closed.".into()),
            )],
            &HashMap::new(),
        );
        assert_eq!(rows[0].state, UlState::Failed("peer closed".into()));
    }

    #[test]
    fn names_roots_like_the_library() {
        let base = std::env::temp_dir().join("bawkseek-roots-test");
        let a = base.join("one").join("Music");
        let b = base.join("two").join("Music");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let missing = base.join("gone");
        let names = virtual_roots(&[a.clone(), missing, b.clone()]);
        std::fs::remove_dir_all(&base).unwrap();
        assert_eq!(
            names,
            vec![Some("Music".into()), None, Some("Music (2)".into())]
        );
    }

    #[test]
    fn detects_nested_shares() {
        let dirs = vec![PathBuf::from(r"D:\Music")];
        assert!(overlaps(&dirs, Path::new(r"D:\Music\Rock")));
        assert!(overlaps(&dirs, Path::new(r"D:\")));
        assert!(!overlaps(&dirs, Path::new(r"D:\Musicals")));
        assert!(!overlaps(&dirs, Path::new(r"E:\Music")));
    }
}
