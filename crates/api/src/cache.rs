//! An in-memory LRU cache in front of cacheable Web API GETs.
//!
//! # Phase 3 seam
//!
//! This is deliberately minimal: a fixed-capacity [`lru::LruCache`] keyed by a
//! string. It exists so the client can short-circuit repeat fetches of stable
//! objects (an album, an artist) within a session.
//!
//! TODO(Phase 9): the real `cache` crate brings a SQLite metadata store with
//! stale-while-revalidate and TTLs. When it lands, this type becomes a thin
//! façade over it (or is dropped entirely). Keep the [`ObjectCache`] surface
//! small so that swap is cheap. Cache *invalidation* (e.g. on a library
//! mutation) is also deferred to Phase 9 — for now entries simply age out by
//! LRU eviction, which is acceptable for a single session.

use std::num::NonZeroUsize;
use std::sync::Mutex;

use lru::LruCache;

/// A typed, cloneable value stored in the cache.
///
/// The cache is heterogeneous (it holds albums, artists, …), so values are
/// boxed behind `Any`. Callers downcast on the way out.
type Entry = std::sync::Arc<dyn std::any::Any + Send + Sync>;

/// A small in-memory LRU cache for immutable-ish Spotify objects.
///
/// Cloneable and `Send + Sync`: the real API client holds one and shares it.
#[derive(Clone)]
pub struct ObjectCache {
    inner: std::sync::Arc<Mutex<LruCache<String, Entry>>>,
}

impl ObjectCache {
    /// Create a cache holding at most `capacity` entries.
    ///
    /// A `capacity` of zero is bumped to one; `LruCache` requires a non-zero
    /// capacity and a zero-capacity cache would be pointless anyway.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap_or(NonZeroUsize::MIN);
        Self {
            inner: std::sync::Arc::new(Mutex::new(LruCache::new(cap))),
        }
    }

    /// Fetch a cached value by key, downcast to `T`.
    ///
    /// Returns `None` on a miss, a type mismatch, or a poisoned lock (a
    /// poisoned cache lock is treated as an empty cache rather than a panic).
    #[must_use]
    pub fn get<T: Clone + Send + Sync + 'static>(&self, key: &str) -> Option<T> {
        let mut guard = self.inner.lock().ok()?;
        let entry = guard.get(key)?;
        entry.downcast_ref::<T>().cloned()
    }

    /// Insert (or replace) a value under `key`.
    ///
    /// A poisoned lock silently drops the write — the cache is best-effort.
    pub fn put<T: Send + Sync + 'static>(&self, key: impl Into<String>, value: T) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.put(key.into(), std::sync::Arc::new(value));
        }
    }

    /// Drop every cached entry.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.clear();
        }
    }
}

impl Default for ObjectCache {
    /// A cache sized for a typical browsing session.
    fn default() -> Self {
        Self::new(256)
    }
}

impl std::fmt::Debug for ObjectCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.inner.lock().map(|g| g.len()).unwrap_or(0);
        f.debug_struct("ObjectCache").field("len", &len).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_value() {
        let cache = ObjectCache::new(4);
        cache.put("k", String::from("v"));
        assert_eq!(cache.get::<String>("k"), Some("v".to_owned()));
        assert_eq!(cache.get::<String>("missing"), None);
    }

    #[test]
    fn wrong_type_is_a_miss_not_a_panic() {
        let cache = ObjectCache::new(4);
        cache.put("k", 42_u32);
        assert_eq!(cache.get::<String>("k"), None);
    }

    #[test]
    fn evicts_least_recently_used() {
        let cache = ObjectCache::new(2);
        cache.put("a", 1_u32);
        cache.put("b", 2_u32);
        // Touch "a" so "b" becomes the LRU entry.
        let _ = cache.get::<u32>("a");
        cache.put("c", 3_u32);
        assert_eq!(cache.get::<u32>("b"), None);
        assert_eq!(cache.get::<u32>("a"), Some(1));
        assert_eq!(cache.get::<u32>("c"), Some(3));
    }
}
