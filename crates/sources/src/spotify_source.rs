//! The Spotify [`MusicSource`] adapter.
//!
//! Wraps the Spotify Web API ([`SpotifyApi`]) and maps its `models`-shaped
//! results onto the unified [`entity`](crate::entity) types, so Spotify sits
//! beside OpenSubsonic with no special-casing. Spotify audio
//! plays through librespot, so [`MusicSource::stream_url`] is `None` — the
//! engine routes a Spotify track to its librespot backend by source kind.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use spottyfi_api::{SearchType, SpotifyApi};
use spottyfi_models as m;

use crate::entity::{Album, Artist, SearchResults, Track};
use crate::identity::{SourceId, SourceKind, SourceRef};
use crate::source::{MusicSource, SourceError, SourceResult};

/// The fixed display name — there is only ever one Spotify.
const DISPLAY_NAME: &str = "Spotify";

/// Spotify, presented as a [`MusicSource`].
pub struct SpotifySource {
    /// This source instance's id.
    id: SourceId,
    /// The Spotify Web API client.
    api: Arc<dyn SpotifyApi>,
}

impl SpotifySource {
    /// Build a source from an id and a Spotify API client.
    #[must_use]
    pub fn new(id: SourceId, api: Arc<dyn SpotifyApi>) -> Self {
        Self { id, api }
    }

    /// A [`SourceRef`] into this source for a native Spotify `id`.
    fn make_ref(&self, id: impl Into<String>) -> SourceRef {
        SourceRef::new(self.id.clone(), SourceKind::Spotify, id)
    }

    /// Map a full Spotify track onto a unified [`Track`] — `None` for a
    /// local-file track, which has no Spotify id and cannot be played.
    fn track(&self, track: m::Track) -> Option<Track> {
        let id = track.id?;
        Some(Track {
            source: self.make_ref(id.0),
            title: track.name,
            artist: primary_artist(&track.artists),
            artists: track.artists.iter().map(|a| a.name.clone()).collect(),
            album: track.album.name.clone(),
            album_ref: track.album.id.map(|aid| self.make_ref(aid.0)),
            artist_ref: track
                .artists
                .first()
                .and_then(|a| a.id.clone())
                .map(|aid| self.make_ref(aid.0)),
            duration: Duration::from_millis(u64::from(track.duration_ms)),
            track_number: Some(track.track_number),
            art_url: track.album.images.first().map(|image| image.url.clone()),
            mbid: None,
            isrc: None,
            playable: true,
        })
    }

    /// Map an album's simplified track onto a unified [`Track`], filling the
    /// album fields the simplified shape omits from its parent album.
    fn simplified_track(&self, track: m::SimplifiedTrack, album: &m::Album) -> Option<Track> {
        let id = track.id?;
        Some(Track {
            source: self.make_ref(id.0),
            title: track.name,
            artist: primary_artist(&track.artists),
            artists: track.artists.iter().map(|a| a.name.clone()).collect(),
            album: album.name.clone(),
            album_ref: Some(self.make_ref(album.id.0.clone())),
            artist_ref: track
                .artists
                .first()
                .and_then(|a| a.id.clone())
                .map(|aid| self.make_ref(aid.0)),
            duration: Duration::from_millis(u64::from(track.duration_ms)),
            track_number: Some(track.track_number),
            art_url: album.images.first().map(|image| image.url.clone()),
            mbid: None,
            isrc: None,
            playable: true,
        })
    }

    /// Map a full Spotify artist onto a unified [`Artist`].
    fn artist(&self, artist: m::Artist) -> Artist {
        Artist {
            source: self.make_ref(artist.id.0),
            name: artist.name,
            art_url: artist.images.first().map(|image| image.url.clone()),
            mbid: None,
        }
    }

    /// Map a simplified Spotify album onto a unified [`Album`] — `None` for a
    /// local-file album, which has no id.
    fn simplified_album(&self, album: m::SimplifiedAlbum) -> Option<Album> {
        let id = album.id?;
        Some(Album {
            source: self.make_ref(id.0),
            name: album.name,
            artist: primary_artist(&album.artists),
            artist_ref: album
                .artists
                .first()
                .and_then(|a| a.id.clone())
                .map(|aid| self.make_ref(aid.0)),
            year: album.release_date.as_deref().and_then(parse_year),
            art_url: album.images.first().map(|image| image.url.clone()),
            // Spotify's simplified album (search results) omits the count.
            track_count: None,
            mbid: None,
        })
    }
}

#[async_trait]
impl MusicSource for SpotifySource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Spotify
    }

    fn display_name(&self) -> &str {
        DISPLAY_NAME
    }

    fn can_play(&self) -> bool {
        true
    }

    fn stream_url(&self, _track_id: &str) -> Option<String> {
        // Spotify audio plays through librespot, not a plain URL.
        None
    }

    fn cover_art_url(&self, _art_id: &str) -> Option<String> {
        // Spotify entities already carry absolute image URLs.
        None
    }

    async fn search(&self, query: &str, limit: u32) -> SourceResult<SearchResults> {
        let results = self
            .api
            .search(
                query,
                &[SearchType::Track, SearchType::Album, SearchType::Artist],
                limit,
            )
            .await
            .map_err(map_err)?;
        Ok(SearchResults {
            tracks: results
                .tracks
                .items
                .into_iter()
                .filter_map(|track| self.track(track))
                .collect(),
            albums: results
                .albums
                .items
                .into_iter()
                .filter_map(|album| self.simplified_album(album))
                .collect(),
            artists: results
                .artists
                .items
                .into_iter()
                .map(|artist| self.artist(artist))
                .collect(),
        })
    }

    async fn album_tracks(&self, album_id: &str) -> SourceResult<Vec<Track>> {
        let album = self.api.album(album_id).await.map_err(map_err)?;
        Ok(album
            .tracks
            .items
            .clone()
            .into_iter()
            .filter_map(|track| self.simplified_track(track, &album))
            .collect())
    }

    async fn artist_albums(&self, artist_id: &str) -> SourceResult<Vec<Album>> {
        let page = self
            .api
            .artist_albums(artist_id, 0, 50)
            .await
            .map_err(map_err)?;
        Ok(page
            .items
            .into_iter()
            .filter_map(|album| self.simplified_album(album))
            .collect())
    }
}

/// The primary (first-billed) artist's name, or empty when there is none.
fn primary_artist(artists: &[m::SimplifiedArtist]) -> String {
    artists
        .first()
        .map(|artist| artist.name.clone())
        .unwrap_or_default()
}

/// Extract the year from a Spotify release date (`YYYY`, `YYYY-MM`, …).
fn parse_year(release_date: &str) -> Option<u32> {
    release_date.get(0..4).and_then(|year| year.parse().ok())
}

/// Map a [`spottyfi_api::ApiError`] onto a [`SourceError`].
fn map_err(err: spottyfi_api::ApiError) -> SourceError {
    SourceError::Other(err.to_string())
}
