use tokio::net::TcpStream;

use crate::engine::Engine;
use crate::peers::ConnId;

impl Engine {
    pub(crate) fn child_connected(&mut self, _username: String, _stream: TcpStream) {}

    pub(crate) fn parent_connected(&mut self, _username: String, _stream: TcpStream) {}

    pub(crate) fn parent_failed(&mut self, _username: &str) {}

    pub(crate) fn on_distrib_frame(&mut self, _conn: ConnId, _code: u8, _body: Vec<u8>) {}

    pub(crate) fn distrib_closed(&mut self, _conn: ConnId, _username: &str) {}
}
