//! The persistent metadata layer: SQLite-backed stale-while-revalidate.
//!
//! [`MetadataLayer`] sits in front of the cacheable Web API GETs (`album`,
//! `artist`, `playlist`). It pairs a tiny in-memory hot cache ([`ObjectCache`])
//! with the persistent [`MetadataCache`] from the `cache` crate, which is the
//! source of truth:
//!
//! - **hot hit** — return immediately, no I/O;
//! - **persistent hit, fresh** — return immediately;
//! - **persistent hit, stale** — return the cached value immediately *and*
//!   signal the caller to refresh in the background;
//! - **miss** — the caller fetches from the network and stores the result.
//!
//! `rusqlite` is blocking, so every persistent-cache call is wrapped in
//! `tokio::task::spawn_blocking`; the egui UI thread is never blocked.

use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;
use spottyfi_cache::{Kind, MetadataCache};

use crate::cache::ObjectCache;

/// The outcome of a metadata-layer lookup.
pub enum Lookup<T> {
    /// A usable cached value. `stale` is `true` when the caller should kick
    /// off a background refresh while still using `value`.
    Hit {
        /// The cached object.
        value: T,
        /// Whether a background refresh should be triggered.
        stale: bool,
    },
    /// No usable cached value — the caller must fetch from the network.
    Miss,
}

/// The persistent metadata layer shared by the API client.
///
/// Cheap to clone: an in-memory cache handle plus an `Arc` to the SQLite store.
#[derive(Clone, Default)]
pub struct MetadataLayer {
    /// The in-memory hot cache (one process session).
    hot: ObjectCache,
    /// The persistent SQLite store; `None` when no cache directory is
    /// available (the layer then degrades to the hot cache only).
    persistent: Option<Arc<MetadataCache>>,
}

impl std::fmt::Debug for MetadataLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetadataLayer")
            .field("hot", &self.hot)
            .field("persistent", &self.persistent.is_some())
            .finish()
    }
}

impl MetadataLayer {
    /// Build a layer over an explicit persistent cache and hot cache.
    #[must_use]
    pub fn new(hot: ObjectCache, persistent: Arc<MetadataCache>) -> Self {
        Self {
            hot,
            persistent: Some(persistent),
        }
    }

    /// Build a hot-cache-only layer (no persistence) — for tests and for the
    /// degraded path when the cache directory cannot be resolved.
    #[must_use]
    pub fn in_memory_only() -> Self {
        Self {
            hot: ObjectCache::default(),
            persistent: None,
        }
    }

    /// Look up `id` of `kind`, decoded to `T`.
    ///
    /// Consults the hot cache first, then the persistent store off-thread. A
    /// hot hit is always treated as fresh (it was written this session). A
    /// persistent hit promotes the value into the hot cache.
    pub async fn get<T>(&self, kind: Kind, id: &str) -> Lookup<T>
    where
        T: Clone + Send + Sync + DeserializeOwned + 'static,
    {
        let hot_key = hot_key(kind, id);
        if let Some(value) = self.hot.get::<T>(&hot_key) {
            return Lookup::Hit {
                value,
                stale: false,
            };
        }
        let Some(persistent) = self.persistent.clone() else {
            return Lookup::Miss;
        };
        let id_owned = id.to_owned();
        let result = tokio::task::spawn_blocking(move || persistent.get::<T>(kind, &id_owned))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
        match result {
            Some(cached) => {
                self.hot.put(hot_key, cached.value.clone());
                Lookup::Hit {
                    value: cached.value,
                    stale: cached.staleness.should_revalidate(),
                }
            }
            None => Lookup::Miss,
        }
    }

    /// Store a freshly-fetched `value` for `id` in both cache layers.
    ///
    /// The persistent write happens off-thread; a write failure is logged but
    /// never surfaced — caching is best-effort.
    pub async fn put<T>(&self, kind: Kind, id: &str, value: &T)
    where
        T: Clone + Send + Sync + Serialize + 'static,
    {
        self.hot.put(hot_key(kind, id), value.clone());
        let Some(persistent) = self.persistent.clone() else {
            return;
        };
        let id_owned = id.to_owned();
        let value_owned = value.clone();
        let join =
            tokio::task::spawn_blocking(move || persistent.put(kind, &id_owned, &value_owned))
                .await;
        match join {
            Ok(Ok(())) => {}
            Ok(Err(err)) => tracing::warn!(%err, ?kind, id, "metadata cache write failed"),
            Err(err) => tracing::warn!(%err, "metadata cache write task panicked"),
        }
    }

    /// Drop every entry from both cache layers.
    pub fn clear(&self) {
        self.hot.clear();
        if let Some(persistent) = &self.persistent {
            if let Err(err) = persistent.clear() {
                tracing::warn!(%err, "metadata cache clear failed");
            }
        }
    }
}

/// Build the hot-cache key for an object — namespaced so kinds never collide.
fn hot_key(kind: Kind, id: &str) -> String {
    let prefix = match kind {
        Kind::Track => "track",
        Kind::Album => "album",
        Kind::Artist => "artist",
        Kind::Playlist => "playlist",
        Kind::PlaylistTracks => "playlist-tracks",
        Kind::Lyrics => "lyrics",
        Kind::Collection => "collection",
    };
    format!("{prefix}:{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_only_round_trips_through_the_hot_cache() {
        let layer = MetadataLayer::in_memory_only();
        layer.put(Kind::Artist, "a1", &"Bjork".to_owned()).await;
        match layer.get::<String>(Kind::Artist, "a1").await {
            Lookup::Hit { value, stale } => {
                assert_eq!(value, "Bjork");
                assert!(!stale, "a hot-cache hit is always fresh");
            }
            Lookup::Miss => panic!("expected a hit"),
        }
    }

    #[tokio::test]
    async fn caches_a_playlist_track_listing() {
        // The playlist-content cache stores the whole resolved listing under
        // `Kind::PlaylistTracks`; a revisit reads it back from the hot cache
        // with no network round-trip.
        let layer = MetadataLayer::in_memory_only();
        let listing = vec!["track-a".to_owned(), "track-b".to_owned()];
        layer
            .put(Kind::PlaylistTracks, "playlist-1", &listing)
            .await;
        match layer
            .get::<Vec<String>>(Kind::PlaylistTracks, "playlist-1")
            .await
        {
            Lookup::Hit { value, stale } => {
                assert_eq!(value, listing);
                assert!(!stale, "a hot-cache hit is fresh — no refetch");
            }
            Lookup::Miss => panic!("expected a cached playlist listing"),
        }
    }

    #[tokio::test]
    async fn an_unknown_id_is_a_miss() {
        let layer = MetadataLayer::in_memory_only();
        assert!(matches!(
            layer.get::<String>(Kind::Track, "nope").await,
            Lookup::Miss
        ));
    }

    #[tokio::test]
    async fn a_stale_persistent_row_is_a_hit_flagged_for_refresh() {
        use spottyfi_cache::Freshness;
        use std::time::Duration;

        // A zero-length freshness window makes any persisted row stale.
        let persistent = Arc::new(
            MetadataCache::open_with_freshness(
                tempfile_db(),
                Freshness::new(Duration::from_secs(0)),
            )
            .expect("open persistent cache"),
        );
        // Seed the persistent store directly, then build a layer with a *cold*
        // hot cache so the lookup must reach the persistent store.
        persistent
            .put(Kind::Album, "x", &"Kid A".to_owned())
            .expect("seed");
        // Age the row past the (zero) window.
        std::thread::sleep(Duration::from_millis(1100));
        let layer = MetadataLayer::new(ObjectCache::default(), persistent);

        match layer.get::<String>(Kind::Album, "x").await {
            Lookup::Hit { value, stale } => {
                assert_eq!(value, "Kid A");
                assert!(stale, "a row past the freshness window must be stale");
            }
            Lookup::Miss => panic!("expected a stale hit"),
        }
    }

    /// A throwaway DB path under the OS temp dir for the persistent-cache test.
    fn tempfile_db() -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        let unique = format!(
            "spottyfi-metadata-test-{}.sqlite",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        path.push(unique);
        path
    }
}
