//! The internal Spotify color-lyrics endpoint — an **undocumented opt-in**.
//!
//! # ⚠ Reverse-engineered, against Spotify's Terms of Service
//!
//! Spotify's official Web API exposes **no** lyrics endpoint. The Spotify
//! desktop/mobile apps fetch synced lyrics from an *internal*, undocumented
//! endpoint on `spclient.wg.spotify.com` (the lyrics catalogue is licensed
//! from musixmatch). Calling it from a third-party client is **not a
//! supported, documented interface** and is **against Spotify's Terms of
//! Service**. It is included here only as a strictly opt-in convenience.
//!
//! Because of that, this provider:
//!
//! - is **never** enabled by default;
//! - is only constructed when the [`TOKEN_ENV`] environment variable is set —
//!   it carries the bearer token the internal endpoint expects;
//! - is documented only briefly, in `docs/questions.md`, and never surfaced
//!   prominently in the README.
//!
//! With no token set, [`SpotifyInternalProvider::from_env`] returns
//! [`LyricsError::NotConfigured`] and the lyrics layer treats this source as
//! simply absent.

use serde::Deserialize;

use super::{Lyrics, LyricsError, LyricsResult, SyncedLine, TrackRef};

/// The environment variable carrying the internal-endpoint bearer token.
///
/// Setting this is the explicit, deliberate opt-in to the undocumented path.
pub const TOKEN_ENV: &str = "SPOTTYFI_LYRICS_TOKEN";

/// The internal color-lyrics endpoint base.
///
/// The full path is `…/color-lyrics/v2/track/{spotify_track_id}`.
const ENDPOINT_BASE: &str = "https://spclient.wg.spotify.com/color-lyrics/v2/track/";

/// The internal Spotify lyrics provider (undocumented, opt-in — see the module
/// docs).
#[derive(Clone)]
pub struct SpotifyInternalProvider {
    /// The shared HTTP client.
    http: reqwest::Client,
    /// The bearer token for the internal endpoint.
    token: String,
}

impl std::fmt::Debug for SpotifyInternalProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is a credential; never print it.
        f.debug_struct("SpotifyInternalProvider")
            .finish_non_exhaustive()
    }
}

impl SpotifyInternalProvider {
    /// Build a provider from an explicit bearer token.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            token: token.into(),
        }
    }

    /// Build a provider from the `SPOTTYFI_LYRICS_TOKEN` environment variable.
    ///
    /// # Errors
    ///
    /// Returns [`LyricsError::NotConfigured`] when the variable is unset or
    /// empty — i.e. the user has not opted in to this undocumented source.
    pub fn from_env() -> LyricsResult<Self> {
        match std::env::var(TOKEN_ENV) {
            Ok(token) if !token.trim().is_empty() => Ok(Self::new(token)),
            _ => Err(LyricsError::NotConfigured),
        }
    }

    /// Fetch lyrics for `track` from the internal color-lyrics endpoint.
    ///
    /// # Errors
    ///
    /// [`LyricsError::NotFound`] when the endpoint has no lyrics for the
    /// track (HTTP 404); [`LyricsError::Provider`] on an auth failure (the
    /// token is stale — these tokens are short-lived); [`LyricsError::Network`]
    /// on a transport failure.
    pub async fn lyrics(&self, track: &TrackRef) -> LyricsResult<Lyrics> {
        let Some(track_id) = track.spotify_track_id() else {
            // Only a `spotify:track:` URI can be looked up here.
            return Err(LyricsError::NotFound);
        };
        let url = format!("{ENDPOINT_BASE}{track_id}");

        let response = self
            .http
            .get(&url)
            .query(&[("format", "json"), ("vocalRemoval", "false")])
            .bearer_auth(&self.token)
            // The internal endpoint requires this client-token-style header;
            // it is part of the reverse-engineered, undocumented contract.
            .header("app-platform", "WebPlayer")
            .send()
            .await
            .map_err(|e| LyricsError::Network(e.to_string()))?;

        match response.status().as_u16() {
            200 => {}
            404 => return Err(LyricsError::NotFound),
            401 | 403 => {
                return Err(LyricsError::Provider(
                    "the internal lyrics token was rejected (it is short-lived; refresh it)"
                        .to_owned(),
                ))
            }
            other => {
                return Err(LyricsError::Provider(format!(
                    "internal lyrics endpoint returned status {other}"
                )))
            }
        }

        let body = response
            .text()
            .await
            .map_err(|e| LyricsError::Network(e.to_string()))?;
        let payload: ColorLyricsResponse =
            serde_json::from_str(&body).map_err(|e| LyricsError::Deserialize(e.to_string()))?;

        Ok(payload.lyrics.into_lyrics())
    }
}

// --- Wire types ------------------------------------------------------------

/// The top-level color-lyrics response: `{ lyrics: {…}, colors: {…} }`.
#[derive(Debug, Deserialize)]
struct ColorLyricsResponse {
    lyrics: LyricsObject,
}

/// The `lyrics` object: a sync type and the timed/plain lines.
#[derive(Debug, Deserialize)]
struct LyricsObject {
    /// `"LINE_SYNCED"`, `"SYLLABLE_SYNCED"` or `"UNSYNCED"`.
    #[serde(rename = "syncType", default)]
    sync_type: String,
    #[serde(default)]
    lines: Vec<LyricsLine>,
}

impl LyricsObject {
    /// Project the wire object onto the domain [`Lyrics`] model.
    fn into_lyrics(self) -> Lyrics {
        let synced = self.sync_type != "UNSYNCED";
        if synced {
            let mut lines: Vec<SyncedLine> = self
                .lines
                .into_iter()
                .filter_map(|l| {
                    let millis: u64 = l.start_time_ms.parse().ok()?;
                    Some(SyncedLine {
                        at: std::time::Duration::from_millis(millis),
                        text: l.words,
                    })
                })
                .collect();
            lines.sort_by_key(|l| l.at);
            Lyrics::Synced(lines)
        } else {
            let lines: Vec<String> = self
                .lines
                .into_iter()
                .map(|l| l.words)
                .filter(|w| !w.trim().is_empty())
                .collect();
            Lyrics::Plain(lines)
        }
    }
}

/// One lyric line: its words and (for synced lyrics) its start time.
#[derive(Debug, Deserialize)]
struct LyricsLine {
    /// Milliseconds from the track start; `"0"` for unsynced lines.
    #[serde(rename = "startTimeMs", default)]
    start_time_ms: String,
    /// The line's text. The endpoint uses `"♪"` for instrumental gaps.
    #[serde(default)]
    words: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn from_env_errors_without_a_token() {
        if std::env::var(TOKEN_ENV).is_err() {
            assert!(matches!(
                SpotifyInternalProvider::from_env(),
                Err(LyricsError::NotConfigured)
            ));
        }
    }

    #[test]
    fn debug_never_prints_the_token() {
        let provider = SpotifyInternalProvider::new("secret-token");
        assert!(!format!("{provider:?}").contains("secret-token"));
    }

    #[test]
    fn parses_a_line_synced_response() {
        let json = r#"{"lyrics":{"syncType":"LINE_SYNCED","lines":[
            {"startTimeMs":"1000","words":"first"},
            {"startTimeMs":"3000","words":"second"}]},"colors":{}}"#;
        let payload: ColorLyricsResponse = serde_json::from_str(json).expect("parse response");
        let Lyrics::Synced(lines) = payload.lyrics.into_lyrics() else {
            panic!("expected synced lyrics");
        };
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].at, Duration::from_millis(1000));
        assert_eq!(lines[1].text, "second");
    }

    #[test]
    fn parses_an_unsynced_response_as_plain() {
        let json = r#"{"lyrics":{"syncType":"UNSYNCED","lines":[
            {"startTimeMs":"0","words":"line one"},
            {"startTimeMs":"0","words":"line two"}]},"colors":{}}"#;
        let payload: ColorLyricsResponse = serde_json::from_str(json).expect("parse response");
        let Lyrics::Plain(lines) = payload.lyrics.into_lyrics() else {
            panic!("expected plain lyrics");
        };
        assert_eq!(lines, vec!["line one", "line two"]);
    }
}
