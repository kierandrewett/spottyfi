//! The SQLite metadata cache.
//!
//! Stores `spottyfi_models` objects as JSON blobs keyed by Spotify id, each
//! with a `last_fetched` Unix timestamp. The store is the persistent source of
//! truth for cacheable Web API GETs; the `api` crate consults it for the
//! stale-while-revalidate behaviour.
//!
//! # Blocking
//!
//! `rusqlite` is synchronous and `Connection` is `!Sync`, so the connection
//! lives behind a [`Mutex`]. Every method here blocks; callers on an async
//! runtime must invoke them from `spawn_blocking` (or a dedicated thread) so
//! the egui UI thread is never blocked — see `docs/threading.md`.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::CacheResult;
use crate::freshness::{Freshness, Staleness};
use crate::migrations;

/// The kind of object a cache row holds — selects the SQL table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A full [`Track`](spottyfi_models::Track).
    Track,
    /// A full [`Album`](spottyfi_models::Album).
    Album,
    /// A full [`Artist`](spottyfi_models::Artist).
    Artist,
    /// A full [`Playlist`](spottyfi_models::Playlist).
    Playlist,
    /// A playlist's fully-resolved track listing — a JSON array of
    /// [`PlaylistTrack`](spottyfi_models::PlaylistTrack), keyed by playlist id.
    PlaylistTracks,
    /// The lyrics fetched for a track — a JSON record carrying the lyrics (or
    /// a "not found" marker), the source provider and a fetched-at stamp,
    /// keyed by Spotify track id. Stored by the `api` crate's lyrics layer.
    Lyrics,
    /// A list-shaped result not keyed by a single Spotify object id — e.g.
    /// the user's full saved-tracks listing — keyed by a fixed string.
    Collection,
}

impl Kind {
    /// The SQL table backing this kind.
    ///
    /// The returned string is a fixed compile-time constant — never user
    /// input — so it is safe to interpolate into a query string.
    const fn table(self) -> &'static str {
        match self {
            Kind::Track => "tracks",
            Kind::Album => "albums",
            Kind::Artist => "artists",
            Kind::Playlist => "playlists",
            Kind::PlaylistTracks => "playlist_tracks",
            Kind::Lyrics => "lyrics",
            Kind::Collection => "collections",
        }
    }
}

/// A cache hit: the decoded value plus whether it needs revalidating.
#[derive(Debug, Clone)]
pub struct Cached<T> {
    /// The decoded cached object.
    pub value: T,
    /// Whether the object is past the freshness window.
    pub staleness: Staleness,
}

/// The SQLite metadata cache.
///
/// Cheap to clone is *not* a goal here: wrap it in an `Arc` to share. Every
/// method is blocking — call from `spawn_blocking`.
pub struct MetadataCache {
    /// The SQLite connection, behind a mutex (`Connection` is `!Sync`).
    conn: Mutex<Connection>,
    /// The stale-while-revalidate policy.
    freshness: Freshness,
}

impl MetadataCache {
    /// Open (creating if absent) the metadata cache at `path` and run pending
    /// migrations. Uses the default [`Freshness`] window.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent directory cannot be created, the
    /// database cannot be opened, or a migration fails.
    pub fn open(path: impl AsRef<Path>) -> CacheResult<Self> {
        Self::open_with_freshness(path, Freshness::default())
    }

    /// As [`Self::open`], with an explicit freshness window.
    ///
    /// # Errors
    ///
    /// See [`Self::open`].
    pub fn open_with_freshness(path: impl AsRef<Path>, freshness: Freshness) -> CacheResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        // WAL keeps a background refresh write from blocking a UI-driven read.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        migrations::run(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            freshness,
        })
    }

    /// Open an in-memory metadata cache (for tests).
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or migrated.
    #[cfg(test)]
    pub fn open_in_memory() -> CacheResult<Self> {
        let mut conn = Connection::open_in_memory()?;
        migrations::run(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            freshness: Freshness::default(),
        })
    }

    /// The current Unix timestamp in seconds.
    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    /// Insert or replace the cached object for `id`, stamping it as fetched now.
    ///
    /// # Errors
    ///
    /// Returns an error if the value cannot be serialised or the write fails.
    /// A poisoned mutex is recovered from rather than panicked on.
    pub fn put<T: Serialize>(&self, kind: Kind, id: &str, value: &T) -> CacheResult<()> {
        let payload = serde_json::to_string(value)?;
        let sql = format!(
            "INSERT INTO {table} (id, payload, last_fetched) VALUES (?1, ?2, ?3) \
             ON CONFLICT(id) DO UPDATE SET payload = excluded.payload, \
             last_fetched = excluded.last_fetched",
            table = kind.table(),
        );
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(&sql, rusqlite::params![id, payload, Self::now()])?;
        Ok(())
    }

    /// Fetch the cached object for `id`, decoded to `T`, with its staleness.
    ///
    /// Returns `Ok(None)` on a cache miss. A row whose payload fails to decode
    /// is treated as a miss (and logged) rather than an error — a stale schema
    /// should never break a fetch.
    ///
    /// # Errors
    ///
    /// Returns an error only if the SQL query itself fails.
    pub fn get<T: DeserializeOwned>(&self, kind: Kind, id: &str) -> CacheResult<Option<Cached<T>>> {
        let sql = format!(
            "SELECT payload, last_fetched FROM {table} WHERE id = ?1",
            table = kind.table(),
        );
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let row = conn
            .query_row(&sql, [id], |row| {
                let payload: String = row.get(0)?;
                let last_fetched: i64 = row.get(1)?;
                Ok((payload, last_fetched))
            })
            .ok();
        let Some((payload, last_fetched)) = row else {
            return Ok(None);
        };
        match serde_json::from_str::<T>(&payload) {
            Ok(value) => {
                let staleness = self.freshness.classify(last_fetched, Self::now());
                Ok(Some(Cached { value, staleness }))
            }
            Err(err) => {
                tracing::warn!(%err, ?kind, id, "cached payload failed to decode; treating as a miss");
                Ok(None)
            }
        }
    }

    /// Delete every row from every cache table.
    ///
    /// # Errors
    ///
    /// Returns an error if any `DELETE` fails.
    pub fn clear(&self) -> CacheResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        for kind in [
            Kind::Track,
            Kind::Album,
            Kind::Artist,
            Kind::Playlist,
            Kind::PlaylistTracks,
            Kind::Lyrics,
        ] {
            conn.execute(&format!("DELETE FROM {}", kind.table()), [])?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for MetadataCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetadataCache")
            .field("freshness", &self.freshness)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::freshness::Freshness;
    use std::time::Duration;

    #[test]
    fn put_then_get_round_trips() {
        let cache = MetadataCache::open_in_memory().expect("open");
        cache
            .put(Kind::Artist, "a1", &"Radiohead".to_owned())
            .expect("put");
        let hit: Cached<String> = cache.get(Kind::Artist, "a1").expect("get").expect("hit");
        assert_eq!(hit.value, "Radiohead");
        // A freshly written object is fresh.
        assert_eq!(hit.staleness, Staleness::Fresh);
    }

    #[test]
    fn missing_id_is_a_miss() {
        let cache = MetadataCache::open_in_memory().expect("open");
        let miss: Option<Cached<String>> = cache.get(Kind::Track, "nope").expect("get");
        assert!(miss.is_none());
    }

    #[test]
    fn put_replaces_an_existing_row() {
        let cache = MetadataCache::open_in_memory().expect("open");
        cache.put(Kind::Album, "x", &1_u32).expect("put 1");
        cache.put(Kind::Album, "x", &2_u32).expect("put 2");
        let hit: Cached<u32> = cache.get(Kind::Album, "x").expect("get").expect("hit");
        assert_eq!(hit.value, 2);
    }

    #[test]
    fn clear_empties_every_table() {
        let cache = MetadataCache::open_in_memory().expect("open");
        cache.put(Kind::Track, "t", &"v".to_owned()).expect("put");
        cache.put(Kind::Album, "a", &"v".to_owned()).expect("put");
        cache.clear().expect("clear");
        let t: Option<Cached<String>> = cache.get(Kind::Track, "t").expect("get");
        let a: Option<Cached<String>> = cache.get(Kind::Album, "a").expect("get");
        assert!(t.is_none() && a.is_none());
    }

    #[test]
    fn playlist_tracks_kind_round_trips_a_list() {
        // The playlist-content cache stores a JSON array keyed by playlist id.
        let cache = MetadataCache::open_in_memory().expect("open");
        let listing = vec!["t1".to_owned(), "t2".to_owned(), "t3".to_owned()];
        cache
            .put(Kind::PlaylistTracks, "pl1", &listing)
            .expect("put");
        let hit: Cached<Vec<String>> = cache
            .get(Kind::PlaylistTracks, "pl1")
            .expect("get")
            .expect("hit");
        assert_eq!(hit.value, listing);
        assert_eq!(hit.staleness, Staleness::Fresh);
        // `clear` empties the playlist-content table too.
        cache.clear().expect("clear");
        let miss: Option<Cached<Vec<String>>> =
            cache.get(Kind::PlaylistTracks, "pl1").expect("get");
        assert!(miss.is_none());
    }

    #[test]
    fn lyrics_kind_round_trips() {
        // The lyrics cache stores an opaque JSON record keyed by track id.
        let cache = MetadataCache::open_in_memory().expect("open");
        let payload = r#"{"provider":"lrclib","lyrics":["a","b"]}"#.to_owned();
        cache
            .put(Kind::Lyrics, "spotify:track:x", &payload)
            .expect("put");
        let hit: Cached<String> = cache
            .get(Kind::Lyrics, "spotify:track:x")
            .expect("get")
            .expect("hit");
        assert_eq!(hit.value, payload);
        assert_eq!(hit.staleness, Staleness::Fresh);
        cache.clear().expect("clear");
        let miss: Option<Cached<String>> = cache.get(Kind::Lyrics, "spotify:track:x").expect("get");
        assert!(miss.is_none());
    }

    #[test]
    fn an_aged_row_reads_back_stale() {
        // A zero-length window makes any non-instant row stale.
        let mut conn = Connection::open_in_memory().expect("open");
        migrations::run(&mut conn).expect("migrate");
        let cache = MetadataCache {
            conn: Mutex::new(conn),
            freshness: Freshness::new(Duration::from_secs(0)),
        };
        // Write a row with a `last_fetched` well in the past.
        {
            let conn = cache.conn.lock().expect("lock");
            conn.execute(
                "INSERT INTO artists (id, payload, last_fetched) VALUES (?1, ?2, ?3)",
                rusqlite::params!["old", "\"v\"", 0_i64],
            )
            .expect("insert");
        }
        let hit: Cached<String> = cache.get(Kind::Artist, "old").expect("get").expect("hit");
        assert_eq!(hit.staleness, Staleness::Stale);
    }
}
