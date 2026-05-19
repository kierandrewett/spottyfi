//! Apple Music catalog types.
//!
//! These are the cleaned-up public shapes the client returns. The raw
//! `{ "data": [ { "id", "type", "attributes": {…} } ] }` envelope is decoded
//! internally (see `client`) and flattened into these.

use std::time::Duration;

/// A song from the Apple Music catalog.
#[derive(Debug, Clone)]
pub struct Song {
    /// The Apple Music catalog id.
    pub id: String,
    /// The song title.
    pub title: String,
    /// The primary artist's name.
    pub artist_name: String,
    /// The album name.
    pub album_name: String,
    /// The track duration.
    pub duration: Duration,
    /// A resolved cover-art URL, if the catalog has artwork.
    pub artwork_url: Option<String>,
    /// The ISRC recording code — a strong cross-service de-duplication key
    /// (Spotify exposes it too).
    pub isrc: Option<String>,
    /// The track number within its album.
    pub track_number: Option<u32>,
}

/// An album from the Apple Music catalog.
#[derive(Debug, Clone)]
pub struct Album {
    /// The Apple Music catalog id.
    pub id: String,
    /// The album name.
    pub name: String,
    /// The album-artist's name.
    pub artist_name: String,
    /// A resolved cover-art URL, if the catalog has artwork.
    pub artwork_url: Option<String>,
    /// How many tracks the album has.
    pub track_count: u32,
    /// The release year, parsed from the release date when present.
    pub year: Option<u32>,
}

/// An artist from the Apple Music catalog.
#[derive(Debug, Clone)]
pub struct Artist {
    /// The Apple Music catalog id.
    pub id: String,
    /// The artist name.
    pub name: String,
    /// A resolved artist-image URL, if the catalog has artwork.
    pub artwork_url: Option<String>,
}

/// The combined result of a catalog search.
#[derive(Debug, Clone, Default)]
pub struct SearchResults {
    /// Matching songs.
    pub songs: Vec<Song>,
    /// Matching albums.
    pub albums: Vec<Album>,
    /// Matching artists.
    pub artists: Vec<Artist>,
}
