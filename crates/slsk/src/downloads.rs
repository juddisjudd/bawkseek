use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use crate::client::Event;
use crate::engine::{Engine, Input};
use crate::proto::peer::PeerMessage;
use crate::transfers::{CHUNK, Limiter, Meter, PROGRESS_EVERY, TransferState, TransferUpdate};

/// How long the uploader has to open the file connection after we accepted its offer.
const AGREED_TIMEOUT: Duration = Duration::from_secs(120);
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
const PLACE_EVERY: Duration = Duration::from_secs(5 * 60);

pub(crate) struct Download {
    username: String,
    filename: String,
    size: u64,
    dest: PathBuf,
    saved: Option<PathBuf>,
    state: TransferState,
    token: Option<u32>,
    agreed_at: Option<Instant>,
    task: Option<AbortHandle>,
}

#[derive(Default)]
pub(crate) struct Downloads {
    items: HashMap<u64, Download>,
    place_checked: Option<Instant>,
    pub(crate) limiter: Arc<Limiter>,
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

/// The destination itself, or the first "name (n).ext" beside it that is free.
fn free_path(dest: &Path) -> PathBuf {
    if !dest.exists() {
        return dest.to_path_buf();
    }
    let stem = dest.file_stem().unwrap_or_default().to_string_lossy();
    let ext = dest
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    (1..)
        .map(|n| dest.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|path| !path.exists())
        .expect("some name is free")
}

impl Engine {
    fn emit_download(&self, id: u64) {
        if let Some(download) = self.downloads.items.get(&id) {
            self.emit(Event::Download(TransferUpdate {
                id,
                username: download.username.clone(),
                filename: download.filename.clone(),
                size: download.size,
                state: download.state.clone(),
                path: Some(
                    download
                        .saved
                        .clone()
                        .unwrap_or_else(|| download.dest.clone()),
                ),
            }));
        }
    }

    fn set_download(&mut self, id: u64, state: TransferState) {
        if let Some(download) = self.downloads.items.get_mut(&id) {
            download.state = state;
        }
        self.emit_download(id);
    }

    fn find_download(&self, username: &str, filename: &str) -> Option<u64> {
        self.downloads
            .items
            .iter()
            .find(|(_, download)| {
                download.username == username
                    && download.filename == filename
                    && !download.state.is_finished()
            })
            .map(|(id, _)| *id)
    }

    pub(crate) fn add_download(
        &mut self,
        id: u64,
        username: String,
        filename: String,
        size: u64,
        dest: PathBuf,
    ) {
        let duplicate = self.find_download(&username, &filename).is_some();
        self.downloads.items.insert(
            id,
            Download {
                username: username.clone(),
                filename: filename.clone(),
                size,
                dest,
                saved: None,
                state: TransferState::Queued { place: None },
                token: None,
                agreed_at: None,
                task: None,
            },
        );
        if duplicate {
            self.set_download(
                id,
                TransferState::Failed("already in the download list".into()),
            );
            return;
        }
        self.emit_download(id);
        self.send_peer(&username, PeerMessage::QueueUpload(filename));
    }

    fn stop_download_task(&mut self, id: u64) {
        if let Some(task) = self
            .downloads
            .items
            .get_mut(&id)
            .and_then(|download| download.task.take())
        {
            task.abort();
        }
    }

    pub(crate) fn pause_download(&mut self, id: u64) {
        let Some(download) = self.downloads.items.get(&id) else {
            return;
        };
        let bytes = match download.state {
            TransferState::Transferring { bytes, .. } => bytes,
            TransferState::Queued { .. } | TransferState::Connecting => 0,
            _ => return,
        };
        self.stop_download_task(id);
        self.set_download(id, TransferState::Paused { bytes });
    }

    /// Asks the peer to queue the file again; a partial file is continued where it stopped.
    pub(crate) fn resume_download(&mut self, id: u64) {
        let Some(download) = self.downloads.items.get_mut(&id) else {
            return;
        };
        if !matches!(
            download.state,
            TransferState::Paused { .. } | TransferState::Failed(_) | TransferState::Cancelled
        ) {
            return;
        }
        download.token = None;
        download.agreed_at = None;
        let (username, filename) = (download.username.clone(), download.filename.clone());
        if self.downloads.items.iter().any(|(other, d)| {
            *other != id
                && d.username == username
                && d.filename == filename
                && !d.state.is_finished()
                && !matches!(d.state, TransferState::Paused { .. })
        }) {
            return;
        }
        self.set_download(id, TransferState::Queued { place: None });
        self.send_peer(&username, PeerMessage::QueueUpload(filename));
    }

    pub(crate) fn cancel_download(&mut self, id: u64) {
        let Some(download) = self.downloads.items.get(&id) else {
            return;
        };
        if matches!(
            download.state,
            TransferState::Done | TransferState::Cancelled
        ) {
            return;
        }
        let part = part_path(&download.dest);
        self.stop_download_task(id);
        tokio::spawn(async move {
            let _ = fs::remove_file(part).await;
        });
        self.set_download(id, TransferState::Cancelled);
    }

    pub(crate) fn remove_download(&mut self, id: u64) {
        self.cancel_download(id);
        self.downloads.items.remove(&id);
    }

    /// The uploader is ready to send one of our queued files.
    pub(crate) fn on_upload_offer(
        &mut self,
        username: &str,
        token: u32,
        filename: String,
        size: Option<u64>,
    ) {
        let found = self
            .downloads
            .items
            .iter()
            .filter(|(_, d)| d.username == username && d.filename == filename)
            .find(|(_, d)| !matches!(d.state, TransferState::Done | TransferState::Cancelled))
            .map(|(id, d)| (*id, d.state.clone()));
        let reply = |allowed: bool, reason: Option<&str>| PeerMessage::TransferResponse {
            token,
            allowed,
            size: None,
            reason: reason.map(str::to_string),
        };
        let Some((id, state)) = found else {
            self.send_peer(username, reply(false, Some("Cancelled")));
            return;
        };
        if matches!(state, TransferState::Paused { .. }) {
            self.send_peer(username, reply(false, Some("Queued")));
            return;
        }
        if let Some(download) = self.downloads.items.get_mut(&id) {
            download.token = Some(token);
            download.agreed_at = Some(Instant::now());
            if let Some(size) = size {
                download.size = size;
            }
        }
        self.set_download(id, TransferState::Connecting);
        self.send_peer(username, reply(true, None));
    }

    /// A file connection from an uploader, carrying the token of an offer we accepted.
    pub(crate) fn on_file_incoming(&mut self, username: String, token: u32, stream: TcpStream) {
        let Some((id, download)) = self.downloads.items.iter_mut().find(|(_, d)| {
            d.username == username && d.token == Some(token) && d.state == TransferState::Connecting
        }) else {
            log::debug!("file connection from {username} with unknown token {token}");
            return;
        };
        let id = *id;
        let job = Job {
            id,
            part: part_path(&download.dest),
            dest: download.dest.clone(),
            size: download.size,
            limiter: self.downloads.limiter.clone(),
            inputs: self.inputs.clone(),
        };
        download.task = Some(tokio::spawn(job.run(stream)).abort_handle());
        self.set_download(id, TransferState::Transferring { bytes: 0, speed: 0 });
    }

    pub(crate) fn on_download_progress(&mut self, id: u64, bytes: u64, speed: u64) {
        let running = self
            .downloads
            .items
            .get(&id)
            .is_some_and(|d| matches!(d.state, TransferState::Transferring { .. }));
        if running {
            self.set_download(id, TransferState::Transferring { bytes, speed });
        }
    }

    pub(crate) fn on_download_finished(&mut self, id: u64, result: Result<PathBuf, String>) {
        let Some(download) = self.downloads.items.get_mut(&id) else {
            return;
        };
        download.task = None;
        if !matches!(download.state, TransferState::Transferring { .. }) {
            return;
        }
        match result {
            Ok(path) => {
                download.saved = Some(path);
                self.set_download(id, TransferState::Done);
            }
            Err(error) => self.set_download(id, TransferState::Failed(error)),
        }
    }

    pub(crate) fn downloads_unreachable(&mut self, username: &str) {
        let ids: Vec<u64> = self
            .downloads
            .items
            .iter()
            .filter(|(_, d)| {
                d.username == username && matches!(d.state, TransferState::Queued { place: None })
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.set_download(
                id,
                TransferState::Failed("could not connect to the user".into()),
            );
        }
    }

    pub(crate) fn on_place_response(&mut self, username: &str, filename: &str, place: u32) {
        if let Some(id) = self.find_download(username, filename)
            && matches!(
                self.downloads.items[&id].state,
                TransferState::Queued { .. }
            )
        {
            self.set_download(id, TransferState::Queued { place: Some(place) });
        }
    }

    pub(crate) fn on_upload_denied(&mut self, username: &str, filename: &str, reason: String) {
        if reason == "Queued" {
            return;
        }
        if let Some(id) = self.find_download(username, filename)
            && !matches!(
                self.downloads.items[&id].state,
                TransferState::Paused { .. }
            )
        {
            self.stop_download_task(id);
            self.set_download(id, TransferState::Failed(reason));
        }
    }

    /// The uploader's side of a transfer broke; the file goes back into its queue.
    pub(crate) fn on_remote_upload_failed(&mut self, username: &str, filename: &str) {
        let Some(id) = self.find_download(username, filename) else {
            return;
        };
        if matches!(
            self.downloads.items[&id].state,
            TransferState::Paused { .. }
        ) {
            return;
        }
        self.stop_download_task(id);
        if let Some(download) = self.downloads.items.get_mut(&id) {
            download.token = None;
            download.agreed_at = None;
        }
        self.set_download(id, TransferState::Queued { place: None });
        self.send_peer(username, PeerMessage::QueueUpload(filename.to_string()));
    }

    pub(crate) fn tick_downloads(&mut self) {
        let late: Vec<u64> = self
            .downloads
            .items
            .iter()
            .filter(|(_, d)| {
                d.state == TransferState::Connecting
                    && d.agreed_at.is_some_and(|at| at.elapsed() > AGREED_TIMEOUT)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in late {
            if let Some(download) = self.downloads.items.get(&id) {
                let (username, filename) = (download.username.clone(), download.filename.clone());
                self.on_remote_upload_failed(&username, &filename);
            }
        }
        if self
            .downloads
            .place_checked
            .is_some_and(|at| at.elapsed() < PLACE_EVERY)
        {
            return;
        }
        self.downloads.place_checked = Some(Instant::now());
        let queued: Vec<(String, String)> = self
            .downloads
            .items
            .values()
            .filter(|d| matches!(d.state, TransferState::Queued { place: Some(_) }))
            .map(|d| (d.username.clone(), d.filename.clone()))
            .collect();
        for (username, filename) in queued {
            self.send_peer(&username, PeerMessage::PlaceInQueueRequest(filename));
        }
    }
}

struct Job {
    id: u64,
    part: PathBuf,
    dest: PathBuf,
    size: u64,
    limiter: Arc<Limiter>,
    inputs: UnboundedSender<Input>,
}

impl Job {
    async fn run(self, stream: TcpStream) {
        let (id, inputs) = (self.id, self.inputs.clone());
        let result = self.receive(stream).await;
        let _ = inputs.send(Input::DownloadFinished { id, result });
    }

    async fn receive(self, mut stream: TcpStream) -> Result<PathBuf, String> {
        let io = |err: std::io::Error| err.to_string();
        if let Some(dir) = self.part.parent() {
            fs::create_dir_all(dir).await.map_err(io)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&self.part)
            .await
            .map_err(io)?;
        let mut bytes = file.metadata().await.map_err(io)?.len();
        if bytes > self.size {
            file.set_len(0).await.map_err(io)?;
            bytes = 0;
        }
        file.seek(SeekFrom::Start(bytes)).await.map_err(io)?;
        stream.write_u64_le(bytes).await.map_err(io)?;
        let mut buf = vec![0u8; CHUNK];
        let mut meter = Meter::new();
        let mut reported = Instant::now();
        while bytes < self.size {
            let want = CHUNK.min((self.size - bytes) as usize);
            self.limiter.take(want).await;
            let read = tokio::time::timeout(STALL_TIMEOUT, stream.read(&mut buf[..want]))
                .await
                .map_err(|_| "the transfer stalled".to_string())?
                .map_err(io)?;
            if read == 0 {
                return Err("the user closed the connection".into());
            }
            file.write_all(&buf[..read]).await.map_err(io)?;
            bytes += read as u64;
            if reported.elapsed() >= PROGRESS_EVERY {
                reported = Instant::now();
                let speed = meter.record(bytes);
                let _ = self.inputs.send(Input::DownloadProgress {
                    id: self.id,
                    bytes,
                    speed,
                });
            }
        }
        file.flush().await.map_err(io)?;
        drop(file);
        drop(stream);
        let dest = free_path(&self.dest);
        fs::rename(&self.part, &dest).await.map_err(io)?;
        Ok(dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_a_free_name() {
        let dir = std::env::temp_dir().join(format!("slsk-free-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("song.mp3");
        assert_eq!(free_path(&dest), dest);
        std::fs::write(&dest, b"x").unwrap();
        assert_eq!(free_path(&dest), dir.join("song (1).mp3"));
        assert_eq!(part_path(&dest), dir.join("song.mp3.part"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
