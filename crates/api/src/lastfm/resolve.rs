//! Resolving Last.fm names back to Spotify objects.
//!
//! Last.fm returns artist/track *names*, not Spotify ids. To make a Last.fm
//! suggestion navigable and playable inside Spottyfi it must be turned into a
//! real Spotify object. The [`LastfmResolver`] does that by running each name
//! through [`SpotifyApi::search`] and picking the best match.
//!
//! Matching is deliberately simple and forgiving: a case-insensitive exact
//! name match is preferred, otherwise Spotify's own top-ranked result is
//! taken (its search is already relevance-ordered). A track search additionally
//! prefers a result whose artist also matches the Last.fm artist name.

use std::sync::Arc;

use spottyfi_models::{Artist, Track};

use super::model::{LastfmArtist, LastfmTrack};
use crate::error::ApiResult;
use crate::traits::{SearchType, SpotifyApi};

/// How many search results to fetch per name when resolving.
const RESOLVE_LIMIT: u32 = 5;

/// Maps Last.fm names onto Spotify objects via the Web API search endpoint.
///
/// Holds an `Arc<dyn SpotifyApi>` so it can be cheaply cloned and moved into
/// an async task.
#[derive(Clone)]
pub struct LastfmResolver {
    /// The Spotify Web API client used to search for each name.
    api: Arc<dyn SpotifyApi>,
}

impl LastfmResolver {
    /// Build a resolver over a Spotify Web API client.
    #[must_use]
    pub fn new(api: Arc<dyn SpotifyApi>) -> Self {
        Self { api }
    }

    /// Resolve one Last.fm artist to a Spotify [`Artist`].
    ///
    /// Returns `Ok(None)` when Spotify has no match for the name — a missing
    /// match is not an error, just a name to skip.
    #[tracing::instrument(skip(self), fields(artist = %artist.name))]
    pub async fn resolve_artist(&self, artist: &LastfmArtist) -> ApiResult<Option<Artist>> {
        let results = self
            .api
            .search(&artist.name, &[SearchType::Artist], RESOLVE_LIMIT)
            .await?;
        Ok(best_artist(&artist.name, &results.artists.items))
    }

    /// Resolve one Last.fm track to a Spotify [`Track`].
    ///
    /// Returns `Ok(None)` when Spotify has no match. The Last.fm artist name is
    /// folded into the query and used to disambiguate when several tracks share
    /// the title.
    #[tracing::instrument(skip(self), fields(track = %track.name, artist = %track.artist))]
    pub async fn resolve_track(&self, track: &LastfmTrack) -> ApiResult<Option<Track>> {
        // Searching "<artist> <title>" is markedly more precise than the bare
        // title for common song names.
        let query = format!("{} {}", track.artist, track.name);
        let results = self
            .api
            .search(&query, &[SearchType::Track], RESOLVE_LIMIT)
            .await?;
        Ok(best_track(
            &track.name,
            &track.artist,
            &results.tracks.items,
        ))
    }

    /// Resolve a batch of Last.fm artists, dropping the ones with no match.
    ///
    /// Each name is searched sequentially so the calls share the client's
    /// rate-limit budget rather than bursting in parallel.
    #[tracing::instrument(skip(self, artists), fields(count = artists.len()))]
    pub async fn resolve_artists(&self, artists: &[LastfmArtist]) -> ApiResult<Vec<Artist>> {
        let mut resolved = Vec::with_capacity(artists.len());
        for artist in artists {
            if let Some(found) = self.resolve_artist(artist).await? {
                resolved.push(found);
            }
        }
        Ok(resolved)
    }

    /// Resolve a batch of Last.fm tracks, dropping the ones with no match.
    #[tracing::instrument(skip(self, tracks), fields(count = tracks.len()))]
    pub async fn resolve_tracks(&self, tracks: &[LastfmTrack]) -> ApiResult<Vec<Track>> {
        let mut resolved = Vec::with_capacity(tracks.len());
        for track in tracks {
            if let Some(found) = self.resolve_track(track).await? {
                resolved.push(found);
            }
        }
        Ok(resolved)
    }
}

/// Pick the best Spotify artist match for a Last.fm name.
///
/// A case-insensitive exact-name match wins; failing that, Spotify's first
/// (highest-ranked) result is used.
fn best_artist(name: &str, candidates: &[Artist]) -> Option<Artist> {
    candidates
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))
        .or_else(|| candidates.first())
        .cloned()
}

/// Pick the best Spotify track match for a Last.fm title + artist.
///
/// Preference order: an exact title match whose artist also matches; then any
/// exact title match; then Spotify's first result.
fn best_track(title: &str, artist: &str, candidates: &[Track]) -> Option<Track> {
    let title_and_artist = candidates.iter().find(|t| {
        t.name.eq_ignore_ascii_case(title)
            && t.artists
                .iter()
                .any(|a| a.name.eq_ignore_ascii_case(artist))
    });
    title_and_artist
        .or_else(|| {
            candidates
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(title))
        })
        .or_else(|| candidates.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use spottyfi_models::{
        AlbumId, ArtistId, Page, SearchResults, SimplifiedAlbum, SimplifiedArtist, TrackId,
    };

    use crate::traits::MockSpotifyApi;

    fn artist(name: &str) -> Artist {
        Artist {
            id: ArtistId::new(format!("artist-{name}")),
            name: name.to_owned(),
            images: Vec::new(),
            genres: Vec::new(),
            popularity: 0,
        }
    }

    fn track(name: &str, artist_name: &str) -> Track {
        Track {
            id: Some(TrackId::new(format!("track-{name}"))),
            name: name.to_owned(),
            artists: vec![SimplifiedArtist {
                id: None,
                name: artist_name.to_owned(),
            }],
            album: SimplifiedAlbum {
                id: Some(AlbumId::new("album-x")),
                name: "Album".to_owned(),
                images: Vec::new(),
                artists: Vec::new(),
                release_date: None,
            },
            duration_ms: 1000,
            explicit: false,
            popularity: 0,
            track_number: 1,
            is_local: false,
        }
    }

    #[test]
    fn best_artist_prefers_an_exact_name_match() {
        let candidates = vec![artist("Radiohead Tribute"), artist("radiohead")];
        // The case-insensitive exact match wins over the first result.
        let chosen = best_artist("Radiohead", &candidates).expect("a match");
        assert_eq!(chosen.name, "radiohead");
    }

    #[test]
    fn best_artist_falls_back_to_the_first_result() {
        let candidates = vec![artist("Some Other Band"), artist("Another")];
        let chosen = best_artist("Radiohead", &candidates).expect("a match");
        assert_eq!(chosen.name, "Some Other Band");
    }

    #[test]
    fn best_artist_is_none_when_there_are_no_candidates() {
        assert!(best_artist("Radiohead", &[]).is_none());
    }

    #[test]
    fn best_track_prefers_a_title_and_artist_match() {
        let candidates = vec![track("Creep", "Wrong Artist"), track("Creep", "Radiohead")];
        let chosen = best_track("Creep", "Radiohead", &candidates).expect("a match");
        assert_eq!(chosen.artists[0].name, "Radiohead");
    }

    #[test]
    fn best_track_falls_back_to_a_title_only_match() {
        let candidates = vec![track("Unrelated", "X"), track("Creep", "Cover Band")];
        let chosen = best_track("Creep", "Radiohead", &candidates).expect("a match");
        assert_eq!(chosen.name, "Creep");
    }

    #[tokio::test]
    async fn resolve_artist_runs_a_search_and_picks_a_match() {
        let mut mock = MockSpotifyApi::new();
        mock.expect_search().returning(|query, types, _| {
            assert_eq!(query, "Radiohead");
            assert_eq!(types, &[SearchType::Artist]);
            Ok(SearchResults {
                artists: Page {
                    items: vec![artist("Radiohead")],
                    ..Page::default()
                },
                ..SearchResults::default()
            })
        });
        let resolver = LastfmResolver::new(Arc::new(mock));
        let found = resolver
            .resolve_artist(&LastfmArtist {
                name: "Radiohead".to_owned(),
            })
            .await
            .expect("search ok")
            .expect("a match");
        assert_eq!(found.name, "Radiohead");
    }

    #[tokio::test]
    async fn resolve_track_searches_artist_and_title_together() {
        let mut mock = MockSpotifyApi::new();
        mock.expect_search().returning(|query, types, _| {
            assert_eq!(query, "Radiohead Creep");
            assert_eq!(types, &[SearchType::Track]);
            Ok(SearchResults {
                tracks: Page {
                    items: vec![track("Creep", "Radiohead")],
                    ..Page::default()
                },
                ..SearchResults::default()
            })
        });
        let resolver = LastfmResolver::new(Arc::new(mock));
        let found = resolver
            .resolve_track(&LastfmTrack {
                name: "Creep".to_owned(),
                artist: "Radiohead".to_owned(),
            })
            .await
            .expect("search ok")
            .expect("a match");
        assert_eq!(found.name, "Creep");
    }

    #[tokio::test]
    async fn resolve_artists_drops_names_with_no_match() {
        let mut mock = MockSpotifyApi::new();
        mock.expect_search().returning(|query, _, _| {
            // "Known" resolves; "Unknown" returns nothing.
            let items = if query == "Known" {
                vec![artist("Known")]
            } else {
                Vec::new()
            };
            Ok(SearchResults {
                artists: Page {
                    items,
                    ..Page::default()
                },
                ..SearchResults::default()
            })
        });
        let resolver = LastfmResolver::new(Arc::new(mock));
        let resolved = resolver
            .resolve_artists(&[
                LastfmArtist {
                    name: "Known".to_owned(),
                },
                LastfmArtist {
                    name: "Unknown".to_owned(),
                },
            ])
            .await
            .expect("search ok");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "Known");
    }
}
