//! The cache crate error type.

use thiserror::Error;

/// Errors raised by the metadata and image caches.
#[derive(Debug, Error)]
pub enum CacheError {
    /// The platform cache directory could not be resolved.
    #[error("could not resolve the platform cache directory")]
    NoCacheDir,

    /// A filesystem operation failed (creating a directory, reading or writing
    /// an image file).
    #[error("cache filesystem error: {0}")]
    Io(#[from] std::io::Error),

    /// A SQLite operation failed.
    #[error("cache database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A cached payload could not be (de)serialised.
    #[error("cache (de)serialisation error: {0}")]
    Serde(#[from] serde_json::Error),

    /// A migration file was malformed or applied out of order.
    #[error("cache migration error: {0}")]
    Migration(String),
}

/// Convenience alias for results from the cache crate.
pub type CacheResult<T> = Result<T, CacheError>;
