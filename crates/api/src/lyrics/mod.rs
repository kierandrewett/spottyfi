//! The lyrics source layer.
//!
//! Spotify's official Web API exposes no lyrics endpoint, so Spottyfi sources
//! lyrics from auxiliary providers:
//!
//! - [`lrclib`] — the **default**: a free, open, community lyrics database
//!   ([lrclib.net](https://lrclib.net/)) with time-synced LRC. It needs **no
//!   API key and no setup**, so it is always compiled in, always available,
//!   and the provider Spottyfi tries first.
//! - [`musixmatch`] — a documented commercial Web API. Gated behind the
//!   **`musixmatch` Cargo feature, off by default**, and needs an API key in
//!   `SPOTTYFI_MUSIXMATCH_KEY`.
//! - [`spotify_internal`] — an **undocumented, reverse-engineered** endpoint
//!   on `spclient.wg.spotify.com`. It is **against Spotify's Terms of
//!   Service** and is only ever attempted when the `SPOTTYFI_LYRICS_TOKEN`
//!   environment variable is set. Never on by default; see `docs/questions.md`.
//!
//! [`LyricsService::from_env`] assembles the providers — lrclib always, the
//! others when configured. Because lrclib needs no setup the service always
//! [`has_source`](LyricsService::has_source).
//!
//! ## Match heuristics
//!
//! A provider that searches (lrclib's `/api/search`) can return several
//! candidates — different recordings of the same song. [`score`] picks the
//! best by **track duration** proximity plus title/artist/album similarity,
//! so the synced lyrics line up with the recording actually playing.
//!
//! Lyrics are modelled by [`Lyrics`]: either time-[`synced`](Lyrics::Synced)
//! lines or [`plain`](Lyrics::Plain) unsynced text. The LRC parser
//! ([`parse_lrc`]) and the current-line selector
//! ([`current_synced_line`]) live in [`model`].

pub mod lrclib;
mod model;
#[cfg(feature = "musixmatch")]
pub mod musixmatch;
pub mod score;
pub mod spotify_internal;

pub use model::{current_synced_line, parse_lrc, Lyrics, SyncedLine};

use serde::{Deserialize, Serialize};

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
/// musixmatch matches on `{title, artist}`; lrclib additionally uses the
/// album and track duration to score candidates; the internal Spotify endpoint
/// is keyed by the bare `spotify:track:` id, parsed out of [`TrackRef::uri`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackRef {
    /// The track's Spotify URI (`spotify:track:…`).
    pub uri: String,
    /// The track title.
    pub title: String,
    /// The primary artist's name (the billing-order first artist).
    pub artist: String,
    /// The album name, when known — used to disambiguate lyrics candidates.
    ///
    /// Empty when the caller has no album to offer; providers that match on
    /// album simply skip the album term in that case.
    pub album: String,
    /// The track's total duration, when known — the strongest matching signal
    /// for picking the right lyrics version among candidates.
    ///
    /// [`Duration::ZERO`] when the caller has no duration to offer.
    pub duration: std::time::Duration,
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

/// Which lyrics provider to use for a lookup.
///
/// Surfaced as a user preference on the Settings page. [`Auto`](Self::Auto)
/// tries every configured provider in a sensible order; the named variants
/// pin the lookup to one provider (falling back to nothing if it is not
/// configured or has no lyrics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LyricsProvider {
    /// Try every configured provider, lrclib first. The default.
    #[default]
    Auto,
    /// Use only lrclib.net — the free, open, no-setup provider.
    Lrclib,
    /// Use only musixmatch (needs the `musixmatch` feature + an API key).
    Musixmatch,
    /// Use only the undocumented internal Spotify endpoint (needs a token).
    SpotifyInternal,
}

/// The lyrics source layer: the configured providers.
///
/// Construct with [`LyricsService::from_env`]. Cloning is cheap — every
/// provider wraps a shared `reqwest::Client`.
#[derive(Clone, Default)]
pub struct LyricsService {
    /// The lrclib provider — always present (it needs no configuration).
    lrclib: lrclib::LrclibProvider,
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
    /// lrclib is always included (it needs no configuration); the other
    /// providers are constructed independently and a provider with no
    /// credentials is simply omitted. This **never fails**.
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
            lrclib: lrclib::LrclibProvider::new(),
            #[cfg(feature = "musixmatch")]
            musixmatch,
            spotify_internal,
        }
    }

    /// Whether at least one lyrics provider is available.
    ///
    /// Always `true`: lrclib is always compiled in and needs no configuration.
    /// Retained so callers written against the pre-lrclib API still compile.
    #[must_use]
    pub fn has_source(&self) -> bool {
        true
    }

    /// Fetch lyrics for `track` using the [`Auto`](LyricsProvider::Auto)
    /// provider order — lrclib first.
    ///
    /// A thin wrapper over [`Self::lyrics_with`]; see it for the error
    /// semantics.
    ///
    /// # Errors
    ///
    /// See [`Self::lyrics_with`].
    #[tracing::instrument(skip(self), fields(uri = %track.uri))]
    pub async fn lyrics(&self, track: &TrackRef) -> LyricsResult<Lyrics> {
        self.lyrics_with(track, LyricsProvider::Auto).await
    }

    /// Fetch lyrics for `track` from the chosen `provider`.
    ///
    /// [`LyricsProvider::Auto`] tries every configured provider in order —
    /// lrclib, then musixmatch, then the internal Spotify endpoint — and the
    /// first to return lyrics wins. A named provider pins the lookup; if that
    /// provider is not configured the result is [`LyricsError::NotFound`].
    ///
    /// # Errors
    ///
    /// - [`LyricsError::NotFound`] when no provider had lyrics for the track
    ///   (or the chosen named provider is not configured).
    /// - The last provider error otherwise (network / provider failure).
    #[tracing::instrument(skip(self), fields(uri = %track.uri, ?provider))]
    pub async fn lyrics_with(
        &self,
        track: &TrackRef,
        provider: LyricsProvider,
    ) -> LyricsResult<Lyrics> {
        let mut last_err: Option<LyricsError> = None;

        // lrclib — the free default; tried for Auto and when pinned.
        if matches!(provider, LyricsProvider::Auto | LyricsProvider::Lrclib) {
            match self.lrclib.lyrics(track).await {
                Ok(lyrics) => return Ok(lyrics),
                Err(LyricsError::NotFound) => {}
                Err(err) => {
                    tracing::warn!(%err, "lrclib lyrics lookup failed");
                    last_err = Some(err);
                }
            }
        }

        // musixmatch — the commercial path; tried for Auto and when pinned.
        #[cfg(feature = "musixmatch")]
        if matches!(provider, LyricsProvider::Auto | LyricsProvider::Musixmatch) {
            if let Some(mxm) = &self.musixmatch {
                match mxm.lyrics(track).await {
                    Ok(lyrics) => return Ok(lyrics),
                    Err(LyricsError::NotFound) => {}
                    Err(err) => {
                        tracing::warn!(%err, "musixmatch lyrics lookup failed");
                        last_err = Some(err);
                    }
                }
            }
        }

        // The undocumented internal endpoint — Auto fallback or when pinned.
        if matches!(
            provider,
            LyricsProvider::Auto | LyricsProvider::SpotifyInternal
        ) {
            if let Some(internal) = &self.spotify_internal {
                match internal.lyrics(track).await {
                    Ok(lyrics) => return Ok(lyrics),
                    Err(LyricsError::NotFound) => {}
                    Err(err) => {
                        tracing::warn!(%err, "internal Spotify lyrics lookup failed");
                        last_err = Some(err);
                    }
                }
            }
        }

        Err(last_err.unwrap_or(LyricsError::NotFound))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track_ref(uri: &str) -> TrackRef {
        TrackRef {
            uri: uri.into(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration: std::time::Duration::from_secs(180),
        }
    }

    #[test]
    fn the_default_service_always_has_a_source() {
        // lrclib needs no configuration, so a source is always available.
        let service = LyricsService::default();
        assert!(service.has_source());
    }

    #[test]
    fn from_env_always_includes_lrclib() {
        // `from_env` must always produce a usable service — lrclib is free.
        let service = LyricsService::from_env();
        assert!(service.has_source());
    }

    #[test]
    fn the_default_provider_choice_is_auto() {
        assert_eq!(LyricsProvider::default(), LyricsProvider::Auto);
    }

    #[test]
    fn track_ref_extracts_the_spotify_id() {
        let track = track_ref("spotify:track:4uLU6hMCjMI75M1A2tKUQC");
        assert_eq!(track.spotify_track_id(), Some("4uLU6hMCjMI75M1A2tKUQC"));
    }

    #[test]
    fn track_ref_rejects_a_non_track_uri() {
        let track = track_ref("spotify:album:abc");
        assert_eq!(track.spotify_track_id(), None);
    }

    #[test]
    fn no_source_configured_has_a_clear_message() {
        assert!(LyricsError::NoSourceConfigured
            .to_string()
            .contains("no lyrics source"));
    }
}
