//! Error types for the authentication crate.

use thiserror::Error;

/// Errors that can occur during the OAuth login, token refresh and keyring
/// persistence flows.
#[derive(Debug, Error)]
pub enum AuthError {
    /// The `SPOTTYFI_CLIENT_ID` environment variable was not set.
    #[error("SPOTTYFI_CLIENT_ID is not set; register a Spotify app and export its Client ID")]
    MissingClientId,

    /// The `SPOTTYFI_REDIRECT_PORT` environment variable was set to an invalid value.
    #[error("SPOTTYFI_REDIRECT_PORT is not a valid port number: {0}")]
    InvalidRedirectPort(String),

    /// Reading from or writing to the platform keyring failed.
    #[error("keyring access failed: {0}")]
    Keyring(#[from] keyring_core::Error),

    /// A network or HTTP-level failure occurred talking to Spotify.
    #[error("network/HTTP error: {0}")]
    Http(String),

    /// Exchanging the authorization code (or refresh token) for an access
    /// token failed.
    #[error("token exchange failed: {0}")]
    TokenExchange(String),

    /// The local callback server could not be started or failed while running.
    #[error("callback server error: {0}")]
    CallbackServer(String),

    /// The callback request could not be parsed (missing/invalid query params).
    #[error("could not parse OAuth callback: {0}")]
    CallbackParse(String),

    /// The `state` value returned by Spotify did not match the one we sent.
    /// This indicates a possible CSRF attack and the login is aborted.
    #[error("OAuth state mismatch: possible CSRF; login aborted")]
    StateMismatch,

    /// The user did not complete the browser login within the allowed window.
    #[error("login timed out waiting for the Spotify callback")]
    Timeout,

    /// Serialising or deserialising a stored token failed.
    #[error("token (de)serialisation failed: {0}")]
    Serde(#[from] serde_json::Error),

    /// The system browser could not be opened.
    #[error("could not open the system browser: {0}")]
    Browser(String),

    /// A background task could not be joined or panicked.
    #[error("internal task failure: {0}")]
    Task(String),
}

/// Convenience result alias for the authentication crate.
pub type AuthResult<T> = Result<T, AuthError>;
