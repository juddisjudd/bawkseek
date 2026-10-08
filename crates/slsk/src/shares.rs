use crate::engine::Engine;
use crate::proto::peer::SharedFileList;
use crate::proto::types::Directory;

impl Engine {
    pub(crate) fn shared_file_list(&self, _username: &str) -> SharedFileList {
        SharedFileList::default()
    }

    pub(crate) fn folder_contents(&self, _username: &str, _folder: &str) -> Vec<Directory> {
        Vec::new()
    }
}
