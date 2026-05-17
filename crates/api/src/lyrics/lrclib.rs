//! The lrclib.net lyrics provider — the **default**, zero-setup lyrics source.
//!
//! [lrclib](https://lrclib.net/) is a free, open, community lyrics database
//! with time-synced LRC for a large catalogue. It needs **no API key and no
//! sign-up**, so unlike [`musixmatch`](super::musixmatch) and
//! [`spotify_internal`](super::spotify_internal) it is always available — it
//! is compiled in unconditionally and is the provider Spottyfi tries first.
//!
//! ## How a track is matched
//!
//! lrclib offers two endpoints:
//!
//! - `GET /api/get` — an *exact* lookup keyed by `{track_name, artist_name,
//!   album_name, duration}`. lrclib matches the duration to within a couple of
//!   seconds, so this is the precise path and is tried first.
//! - `GET /api/search` — a fuzzy search returning an array of candidates. Used
//!   as a fallback when the exact lookup misses; the candidates are then
//!   [duration-scored](super::score) and the best one is chosen, rather than
//!   blindly taking the first.
//!
//! lrclib asks clients to send a descriptive `User-Agent` identifying the app
//! and its repository — [`USER_AGENT`] does that.

use serde::Deserialize;

use super::score::{best_match, Candidate, Query};
use super::{Lyrics, LyricsError, LyricsResult, TrackRef};

/// The lrclib API root. All calls are GETs against this base.
const API_ROOT: &str = "https://lrclib.net/api";

/// The `User-Agent` lrclib asks clients to send — app name, version, repo URL.
pub const USER_AGENT: &str = concat!(
    "Spottyfi/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/spottyfi/spottyfi)"
);

/// The lrclib lyrics provider.
///
/// Cloning is cheap — the inner `reqwest::Client` is shared. Needs no
/// credentials, so there is no `from_env`: it is constructed unconditionally.
#[derive(Clone, Debug)]
pub struct LrclibProvider {
    /// The shared HTTP client.
    http: reqwest::Client,
}

impl Default for LrclibProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LrclibProvider {
    /// Build a provider with a fresh HTTP client.
    #[must_use]
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// Build a provider over a shared HTTP client.
    #[must_use]
    pub fn with_client(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// Fetch lyrics for `track`, preferring time-synced over plain.
    ///
    /// Tries the exact `/api/get` lookup first (it keys on the track duration,
    /// so it lands on the right recording); on a miss it falls back to
    /// `/api/search` and [duration-scores](super::score) the candidates.
    ///
    /// # Errors
    ///
    /// [`LyricsError::NotFound`] when lrclib knows no matching track;
    /// [`LyricsError::Network`] / [`LyricsError::Provider`] on a transport or
    /// API failure; [`LyricsError::Deserialize`] on a malformed response.
    #[tracing::instrument(skip(self), fields(uri = %track.uri))]
    pub async fn lyrics(&self, track: &TrackRef) -> LyricsResult<Lyrics> {
        // The precise path: an exact lookup keyed by duration.
        match self.get_exact(track).await {
            Ok(record) => return record.into_lyrics(),
            Err(LyricsError::NotFound) => {
                tracing::debug!("lrclib exact lookup missed; falling back to search");
            }
            Err(other) => return Err(other),
        }

        // The fallback path: fuzzy search, then duration-scored selection.
        let candidates = self.search(track).await?;
        let record = pick_best(track, candidates).ok_or(LyricsError::NotFound)?;
        record.into_lyrics()
    }

    /// Call `/api/get` — the exact, duration-keyed lookup.
    async fn get_exact(&self, track: &TrackRef) -> LyricsResult<LrclibRecord> {
        let duration_secs = track.duration.as_secs().to_string();
        let mut query: Vec<(&str, &str)> = vec![
            ("track_name", track.title.as_str()),
            ("artist_name", track.artist.as_str()),
        ];
        if !track.album.is_empty() {
            query.push(("album_name", track.album.as_str()));
        }
        if !track.duration.is_zero() {
            query.push(("duration", duration_secs.as_str()));
        }

        let response = self
            .http
            .get(format!("{API_ROOT}/get"))
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .query(&query)
            .send()
            .await
            .map_err(|e| LyricsError::Network(e.to_string()))?;

        match response.status().as_u16() {
            200 => {}
            404 => return Err(LyricsError::NotFound),
            other => {
                return Err(LyricsError::Provider(format!(
                    "lrclib /api/get returned status {other}"
                )))
            }
        }
        let body = response
            .text()
            .await
            .map_err(|e| LyricsError::Network(e.to_string()))?;
        serde_json::from_str(&body).map_err(|e| LyricsError::Deserialize(e.to_string()))
    }

    /// Call `/api/search` — the fuzzy search returning candidate records.
    async fn search(&self, track: &TrackRef) -> LyricsResult<Vec<LrclibRecord>> {
        let response = self
            .http
            .get(format!("{API_ROOT}/search"))
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .query(&[
                ("track_name", track.title.as_str()),
                ("artist_name", track.artist.as_str()),
            ])
            .send()
            .await
            .map_err(|e| LyricsError::Network(e.to_string()))?;

        match response.status().as_u16() {
            200 => {}
            404 => return Ok(Vec::new()),
            other => {
                return Err(LyricsError::Provider(format!(
                    "lrclib /api/search returned status {other}"
                )))
            }
        }
        let body = response
            .text()
            .await
            .map_err(|e| LyricsError::Network(e.to_string()))?;
        serde_json::from_str(&body).map_err(|e| LyricsError::Deserialize(e.to_string()))
    }
}

/// Choose the best-matching record from a search result by duration-scoring.
fn pick_best(track: &TrackRef, candidates: Vec<LrclibRecord>) -> Option<LrclibRecord> {
    let query = Query {
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        duration: track.duration,
    };
    let scored: Vec<Candidate> = candidates.iter().map(LrclibRecord::as_candidate).collect();
    let (index, _) = best_match(&query, &scored)?;
    candidates.into_iter().nth(index)
}

// --- Wire types ------------------------------------------------------------

/// One lrclib record, as returned by both `/api/get` and `/api/search`.
///
/// `/api/search` returns a JSON array of these; `/api/get` returns a single
/// one. The same shape is used by both, so one type covers both endpoints.
/// lrclib's JSON keys are `camelCase` (`trackName`, `syncedLyrics`, …).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LrclibRecord {
    /// The record's track name (search uses this for candidate scoring).
    #[serde(default)]
    track_name: String,
    /// The record's artist name.
    #[serde(default)]
    artist_name: String,
    /// The record's album name.
    #[serde(default)]
    album_name: String,
    /// The record's track duration in seconds (a float; may be fractional).
    #[serde(default)]
    duration: f64,
    /// Whether the track is instrumental — no lyrics, by design.
    #[serde(default)]
    instrumental: bool,
    /// Time-synced LRC text. `None`/absent when the record has no synced
    /// lyrics; lrclib also sends JSON `null` here.
    #[serde(default)]
    synced_lyrics: Option<String>,
    /// Plain, unsynced lyrics text. `None`/absent when there are none.
    #[serde(default)]
    plain_lyrics: Option<String>,
}

impl LrclibRecord {
    /// Project this record onto the scoring [`Candidate`] shape.
    fn as_candidate(&self) -> Candidate {
        Candidate {
            title: self.track_name.clone(),
            artist: self.artist_name.clone(),
            album: self.album_name.clone(),
            duration: std::time::Duration::from_secs_f64(self.duration.max(0.0)),
        }
    }

    /// Project this record onto the domain [`Lyrics`] model.
    ///
    /// Prefers the synced LRC; falls back to plain lyrics. An `instrumental`
    /// record carries no lyrics by design — that is a real, *successful*
    /// answer ([`Lyrics::Instrumental`], shown as "instrumental"), not a
    /// [`LyricsError::NotFound`] miss.
    ///
    /// # Errors
    ///
    /// [`LyricsError::NotFound`] when the record is not instrumental yet
    /// carries neither synced nor plain lyrics.
    fn into_lyrics(self) -> LyricsResult<Lyrics> {
        if self.instrumental {
            // An instrumental track legitimately has no lyrics.
            return Ok(Lyrics::Instrumental);
        }
        if let Some(lrc) = self.synced_lyrics.as_deref() {
            if !lrc.trim().is_empty() {
                return Ok(super::parse_lrc(lrc));
            }
        }
        if let Some(plain) = self.plain_lyrics.as_deref() {
            let lines: Vec<String> = plain
                .lines()
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty())
                .collect();
            if !lines.is_empty() {
                return Ok(Lyrics::Plain(lines));
            }
        }
        Err(LyricsError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn record_json() -> &'static str {
        r#"{
            "id": 3396226,
            "trackName": "Karma Police",
            "artistName": "Radiohead",
            "albumName": "OK Computer",
            "duration": 264.0,
            "instrumental": false,
            "plainLyrics": "Karma police\nArrest this man",
            "syncedLyrics": "[00:18.00]Karma police\n[00:22.00]Arrest this man"
        }"#
    }

    #[test]
    fn parses_a_get_record_into_synced_lyrics() {
        let record: LrclibRecord = serde_json::from_str(record_json()).expect("parse record");
        let Lyrics::Synced(lines) = record.into_lyrics().expect("lyrics") else {
            panic!("expected synced lyrics");
        };
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].at, Duration::from_secs(18));
        assert_eq!(lines[1].text, "Arrest this man");
    }

    #[test]
    fn falls_back_to_plain_when_no_synced_lyrics() {
        let json = r#"{"trackName":"x","artistName":"y","albumName":"z",
            "duration":100.0,"instrumental":false,
            "plainLyrics":"line one\nline two","syncedLyrics":null}"#;
        let record: LrclibRecord = serde_json::from_str(json).expect("parse record");
        let Lyrics::Plain(lines) = record.into_lyrics().expect("lyrics") else {
            panic!("expected plain lyrics");
        };
        assert_eq!(lines, vec!["line one", "line two"]);
    }

    #[test]
    fn an_instrumental_record_maps_to_the_instrumental_variant() {
        let json = r#"{"trackName":"x","artistName":"y","albumName":"z",
            "duration":100.0,"instrumental":true,
            "plainLyrics":null,"syncedLyrics":null}"#;
        let record: LrclibRecord = serde_json::from_str(json).expect("parse record");
        // An instrumental is a successful result: the Instrumental variant,
        // not a NotFound miss and not generic empty lyrics.
        let lyrics = record.into_lyrics().expect("instrumental is not an error");
        assert_eq!(lyrics, Lyrics::Instrumental);
    }

    #[test]
    fn a_record_with_no_lyrics_at_all_is_a_miss() {
        let json = r#"{"trackName":"x","artistName":"y","albumName":"z",
            "duration":100.0,"instrumental":false,
            "plainLyrics":null,"syncedLyrics":null}"#;
        let record: LrclibRecord = serde_json::from_str(json).expect("parse record");
        assert!(matches!(record.into_lyrics(), Err(LyricsError::NotFound)));
    }

    #[test]
    fn search_array_deserialises() {
        let json = r#"[
            {"trackName":"Song","artistName":"A","albumName":"Live","duration":320.0,
             "instrumental":false,"plainLyrics":"live","syncedLyrics":null},
            {"trackName":"Song","artistName":"A","albumName":"Studio","duration":201.0,
             "instrumental":false,"plainLyrics":"studio","syncedLyrics":null}
        ]"#;
        let records: Vec<LrclibRecord> = serde_json::from_str(json).expect("parse search");
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn pick_best_chooses_the_duration_match_from_search() {
        let json = r#"[
            {"trackName":"Song","artistName":"A","albumName":"Live","duration":320.0,
             "instrumental":false,"plainLyrics":"live take","syncedLyrics":null},
            {"trackName":"Song","artistName":"A","albumName":"Studio","duration":201.0,
             "instrumental":false,"plainLyrics":"studio take","syncedLyrics":null}
        ]"#;
        let records: Vec<LrclibRecord> = serde_json::from_str(json).expect("parse search");
        let track = TrackRef {
            uri: "spotify:track:x".into(),
            title: "Song".into(),
            artist: "A".into(),
            album: String::new(),
            duration: Duration::from_secs(200),
        };
        let best = pick_best(&track, records).expect("a best match");
        let Lyrics::Plain(lines) = best.into_lyrics().expect("lyrics") else {
            panic!("expected plain lyrics");
        };
        // The 201s studio cut wins over the 320s live take.
        assert_eq!(lines, vec!["studio take"]);
    }

    #[test]
    fn user_agent_identifies_the_app() {
        assert!(USER_AGENT.starts_with("Spottyfi/"));
        assert!(USER_AGENT.contains("github.com"));
    }
}
