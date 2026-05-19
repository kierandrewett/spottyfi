//! The OpenSubsonic [`MusicSource`] adapter.
//!
//! Wraps a [`SubsonicClient`] and maps its native model onto the unified
//! [`entity`](crate::entity) types, so a Subsonic server slots in beside
//! Spotify with no special-casing anywhere else.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use spottyfi_subsonic::{Album as SubAlbum, Artist as SubArtist, Song as SubSong, SubsonicClient};

use crate::entity::{Album, Artist, SearchResults, Track};
use crate::identity::{SourceId, SourceKind, SourceRef};
use crate::source::{MusicSource, SourceError, SourceResult};

/// Cover art is requested at this pixel size — large enough for the now-
/// playing panel, small enough to stay snappy.
const ART_SIZE: u32 = 512;

/// A configured OpenSubsonic server, presented as a [`MusicSource`].
pub struct SubsonicSource {
    /// This source instance's id.
    id: SourceId,
    /// The display name (the server's nickname).
    name: String,
    /// The underlying API client.
    client: Arc<SubsonicClient>,
}

impl SubsonicSource {
    /// Build a source from an id, a display name and a connected client.
    #[must_use]
    pub fn new(id: SourceId, name: String, client: SubsonicClient) -> Self {
        Self {
            id,
            name,
            client: Arc::new(client),
        }
    }

    /// A [`SourceRef`] into this source for a native `id`.
    fn make_ref(&self, id: impl Into<String>) -> SourceRef {
        SourceRef::new(self.id.clone(), SourceKind::Subsonic, id)
    }

    /// Map a Subsonic song onto a unified [`Track`].
    fn track(&self, song: SubSong) -> Track {
        let artist = song.artist.clone().unwrap_or_default();
        let art_url = song
            .cover_art
            .as_deref()
            .and_then(|art| self.cover_art_url(art));
        Track {
            source: self.make_ref(song.id),
            title: song.title,
            artist: artist.clone(),
            artists: if artist.is_empty() {
                Vec::new()
            } else {
                vec![artist]
            },
            album: song.album.unwrap_or_default(),
            album_ref: song.album_id.map(|id| self.make_ref(id)),
            artist_ref: song.artist_id.map(|id| self.make_ref(id)),
            duration: Duration::from_secs(u64::from(song.duration.unwrap_or(0))),
            track_number: song.track,
            art_url,
            mbid: song.music_brainz_id,
            playable: true,
        }
    }

    /// Map a Subsonic album onto a unified [`Album`].
    fn album(&self, album: SubAlbum) -> Album {
        let art_url = album
            .cover_art
            .as_deref()
            .and_then(|art| self.cover_art_url(art));
        Album {
            source: self.make_ref(album.id),
            name: album.name,
            artist: album.artist.unwrap_or_default(),
            artist_ref: album.artist_id.map(|id| self.make_ref(id)),
            year: album.year,
            art_url,
            track_count: album.song_count.unwrap_or(0),
            mbid: album.music_brainz_id,
        }
    }

    /// Map a Subsonic artist onto a unified [`Artist`].
    fn artist(&self, artist: SubArtist) -> Artist {
        let art_url = artist.artist_image_url.clone().or_else(|| {
            artist
                .cover_art
                .as_deref()
                .and_then(|art| self.cover_art_url(art))
        });
        Artist {
            source: self.make_ref(artist.id),
            name: artist.name,
            art_url,
            mbid: artist.music_brainz_id,
        }
    }
}

#[async_trait]
impl MusicSource for SubsonicSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Subsonic
    }

    fn display_name(&self) -> &str {
        &self.name
    }

    fn can_play(&self) -> bool {
        true
    }

    fn stream_url(&self, track_id: &str) -> Option<String> {
        self.client.stream_url(track_id).ok()
    }

    fn cover_art_url(&self, art_id: &str) -> Option<String> {
        self.client.cover_art_url(art_id, Some(ART_SIZE)).ok()
    }

    async fn search(&self, query: &str, limit: u32) -> SourceResult<SearchResults> {
        let result = self
            .client
            .search(query, limit, limit, limit)
            .await
            .map_err(map_err)?;
        Ok(SearchResults {
            tracks: result.song.into_iter().map(|s| self.track(s)).collect(),
            albums: result.album.into_iter().map(|a| self.album(a)).collect(),
            artists: result.artist.into_iter().map(|a| self.artist(a)).collect(),
        })
    }

    async fn album_tracks(&self, album_id: &str) -> SourceResult<Vec<Track>> {
        let album = self.client.album(album_id).await.map_err(map_err)?;
        Ok(album.song.into_iter().map(|s| self.track(s)).collect())
    }

    async fn artist_albums(&self, artist_id: &str) -> SourceResult<Vec<Album>> {
        let artist = self.client.artist(artist_id).await.map_err(map_err)?;
        Ok(artist.album.into_iter().map(|a| self.album(a)).collect())
    }
}

/// Map a [`SubsonicError`](spottyfi_subsonic::SubsonicError) onto a
/// [`SourceError`].
fn map_err(err: spottyfi_subsonic::SubsonicError) -> SourceError {
    use spottyfi_subsonic::SubsonicError;
    match err {
        SubsonicError::Http(message) | SubsonicError::BadUrl(message) => {
            SourceError::Unavailable(message)
        }
        SubsonicError::Api { code: 70, .. } => SourceError::NotFound,
        SubsonicError::Api { code, message } => {
            SourceError::Other(format!("subsonic error {code}: {message}"))
        }
        SubsonicError::Decode(message) => SourceError::Other(message),
    }
}
