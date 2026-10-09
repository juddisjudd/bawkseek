use std::collections::HashMap;
use std::time::{Duration, Instant};

const FIRST_RUN: Duration = Duration::from_secs(20);
const BASELINE: Duration = Duration::from_secs(60);
const DEFAULT_INTERVAL: Duration = Duration::from_secs(12 * 60);

/// Saved searches the server lets us run again on its schedule; every run adds to the same tab.
#[derive(Default)]
pub struct Wishlist {
    wishes: Vec<String>,
    quiet_until: HashMap<String, Instant>,
    next_run: Option<Instant>,
    interval: Option<Duration>,
}

impl Wishlist {
    pub fn set(&mut self, wishes: Vec<String>) {
        self.wishes = wishes;
        self.next_run = Some(Instant::now() + FIRST_RUN);
    }

    pub fn add(&mut self, query: String) {
        if self.wishes.contains(&query) {
            return;
        }
        self.quiet_until
            .insert(query.clone(), Instant::now() + BASELINE);
        self.wishes.push(query);
        if self.next_run.is_none() {
            self.next_run = Some(Instant::now() + FIRST_RUN);
        }
    }

    pub fn remove(&mut self, query: &str) {
        self.wishes.retain(|wish| wish != query);
        self.quiet_until.remove(query);
    }

    pub fn is_wish(&self, query: &str) -> bool {
        self.wishes.iter().any(|wish| wish == query)
    }

    pub fn set_interval(&mut self, seconds: u32) {
        self.interval = Some(Duration::from_secs(u64::from(seconds.max(60))));
    }

    /// The wishes to search again, once the server's interval has passed.
    pub fn due(&mut self) -> Vec<String> {
        if self.wishes.is_empty() || self.next_run.is_none_or(|at| Instant::now() < at) {
            return Vec::new();
        }
        self.next_run = Some(Instant::now() + self.interval.unwrap_or(DEFAULT_INTERVAL));
        self.wishes.clone()
    }

    pub fn fresh(&mut self, query: &str, added: usize) -> usize {
        self.fresh_at(query, added, Instant::now())
    }

    /// How many of `added` new files to announce; results trickle in for a while, so the first minute only builds a baseline.
    fn fresh_at(&mut self, query: &str, added: usize, now: Instant) -> usize {
        if added == 0 && !self.quiet_until.contains_key(query) {
            return 0;
        }
        let quiet_until = *self
            .quiet_until
            .entry(query.to_string())
            .or_insert(now + BASELINE);
        if now < quiet_until { 0 } else { added }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_only_new_results_after_the_baseline() {
        let mut wishlist = Wishlist::default();
        let start = Instant::now();
        let later = |secs| start + Duration::from_secs(secs);
        assert_eq!(wishlist.fresh_at("q", 0, start), 0);
        assert_eq!(wishlist.fresh_at("q", 1, later(10)), 0);
        assert_eq!(wishlist.fresh_at("q", 1, later(40)), 0);
        assert_eq!(wishlist.fresh_at("q", 1, later(75)), 1);
        assert_eq!(wishlist.fresh_at("q", 2, later(80)), 2);
    }

    #[test]
    fn reruns_on_the_servers_interval() {
        let mut wishlist = Wishlist::default();
        assert!(wishlist.due().is_empty());
        wishlist.set(vec!["a".into()]);
        wishlist.next_run = Some(Instant::now());
        wishlist.set_interval(120);
        assert_eq!(wishlist.due(), vec!["a".to_string()]);
        assert!(wishlist.due().is_empty());
    }
}
