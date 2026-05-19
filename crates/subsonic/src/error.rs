//! The OpenSubsonic client error type.

use thiserror::Error;

/// An error raised by the OpenSubsonic client.
#[derive(Debug, Error)]
pub enum SubsonicError {
    /// The HTTP request itself failed — DNS, TLS, a refused connection or a
    /// timeout. The wrapped string is the underlying `reqwest` error.
    #[error("subsonic request failed: {0}")]
    Http(String),

    /// The server replied, but the body could not be decoded as the expected
    /// Subsonic JSON.
    #[error("decoding the subsonic response failed: {0}")]
    Decode(String),

    /// The server returned a Subsonic-level error (`status: "failed"`), with
    /// its numeric [error code](https://opensubsonic.netlify.app/docs/responses/error/).
    #[error("subsonic server error {code}: {message}")]
    Api {
        /// The Subsonic error code (e.g. `40` — wrong username or password).
        code: u32,
        /// The human-readable message the server supplied.
        message: String,
    },

    /// The configured server URL was empty or could not be parsed.
    #[error("invalid subsonic server URL: {0}")]
    BadUrl(String),
}

/// Convenience alias for results from the OpenSubsonic client.
pub type SubsonicResult<T> = Result<T, SubsonicError>;
