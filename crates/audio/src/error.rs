//! The audio engine error type.

use thiserror::Error;

/// Errors raised by the audio engine.
#[derive(Debug, Error)]
pub enum AudioError {
    /// The librespot session failed to connect to a Spotify access point.
    ///
    /// Most commonly a rejected or expired access token, or no network.
    #[error("connecting the librespot session: {0}")]
    Connect(String),

    /// A Spotify URI (or `open.spotify.com` URL) could not be parsed.
    #[error("invalid Spotify URI: {0}")]
    InvalidUri(String),

    /// The supplied URI parsed, but does not refer to a playable item
    /// (a track or episode). Albums, artists and playlists are not playable
    /// without a queue, which arrives in Phase 8.
    #[error("Spotify URI is not directly playable: {0}")]
    NotPlayable(String),

    /// No audio backend could be initialised (e.g. no ALSA device).
    #[error("no audio output backend available")]
    NoBackend,

    /// A control command was issued before the engine had a live player.
    #[error("the audio engine is not running")]
    NotRunning,
}

/// Convenience alias for results from the audio engine.
pub type AudioResult<T> = Result<T, AudioError>;
