//! The Last.fm JSON response shapes and their projection onto plain types.
//!
//! Last.fm's `format=json` responses are deeply nested and stringly-typed —
//! counts come back as strings, lists are wrapped in a named object. The
//! `Raw*` types here mirror that shape exactly so `serde_json` can parse them;
//! the public [`LastfmArtist`] / [`LastfmTrack`] types are the flat,
//! convenient projections the rest of the crate uses.

use serde::{Deserialize, Serialize};

/// A Last.fm artist: just a name (Last.fm has no Spotify ids).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastfmArtist {
    /// The artist's name, used verbatim as the search query when resolving
    /// the artist back to a Spotify object.
    pub name: String,
}

/// A Last.fm track: a title plus its artist's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastfmTrack {
    /// The track title.
    pub name: String,
    /// The performing artist's name.
    pub artist: String,
}

// --- Raw wire types --------------------------------------------------------

/// The `{ "error": N, "message": "..." }` body Last.fm returns on failure.
#[derive(Debug, Deserialize)]
pub(super) struct RawError {
    /// Last.fm's numeric error code.
    pub error: u32,
    /// The human-readable error message.
    pub message: String,
}

/// A raw artist object: Last.fm nests the name under `name`.
#[derive(Debug, Deserialize)]
pub(super) struct RawArtist {
    /// The artist name.
    pub name: String,
}

/// A raw track object. The artist is either a bare string (`chart.getTopTracks`,
/// `artist.getTopTracks`) or a nested `{ "name": ... }` object
/// (`tag.getTopTracks`, `track.getSimilar`) — [`RawTrackArtist`] accepts both.
#[derive(Debug, Deserialize)]
pub(super) struct RawTrack {
    /// The track title.
    pub name: String,
    /// The performing artist, in whichever shape this endpoint uses.
    pub artist: RawTrackArtist,
}

/// The two shapes Last.fm uses for a track's `artist` field.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawTrackArtist {
    /// A bare artist name string.
    Name(String),
    /// A nested `{ "name": ... }` object.
    Object {
        /// The artist name.
        name: String,
    },
}

impl RawTrackArtist {
    /// The artist name, whichever shape it arrived in.
    fn into_name(self) -> String {
        match self {
            RawTrackArtist::Name(name) => name,
            RawTrackArtist::Object { name } => name,
        }
    }
}

impl From<RawArtist> for LastfmArtist {
    fn from(raw: RawArtist) -> Self {
        Self { name: raw.name }
    }
}

impl From<RawTrack> for LastfmTrack {
    fn from(raw: RawTrack) -> Self {
        Self {
            name: raw.name,
            artist: raw.artist.into_name(),
        }
    }
}

/// `chart.getTopArtists` / `tag.getTopArtists`: `{ "artists": { "artist": [..] } }`.
#[derive(Debug, Deserialize)]
pub(super) struct ArtistsResponse {
    /// The wrapped artist list.
    pub artists: ArtistList,
}

/// `artist.getSimilar`: `{ "similarartists": { "artist": [..] } }`.
#[derive(Debug, Deserialize)]
pub(super) struct SimilarArtistsResponse {
    /// The wrapped artist list.
    pub similarartists: ArtistList,
}

/// The inner `{ "artist": [..] }` wrapper Last.fm puts every artist list in.
#[derive(Debug, Deserialize)]
pub(super) struct ArtistList {
    /// The artists themselves.
    #[serde(default)]
    pub artist: Vec<RawArtist>,
}

/// `chart.getTopTracks` / `tag.getTopTracks` / `artist.getTopTracks`:
/// `{ "tracks": { "track": [..] } }`.
#[derive(Debug, Deserialize)]
pub(super) struct TracksResponse {
    /// The wrapped track list.
    pub tracks: TrackList,
}

/// `track.getSimilar`: `{ "similartracks": { "track": [..] } }`.
#[derive(Debug, Deserialize)]
pub(super) struct SimilarTracksResponse {
    /// The wrapped track list.
    pub similartracks: TrackList,
}

/// The inner `{ "track": [..] }` wrapper Last.fm puts every track list in.
#[derive(Debug, Deserialize)]
pub(super) struct TrackList {
    /// The tracks themselves.
    #[serde(default)]
    pub track: Vec<RawTrack>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chart_top_artists() {
        // The shape `chart.getTopArtists` returns, trimmed to the fields used.
        let json = r#"{
            "artists": {
                "artist": [
                    { "name": "Radiohead", "playcount": "12345", "mbid": "x" },
                    { "name": "The Beatles", "playcount": "9999" }
                ]
            }
        }"#;
        let parsed: ArtistsResponse = serde_json::from_str(json).expect("valid json");
        let artists: Vec<LastfmArtist> =
            parsed.artists.artist.into_iter().map(Into::into).collect();
        assert_eq!(artists.len(), 2);
        assert_eq!(artists[0].name, "Radiohead");
        assert_eq!(artists[1].name, "The Beatles");
    }

    #[test]
    fn parses_chart_top_tracks_with_bare_artist_string() {
        // `chart.getTopTracks` gives `artist` as a nested object.
        let json = r#"{
            "tracks": {
                "track": [
                    { "name": "Creep", "artist": { "name": "Radiohead" } }
                ]
            }
        }"#;
        let parsed: TracksResponse = serde_json::from_str(json).expect("valid json");
        let tracks: Vec<LastfmTrack> = parsed.tracks.track.into_iter().map(Into::into).collect();
        assert_eq!(tracks[0].name, "Creep");
        assert_eq!(tracks[0].artist, "Radiohead");
    }

    #[test]
    fn parses_artist_top_tracks_with_string_artist() {
        // `artist.getTopTracks` gives `artist` the same nested-object shape;
        // a bare string is also accepted by the untagged enum.
        let json = r#"{
            "tracks": {
                "track": [
                    { "name": "Song A", "artist": "Plain String Artist" }
                ]
            }
        }"#;
        let parsed: TracksResponse = serde_json::from_str(json).expect("valid json");
        let tracks: Vec<LastfmTrack> = parsed.tracks.track.into_iter().map(Into::into).collect();
        assert_eq!(tracks[0].artist, "Plain String Artist");
    }

    #[test]
    fn parses_similar_artists() {
        let json = r#"{
            "similarartists": {
                "artist": [ { "name": "Thom Yorke" } ]
            }
        }"#;
        let parsed: SimilarArtistsResponse = serde_json::from_str(json).expect("valid json");
        assert_eq!(parsed.similarartists.artist[0].name, "Thom Yorke");
    }

    #[test]
    fn parses_similar_tracks() {
        let json = r#"{
            "similartracks": {
                "track": [ { "name": "Karma Police", "artist": { "name": "Radiohead" } } ]
            }
        }"#;
        let parsed: SimilarTracksResponse = serde_json::from_str(json).expect("valid json");
        let tracks: Vec<LastfmTrack> = parsed
            .similartracks
            .track
            .into_iter()
            .map(Into::into)
            .collect();
        assert_eq!(tracks[0].name, "Karma Police");
        assert_eq!(tracks[0].artist, "Radiohead");
    }

    #[test]
    fn parses_an_error_body() {
        let json = r#"{ "error": 10, "message": "Invalid API key" }"#;
        let parsed: RawError = serde_json::from_str(json).expect("valid json");
        assert_eq!(parsed.error, 10);
        assert_eq!(parsed.message, "Invalid API key");
    }

    #[test]
    fn an_empty_list_parses_to_no_items() {
        // A tag with no tracks omits the `track` array entirely.
        let json = r#"{ "tracks": {} }"#;
        let parsed: TracksResponse = serde_json::from_str(json).expect("valid json");
        assert!(parsed.tracks.track.is_empty());
    }
}
