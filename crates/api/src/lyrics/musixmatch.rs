//! The musixmatch lyrics provider — the *legitimate* lyrics path.
//!
//! [musixmatch](https://developer.musixmatch.com/) offers a documented Web API
//! with time-synced ("richsync"/"subtitle") lyrics for a large catalogue. It
//! needs an API key, supplied via the [`API_KEY_ENV`] environment variable.
//!
//! This whole module is gated behind the **`musixmatch` Cargo feature**, which
//! is **off by default** — a build without the feature carries no musixmatch
//! code at all. With the feature on but no key configured,
//! [`MusixmatchProvider::from_env`] returns [`LyricsError::NotConfigured`] and
//! the lyrics layer degrades to "no source configured" rather than failing.
//!
//! ## How a track is matched
//!
//! Spottyfi knows a track's Spotify id, title and artist. musixmatch is keyed
//! by its own ids, so a lookup is two steps: `matcher.track.get` (resolve the
//! `{title, artist}` pair to a musixmatch track) then `track.subtitle.get` /
//! `track.lyrics.get` for the timed or plain lyrics. The subtitle body is LRC,
//! which [`parse_lrc`](super::parse_lrc) understands directly.

use serde::Deserialize;

use super::{Lyrics, LyricsError, LyricsResult, TrackRef};

/// The environment variable carrying the musixmatch API key.
pub const API_KEY_ENV: &str = "SPOTTYFI_MUSIXMATCH_KEY";

/// The musixmatch API root. All calls are GETs against this base.
const API_ROOT: &str = "https://api.musixmatch.com/ws/1.1/";

/// The musixmatch lyrics provider.
///
/// Cloning is cheap — the inner `reqwest::Client` and key are shared.
#[derive(Clone)]
pub struct MusixmatchProvider {
    /// The shared HTTP client.
    http: reqwest::Client,
    /// The musixmatch API key.
    api_key: String,
}

impl std::fmt::Debug for MusixmatchProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key is a credential; never print it.
        f.debug_struct("MusixmatchProvider").finish_non_exhaustive()
    }
}

impl MusixmatchProvider {
    /// Build a provider from an explicit API key.
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
        }
    }

    /// Build a provider from the `SPOTTYFI_MUSIXMATCH_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns [`LyricsError::NotConfigured`] when the variable is unset or
    /// empty. Callers treat that as "this source is off" and degrade.
    pub fn from_env() -> LyricsResult<Self> {
        match std::env::var(API_KEY_ENV) {
            Ok(key) if !key.trim().is_empty() => Ok(Self::new(key)),
            _ => Err(LyricsError::NotConfigured),
        }
    }

    /// Run a musixmatch GET, deserialising the `message.body` into `T`.
    ///
    /// musixmatch wraps every response in `{ message: { header, body } }`; the
    /// header carries a `status_code` that is checked before the body is read.
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: &[(&str, &str)],
    ) -> LyricsResult<T> {
        let mut query: Vec<(&str, &str)> = vec![("apikey", self.api_key.as_str())];
        query.extend_from_slice(params);

        let url = format!("{API_ROOT}{method}");
        let response = self
            .http
            .get(&url)
            .query(&query)
            .send()
            .await
            .map_err(|e| LyricsError::Network(e.to_string()))?;
        let body = response
            .text()
            .await
            .map_err(|e| LyricsError::Network(e.to_string()))?;

        let envelope: Envelope<T> =
            serde_json::from_str(&body).map_err(|e| LyricsError::Deserialize(e.to_string()))?;

        match envelope.message.header.status_code {
            200 => envelope.message.body.ok_or(LyricsError::NotFound),
            // 404 from a matcher/lyrics call simply means "no lyrics here".
            404 => Err(LyricsError::NotFound),
            401..=403 => Err(LyricsError::Provider(format!(
                "musixmatch rejected the request (status {}); check the API key/quota",
                envelope.message.header.status_code
            ))),
            other => Err(LyricsError::Provider(format!(
                "musixmatch returned status {other}"
            ))),
        }
    }

    /// Fetch lyrics for `track`, preferring time-synced over plain.
    ///
    /// First resolves the `{title, artist}` pair to a musixmatch track id via
    /// `matcher.track.get`, then asks for the subtitle (LRC, time-synced); if
    /// the track has no subtitle it falls back to the plain `track.lyrics.get`.
    ///
    /// # Errors
    ///
    /// [`LyricsError::NotFound`] when musixmatch knows neither the track nor
    /// any lyrics for it; [`LyricsError::Network`] / [`LyricsError::Provider`]
    /// on a transport or API failure.
    pub async fn lyrics(&self, track: &TrackRef) -> LyricsResult<Lyrics> {
        // Step 1 — resolve the track to a musixmatch id.
        let matched: MatcherBody = self
            .get(
                "matcher.track.get",
                &[
                    ("q_track", track.title.as_str()),
                    ("q_artist", track.artist.as_str()),
                ],
            )
            .await?;
        let track_id = matched.track.track_id.to_string();

        // Step 2 — try the time-synced subtitle (LRC) first.
        match self
            .get::<SubtitleBody>(
                "track.subtitle.get",
                &[("track_id", track_id.as_str()), ("subtitle_format", "lrc")],
            )
            .await
        {
            Ok(body) => {
                let lrc = body.subtitle.subtitle_body;
                if !lrc.trim().is_empty() {
                    return Ok(super::parse_lrc(&lrc));
                }
            }
            // No subtitle — fall through to the plain lyrics below.
            Err(LyricsError::NotFound) => {}
            Err(other) => return Err(other),
        }

        // Step 3 — fall back to plain, unsynced lyrics.
        let body: LyricsBody = self
            .get("track.lyrics.get", &[("track_id", track_id.as_str())])
            .await?;
        let plain = body.lyrics.lyrics_body;
        if plain.trim().is_empty() {
            return Err(LyricsError::NotFound);
        }
        let lines: Vec<String> = plain
            .lines()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            // musixmatch appends a tracking disclaimer line; drop it.
            .filter(|l| !l.contains("*******"))
            .filter(|l| !l.starts_with("This Lyrics is NOT for Commercial use"))
            .collect();
        Ok(Lyrics::Plain(lines))
    }
}

// --- Wire types ------------------------------------------------------------

/// The `{ message: { header, body } }` envelope wrapping every response.
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    message: Message<T>,
}

/// The `message` object: a header and an optional typed body.
#[derive(Debug, Deserialize)]
struct Message<T> {
    header: Header,
    body: Option<T>,
}

/// The response header, carrying musixmatch's own status code.
#[derive(Debug, Deserialize)]
struct Header {
    status_code: i32,
}

/// The body of `matcher.track.get`.
#[derive(Debug, Deserialize)]
struct MatcherBody {
    track: MatchedTrack,
}

/// The matched track — only its id is needed for the follow-up calls.
#[derive(Debug, Deserialize)]
struct MatchedTrack {
    track_id: u64,
}

/// The body of `track.subtitle.get`.
#[derive(Debug, Deserialize)]
struct SubtitleBody {
    subtitle: Subtitle,
}

/// The subtitle object; `subtitle_body` is LRC text.
#[derive(Debug, Deserialize)]
struct Subtitle {
    #[serde(default)]
    subtitle_body: String,
}

/// The body of `track.lyrics.get`.
#[derive(Debug, Deserialize)]
struct LyricsBody {
    lyrics: PlainLyrics,
}

/// The plain-lyrics object; `lyrics_body` is unsynced text.
#[derive(Debug, Deserialize)]
struct PlainLyrics {
    #[serde(default)]
    lyrics_body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_errors_without_a_key() {
        if std::env::var(API_KEY_ENV).is_err() {
            assert!(matches!(
                MusixmatchProvider::from_env(),
                Err(LyricsError::NotConfigured)
            ));
        }
    }

    #[test]
    fn debug_never_prints_the_key() {
        let provider = MusixmatchProvider::new("super-secret-key");
        let rendered = format!("{provider:?}");
        assert!(!rendered.contains("super-secret-key"));
    }

    #[test]
    fn envelope_deserialises_status_and_body() {
        let json = r#"{"message":{"header":{"status_code":200},
            "body":{"track":{"track_id":42}}}}"#;
        let envelope: Envelope<MatcherBody> = serde_json::from_str(json).expect("parse envelope");
        assert_eq!(envelope.message.header.status_code, 200);
        assert_eq!(envelope.message.body.expect("body").track.track_id, 42);
    }

    #[test]
    fn subtitle_body_carries_lrc() {
        let json = r#"{"message":{"header":{"status_code":200},
            "body":{"subtitle":{"subtitle_body":"[00:01.00]hi"}}}}"#;
        let envelope: Envelope<SubtitleBody> = serde_json::from_str(json).expect("parse subtitle");
        assert_eq!(
            envelope.message.body.expect("body").subtitle.subtitle_body,
            "[00:01.00]hi"
        );
    }
}
