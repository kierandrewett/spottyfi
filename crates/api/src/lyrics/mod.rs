//! The lyrics source layer.
//!
//! Spotify's official Web API exposes no lyrics endpoint, so Spottyfi sources
//! lyrics from auxiliary providers, each behind its own opt-in:
//!
//! - [`musixmatch`] — the **legitimate** path: a documented Web API with
//!   time-synced LRC for a large catalogue. Gated behind the **`musixmatch`
//!   Cargo feature, off by default**, and needs an API key in
//!   `SPOTTYFI_MUSIXMATCH_KEY`.
//! - [`spotify_internal`] — an **undocumented, reverse-engineered** endpoint
//!   on `spclient.wg.spotify.com`. It is **against Spotify's Terms of
//!   Service** and is only ever attempted when the `SPOTTYFI_LYRICS_TOKEN`
//!   environment variable is set. Never on by default; see `docs/questions.md`.
//!
//! [`LyricsService::from_env`] assembles whichever providers are configured.
//! With **none** configured it is still a valid service: every lookup returns
//! [`LyricsError::NoSourceConfigured`] — a clean, calm result, never a panic.
//!
//! Lyrics are modelled by [`Lyrics`]: either time-[`synced`](Lyrics::Synced)
//! lines or [`plain`](Lyrics::Plain) unsynced text. The LRC parser
//! ([`parse_lrc`]) and the current-line selector
//! ([`current_synced_line`]) live in [`model`].

mod model;
#[cfg(feature = "musixmatch")]
pub mod musixmatch;
pub mod spotify_internal;

pub use model::{current_synced_line, parse_lrc, Lyrics, SyncedLine};

/// Errors raised by the lyrics layer.
#[derive(Debug, thiserror::Error)]
pub enum LyricsError {
    /// No lyrics source is configured at all.
    ///
    /// This is the *expected* state when neither the `musixmatch` feature +
    /// `SPOTTYFI_MUSIXMATCH_KEY` nor `SPOTTYFI_LYRICS_TOKEN` is set. The UI
    /// treats it as "no lyrics source configured", not as a failure.
    #[error("no lyrics source is configured")]
    NoSourceConfigured,

    /// A specific provider has no credentials configured.
    ///
    /// Internal to provider construction — [`LyricsService::from_env`] folds
    /// it into "this provider is simply absent".
    #[error("this lyrics provider is not configured")]
    NotConfigured,

    /// No provider had lyrics for the requested track.
    #[error("no lyrics found for this track")]
    NotFound,

    /// A provider answered with an API-level error (bad key, quota, …).
    #[error("lyrics provider error: {0}")]
    Provider(String),

    /// A network- or transport-level failure talking to a provider.
    #[error("network error fetching lyrics: {0}")]
    Network(String),

    /// A provider response could not be deserialised into the expected shape.
    #[error("failed to deserialise the lyrics response: {0}")]
    Deserialize(String),
}

/// Convenience alias for results from the lyrics layer.
pub type LyricsResult<T> = Result<T, LyricsError>;

/// The minimal description of a track a lyrics provider needs to look it up.
///
/// musixmatch matches on `{title, artist}`; the internal Spotify endpoint is
/// keyed by the bare `spotify:track:` id, parsed out of [`TrackRef::uri`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackRef {
    /// The track's Spotify URI (`spotify:track:…`).
    pub uri: String,
    /// The track title.
    pub title: String,
    /// The primary artist's name (the billing-order first artist).
    pub artist: String,
}

impl TrackRef {
    /// The bare base-62 track id, parsed from a `spotify:track:…` URI.
    ///
    /// Returns `None` for any URI that is not a track URI.
    #[must_use]
    pub fn spotify_track_id(&self) -> Option<&str> {
        self.uri.strip_prefix("spotify:track:")
    }
}

/// The lyrics source layer: the configured providers, tried in order.
///
/// Construct with [`LyricsService::from_env`]. Cloning is cheap — every
/// provider wraps a shared `reqwest::Client`.
#[derive(Clone, Default)]
pub struct LyricsService {
    /// The musixmatch provider, when the feature is on and a key is set.
    #[cfg(feature = "musixmatch")]
    musixmatch: Option<musixmatch::MusixmatchProvider>,
    /// The internal Spotify provider, when `SPOTTYFI_LYRICS_TOKEN` is set.
    spotify_internal: Option<spotify_internal::SpotifyInternalProvider>,
}

impl std::fmt::Debug for LyricsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LyricsService")
            .field("has_source", &self.has_source())
            .finish()
    }
}

impl LyricsService {
    /// Assemble the lyrics service from the environment.
    ///
    /// Each provider is constructed independently; a provider with no
    /// credentials is simply omitted. This **never fails** — with no provider
    /// configured the service is still valid, and every lookup returns
    /// [`LyricsError::NoSourceConfigured`].
    #[must_use]
    pub fn from_env() -> Self {
        #[cfg(feature = "musixmatch")]
        let musixmatch = match musixmatch::MusixmatchProvider::from_env() {
            Ok(provider) => Some(provider),
            Err(err) => {
                tracing::info!(%err, "musixmatch lyrics provider not configured");
                None
            }
        };

        let spotify_internal = match spotify_internal::SpotifyInternalProvider::from_env() {
            Ok(provider) => {
                tracing::warn!(
                    "the internal Spotify lyrics endpoint is enabled \
                     ({}): it is undocumented and against Spotify's ToS",
                    spotify_internal::TOKEN_ENV,
                );
                Some(provider)
            }
            Err(_) => None,
        };

        Self {
            #[cfg(feature = "musixmatch")]
            musixmatch,
            spotify_internal,
        }
    }

    /// Whether at least one lyrics provider is configured.
    #[must_use]
    pub fn has_source(&self) -> bool {
        #[cfg_attr(not(feature = "musixmatch"), allow(unused_mut))]
        let mut has = self.spotify_internal.is_some();
        #[cfg(feature = "musixmatch")]
        {
            has = has || self.musixmatch.is_some();
        }
        has
    }

    /// Fetch lyrics for `track`, trying each configured provider in turn.
    ///
    /// The musixmatch provider (the legitimate path) is preferred; the
    /// internal Spotify endpoint is the fallback. The first provider that
    /// returns lyrics wins.
    ///
    /// # Errors
    ///
    /// - [`LyricsError::NoSourceConfigured`] when no provider is configured.
    /// - [`LyricsError::NotFound`] when every provider was tried and none had
    ///   lyrics for the track.
    /// - The last provider error otherwise (network / provider failure).
    #[tracing::instrument(skip(self), fields(uri = %track.uri))]
    pub async fn lyrics(&self, track: &TrackRef) -> LyricsResult<Lyrics> {
        if !self.has_source() {
            return Err(LyricsError::NoSourceConfigured);
        }

        let mut last_err: Option<LyricsError> = None;

        // The legitimate path first.
        #[cfg(feature = "musixmatch")]
        if let Some(provider) = &self.musixmatch {
            match provider.lyrics(track).await {
                Ok(lyrics) => return Ok(lyrics),
                Err(LyricsError::NotFound) => {}
                Err(err) => {
                    tracing::warn!(%err, "musixmatch lyrics lookup failed");
                    last_err = Some(err);
                }
            }
        }

        // The undocumented fallback.
        if let Some(provider) = &self.spotify_internal {
            match provider.lyrics(track).await {
                Ok(lyrics) => return Ok(lyrics),
                Err(LyricsError::NotFound) => {}
                Err(err) => {
                    tracing::warn!(%err, "internal Spotify lyrics lookup failed");
                    last_err = Some(err);
                }
            }
        }

        Err(last_err.unwrap_or(LyricsError::NotFound))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_service_reports_no_source() {
        let service = LyricsService::default();
        assert!(!service.has_source());
    }

    #[tokio::test]
    async fn an_empty_service_returns_no_source_configured() {
        let service = LyricsService::default();
        let track = TrackRef {
            uri: "spotify:track:abc".into(),
            title: "Song".into(),
            artist: "Artist".into(),
        };
        assert!(matches!(
            service.lyrics(&track).await,
            Err(LyricsError::NoSourceConfigured)
        ));
    }

    #[test]
    fn track_ref_extracts_the_spotify_id() {
        let track = TrackRef {
            uri: "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into(),
            title: "x".into(),
            artist: "y".into(),
        };
        assert_eq!(track.spotify_track_id(), Some("4uLU6hMCjMI75M1A2tKUQC"));
    }

    #[test]
    fn track_ref_rejects_a_non_track_uri() {
        let track = TrackRef {
            uri: "spotify:album:abc".into(),
            title: "x".into(),
            artist: "y".into(),
        };
        assert_eq!(track.spotify_track_id(), None);
    }

    #[test]
    fn no_source_configured_has_a_clear_message() {
        assert!(LyricsError::NoSourceConfigured
            .to_string()
            .contains("no lyrics source"));
    }
}
