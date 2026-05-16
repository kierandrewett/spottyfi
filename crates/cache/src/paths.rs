//! Resolving the XDG cache directory for the metadata DB and image cache.

use std::path::PathBuf;

use crate::error::{CacheError, CacheResult};

/// The metadata cache database file name, under the cache directory.
const DB_FILE: &str = "metadata.sqlite";

/// The image cache subdirectory, under the cache directory.
const IMAGE_DIR: &str = "images";

/// The platform cache directory for Spottyfi (`~/.cache/spottyfi` on Linux).
///
/// # Errors
///
/// Returns [`CacheError::NoCacheDir`] when the platform has no resolvable
/// cache directory.
pub fn cache_dir() -> CacheResult<PathBuf> {
    directories::ProjectDirs::from("dev", "drewett", "spottyfi")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .ok_or(CacheError::NoCacheDir)
}

/// The path to the SQLite metadata cache database.
///
/// # Errors
///
/// Returns [`CacheError::NoCacheDir`] when the cache directory cannot be
/// resolved.
pub fn metadata_db_path() -> CacheResult<PathBuf> {
    Ok(cache_dir()?.join(DB_FILE))
}

/// The path to the on-disk image cache directory.
///
/// # Errors
///
/// Returns [`CacheError::NoCacheDir`] when the cache directory cannot be
/// resolved.
pub fn image_cache_dir() -> CacheResult<PathBuf> {
    Ok(cache_dir()?.join(IMAGE_DIR))
}
