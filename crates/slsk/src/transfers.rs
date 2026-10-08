use tokio::net::TcpStream;

use crate::engine::Engine;
use crate::proto::peer::PeerMessage;

impl Engine {
    pub(crate) fn on_transfer_message(&mut self, username: &str, message: PeerMessage) {
        log::debug!("transfer message from {username}: {}", message.code());
    }

    pub(crate) fn transfers_unreachable(&mut self, _username: &str) {}

    pub(crate) fn upload_unreachable(&mut self, _id: u64) {}

    pub(crate) fn upload_connected(&mut self, _id: u64, _stream: TcpStream) {}

    pub(crate) fn on_file_incoming(&mut self, _username: String, _token: u32, _stream: TcpStream) {}

    pub(crate) fn queued_uploads(&self) -> usize {
        0
    }

    pub(crate) fn free_upload_slots(&self) -> usize {
        1
    }
}
