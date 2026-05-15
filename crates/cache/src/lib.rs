//! Caching: a SQLite metadata cache and an on-disk image cache.
//!
//! Implements stale-while-revalidate for API responses and a size-capped LRU
//! image cache keyed by `sha1(url)`.
//!
//! Phase 0: placeholder. The caches arrive in Phase 9.
#![warn(missing_docs)]
