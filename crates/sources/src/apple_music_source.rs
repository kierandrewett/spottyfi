//! The Apple Music [`MusicSource`] adapter — a catalog-only source.
//!
//! Apple Music audio is FairPlay-DRM protected, so this source reports
//! [`MusicSource::can_play`] as `false`: its entities are searchable and
//! browsable, and de-duplicate (by ISRC) against a playable source, but it
//! cannot stream on its own.

use std::sync::Arc;

use async_trait::async_trait;
use spottyfi_applemusic::{AppleMusicClient, Song as AmSong};

use crate::entity::{Album, Artist, SearchResults, Track};
use crate::identity::{SourceId, SourceKind, SourceRef};
use crate::source::{MusicSource, SourceError, SourceResult};

/// The fixed display name — there is only ever one Apple Music.
const DISPLAY_NAME: &str = "Apple Music";

/// Apple Music, presented as a catalog-only [`MusicSource`].
pub struct AppleMusicSource {
    /// This source instance's id.
    id: SourceId,
    /// The underlying catalog client.
    client: Arc<AppleMusicClient>,
}

impl AppleMusicSource {
    /// Build a source from an id and a connected catalog client.
    #[must_use]
    pub fn new(id: SourceId, client: AppleMusicClient) -> Self {
        Self {
            id,
            client: Arc::new(client),
        }
    }

    /// A [`SourceRef`] into this source for a native catalog `id`.
    fn make_ref(&self, id: impl Into<String>) -> SourceRef {
        SourceRef::new(self.id.clone(), SourceKind::AppleMusic, id)
    }

    /// Map an Apple Music song onto a unified, *unplayable* [`Track`].
    fn track(&self, song: AmSong) -> Track {
        Track {
            source: self.make_ref(song.id),
            title: song.title,
            artist: song.artist_name.clone(),
            artists: if song.artist_name.is_empty() {
                Vec::new()
            } else {
                vec![song.artist_name]
            },
            album: song.album_name,
            album_ref: None,
            artist_ref: None,
            duration: song.duration,
            track_number: song.track_number,
            art_url: song.artwork_url,
            mbid: None,
            // The cross-service de-dup key — lets an Apple Music search hit
            // resolve onto a playable Spotify/Subsonic copy.
            isrc: song.isrc,
            playable: false,
        }
    }
}

#[async_trait]
impl MusicSource for AppleMusicSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::AppleMusic
    }

    fn display_name(&self) -> &str {
        DISPLAY_NAME
    }

    fn can_play(&self) -> bool {
        false
    }

    fn stream_url(&self, _track_id: &str) -> Option<String> {
        // FairPlay-protected — never directly streamable.
        None
    }

    fn cover_art_url(&self, _art_id: &str) -> Option<String> {
        // Apple Music entities already carry an absolute artwork URL.
        None
    }

    async fn search(&self, query: &str, limit: u32) -> SourceResult<SearchResults> {
        let results = self.client.search(query, limit).await.map_err(map_err)?;
        Ok(SearchResults {
            tracks: results.songs.into_iter().map(|s| self.track(s)).collect(),
            albums: results
                .albums
                .into_iter()
                .map(|album| Album {
                    source: self.make_ref(album.id),
                    name: album.name,
                    artist: album.artist_name,
                    artist_ref: None,
                    year: album.year,
                    art_url: album.artwork_url,
                    track_count: album.track_count,
                    mbid: None,
                })
                .collect(),
            artists: results
                .artists
                .into_iter()
                .map(|artist| Artist {
                    source: self.make_ref(artist.id),
                    name: artist.name,
                    art_url: artist.artwork_url,
                    mbid: None,
                })
                .collect(),
        })
    }

    async fn album_tracks(&self, _album_id: &str) -> SourceResult<Vec<Track>> {
        // The catalog client does not yet fetch album track relationships;
        // Apple Music drill-down arrives with that. Search is the live path.
        Ok(Vec::new())
    }

    async fn artist_albums(&self, _artist_id: &str) -> SourceResult<Vec<Album>> {
        Ok(Vec::new())
    }
}

/// Map an [`AppleMusicError`](spottyfi_applemusic::AppleMusicError) onto a
/// [`SourceError`].
fn map_err(err: spottyfi_applemusic::AppleMusicError) -> SourceError {
    use spottyfi_applemusic::AppleMusicError;
    match err {
        AppleMusicError::Http(message) => SourceError::Unavailable(message),
        AppleMusicError::Api { status: 404, .. } => SourceError::NotFound,
        AppleMusicError::Api { status, message } => {
            SourceError::Other(format!("apple music error {status}: {message}"))
        }
        AppleMusicError::Decode(message) => SourceError::Other(message),
    }
}
