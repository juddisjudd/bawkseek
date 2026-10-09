use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use crate::client::{Event, next_token};
use crate::engine::{Engine, Input};
use crate::peers::Purpose;
use crate::proto::peer::{Direction, PeerMessage};
use crate::proto::server::ServerRequest;
use crate::proto::types::ConnectionType;
use crate::transfers::{CHUNK, Limiter, Meter, PROGRESS_EVERY, TransferState, TransferUpdate};

pub const DEFAULT_UPLOAD_SLOTS: usize = 10;
/// Files one user may have waiting in our queue at once.
const MAX_QUEUED_PER_USER: usize = 1000;
const OFFER_TIMEOUT: Duration = Duration::from_secs(60);
const OFFSET_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);
/// The downloader closes a finished file connection; this is how long we wait for that.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

struct Upload {
    username: String,
    filename: String,
    real_path: PathBuf,
    size: u64,
    state: TransferState,
    token: Option<u32>,
    offered_at: Option<Instant>,
    task: Option<AbortHandle>,
}

pub(crate) struct Uploads {
    items: BTreeMap<u64, Upload>,
    pub(crate) slots: usize,
    pub(crate) privileged: HashSet<String>,
    pub(crate) banned: HashSet<String>,
    pub(crate) limiter: Arc<Limiter>,
}

impl Default for Uploads {
    fn default() -> Self {
        Self {
            items: BTreeMap::new(),
            slots: DEFAULT_UPLOAD_SLOTS,
            privileged: HashSet::new(),
            banned: HashSet::new(),
            limiter: Arc::default(),
        }
    }
}

impl Uploads {
    fn active(&self) -> usize {
        self.items
            .values()
            .filter(|upload| {
                matches!(
                    upload.state,
                    TransferState::Connecting | TransferState::Transferring { .. }
                )
            })
            .count()
    }

    /// Queued uploads in the order they will start: privileged users first, then first come, first served.
    fn queue(&self) -> Vec<u64> {
        let mut queued: Vec<(bool, u64)> = self
            .items
            .iter()
            .filter(|(_, upload)| matches!(upload.state, TransferState::Queued { .. }))
            .map(|(id, upload)| (!self.privileged.contains(&upload.username), *id))
            .collect();
        queued.sort();
        queued.into_iter().map(|(_, id)| id).collect()
    }
}

impl Engine {
    fn emit_upload(&self, id: u64) {
        if let Some(upload) = self.uploads.items.get(&id) {
            self.emit(Event::Upload(TransferUpdate {
                id,
                username: upload.username.clone(),
                filename: upload.filename.clone(),
                size: upload.size,
                state: upload.state.clone(),
                path: Some(upload.real_path.clone()),
            }));
        }
    }

    /// Reports the new state; finished uploads are dropped right after, so the list never grows without bound.
    fn set_upload(&mut self, id: u64, state: TransferState) {
        let finished = state.is_finished();
        if let Some(upload) = self.uploads.items.get_mut(&id) {
            upload.state = state;
        }
        self.emit_upload(id);
        if finished {
            self.uploads.items.remove(&id);
        }
    }

    pub(crate) fn queued_uploads(&self) -> usize {
        self.uploads.queue().len()
    }

    pub(crate) fn free_upload_slots(&self) -> usize {
        self.uploads.slots.saturating_sub(self.uploads.active())
    }

    pub(crate) fn on_queue_upload(&mut self, username: &str, filename: String) {
        let deny = |reason: &str| PeerMessage::UploadDenied {
            filename: filename.clone(),
            reason: reason.to_string(),
        };
        if self.uploads.banned.contains(username) {
            self.send_peer(username, deny("Banned"));
            return;
        }
        let Some(file) = self.shares.get(&filename) else {
            self.send_peer(username, deny("File not shared."));
            return;
        };
        let (real_path, size) = (file.real_path.clone(), file.size);
        let mine: Vec<&Upload> = self
            .uploads
            .items
            .values()
            .filter(|upload| upload.username == username)
            .collect();
        if mine.iter().any(|upload| upload.filename == filename) {
            return;
        }
        if mine.len() >= MAX_QUEUED_PER_USER {
            self.send_peer(username, deny("Too many files"));
            return;
        }
        let id = self.next_id();
        self.uploads.items.insert(
            id,
            Upload {
                username: username.to_string(),
                filename,
                real_path,
                size,
                state: TransferState::Queued { place: None },
                token: None,
                offered_at: None,
                task: None,
            },
        );
        self.pump_uploads();
        self.renumber_queue();
    }

    /// Old clients ask for files with a download-direction transfer request; it is queued like any other.
    pub(crate) fn on_legacy_download_request(
        &mut self,
        username: &str,
        token: u32,
        filename: String,
    ) {
        self.send_peer(
            username,
            PeerMessage::TransferResponse {
                token,
                allowed: false,
                size: None,
                reason: Some("Queued".into()),
            },
        );
        self.on_queue_upload(username, filename);
    }

    fn renumber_queue(&mut self) {
        for (place, id) in self.uploads.queue().into_iter().enumerate() {
            let place = Some(place as u32 + 1);
            if let Some(upload) = self.uploads.items.get_mut(&id)
                && upload.state != (TransferState::Queued { place })
            {
                upload.state = TransferState::Queued { place };
                self.emit_upload(id);
            }
        }
    }

    pub(crate) fn pump_uploads(&mut self) {
        while self.uploads.active() < self.uploads.slots {
            let Some(id) = self.uploads.queue().first().copied() else {
                break;
            };
            let token = next_token(&self.tokens);
            let Some(upload) = self.uploads.items.get_mut(&id) else {
                break;
            };
            upload.token = Some(token);
            upload.offered_at = Some(Instant::now());
            let (username, filename, size) = (
                upload.username.clone(),
                upload.filename.clone(),
                upload.size,
            );
            self.set_upload(id, TransferState::Connecting);
            self.send_peer(
                &username,
                PeerMessage::TransferRequest {
                    direction: Direction::Upload,
                    token,
                    filename,
                    size: Some(size),
                },
            );
        }
    }

    pub(crate) fn on_offer_answer(
        &mut self,
        username: &str,
        token: u32,
        allowed: bool,
        reason: Option<String>,
    ) {
        let Some(id) = self
            .uploads
            .items
            .iter()
            .find(|(_, upload)| upload.username == username && upload.token == Some(token))
            .map(|(id, _)| *id)
        else {
            return;
        };
        if !allowed {
            let reason = reason.unwrap_or_else(|| "Cancelled".into());
            self.set_upload(id, TransferState::Failed(format!("refused: {reason}")));
            self.pump_uploads();
            return;
        }
        if let Some(upload) = self.uploads.items.get_mut(&id) {
            upload.offered_at = None;
        }
        self.open_peer(username, ConnectionType::File, Purpose::Upload(id));
    }

    pub(crate) fn upload_unreachable(&mut self, id: u64) {
        if self.uploads.items.contains_key(&id) {
            self.set_upload(
                id,
                TransferState::Failed("could not connect to the user".into()),
            );
            self.pump_uploads();
        }
    }

    pub(crate) fn upload_connected(&mut self, id: u64, stream: TcpStream) {
        let Some(upload) = self.uploads.items.get_mut(&id) else {
            return;
        };
        let Some(token) = upload.token else {
            return;
        };
        let job = Job {
            id,
            token,
            path: upload.real_path.clone(),
            size: upload.size,
            limiter: self.uploads.limiter.clone(),
            inputs: self.inputs.clone(),
        };
        upload.task = Some(tokio::spawn(job.run(stream)).abort_handle());
        self.set_upload(id, TransferState::Transferring { bytes: 0, speed: 0 });
    }

    pub(crate) fn on_upload_progress(&mut self, id: u64, bytes: u64, speed: u64) {
        let running = self
            .uploads
            .items
            .get(&id)
            .is_some_and(|upload| matches!(upload.state, TransferState::Transferring { .. }));
        if running {
            self.set_upload(id, TransferState::Transferring { bytes, speed });
        }
    }

    pub(crate) fn on_upload_finished(&mut self, id: u64, result: Result<u64, String>) {
        let Some(upload) = self.uploads.items.get(&id) else {
            return;
        };
        let (username, filename) = (upload.username.clone(), upload.filename.clone());
        match result {
            Ok(speed) => {
                self.send_server(ServerRequest::SendUploadSpeed(speed as u32));
                self.set_upload(id, TransferState::Done);
            }
            Err(error) => {
                self.send_peer(&username, PeerMessage::UploadFailed(filename));
                self.set_upload(id, TransferState::Failed(error));
            }
        }
        self.pump_uploads();
        self.renumber_queue();
    }

    pub(crate) fn cancel_upload(&mut self, id: u64) {
        let Some(upload) = self.uploads.items.get_mut(&id) else {
            return;
        };
        if let Some(task) = upload.task.take() {
            task.abort();
        }
        let (username, filename) = (upload.username.clone(), upload.filename.clone());
        self.send_peer(
            &username,
            PeerMessage::UploadDenied {
                filename,
                reason: "Cancelled".into(),
            },
        );
        self.set_upload(id, TransferState::Cancelled);
        self.pump_uploads();
        self.renumber_queue();
    }

    pub(crate) fn on_place_request(&mut self, username: &str, filename: &str) {
        let place = self.uploads.queue().into_iter().position(|id| {
            let upload = &self.uploads.items[&id];
            upload.username == username && upload.filename == filename
        });
        if let Some(place) = place {
            self.send_peer(
                username,
                PeerMessage::PlaceInQueueResponse {
                    filename: filename.to_string(),
                    place: place as u32 + 1,
                },
            );
        }
    }

    pub(crate) fn set_upload_slots(&mut self, slots: usize) {
        self.uploads.slots = slots.max(1);
        self.pump_uploads();
        self.renumber_queue();
    }

    pub(crate) fn tick_uploads(&mut self) {
        let late: Vec<u64> = self
            .uploads
            .items
            .iter()
            .filter(|(_, upload)| {
                upload
                    .offered_at
                    .is_some_and(|at| at.elapsed() > OFFER_TIMEOUT)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in late {
            self.set_upload(id, TransferState::Failed("the user did not answer".into()));
        }
        self.pump_uploads();
    }
}

struct Job {
    id: u64,
    token: u32,
    path: PathBuf,
    size: u64,
    limiter: Arc<Limiter>,
    inputs: UnboundedSender<Input>,
}

impl Job {
    async fn run(self, stream: TcpStream) {
        let (id, inputs) = (self.id, self.inputs.clone());
        let result = self.send(stream).await;
        let _ = inputs.send(Input::UploadFinished { id, result });
    }

    /// Returns the average speed in bytes per second.
    async fn send(self, mut stream: TcpStream) -> Result<u64, String> {
        let io = |err: std::io::Error| err.to_string();
        stream.write_u32_le(self.token).await.map_err(io)?;
        let offset = tokio::time::timeout(OFFSET_TIMEOUT, stream.read_u64_le())
            .await
            .map_err(|_| "the user did not start the transfer".to_string())?
            .map_err(io)?;
        if offset > self.size {
            return Err("the user asked for a bad offset".into());
        }
        let mut file = File::open(&self.path).await.map_err(io)?;
        file.seek(SeekFrom::Start(offset)).await.map_err(io)?;
        let mut buf = vec![0u8; CHUNK];
        let mut bytes = offset;
        let mut meter = Meter::new();
        let mut reported = Instant::now();
        while bytes < self.size {
            let want = CHUNK.min((self.size - bytes) as usize);
            let read = file.read(&mut buf[..want]).await.map_err(io)?;
            if read == 0 {
                return Err("the file got shorter".into());
            }
            self.limiter.take(read).await;
            tokio::time::timeout(WRITE_TIMEOUT, stream.write_all(&buf[..read]))
                .await
                .map_err(|_| "the transfer stalled".to_string())?
                .map_err(io)?;
            bytes += read as u64;
            if reported.elapsed() >= PROGRESS_EVERY {
                reported = Instant::now();
                let speed = meter.record(bytes);
                let _ = self.inputs.send(Input::UploadProgress {
                    id: self.id,
                    bytes,
                    speed,
                });
            }
        }
        let speed = meter.average(bytes - offset);
        let mut probe = [0u8; 1];
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, stream.read(&mut probe)).await;
        Ok(speed)
    }
}
