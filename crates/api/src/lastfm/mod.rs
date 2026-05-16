//! A small [Last.fm](https://www.last.fm/api) Web API client.
//!
//! ## Why Last.fm?
//!
//! Spotify restricted its discovery endpoints — Recommendations, Featured
//! Playlists, a Category's playlists — to apps that held extended quota before
//! 2024-11-27. A newly-registered app such as Spottyfi gets 403/404 from them
//! (see `docs/questions.md` #7). With the maintainer's approval, Phase 7's
//! Browse surface sources charts and recommendations from Last.fm instead.
//!
//! ## Surface
//!
//! [`LastfmClient`] wraps the seven endpoints Browse needs — `chart.getTop*`,
//! `tag.getTop*`, `artist.getSimilar`, `track.getSimilar`,
//! `artist.getTopTracks`. Every method is `async` and returns
//! [`LastfmArtist`]/[`LastfmTrack`] *names* — Last.fm has no Spotify ids.
//! [`LastfmResolver`] maps those names back to real Spotify objects via
//! [`SpotifyApi::search`](crate::SpotifyApi::search).
//!
//! ## Configuration & graceful degradation
//!
//! The client needs a free Last.fm API key
//! (<https://www.last.fm/api/account/create>) supplied via the
//! `SPOTTYFI_LASTFM_API_KEY` environment variable. [`LastfmClient::from_env`]
//! reads it; when the variable is unset it returns
//! [`LastfmError::NotConfigured`] and *never panics*. Browse degrades calmly:
//! the Spotify category grid still renders, and Last.fm-backed sections show a
//! "set the key" note rather than an error.

mod model;
mod resolve;

use serde::de::DeserializeOwned;

pub use model::{LastfmArtist, LastfmTrack};
pub use resolve::LastfmResolver;

use model::{
    ArtistsResponse, RawError, SimilarArtistsResponse, SimilarTracksResponse, TracksResponse,
};

/// The Last.fm API root. All methods are GETs against this URL.
const API_ROOT: &str = "https://ws.audioscrobbler.com/2.0/";

/// The environment variable carrying the free Last.fm API key.
pub const API_KEY_ENV: &str = "SPOTTYFI_LASTFM_API_KEY";

/// Errors raised by the Last.fm client.
#[derive(Debug, thiserror::Error)]
pub enum LastfmError {
    /// No API key was configured (`SPOTTYFI_LASTFM_API_KEY` is unset).
    ///
    /// This is the *expected* state when the maintainer has not supplied a
    /// key; the UI treats it as "feature off", not as a failure.
    #[error("Last.fm is not configured (set the {API_KEY_ENV} environment variable)")]
    NotConfigured,

    /// Last.fm answered with an error body (`{ "error": N, "message": ... }`).
    #[error("Last.fm API error {code}: {message}")]
    Api {
        /// Last.fm's numeric error code.
        code: u32,
        /// Last.fm's human-readable message.
        message: String,
    },

    /// A network- or transport-level failure talking to Last.fm.
    #[error("network error talking to Last.fm: {0}")]
    Network(String),

    /// A response body could not be deserialised into the expected shape.
    #[error("failed to deserialise the Last.fm response: {0}")]
    Deserialize(String),
}

/// Convenience alias for results from the Last.fm client.
pub type LastfmResult<T> = Result<T, LastfmError>;

/// A client for the slice of the Last.fm API that Spottyfi's Browse uses.
///
/// Cloning is cheap — the inner `reqwest::Client` and the key are shared.
#[derive(Clone)]
pub struct LastfmClient {
    /// The shared HTTP client.
    http: reqwest::Client,
    /// The Last.fm API key.
    api_key: String,
}

impl std::fmt::Debug for LastfmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key is a secret-ish credential; never print it.
        f.debug_struct("LastfmClient").finish_non_exhaustive()
    }
}

impl LastfmClient {
    /// Build a client from an explicit API key.
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
        }
    }

    /// Build a client from the `SPOTTYFI_LASTFM_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns [`LastfmError::NotConfigured`] when the variable is unset or
    /// empty. Callers treat that as "the feature is off" and degrade — they
    /// must never propagate it as a hard failure.
    pub fn from_env() -> LastfmResult<Self> {
        match std::env::var(API_KEY_ENV) {
            Ok(key) if !key.trim().is_empty() => Ok(Self::new(key)),
            _ => Err(LastfmError::NotConfigured),
        }
    }

    /// Run a Last.fm `method` GET with extra query parameters, deserialising
    /// the JSON body into `T`.
    ///
    /// `format=json` and the API key are always appended. A Last.fm error body
    /// is detected and mapped onto [`LastfmError::Api`] before the success
    /// shape is parsed.
    async fn get<T: DeserializeOwned>(
        &self,
        method: &str,
        params: &[(&str, &str)],
    ) -> LastfmResult<T> {
        let mut query: Vec<(&str, &str)> = vec![
            ("method", method),
            ("api_key", self.api_key.as_str()),
            ("format", "json"),
        ];
        query.extend_from_slice(params);

        let response = self
            .http
            .get(API_ROOT)
            .query(&query)
            .send()
            .await
            .map_err(|e| LastfmError::Network(e.to_string()))?;

        let body = response
            .text()
            .await
            .map_err(|e| LastfmError::Network(e.to_string()))?;

        // Last.fm answers errors with HTTP 200 and an `{ "error": ... }` body,
        // so the body has to be sniffed for the error shape first.
        if let Ok(err) = serde_json::from_str::<RawError>(&body) {
            return Err(LastfmError::Api {
                code: err.error,
                message: err.message,
            });
        }

        serde_json::from_str::<T>(&body).map_err(|e| LastfmError::Deserialize(e.to_string()))
    }

    /// The global top artists chart (`chart.getTopArtists`).
    #[tracing::instrument(skip(self))]
    pub async fn chart_top_artists(&self, limit: u32) -> LastfmResult<Vec<LastfmArtist>> {
        let limit = limit.to_string();
        let resp: ArtistsResponse = self
            .get("chart.gettopartists", &[("limit", &limit)])
            .await?;
        Ok(resp.artists.artist.into_iter().map(Into::into).collect())
    }

    /// The global top tracks chart (`chart.getTopTracks`).
    #[tracing::instrument(skip(self))]
    pub async fn chart_top_tracks(&self, limit: u32) -> LastfmResult<Vec<LastfmTrack>> {
        let limit = limit.to_string();
        let resp: TracksResponse = self.get("chart.gettoptracks", &[("limit", &limit)]).await?;
        Ok(resp.tracks.track.into_iter().map(Into::into).collect())
    }

    /// The top artists for a tag/genre (`tag.getTopArtists`).
    #[tracing::instrument(skip(self))]
    pub async fn tag_top_artists(&self, tag: &str, limit: u32) -> LastfmResult<Vec<LastfmArtist>> {
        let limit = limit.to_string();
        let resp: ArtistsResponse = self
            .get("tag.gettopartists", &[("tag", tag), ("limit", &limit)])
            .await?;
        Ok(resp.artists.artist.into_iter().map(Into::into).collect())
    }

    /// The top tracks for a tag/genre (`tag.getTopTracks`).
    #[tracing::instrument(skip(self))]
    pub async fn tag_top_tracks(&self, tag: &str, limit: u32) -> LastfmResult<Vec<LastfmTrack>> {
        let limit = limit.to_string();
        let resp: TracksResponse = self
            .get("tag.gettoptracks", &[("tag", tag), ("limit", &limit)])
            .await?;
        Ok(resp.tracks.track.into_iter().map(Into::into).collect())
    }

    /// Artists similar to a named artist (`artist.getSimilar`).
    #[tracing::instrument(skip(self))]
    pub async fn similar_artists(
        &self,
        artist: &str,
        limit: u32,
    ) -> LastfmResult<Vec<LastfmArtist>> {
        let limit = limit.to_string();
        let resp: SimilarArtistsResponse = self
            .get(
                "artist.getsimilar",
                &[("artist", artist), ("limit", &limit), ("autocorrect", "1")],
            )
            .await?;
        Ok(resp
            .similarartists
            .artist
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Tracks similar to a named track (`track.getSimilar`).
    #[tracing::instrument(skip(self))]
    pub async fn similar_tracks(
        &self,
        artist: &str,
        track: &str,
        limit: u32,
    ) -> LastfmResult<Vec<LastfmTrack>> {
        let limit = limit.to_string();
        let resp: SimilarTracksResponse = self
            .get(
                "track.getsimilar",
                &[
                    ("artist", artist),
                    ("track", track),
                    ("limit", &limit),
                    ("autocorrect", "1"),
                ],
            )
            .await?;
        Ok(resp
            .similartracks
            .track
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// An artist's most-played tracks (`artist.getTopTracks`).
    #[tracing::instrument(skip(self))]
    pub async fn artist_top_tracks(
        &self,
        artist: &str,
        limit: u32,
    ) -> LastfmResult<Vec<LastfmTrack>> {
        let limit = limit.to_string();
        let resp: TracksResponse = self
            .get(
                "artist.gettoptracks",
                &[("artist", artist), ("limit", &limit), ("autocorrect", "1")],
            )
            .await?;
        Ok(resp.tracks.track.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_errors_without_a_key() {
        // The variable is process-global; this test asserts the "unset" path
        // by reading whatever the environment has. CI runs without the key.
        if std::env::var(API_KEY_ENV).is_err() {
            assert!(matches!(
                LastfmClient::from_env(),
                Err(LastfmError::NotConfigured)
            ));
        }
    }

    #[test]
    fn not_configured_has_a_clear_message() {
        let msg = LastfmError::NotConfigured.to_string();
        assert!(msg.contains(API_KEY_ENV));
    }

    #[test]
    fn debug_never_prints_the_key() {
        let client = LastfmClient::new("super-secret-key");
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("super-secret-key"));
    }
}
