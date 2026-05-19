//! The set of configured sources the app searches and browses at once.

use std::sync::Arc;

use crate::dedup::{dedup_albums, dedup_artists, dedup_tracks, Deduped, DedupedTrack};
use crate::entity::{Album, Artist};
use crate::identity::SourceId;
use crate::source::MusicSource;

/// A whole-library search, merged across every source and de-duplicated.
#[derive(Debug, Clone, Default)]
pub struct DedupedSearch {
    /// Matching tracks, one entry per real song with its alternative sources.
    pub tracks: Vec<DedupedTrack>,
    /// Matching albums.
    pub albums: Vec<Deduped<Album>>,
    /// Matching artists.
    pub artists: Vec<Deduped<Artist>>,
}

/// The registry of configured music sources.
///
/// The app holds one of these. Searching it queries every source
/// concurrently and de-duplicates the merged result, so the user sees one
/// list — "the best available source, switchable in the player" — rather
/// than the same song repeated per backend.
#[derive(Default)]
pub struct SourceRegistry {
    /// The configured sources, in user-defined order.
    sources: Vec<Arc<dyn MusicSource>>,
}

impl SourceRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a source, replacing any existing source with the same id.
    pub fn add(&mut self, source: Arc<dyn MusicSource>) {
        self.remove(&source.id().clone());
        self.sources.push(source);
    }

    /// Remove the source with `id`, if present.
    pub fn remove(&mut self, id: &SourceId) {
        self.sources.retain(|source| source.id() != id);
    }

    /// Every configured source, in order.
    #[must_use]
    pub fn sources(&self) -> &[Arc<dyn MusicSource>] {
        &self.sources
    }

    /// The source with `id`, if configured.
    #[must_use]
    pub fn get(&self, id: &SourceId) -> Option<Arc<dyn MusicSource>> {
        self.sources
            .iter()
            .find(|source| source.id() == id)
            .map(Arc::clone)
    }

    /// How many sources are configured.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sources.len()
    }

    /// Whether no source is configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Search every source concurrently, then merge and de-duplicate.
    ///
    /// A source that errors is logged and skipped — one unreachable server
    /// never breaks search across the others.
    pub async fn search_all(&self, query: &str, limit: u32) -> DedupedSearch {
        let searches = self.sources.iter().map(|source| async move {
            (
                source.display_name().to_owned(),
                source.search(query, limit).await,
            )
        });
        let results = futures::future::join_all(searches).await;

        let mut tracks = Vec::new();
        let mut albums = Vec::new();
        let mut artists = Vec::new();
        for (name, result) in results {
            match result {
                Ok(found) => {
                    tracks.extend(found.tracks);
                    albums.extend(found.albums);
                    artists.extend(found.artists);
                }
                Err(err) => tracing::warn!(source = %name, %err, "search failed for a source"),
            }
        }
        DedupedSearch {
            tracks: dedup_tracks(tracks),
            albums: dedup_albums(albums),
            artists: dedup_artists(artists),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{SearchResults, Track};
    use crate::identity::{SourceKind, SourceRef};
    use crate::source::{SourceError, SourceResult};
    use async_trait::async_trait;
    use std::time::Duration;

    struct FakeSource {
        id: SourceId,
        kind: SourceKind,
        track_title: &'static str,
    }

    #[async_trait]
    impl MusicSource for FakeSource {
        fn id(&self) -> &SourceId {
            &self.id
        }
        fn kind(&self) -> SourceKind {
            self.kind
        }
        fn display_name(&self) -> &str {
            "fake"
        }
        fn can_play(&self) -> bool {
            true
        }
        fn stream_url(&self, _: &str) -> Option<String> {
            None
        }
        fn cover_art_url(&self, _: &str) -> Option<String> {
            None
        }
        async fn search(&self, _: &str, _: u32) -> SourceResult<SearchResults> {
            Ok(SearchResults {
                tracks: vec![Track {
                    source: SourceRef::new(self.id.clone(), self.kind, "t"),
                    title: self.track_title.to_owned(),
                    artist: "Artist".to_owned(),
                    artists: vec!["Artist".to_owned()],
                    album: "Album".to_owned(),
                    album_ref: None,
                    artist_ref: None,
                    duration: Duration::from_secs(200),
                    track_number: None,
                    art_url: None,
                    mbid: None,
                    isrc: None,
                    playable: true,
                }],
                albums: Vec::new(),
                artists: Vec::new(),
            })
        }
        async fn album_tracks(&self, _: &str) -> SourceResult<Vec<Track>> {
            Err(SourceError::NotFound)
        }
        async fn artist_albums(&self, _: &str) -> SourceResult<Vec<Album>> {
            Err(SourceError::NotFound)
        }
    }

    #[tokio::test]
    async fn search_all_merges_and_dedupes() {
        let mut registry = SourceRegistry::new();
        registry.add(Arc::new(FakeSource {
            id: SourceId("spotify".to_owned()),
            kind: SourceKind::Spotify,
            track_title: "Shared Song",
        }));
        registry.add(Arc::new(FakeSource {
            id: SourceId("subsonic".to_owned()),
            kind: SourceKind::Subsonic,
            track_title: "shared song",
        }));
        let search = registry.search_all("song", 10).await;
        assert_eq!(search.tracks.len(), 1, "the two sources collapse to one");
        assert_eq!(search.tracks[0].source_count(), 2);
        assert_eq!(search.tracks[0].primary.source.kind, SourceKind::Subsonic);
    }

    #[test]
    fn add_replaces_a_source_with_the_same_id() {
        let mut registry = SourceRegistry::new();
        let source = || {
            Arc::new(FakeSource {
                id: SourceId("s".to_owned()),
                kind: SourceKind::Spotify,
                track_title: "x",
            })
        };
        registry.add(source());
        registry.add(source());
        assert_eq!(registry.len(), 1);
    }
}
