use std::time::{Duration, Instant};

use soulseek_rs::{Client, Recommendation, SimilarUser};

const WINDOW: Duration = Duration::from_secs(30);
const REFRESH: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemDetails {
    pub item: String,
    pub recommendations: Vec<Recommendation>,
    pub users: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Discovery {
    pub recommended: Vec<Recommendation>,
    pub unrecommended: Vec<Recommendation>,
    pub global: Vec<Recommendation>,
    pub similar: Vec<SimilarUser>,
    pub item: Option<ItemDetails>,
}

/// Interests and the recommendations built from them; replies carry no signal, so they are polled for a while.
#[derive(Default)]
pub struct Discover {
    likes: Vec<String>,
    dislikes: Vec<String>,
    item: Option<String>,
    until: Option<Instant>,
    checked: Option<Instant>,
    sent: Discovery,
}

impl Discover {
    pub fn set_interests(&mut self, likes: Vec<String>, dislikes: Vec<String>) {
        self.likes = likes;
        self.dislikes = dislikes;
    }

    /// The library keeps interests in memory only, so the saved ones are announced at login.
    pub fn announce(&self, client: &Client) {
        for like in &self.likes {
            let _ = client.add_interest(like);
        }
        for dislike in &self.dislikes {
            let _ = client.add_dislike(dislike);
        }
    }

    pub fn set_interest(&mut self, client: Option<&Client>, item: String, like: bool, add: bool) {
        let list = if like {
            &mut self.likes
        } else {
            &mut self.dislikes
        };
        list.retain(|existing| *existing != item);
        if add {
            list.push(item.clone());
        }
        if let Some(client) = client {
            let _ = match (like, add) {
                (true, true) => client.add_interest(&item),
                (true, false) => client.remove_interest(&item),
                (false, true) => client.add_dislike(&item),
                (false, false) => client.remove_dislike(&item),
            };
            self.refresh(client);
        }
    }

    pub fn refresh(&mut self, client: &Client) {
        let _ = client.request_recommendations();
        let _ = client.request_global_recommendations();
        let _ = client.request_similar_users();
        self.watch();
    }

    pub fn open_item(&mut self, client: &Client, item: String) {
        let _ = client.request_item_recommendations(&item);
        let _ = client.request_item_similar_users(&item);
        self.item = Some(item);
        self.watch();
    }

    fn watch(&mut self) {
        self.until = Some(Instant::now() + WINDOW);
        self.checked = None;
    }

    pub fn poll(&mut self, client: &Client) -> Option<Discovery> {
        let until = self.until?;
        if Instant::now() >= until {
            self.until = None;
        }
        if self.checked.is_some_and(|at| at.elapsed() < REFRESH) {
            return None;
        }
        self.checked = Some(Instant::now());
        let (recommended, unrecommended) = client.recommendations().unwrap_or_default();
        let next = Discovery {
            recommended,
            unrecommended,
            global: client
                .global_recommendations()
                .map(|(recommended, _)| recommended)
                .unwrap_or_default(),
            similar: client.similar_users(),
            item: self.item.as_ref().map(|item| ItemDetails {
                item: item.clone(),
                recommendations: client.item_recommendations(item),
                users: client.item_similar_users(item),
            }),
        };
        if next == self.sent {
            return None;
        }
        self.sent = next.clone();
        Some(next)
    }
}
