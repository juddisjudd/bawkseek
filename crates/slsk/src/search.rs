use crate::client::Event;
use crate::engine::Engine;
use crate::proto::peer::SearchReply;

impl Engine {
    pub(crate) fn on_search_reply(&mut self, _username: &str, reply: SearchReply) {
        self.emit(Event::SearchReply(reply));
    }
}
