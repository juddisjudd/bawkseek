use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use gpui_kit::*;

/// Keeps the most recently drawn covers and unloads the rest, so scrolling a large library does not grow memory.
pub struct CoverCache {
    items: HashMap<u64, (ImageCacheItem, u64)>,
    clock: u64,
    capacity: usize,
}

impl CoverCache {
    pub fn new(capacity: usize, cx: &mut App) -> Entity<Self> {
        let cache = cx.new(|_| Self {
            items: HashMap::new(),
            clock: 0,
            capacity,
        });
        cx.observe_release(&cache, |cache, cx| {
            for (_, (item, _)) in cache.items.drain() {
                if let Some(Ok(image)) = item.get() {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        cache
    }

    fn evict(&mut self, window: &mut Window, cx: &mut App) {
        while self.items.len() > self.capacity {
            let Some(oldest) = self
                .items
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key)
            else {
                return;
            };
            if let Some((item, _)) = self.items.remove(&oldest)
                && let Some(Ok(image)) = item.get()
            {
                cx.drop_image(image, Some(window));
            }
        }
    }
}

impl ImageCache for CoverCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let mut hasher = DefaultHasher::new();
        resource.hash(&mut hasher);
        let key = hasher.finish();
        self.clock += 1;
        let clock = self.clock;
        if let std::collections::hash_map::Entry::Vacant(slot) = self.items.entry(key) {
            slot.insert((ImageCacheItem::new(resource, cx), clock));
            self.evict(window, cx);
        }
        let (item, used) = self.items.get_mut(&key)?;
        *used = clock;
        item.use_image(window)
    }
}
