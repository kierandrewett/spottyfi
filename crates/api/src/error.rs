//! The Web API client error type.

use std::time::Duration;

use thiserror::Error;

/// Errors raised by the Spotify Web API client.
#[derive(Debug, Error)]
pub enum ApiError {
    /// No valid access token was available, or the token was rejected
    /// (HTTP 401). The session needs re-authentication or a token refresh.
    #[error("authentication failed or token expired: {0}")]
    Auth(String),

    /// The request was rate-limited (HTTP 429) and retries were exhausted.
    ///
    /// Carries the `Retry-After` delay Spotify last asked for, when one was
    /// present, so a caller can surface a sensible "try again in N seconds".
    #[error("rate limited by Spotify; retries exhausted (retry after {retry_after:?})")]
    RateLimited {
        /// The `Retry-After` delay from the final 429 response, if any.
        retry_after: Option<Duration>,
    },

    /// The endpoint is unavailable to this Spotify application.
    ///
    /// Spotify deprecated several Web API endpoints for apps registered after
    /// 2024-11-27 (Recommendations, Featured Playlists, a Category's
    /// playlists, Related Artists, Audio Features/Analysis). They return
    /// 403/404 for such apps. The affected [`SpotifyApi`](crate::SpotifyApi)
    /// methods map that response onto this variant rather than a misleading
    /// `NotFound` or empty result. See `docs/questions.md`.
    #[error("endpoint '{endpoint}' is unavailable to this Spotify app (deprecated 2024-11-27)")]
    EndpointUnavailable {
        /// A short label for the endpoint, e.g. `recommendations`.
        endpoint: &'static str,
    },

    /// The requested object does not exist (HTTP 404).
    #[error("not found: {0}")]
    NotFound(String),

    /// A network- or transport-level failure: connection refused, timeout,
    /// TLS error, or an HTTP status not otherwise specialised.
    #[error("network or HTTP error: {0}")]
    Network(String),

    /// A response body could not be deserialised into the expected shape.
    #[error("failed to deserialise the Spotify response: {0}")]
    Deserialize(String),
}

impl ApiError {
    /// Whether retrying the same request might succeed.
    ///
    /// Rate limiting and transient network failures are retryable; auth
    /// failures, missing objects, deprecated endpoints and malformed responses
    /// are not.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, ApiError::RateLimited { .. } | ApiError::Network(_))
    }
}

/// Convenience alias for results from the Web API client.
pub type ApiResult<T> = Result<T, ApiError>;
