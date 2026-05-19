//! The Apple Music client error type.

use thiserror::Error;

/// An error raised by the Apple Music catalog client.
#[derive(Debug, Error)]
pub enum AppleMusicError {
    /// The HTTP request itself failed — DNS, TLS, a refused connection or a
    /// timeout.
    #[error("apple music request failed: {0}")]
    Http(String),

    /// The server replied, but the body could not be decoded.
    #[error("decoding the apple music response failed: {0}")]
    Decode(String),

    /// Apple Music returned an API error. `403`/`401` almost always mean the
    /// developer token is missing, malformed or expired.
    #[error("apple music api error {status}: {message}")]
    Api {
        /// The HTTP status code.
        status: u16,
        /// The human-readable message, from the `errors` array when present.
        message: String,
    },
}

/// Convenience alias for results from the Apple Music client.
pub type AppleMusicResult<T> = Result<T, AppleMusicError>;
