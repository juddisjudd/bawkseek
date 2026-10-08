use std::time::{Duration, Instant};

use slsk::{Client, Config, Event, Session};

fn main() {
    let username = std::env::var("SLSK_USER").expect("set SLSK_USER");
    let password = std::env::var("SLSK_PASS").expect("set SLSK_PASS");
    let seconds: u64 = std::env::var("SLSK_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(15);
    let peer = std::env::var("SLSK_PEER").ok();
    let mut config = Config::new(username, password);
    if let Some(port) = std::env::var("SLSK_PORT").ok().and_then(|p| p.parse().ok()) {
        config.listen_port = port;
    }
    let mut client = Client::start(config).expect("start runtime");
    let until = Instant::now() + Duration::from_secs(seconds);
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
            }
            Event::Server(message) => println!("server: {}", summary(&format!("{message:?}"))),
            Event::Shares { username, list } => {
                println!("shares of {username}: {} folders", list.dirs.len());
                for dir in &list.dirs {
                    println!("  {} ({} files)", dir.name, dir.files.len());
                }
                if let (Some(peer), Some(dir)) = (&peer, list.dirs.first()) {
                    client.folder_contents(peer, &dir.name);
                }
            }
            other => println!("{}", summary(&format!("{other:?}"))),
        }
    }
}

fn summary(text: &str) -> String {
    text.chars().take(200).collect()
}
