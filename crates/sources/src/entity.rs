//! Source-neutral music entities.
//!
//! Every source (Spotify, Subsonic, Apple Music) maps its own native model
//! onto these unified types, so the rest of the app — search, browse, the
//! player, de-duplication — works against one vocabulary and never has to
//! special-case a backend. Each entity carries a [`SourceRef`] so its origin
//! is always known and can be shown on a badge.

use std::time::Duration;

use crate::identity::SourceRef;

/// A track from some source.
#[derive(Debug, Clone)]
pub struct Track {
    /// Where this track came from.
    pub source: SourceRef,
    /// The track title.
    pub title: String,
    /// The primary artist's display name.
    pub artist: String,
    /// Every credited artist, in billing order.
    pub artists: Vec<String>,
    /// The album name.
    pub album: String,
    /// A reference to the album, for drill-down (when the source provides it).
    pub album_ref: Option<SourceRef>,
    /// A reference to the primary artist, for drill-down.
    pub artist_ref: Option<SourceRef>,
    /// The track duration.
    pub duration: Duration,
    /// The track number within its album, when known.
    pub track_number: Option<u32>,
    /// A cover-art URL, when the source exposes one directly.
    pub art_url: Option<String>,
    /// The MusicBrainz recording id, when known — a high-confidence
    /// de-duplication key.
    pub mbid: Option<String>,
    /// The ISRC recording code, when known. Spotify and Apple Music both
    /// expose it, so it is the strongest cross-source de-duplication key.
    pub isrc: Option<String>,
    /// Whether this track can actually be played from this source. `false`
    /// for a catalog-only source (Apple Music before CEF playback) — it can
    /// still be shown and de-duplicated against a playable source.
    pub playable: bool,
}

/// An album from some source.
#[derive(Debug, Clone)]
pub struct Album {
    /// Where this album came from.
    pub source: SourceRef,
    /// The album name.
    pub name: String,
    /// The album-artist's display name.
    pub artist: String,
    /// A reference to the album-artist, for drill-down.
    pub artist_ref: Option<SourceRef>,
    /// The release year, when known.
    pub year: Option<u32>,
    /// A cover-art URL, when the source exposes one directly.
    pub art_url: Option<String>,
    /// How many tracks the album has, when the source reports it. `None`
    /// from a source whose search results omit it (e.g. Spotify's simplified
    /// album shape) — distinct from a known `Some(0)`.
    pub track_count: Option<u32>,
    /// The MusicBrainz release id, when known.
    pub mbid: Option<String>,
}

/// An artist from some source.
#[derive(Debug, Clone)]
pub struct Artist {
    /// Where this artist came from.
    pub source: SourceRef,
    /// The artist name.
    pub name: String,
    /// An artist-image URL, when the source exposes one directly.
    pub art_url: Option<String>,
    /// The MusicBrainz artist id, when known.
    pub mbid: Option<String>,
}

/// The combined result of a search across one source.
#[derive(Debug, Clone, Default)]
pub struct SearchResults {
    /// Matching tracks.
    pub tracks: Vec<Track>,
    /// Matching albums.
    pub albums: Vec<Album>,
    /// Matching artists.
    pub artists: Vec<Artist>,
}

impl SearchResults {
    /// Whether the search found nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.albums.is_empty() && self.artists.is_empty()
    }
}
