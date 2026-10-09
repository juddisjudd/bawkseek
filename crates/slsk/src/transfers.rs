use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::engine::Engine;
use crate::proto::peer::{Direction, PeerMessage};

/// How often a running transfer reports its progress.
pub(crate) const PROGRESS_EVERY: Duration = Duration::from_millis(250);
pub(crate) const CHUNK: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferState {
    /// Waiting in a queue: the peer's queue for downloads, ours for uploads.
    Queued {
        place: Option<u32>,
    },
    /// The transfer was agreed and the file connection is being opened.
    Connecting,
    Transferring {
        bytes: u64,
        speed: u64,
    },
    Paused {
        bytes: u64,
    },
    Done,
    Failed(String),
    Cancelled,
}

impl TransferState {
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            TransferState::Done | TransferState::Failed(_) | TransferState::Cancelled
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferUpdate {
    pub id: u64,
    pub username: String,
    pub filename: String,
    pub size: u64,
    pub state: TransferState,
    /// Where a download is or will be saved, or the shared file an upload reads from.
    pub path: Option<PathBuf>,
}

/// A token bucket shared by every transfer in one direction; a rate of 0 means no limit.
#[derive(Default)]
pub(crate) struct Limiter {
    rate: AtomicU64,
    bucket: Mutex<Option<(f64, Instant)>>,
}

impl Limiter {
    pub(crate) fn set_rate(&self, bytes_per_second: u64) {
        self.rate.store(bytes_per_second, Ordering::Relaxed);
    }

    pub(crate) async fn take(&self, bytes: usize) {
        let rate = self.rate.load(Ordering::Relaxed);
        if rate == 0 {
            return;
        }
        let rate = rate as f64;
        let mut bucket = self.bucket.lock().await;
        let now = Instant::now();
        let (tokens, last) = bucket.unwrap_or((rate, now));
        let tokens =
            (tokens + now.duration_since(last).as_secs_f64() * rate).min(rate) - bytes as f64;
        *bucket = Some((tokens, now));
        if tokens < 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(-tokens / rate)).await;
        }
    }
}

/// Bytes per second over a sliding window, for the speed shown while a transfer runs.
pub(crate) struct Meter {
    started: Instant,
    samples: Vec<(Instant, u64)>,
}

impl Meter {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            samples: Vec::new(),
        }
    }

    pub(crate) fn record(&mut self, bytes: u64) -> u64 {
        let now = Instant::now();
        self.samples.push((now, bytes));
        self.samples
            .retain(|(at, _)| now.duration_since(*at) <= Duration::from_secs(5));
        match (self.samples.first(), self.samples.last()) {
            (Some((first_at, first)), Some((last_at, last))) if last_at > first_at => {
                ((last - first) as f64 / last_at.duration_since(*first_at).as_secs_f64()) as u64
            }
            _ => 0,
        }
    }

    pub(crate) fn average(&self, bytes: u64) -> u64 {
        let seconds = self.started.elapsed().as_secs_f64();
        if seconds > 0.0 {
            (bytes as f64 / seconds) as u64
        } else {
            0
        }
    }
}

impl Engine {
    pub(crate) fn on_transfer_message(&mut self, username: &str, message: PeerMessage) {
        match message {
            PeerMessage::TransferRequest {
                direction: Direction::Upload,
                token,
                filename,
                size,
            } => self.on_upload_offer(username, token, filename, size),
            PeerMessage::TransferRequest {
                direction: Direction::Download,
                token,
                filename,
                ..
            } => self.on_legacy_download_request(username, token, filename),
            PeerMessage::TransferResponse {
                token,
                allowed,
                reason,
                ..
            } => self.on_offer_answer(username, token, allowed, reason),
            PeerMessage::QueueUpload(filename) => self.on_queue_upload(username, filename),
            PeerMessage::PlaceInQueueRequest(filename) => {
                self.on_place_request(username, &filename)
            }
            PeerMessage::PlaceInQueueResponse { filename, place } => {
                self.on_place_response(username, &filename, place)
            }
            PeerMessage::UploadDenied { filename, reason } => {
                self.on_upload_denied(username, &filename, reason)
            }
            PeerMessage::UploadFailed(filename) => {
                self.on_remote_upload_failed(username, &filename)
            }
            other => log::debug!("unhandled peer message {} from {username}", other.code()),
        }
    }

    pub(crate) fn transfers_unreachable(&mut self, username: &str) {
        self.downloads_unreachable(username);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn limits_the_rate() {
        let limiter = Limiter::default();
        limiter.set_rate(1000);
        let start = Instant::now();
        for _ in 0..3 {
            limiter.take(1000).await;
        }
        let elapsed = start.elapsed().as_secs_f64();
        assert!((1.9..2.2).contains(&elapsed), "{elapsed}");
        limiter.set_rate(0);
        let start = Instant::now();
        limiter.take(1_000_000).await;
        assert!(start.elapsed() < Duration::from_millis(1));
    }
}
