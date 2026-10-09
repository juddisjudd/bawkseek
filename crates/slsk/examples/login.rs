use std::path::PathBuf;
use std::time::{Duration, Instant};

use slsk::{Client, Config, Event, SearchScope, Session};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn main() {
    env_logger::init();
    let mut config = Config::new(env("SLSK_USER").unwrap(), env("SLSK_PASS").unwrap());
    if let Some(port) = env("SLSK_PORT").and_then(|p| p.parse().ok()) {
        config.listen_port = port;
    }
    if let Some(share) = env("SLSK_SHARE") {
        config.shared_dirs = vec![PathBuf::from(share)];
    }
    let seconds: u64 = env("SLSK_SECONDS")
        .and_then(|s| s.parse().ok())
        .unwrap_or(15);
    let peer = env("SLSK_PEER");
    let query = env("SLSK_SEARCH");
    let mut client = Client::start(config).expect("start runtime");
    let until = Instant::now() + Duration::from_secs(seconds);
    let (mut replies, mut files) = (0, 0);
    while Instant::now() < until {
        let Some(event) = client.wait_event(Duration::from_millis(500)) else {
            continue;
        };
        match &event {
            Event::Session(Session::LoggedIn { .. }) => {
                println!("{event:?}");
                if let Some(peer) = &peer {
                    client.user_info(peer);
                    client.browse(peer);
                }
                if let Some(query) = &query {
                    let token = client.search(SearchScope::Network, query);
                    println!("searching {query:?} as {token}");
                }
            }
            Event::Server(_) => {}
            Event::SearchReply(reply) => {
                replies += 1;
                files += reply.files.len();
                if replies <= 3 {
                    println!(
                        "reply from {}: {} files, first {:?}",
                        reply.username,
                        reply.files.len(),
                        reply.files.first().map(|f| &f.name)
                    );
                }
            }
            Event::Shares { username, list } => {
                println!("shares of {username}: {} folders", list.dirs.len());
                for dir in &list.dirs {
                    println!("  {} ({} files)", dir.name, dir.files.len());
                }
            }
            other => println!("{}", summary(&format!("{other:?}"))),
        }
    }
    if query.is_some() {
        println!("{replies} replies, {files} files");
    }
}

fn summary(text: &str) -> String {
    text.chars().take(220).collect()
}
