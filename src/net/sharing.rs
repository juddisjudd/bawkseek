use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use slsk::{TransferState, TransferUpdate};

use super::{UlState, UploadRow};
use crate::format;

/// Upload rows by id, kept after they finish until the user clears them.
#[derive(Default)]
pub struct Uploads {
    rows: BTreeMap<u64, UploadRow>,
    dirty: bool,
}

impl Uploads {
    pub fn apply(&mut self, update: TransferUpdate) {
        let row = to_row(update);
        self.rows.insert(row.id, row);
        self.dirty = true;
    }

    pub fn clear_finished(&mut self) {
        self.rows.retain(|_, row| {
            !matches!(
                row.state,
                UlState::Completed | UlState::Cancelled | UlState::Failed(_)
            )
        });
        self.dirty = true;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn take_changed(&mut self) -> Option<Arc<Vec<UploadRow>>> {
        if !std::mem::take(&mut self.dirty) {
            return None;
        }
        Some(Arc::new(self.rows.values().cloned().collect()))
    }
}

/// A failure reason the way the UI shows them: lowercase, without the trailing dot some clients add.
pub fn tidy_reason(reason: &str) -> String {
    reason.trim_end_matches('.').to_lowercase()
}

fn to_row(update: TransferUpdate) -> UploadRow {
    let (folder, name) = format::split_path(&update.filename);
    let (state, sent) = match update.state {
        TransferState::Queued { place } => (
            UlState::Queued {
                place: place.unwrap_or(0),
            },
            0,
        ),
        TransferState::Connecting => (UlState::Active { speed: 0 }, 0),
        TransferState::Transferring { bytes, speed } => (UlState::Active { speed }, bytes),
        TransferState::Paused { bytes } => (UlState::Active { speed: 0 }, bytes),
        TransferState::Done => (UlState::Completed, update.size),
        TransferState::Cancelled => (UlState::Cancelled, 0),
        TransferState::Failed(reason) => (UlState::Failed(tidy_reason(&reason)), 0),
    };
    UploadRow {
        id: update.id,
        folder: folder.to_string(),
        name: name.to_string(),
        username: update.username,
        filename: update.filename,
        size: update.size,
        sent,
        state,
    }
}

/// The names other users see for each shared folder.
pub fn virtual_roots(dirs: &[PathBuf]) -> Vec<Option<String>> {
    slsk::shares::virtual_roots(dirs)
}

/// Shared roots must not nest, or every file inside is indexed twice.
pub fn overlaps(dirs: &[PathBuf], candidate: &Path) -> bool {
    dirs.iter()
        .any(|dir| dir.starts_with(candidate) || candidate.starts_with(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(id: u64, state: TransferState) -> TransferUpdate {
        TransferUpdate {
            id,
            username: "ann".into(),
            filename: "Music\\a.flac".into(),
            size: 100,
            state,
            path: None,
        }
    }

    #[test]
    fn keeps_finished_rows_until_cleared() {
        let mut uploads = Uploads::default();
        uploads.apply(update(1, TransferState::Done));
        uploads.apply(update(
            2,
            TransferState::Transferring {
                bytes: 40,
                speed: 10,
            },
        ));
        uploads.apply(update(3, TransferState::Failed("Peer closed.".into())));
        let rows = uploads.take_changed().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "a.flac");
        assert_eq!(rows[0].folder, "Music");
        assert!((rows[1].progress() - 0.4).abs() < f32::EPSILON);
        assert_eq!(rows[2].state, UlState::Failed("peer closed".into()));
        assert!(uploads.take_changed().is_none());
        uploads.clear_finished();
        assert_eq!(uploads.take_changed().unwrap().len(), 1);
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
