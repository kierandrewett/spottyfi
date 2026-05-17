//! Persistent caching of fetched lyrics.
//!
//! A lyrics lookup is a network round-trip — sometimes several, when a
//! provider has to search and score candidates. Caching the result means
//! revisiting a track (or replaying it later in a session) renders its lyrics
//! straight from the SQLite store with no refetch.
//!
//! The cache sits over the `cache` crate's [`MetadataCache`], using its
//! [`Kind::Lyrics`] table. Each row is a [`CachedLyrics`] record keyed by the
//! Spotify track id.
//!
//! ## Misses are cached too
//!
//! A "no lyrics found" result is itself cached — as
//! [`CachedOutcome::NotFound`] — so a track with no lyrics is not re-searched
//! on every single visit. Misses are kept on a **much shorter** TTL than hits
//! ([`MISS_TTL`] vs [`HIT_TTL`]): catalogue lyrics rarely appear, but they do
//! get added, so a miss is allowed to lapse and re-check before long while a
//! found set of lyrics is treated as effectively permanent.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use spottyfi_cache::{Kind, MetadataCache};

use super::Lyrics;

/// How long a cached *hit* (real lyrics) stays valid.
///
/// Lyrics for a recording do not change, so this is long — a found set is
/// effectively permanent for the lifetime of the on-disk cache.
pub const HIT_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 30);

/// How long a cached *miss* ("no lyrics found") stays valid.
///
/// Far shorter than [`HIT_TTL`]: lyrics do get added to the catalogue, so a
/// miss is re-checked against the providers after a few hours rather than
/// being trusted for a month.
pub const MISS_TTL: Duration = Duration::from_secs(60 * 60 * 6);

/// The outcome of a lyrics fetch, as stored in the cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CachedOutcome {
    /// Lyrics were found.
    Found(Lyrics),
    /// No provider had lyrics for the track — a cached miss.
    NotFound,
}

/// One cached lyrics record: the outcome, its provider and when it was fetched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedLyrics {
    /// The fetch outcome — real lyrics, or a cached miss.
    pub outcome: CachedOutcome,
    /// The provider the lyrics came from (`"lrclib"`, `"musixmatch"`, …).
    ///
    /// Empty for a [`CachedOutcome::NotFound`] miss (no provider produced it).
    pub provider: String,
    /// The Unix timestamp (seconds) at which the lyrics were fetched.
    pub fetched_at: i64,
}

impl CachedLyrics {
    /// Build a "found" record stamped as fetched now.
    #[must_use]
    pub fn found(lyrics: Lyrics, provider: impl Into<String>) -> Self {
        Self {
            outcome: CachedOutcome::Found(lyrics),
            provider: provider.into(),
            fetched_at: now(),
        }
    }

    /// Build a "no lyrics found" miss record stamped as fetched now.
    #[must_use]
    pub fn miss() -> Self {
        Self {
            outcome: CachedOutcome::NotFound,
            provider: String::new(),
            fetched_at: now(),
        }
    }

    /// Whether this record is still within its TTL at `now_ts`.
    ///
    /// A found record uses [`HIT_TTL`]; a miss uses the shorter [`MISS_TTL`].
    /// A `fetched_at` in the future (clock skew) counts as fresh.
    #[must_use]
    pub fn is_fresh_at(&self, now_ts: i64) -> bool {
        let ttl = match self.outcome {
            CachedOutcome::Found(_) => HIT_TTL,
            CachedOutcome::NotFound => MISS_TTL,
        };
        let age = now_ts.saturating_sub(self.fetched_at);
        age < 0 || Duration::from_secs(age.unsigned_abs()) <= ttl
    }
}

/// The current Unix timestamp in seconds.
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The persistent lyrics cache — a thin typed layer over [`MetadataCache`].
///
/// Cloning is cheap (the inner cache is shared behind an `Arc`).
#[derive(Clone)]
pub struct LyricsCache {
    /// The shared SQLite metadata store the lyrics rows live in.
    store: Arc<MetadataCache>,
}

impl std::fmt::Debug for LyricsCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LyricsCache").finish_non_exhaustive()
    }
}

impl LyricsCache {
    /// Build a lyrics cache over a shared metadata store.
    #[must_use]
    pub fn new(store: Arc<MetadataCache>) -> Self {
        Self { store }
    }

    /// Look up the cached lyrics for `track_id`.
    ///
    /// Returns `None` on a cache miss **or** when the cached record has lapsed
    /// past its TTL (so a stale miss is re-checked, a very old hit refreshed).
    /// A row whose payload fails to decode is treated as a miss.
    ///
    /// # Errors
    ///
    /// Returns an error only if the underlying SQL query itself fails.
    #[tracing::instrument(skip(self))]
    pub fn get(&self, track_id: &str) -> spottyfi_cache::CacheResult<Option<CachedLyrics>> {
        let Some(hit) = self.store.get::<CachedLyrics>(Kind::Lyrics, track_id)? else {
            return Ok(None);
        };
        if hit.value.is_fresh_at(now()) {
            Ok(Some(hit.value))
        } else {
            // Past its TTL — treat as a miss so the caller re-fetches.
            tracing::debug!(track_id, "cached lyrics lapsed past TTL");
            Ok(None)
        }
    }

    /// Store a lyrics `record` for `track_id`, replacing any existing row.
    ///
    /// # Errors
    ///
    /// Returns an error if the record cannot be serialised or the write fails.
    #[tracing::instrument(skip(self, record))]
    pub fn put(&self, track_id: &str, record: &CachedLyrics) -> spottyfi_cache::CacheResult<()> {
        self.store.put(Kind::Lyrics, track_id, record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::SyncedLine;

    fn synced() -> Lyrics {
        Lyrics::Synced(vec![SyncedLine {
            at: Duration::from_secs(1),
            text: "hello".into(),
        }])
    }

    /// A lyrics cache over a throwaway SQLite file.
    ///
    /// The returned [`tempfile::TempDir`] must be kept alive for the cache's
    /// lifetime — dropping it deletes the database file.
    fn cache() -> (LyricsCache, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = MetadataCache::open(dir.path().join("cache.db")).expect("open cache");
        (LyricsCache::new(Arc::new(store)), dir)
    }

    #[test]
    fn a_found_record_round_trips() {
        let (cache, _dir) = cache();
        let record = CachedLyrics::found(synced(), "lrclib");
        cache.put("spotify:track:x", &record).expect("put");
        let hit = cache
            .get("spotify:track:x")
            .expect("get")
            .expect("a cache hit");
        assert_eq!(hit.outcome, CachedOutcome::Found(synced()));
        assert_eq!(hit.provider, "lrclib");
    }

    #[test]
    fn a_miss_record_round_trips() {
        let (cache, _dir) = cache();
        cache
            .put("spotify:track:y", &CachedLyrics::miss())
            .expect("put");
        let hit = cache
            .get("spotify:track:y")
            .expect("get")
            .expect("the cached miss");
        assert_eq!(hit.outcome, CachedOutcome::NotFound);
    }

    #[test]
    fn an_absent_track_is_a_cache_miss() {
        let (cache, _dir) = cache();
        assert!(cache.get("spotify:track:absent").expect("get").is_none());
    }

    #[test]
    fn put_replaces_an_existing_record() {
        let (cache, _dir) = cache();
        cache
            .put("spotify:track:z", &CachedLyrics::miss())
            .expect("put miss");
        cache
            .put(
                "spotify:track:z",
                &CachedLyrics::found(synced(), "musixmatch"),
            )
            .expect("put found");
        let hit = cache.get("spotify:track:z").expect("get").expect("hit");
        assert!(matches!(hit.outcome, CachedOutcome::Found(_)));
        assert_eq!(hit.provider, "musixmatch");
    }

    #[test]
    fn a_fresh_hit_outlives_a_fresh_miss() {
        // A found record uses the long TTL; a miss the short one.
        let found = CachedLyrics::found(synced(), "lrclib");
        let miss = CachedLyrics::miss();
        let now = found.fetched_at;
        // Just past the miss TTL but well inside the hit TTL.
        let later = now + MISS_TTL.as_secs() as i64 + 1;
        assert!(found.is_fresh_at(later));
        assert!(!miss.is_fresh_at(later));
    }

    #[test]
    fn a_lapsed_record_reads_back_as_a_miss() {
        let (cache, _dir) = cache();
        // Hand-write a record fetched long enough ago to be past the hit TTL.
        let stale = CachedLyrics {
            outcome: CachedOutcome::Found(synced()),
            provider: "lrclib".into(),
            fetched_at: now() - HIT_TTL.as_secs() as i64 - 1,
        };
        cache.put("spotify:track:old", &stale).expect("put");
        // A lapsed record is reported as a miss so the caller re-fetches.
        assert!(cache.get("spotify:track:old").expect("get").is_none());
    }

    #[test]
    fn future_fetched_at_from_clock_skew_is_fresh() {
        let mut record = CachedLyrics::miss();
        record.fetched_at = now() + 10_000;
        assert!(record.is_fresh_at(now()));
    }
}
