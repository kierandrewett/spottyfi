//! The on-disk image cache.
//!
//! Remote album art / avatars are cached as files named `sha1(url).webp` under
//! the platform cache directory. The cache is a simple disk LRU: each lookup
//! "touches" the file's modified time, and when the total cache size exceeds
//! the cap, the least-recently-touched files are deleted until it fits.
//!
//! # Blocking
//!
//! Every method here does filesystem I/O and blocks. The `ui` image loader
//! calls them off the egui thread (the `ehttp` fetch callback runs on a worker
//! thread); a tokio caller should use `spawn_blocking`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha1::{Digest, Sha1};

use crate::error::CacheResult;

/// The default on-disk image cache size cap: 500 MB.
pub const DEFAULT_CAPACITY_BYTES: u64 = 500 * 1024 * 1024;

/// The file extension for cached image files.
const EXT: &str = "webp";

/// An on-disk LRU cache for encoded image bytes.
///
/// Cheap to clone — it holds only the cache directory path and the byte cap.
#[derive(Debug, Clone)]
pub struct ImageCache {
    /// The directory holding the `sha1(url).webp` files.
    dir: PathBuf,
    /// The maximum total size of the cache directory, in bytes.
    capacity_bytes: u64,
}

impl ImageCache {
    /// Open (creating if absent) an image cache at `dir` with the default cap.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    pub fn open(dir: impl Into<PathBuf>) -> CacheResult<Self> {
        Self::open_with_capacity(dir, DEFAULT_CAPACITY_BYTES)
    }

    /// As [`Self::open`], with an explicit byte cap.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    pub fn open_with_capacity(dir: impl Into<PathBuf>, capacity_bytes: u64) -> CacheResult<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            capacity_bytes,
        })
    }

    /// The configured byte cap.
    #[must_use]
    pub fn capacity_bytes(&self) -> u64 {
        self.capacity_bytes
    }

    /// The on-disk path for `url`'s cached image (whether or not it exists).
    fn path_for(&self, url: &str) -> PathBuf {
        let mut hasher = Sha1::new();
        hasher.update(url.as_bytes());
        let digest = hasher.finalize();
        let hex = digest.iter().fold(String::with_capacity(40), |mut acc, b| {
            use std::fmt::Write as _;
            // Writing to a String never fails.
            let _ = write!(acc, "{b:02x}");
            acc
        });
        self.dir.join(format!("{hex}.{EXT}"))
    }

    /// Read the cached image bytes for `url`, or `None` on a miss.
    ///
    /// A hit "touches" the file (updates its modified time) so the LRU
    /// eviction treats it as recently used.
    ///
    /// # Errors
    ///
    /// Returns an error only if the file exists but cannot be read.
    pub fn get(&self, url: &str) -> CacheResult<Option<Vec<u8>>> {
        let path = self.path_for(url);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        // Best-effort LRU touch; a failure to touch is not worth failing the
        // read over (the file is simply treated as older than it is).
        touch(&path);
        Ok(Some(bytes))
    }

    /// Whether `url` is currently cached on disk.
    #[must_use]
    pub fn contains(&self, url: &str) -> bool {
        self.path_for(url).exists()
    }

    /// Write `bytes` as the cached image for `url`, then evict if over cap.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn put(&self, url: &str, bytes: &[u8]) -> CacheResult<()> {
        let path = self.path_for(url);
        std::fs::write(&path, bytes)?;
        // Eviction failure must not lose the just-written entry; log and move
        // on so the cache simply runs a little over cap until the next write.
        if let Err(err) = self.evict_to_capacity() {
            tracing::warn!(%err, "image cache eviction failed");
        }
        Ok(())
    }

    /// Delete every file in the image cache directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be read; individual
    /// unremovable files are logged and skipped.
    pub fn clear(&self) -> CacheResult<()> {
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            if entry.path().is_file() {
                if let Err(err) = std::fs::remove_file(entry.path()) {
                    tracing::warn!(%err, path = %entry.path().display(), "could not remove cached image");
                }
            }
        }
        Ok(())
    }

    /// The total size of the cache directory in bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be read.
    pub fn total_size_bytes(&self) -> CacheResult<u64> {
        let mut total = 0;
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total += meta.len();
                }
            }
        }
        Ok(total)
    }

    /// Evict least-recently-used files until the cache fits its byte cap.
    ///
    /// Files are ordered by modified time (the LRU proxy that [`get`] and
    /// [`put`] keep current); the oldest are deleted first. A no-op when the
    /// cache is already within cap.
    ///
    /// [`get`]: Self::get
    /// [`put`]: Self::put
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be read.
    pub fn evict_to_capacity(&self) -> CacheResult<()> {
        let mut files: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
        let mut total = 0u64;
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            total += meta.len();
            files.push((entry.path(), mtime, meta.len()));
        }

        if total <= self.capacity_bytes {
            return Ok(());
        }

        // Oldest first — these are evicted until the cache is under cap.
        files.sort_by_key(|(_, mtime, _)| *mtime);
        for (path, _, size) in files {
            if total <= self.capacity_bytes {
                break;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    total = total.saturating_sub(size);
                    tracing::debug!(path = %path.display(), size, "evicted cached image");
                }
                Err(err) => {
                    tracing::warn!(%err, path = %path.display(), "could not evict cached image");
                }
            }
        }
        Ok(())
    }
}

/// Best-effort: bump a file's modified time to "now" so an LRU pass sees it as
/// recently used. Any failure is swallowed.
fn touch(path: &Path) {
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_cache(capacity: u64) -> (tempfile::TempDir, ImageCache) {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = ImageCache::open_with_capacity(dir.path(), capacity).expect("open image cache");
        (dir, cache)
    }

    #[test]
    fn put_then_get_round_trips() {
        let (_dir, cache) = temp_cache(DEFAULT_CAPACITY_BYTES);
        cache.put("https://i.scdn.co/x", b"webpbytes").expect("put");
        assert!(cache.contains("https://i.scdn.co/x"));
        let got = cache.get("https://i.scdn.co/x").expect("get").expect("hit");
        assert_eq!(got, b"webpbytes");
    }

    #[test]
    fn distinct_urls_get_distinct_files() {
        let (_dir, cache) = temp_cache(DEFAULT_CAPACITY_BYTES);
        assert_ne!(cache.path_for("a"), cache.path_for("b"));
        // Same url is stable.
        assert_eq!(cache.path_for("a"), cache.path_for("a"));
    }

    #[test]
    fn miss_returns_none() {
        let (_dir, cache) = temp_cache(DEFAULT_CAPACITY_BYTES);
        assert!(cache.get("never-cached").expect("get").is_none());
        assert!(!cache.contains("never-cached"));
    }

    #[test]
    fn clear_empties_the_cache() {
        let (_dir, cache) = temp_cache(DEFAULT_CAPACITY_BYTES);
        cache.put("a", b"123").expect("put");
        cache.put("b", b"456").expect("put");
        cache.clear().expect("clear");
        assert!(!cache.contains("a"));
        assert_eq!(cache.total_size_bytes().expect("size"), 0);
    }

    #[test]
    fn eviction_drops_least_recently_used_when_over_cap() {
        // Cap of 10 bytes; three 4-byte entries cannot all fit.
        let (_dir, cache) = temp_cache(10);

        cache.put("old", b"AAAA").expect("put old");
        // Space the modified times apart so the LRU ordering is unambiguous.
        std::thread::sleep(Duration::from_millis(20));
        cache.put("mid", b"BBBB").expect("put mid");
        std::thread::sleep(Duration::from_millis(20));

        // Touch "old" so "mid" becomes the least-recently-used entry.
        let _ = cache.get("old").expect("touch old");
        std::thread::sleep(Duration::from_millis(20));

        // This write pushes the cache over cap and triggers eviction.
        cache.put("new", b"CCCC").expect("put new");

        assert!(cache.total_size_bytes().expect("size") <= 10);
        // "mid" was the LRU entry and should have been evicted; "old" (touched)
        // and "new" (just written) should survive.
        assert!(!cache.contains("mid"), "least-recently-used entry evicted");
        assert!(cache.contains("old"), "touched entry retained");
        assert!(cache.contains("new"), "newest entry retained");
    }

    #[test]
    fn eviction_is_a_noop_under_cap() {
        let (_dir, cache) = temp_cache(DEFAULT_CAPACITY_BYTES);
        cache.put("a", b"small").expect("put");
        cache.evict_to_capacity().expect("evict");
        assert!(cache.contains("a"));
    }
}
