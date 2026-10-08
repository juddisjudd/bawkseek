use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "bawkseek";
pub const DEFAULT_LISTEN_PORT: u16 = 2234;
pub const DEFAULT_UPLOAD_SLOTS: usize = 10;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub username: String,
    pub remember: bool,
    pub download_dir: PathBuf,
    pub listen_port: u16,
    pub light_theme: bool,
    pub shared_dirs: Vec<PathBuf>,
    pub upload_slots: usize,
    pub buddies: Vec<String>,
    pub ignored: Vec<String>,
    pub likes: Vec<String>,
    pub dislikes: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            username: String::new(),
            remember: true,
            download_dir: default_download_dir(),
            listen_port: DEFAULT_LISTEN_PORT,
            light_theme: false,
            shared_dirs: Vec::new(),
            upload_slots: DEFAULT_UPLOAD_SLOTS,
            buddies: Vec::new(),
            ignored: Vec::new(),
            likes: Vec::new(),
            dislikes: Vec::new(),
        }
    }
}

fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_default()
        .join("bawkseek")
}

pub fn data_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("bawkseek"))
}

/// A per-account JSON file under the data folder, with the username made safe as a file name.
pub fn account_file(kind: &str, owner: &str) -> Option<PathBuf> {
    let safe: String = owner
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(data_dir()?.join(kind).join(format!("{safe}.json")))
}

pub fn load_wishlist(owner: &str) -> Vec<String> {
    account_file("wishlist", owner)
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_wishlist(owner: &str, wishes: &[String]) {
    let Some(path) = account_file("wishlist", owner) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(wishes) {
        let _ = fs::write(path, json);
    }
}

fn config_path() -> Option<PathBuf> {
    Some(data_dir()?.join("config.json"))
}

impl Config {
    pub fn load() -> Self {
        config_path()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = config_path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        fs::write(path, json)
    }
}

pub fn saved_password(username: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, username)
        .ok()?
        .get_password()
        .ok()
}

pub fn store_password(username: &str, password: &str) {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, username) {
        let _ = entry.set_password(password);
    }
}

pub fn forget_password(username: &str) {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, username) {
        let _ = entry.delete_credential();
    }
}
