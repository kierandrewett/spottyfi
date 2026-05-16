//! Caching: a SQLite metadata cache and an on-disk image cache.
//!
//! ## Metadata cache
//!
//! [`MetadataCache`] is a SQLite store (`rusqlite`, bundled) for cacheable Web
//! API objects — tracks, albums, artists, playlists. Each row holds a
//! JSON-encoded `spottyfi_models` object and a `last_fetched` timestamp.
//! [`Freshness`] implements **stale-while-revalidate**: a cached object inside
//! the freshness window is [`Fresh`](Staleness::Fresh) and served as-is; past
//! it the object is [`Stale`](Staleness::Stale) — still served immediately, but
//! the caller should trigger a background refresh. The schema is versioned by
//! plain `.sql` migration files applied by the [`migrations`] runner.
//!
//! ## Image cache
//!
//! [`ImageCache`] is an on-disk cache for encoded album art / avatars, keyed by
//! `sha1(url).webp`. It is a size-capped LRU: the cap defaults to
//! [`image::DEFAULT_CAPACITY_BYTES`] (500 MB) and the least-recently-used files
//! are evicted when a write pushes the directory over cap.
//!
//! ## Blocking
//!
//! Both caches do blocking I/O (SQLite, the filesystem). Callers on the tokio
//! runtime must use `spawn_blocking`; the egui UI thread must never call them
//! directly — see `docs/threading.md`.
#![warn(missing_docs)]
// `unwrap`/`expect` are denied in library code but allowed in unit tests,
// per the workspace convention in `PLAN.md`.
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod error;
pub mod freshness;
pub mod image;
mod metadata;
pub mod migrations;
pub mod paths;

pub use crate::error::{CacheError, CacheResult};
pub use crate::freshness::{Freshness, Staleness};
pub use crate::image::ImageCache;
pub use crate::metadata::{Cached, Kind, MetadataCache};
