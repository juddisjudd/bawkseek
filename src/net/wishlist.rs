use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use soulseek_rs::{Client, SearchResult};

const FIRST_RUN: Duration = Duration::from_secs(20);

type Key = (String, String);

/// Saved searches the server re-runs on its own schedule; each run wipes the library's
/// previous results, so they are kept and merged here.
#[derive(Default)]
pub struct Wishlist {
    wishes: Vec<String>,
    kept: HashMap<String, Vec<SearchResult>>,
    seen: HashMap<String, HashSet<Key>>,
    next_run: Option<Instant>,
}

impl Wishlist {
    pub fn set(&mut self, wishes: Vec<String>) {
        self.wishes = wishes;
        self.next_run = Some(Instant::now() + FIRST_RUN);
    }

    pub fn add(&mut self, client: Option<&Client>, query: String) {
        if self.wishes.contains(&query) {
            return;
        }
        if let Some(client) = client {
            let current = client.get_search_results(&query);
            self.seen.insert(query.clone(), keys(&current));
        }
        self.wishes.push(query);
        if self.next_run.is_none() {
            self.next_run = Some(Instant::now() + FIRST_RUN);
        }
    }

    pub fn remove(&mut self, query: &str) {
        self.wishes.retain(|wish| wish != query);
        self.kept.remove(query);
        self.seen.remove(query);
    }

    pub fn is_wish(&self, query: &str) -> bool {
        self.wishes.iter().any(|wish| wish == query)
    }

    pub fn wishes(&self) -> &[String] {
        &self.wishes
    }

    /// Re-runs every wish once the server's interval has passed; returns true when it did.
    pub fn run_due(&mut self, client: &Client) -> bool {
        if self.wishes.is_empty() || self.next_run.is_none_or(|at| Instant::now() < at) {
            return false;
        }
        for wish in &self.wishes {
            let current = client.get_search_results(wish);
            let kept = self.kept.remove(wish).unwrap_or_default();
            self.kept.insert(wish.clone(), merge(kept, current));
            let _ = client.start_wishlist_search(wish);
        }
        self.next_run = Some(Instant::now() + client.wishlist_interval());
        true
    }

    /// Earlier runs' results plus the current run's, without repeating a user's file.
    pub fn view(&self, query: &str, current: Vec<SearchResult>) -> Vec<SearchResult> {
        let kept = self.kept.get(query).cloned().unwrap_or_default();
        merge(kept, current)
    }

    /// How many of `results` are new since the last call, remembering them; the first call only sets a baseline.
    pub fn fresh(&mut self, query: &str, results: &[SearchResult]) -> usize {
        let Some(seen) = self.seen.get_mut(query) else {
            self.seen.insert(query.to_string(), keys(results));
            return 0;
        };
        let mut fresh = 0;
        for key in keys(results) {
            if seen.insert(key) {
                fresh += 1;
            }
        }
        fresh
    }
}

fn keys(results: &[SearchResult]) -> HashSet<Key> {
    results
        .iter()
        .flat_map(|result| {
            result
                .files
                .iter()
                .map(|file| (result.username.clone(), file.name.clone()))
        })
        .collect()
}

fn merge(mut kept: Vec<SearchResult>, current: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut known = keys(&kept);
    for mut result in current {
        result
            .files
            .retain(|file| known.insert((result.username.clone(), file.name.clone())));
        if !result.files.is_empty() {
            kept.push(result);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use soulseek_rs::File;

    use super::*;

    fn result(user: &str, files: &[&str]) -> SearchResult {
        SearchResult {
            token: 1,
            files: files
                .iter()
                .map(|name| File {
                    username: user.into(),
                    name: (*name).into(),
                    size: 1,
                    attribs: HashMap::new(),
                })
                .collect(),
            slots: 1,
            speed: 1,
            username: user.into(),
        }
    }

    #[test]
    fn merges_runs_without_repeating_files() {
        let first = vec![result("ann", &["a\\1.mp3", "a\\2.mp3"])];
        let second = vec![
            result("ann", &["a\\2.mp3", "a\\3.mp3"]),
            result("bob", &["b\\1.mp3"]),
        ];
        let merged = merge(first, second);
        let files: usize = merged.iter().map(|result| result.files.len()).sum();
        assert_eq!(files, 4);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[1].files[0].name, "a\\3.mp3");
    }

    #[test]
    fn counts_only_new_results() {
        let mut wishlist = Wishlist::default();
        assert_eq!(wishlist.fresh("q", &[result("ann", &["1", "2"])]), 0);
        assert_eq!(wishlist.fresh("q", &[result("ann", &["2", "3"])]), 1);
        assert_eq!(wishlist.fresh("q", &[result("bob", &["2"])]), 1);
    }
}
