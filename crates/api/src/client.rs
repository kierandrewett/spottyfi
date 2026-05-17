//! The real [`SpotifyApi`] implementation, backed by `rspotify`.

use std::sync::Arc;

use async_trait::async_trait;
use rspotify::clients::{BaseClient as _, OAuthClient as _};
use rspotify::model as rs;
use rspotify::model::Id as _;
use rspotify::{AuthCodePkceSpotify, ClientResult};

use spottyfi_auth::Session;
use spottyfi_models::{
    Album, Artist, Category, Device, Page, Playlist, PlaylistTrack, SavedTrack, SearchResults,
    SimplifiedAlbum, SimplifiedPlaylist, Track, User,
};

use spottyfi_cache::Kind;

use crate::error::{ApiError, ApiResult};
use crate::map;
use crate::metadata::{Lookup, MetadataLayer};
use crate::retry::{classify, RetryDecision, RetryPolicy};
use crate::traits::{ItemStream, SearchType, SpotifyApi};

/// The page size used when streaming an unbounded endpoint.
const STREAM_PAGE_SIZE: u32 = 50;

/// The Spotify Web API client.
///
/// Wraps the `rspotify` client from an authenticated [`Session`], adds the
/// retry / rate-limit policy and the [`MetadataLayer`] (hot cache + persistent
/// SQLite store), and maps every response onto [`spottyfi_models`] types.
#[derive(Clone)]
pub struct SpotifyClient {
    rspotify: Arc<AuthCodePkceSpotify>,
    policy: RetryPolicy,
    cache: MetadataLayer,
}

impl std::fmt::Debug for SpotifyClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpotifyClient")
            .field("policy", &self.policy)
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

impl SpotifyClient {
    /// Build a client from an authenticated [`Session`].
    ///
    /// Opens the persistent SQLite metadata cache under the platform cache
    /// directory; if that fails (no cache dir, unwritable DB, migration
    /// error), the client degrades to an in-memory-only cache and logs the
    /// failure rather than refusing to start.
    #[must_use]
    pub fn new(session: &Session) -> Self {
        let cache = match open_metadata_layer() {
            Ok(layer) => layer,
            Err(err) => {
                tracing::warn!(%err, "persistent metadata cache unavailable; using in-memory cache only");
                MetadataLayer::in_memory_only()
            }
        };
        Self {
            rspotify: session.client(),
            policy: RetryPolicy::default(),
            cache,
        }
    }

    /// Build a client with an explicit retry policy and metadata layer (for
    /// tests and tuning).
    #[must_use]
    pub fn with_config(session: &Session, policy: RetryPolicy, cache: MetadataLayer) -> Self {
        Self {
            rspotify: session.client(),
            policy,
            cache,
        }
    }

    /// Run an `rspotify` call, retrying on 429 / transient failures per the
    /// [`RetryPolicy`], honouring `Retry-After`.
    ///
    /// `op` is invoked afresh for each attempt because `rspotify`'s futures
    /// are single-use.
    async fn request<T, F, Fut>(&self, op: F) -> ApiResult<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = ClientResult<T>>,
    {
        let mut attempt = 0;
        loop {
            match op().await {
                Ok(value) => return Ok(value),
                Err(err) => {
                    // Decide in a tight scope so the `!Send` `ThreadRng` is
                    // dropped before the `sleep` await below.
                    let decision = {
                        let mut rng = rand::rng();
                        classify(err, attempt, &self.policy, &mut rng)
                    };
                    match decision {
                        RetryDecision::Fail(api_err) => return Err(api_err),
                        RetryDecision::RetryAfter(delay) => {
                            tracing::warn!(
                                attempt,
                                delay_ms = delay.as_millis(),
                                "Spotify request failed; backing off before retry"
                            );
                            tokio::time::sleep(delay).await;
                            attempt += 1;
                        }
                    }
                }
            }
        }
    }

    /// As [`Self::request`], but a `NotFound` (HTTP 403/404) is rewritten to
    /// [`ApiError::EndpointUnavailable`] — used for endpoints Spotify
    /// deprecated for apps registered after 2024-11-27.
    async fn request_deprecated<T, F, Fut>(&self, endpoint: &'static str, op: F) -> ApiResult<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = ClientResult<T>>,
    {
        match self.request(op).await {
            Err(ApiError::NotFound(_)) => Err(ApiError::EndpointUnavailable { endpoint }),
            other => other,
        }
    }

    /// Serve `kind`/`cache_id` with stale-while-revalidate semantics.
    ///
    /// - A **fresh** cache hit is returned without any network call.
    /// - A **stale** cache hit is returned immediately, and `fetch` is spawned
    ///   on the runtime to refresh the cache in the background — the slow path
    ///   never blocks the caller.
    /// - A **miss** awaits `fetch`, caches the result and returns it.
    ///
    /// `fetch` is a cloneable factory because it may be invoked twice (once for
    /// the foreground miss, once for a background refresh) and `rspotify`
    /// futures are single-use.
    async fn cached<T, F, Fut>(&self, kind: Kind, cache_id: &str, fetch: F) -> ApiResult<T>
    where
        T: Clone + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static,
        F: Fn() -> Fut + Clone + Send + 'static,
        Fut: std::future::Future<Output = ApiResult<T>> + Send,
    {
        match self.cache.get::<T>(kind, cache_id).await {
            Lookup::Hit { value, stale } => {
                if stale {
                    tracing::debug!(
                        ?kind,
                        cache_id,
                        "cache hit (stale); refreshing in background"
                    );
                    self.spawn_refresh(kind, cache_id.to_owned(), fetch);
                } else {
                    tracing::debug!(?kind, cache_id, "cache hit (fresh)");
                }
                Ok(value)
            }
            Lookup::Miss => {
                tracing::debug!(?kind, cache_id, "cache miss; fetching");
                let value = fetch().await?;
                self.cache.put(kind, cache_id, &value).await;
                Ok(value)
            }
        }
    }

    /// Spawn a background task that refreshes `kind`/`cache_id` in the cache.
    ///
    /// A refresh failure is logged and dropped — the stale value the caller
    /// already returned remains usable until the next attempt.
    fn spawn_refresh<T, F, Fut>(&self, kind: Kind, cache_id: String, fetch: F)
    where
        T: Clone + Send + Sync + serde::Serialize + 'static,
        F: Fn() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ApiResult<T>> + Send,
    {
        let cache = self.cache.clone();
        tokio::spawn(async move {
            match fetch().await {
                Ok(value) => {
                    cache.put(kind, &cache_id, &value).await;
                    tracing::debug!(?kind, cache_id, "background cache refresh complete");
                }
                Err(err) => {
                    tracing::debug!(%err, ?kind, cache_id, "background cache refresh failed");
                }
            }
        });
    }
}

/// Open the persistent [`MetadataLayer`] under the platform cache directory.
fn open_metadata_layer() -> Result<MetadataLayer, spottyfi_cache::CacheError> {
    let db_path = spottyfi_cache::paths::metadata_db_path()?;
    let cache = spottyfi_cache::MetadataCache::open(db_path)?;
    Ok(MetadataLayer::new(
        crate::cache::ObjectCache::default(),
        Arc::new(cache),
    ))
}

/// Build a `TrackId` from a bare id or URI, mapping a parse failure onto a
/// `NotFound` error.
fn track_id(id: &str) -> ApiResult<rs::TrackId<'static>> {
    rs::TrackId::from_id_or_uri(id)
        .map(rs::TrackId::into_static)
        .map_err(|e| ApiError::NotFound(format!("invalid track id '{id}': {e}")))
}

/// Build an `AlbumId` from a bare id or URI.
fn album_id(id: &str) -> ApiResult<rs::AlbumId<'static>> {
    rs::AlbumId::from_id_or_uri(id)
        .map(rs::AlbumId::into_static)
        .map_err(|e| ApiError::NotFound(format!("invalid album id '{id}': {e}")))
}

/// Build an `ArtistId` from a bare id or URI.
fn artist_id(id: &str) -> ApiResult<rs::ArtistId<'static>> {
    rs::ArtistId::from_id_or_uri(id)
        .map(rs::ArtistId::into_static)
        .map_err(|e| ApiError::NotFound(format!("invalid artist id '{id}': {e}")))
}

/// Build a `PlaylistId` from a bare id or URI.
fn playlist_id(id: &str) -> ApiResult<rs::PlaylistId<'static>> {
    rs::PlaylistId::from_id_or_uri(id)
        .map(rs::PlaylistId::into_static)
        .map_err(|e| ApiError::NotFound(format!("invalid playlist id '{id}': {e}")))
}

/// Translate a Spottyfi [`SearchType`] to the `rspotify` enum.
fn search_type(t: SearchType) -> rs::SearchType {
    match t {
        SearchType::Track => rs::SearchType::Track,
        SearchType::Artist => rs::SearchType::Artist,
        SearchType::Album => rs::SearchType::Album,
        SearchType::Playlist => rs::SearchType::Playlist,
    }
}

#[async_trait]
impl SpotifyApi for SpotifyClient {
    #[tracing::instrument(skip(self))]
    async fn current_user(&self) -> ApiResult<User> {
        let user = self.request(|| self.rspotify.me()).await?;
        Ok(map::private_user(&user))
    }

    #[tracing::instrument(skip(self))]
    async fn user_playlists(&self, offset: u32, limit: u32) -> ApiResult<Page<SimplifiedPlaylist>> {
        let page = self
            .request(|| {
                self.rspotify
                    .current_user_playlists_manual(Some(limit), Some(offset))
            })
            .await?;
        Ok(map::page(&page, map::simplified_playlist))
    }

    #[tracing::instrument(skip(self))]
    fn user_playlists_stream(&self) -> ItemStream<SimplifiedPlaylist> {
        let this = self.clone();
        paginate(move |offset, limit| {
            let this = this.clone();
            async move { this.user_playlists(offset, limit).await }
        })
    }

    #[tracing::instrument(skip(self))]
    async fn playlist(&self, playlist_id: &str) -> ApiResult<Playlist> {
        let id = self::playlist_id(playlist_id)?;
        let cache_id = id.id().to_owned();
        let this = self.clone();
        self.cached(Kind::Playlist, &cache_id, move || {
            let this = this.clone();
            let id = id.clone();
            async move {
                let full = this
                    .request(|| {
                        this.rspotify
                            .playlist(id.as_ref(), None, Some(rs::Market::FromToken))
                    })
                    .await?;
                Ok(map::playlist(&full))
            }
        })
        .await
    }

    #[tracing::instrument(skip(self))]
    async fn playlist_tracks(
        &self,
        playlist_id: &str,
        offset: u32,
        limit: u32,
    ) -> ApiResult<Page<PlaylistTrack>> {
        let id = self::playlist_id(playlist_id)?;
        let page = self
            .request(|| {
                self.rspotify.playlist_items_manual(
                    id.as_ref(),
                    None,
                    Some(rs::Market::FromToken),
                    Some(limit),
                    Some(offset),
                )
            })
            .await?;
        Ok(map::page(&page, map::playlist_item))
    }

    #[tracing::instrument(skip(self))]
    fn playlist_tracks_stream(&self, playlist_id: &str) -> ItemStream<PlaylistTrack> {
        let this = self.clone();
        let id = playlist_id.to_owned();
        paginate(move |offset, limit| {
            let this = this.clone();
            let id = id.clone();
            async move { this.playlist_tracks(&id, offset, limit).await }
        })
    }

    #[tracing::instrument(skip(self))]
    async fn playlist_tracks_all(&self, playlist_id: &str) -> ApiResult<Vec<PlaylistTrack>> {
        use futures::StreamExt as _;
        let id = self::playlist_id(playlist_id)?;
        let cache_id = id.id().to_owned();
        let this = self.clone();
        self.cached(Kind::PlaylistTracks, &cache_id, move || {
            let this = this.clone();
            let id = id.clone();
            async move {
                // Drive every page of the listing, collecting into one Vec.
                // The first page error ends the stream and fails the fetch.
                let mut stream = this.playlist_tracks_stream(id.id());
                let mut tracks = Vec::new();
                while let Some(item) = stream.next().await {
                    tracks.push(item?);
                }
                Ok(tracks)
            }
        })
        .await
    }

    #[tracing::instrument(skip(self))]
    async fn saved_tracks(&self, offset: u32, limit: u32) -> ApiResult<Page<SavedTrack>> {
        let page = self
            .request(|| {
                self.rspotify.current_user_saved_tracks_manual(
                    Some(rs::Market::FromToken),
                    Some(limit),
                    Some(offset),
                )
            })
            .await?;
        Ok(map::page(&page, map::saved_track))
    }

    #[tracing::instrument(skip(self))]
    fn saved_tracks_stream(&self) -> ItemStream<SavedTrack> {
        let this = self.clone();
        paginate(move |offset, limit| {
            let this = this.clone();
            async move { this.saved_tracks(offset, limit).await }
        })
    }

    #[tracing::instrument(skip(self))]
    async fn saved_albums(&self, offset: u32, limit: u32) -> ApiResult<Page<Album>> {
        let page = self
            .request(|| {
                self.rspotify.current_user_saved_albums_manual(
                    Some(rs::Market::FromToken),
                    Some(limit),
                    Some(offset),
                )
            })
            .await?;
        Ok(map::page(&page, map::saved_album))
    }

    #[tracing::instrument(skip(self))]
    fn saved_albums_stream(&self) -> ItemStream<Album> {
        let this = self.clone();
        paginate(move |offset, limit| {
            let this = this.clone();
            async move { this.saved_albums(offset, limit).await }
        })
    }

    #[tracing::instrument(skip(self))]
    async fn album(&self, album_id: &str) -> ApiResult<Album> {
        // Parse up-front so an invalid id fails fast, and so the cache key is
        // the bare id regardless of whether a URI or a bare id was passed.
        let id = self::album_id(album_id)?;
        let cache_id = id.id().to_owned();
        let this = self.clone();
        self.cached(Kind::Album, &cache_id, move || {
            let this = this.clone();
            let id = id.clone();
            async move {
                let full = this
                    .request(|| {
                        this.rspotify
                            .album(id.as_ref(), Some(rs::Market::FromToken))
                    })
                    .await?;
                Ok(map::album(&full))
            }
        })
        .await
    }

    #[tracing::instrument(skip(self))]
    async fn artist(&self, artist_id: &str) -> ApiResult<Artist> {
        let id = self::artist_id(artist_id)?;
        let cache_id = id.id().to_owned();
        let this = self.clone();
        self.cached(Kind::Artist, &cache_id, move || {
            let this = this.clone();
            let id = id.clone();
            async move {
                let full = this.request(|| this.rspotify.artist(id.as_ref())).await?;
                Ok(map::artist(&full))
            }
        })
        .await
    }

    #[tracing::instrument(skip(self))]
    async fn artist_albums(
        &self,
        artist_id: &str,
        offset: u32,
        limit: u32,
    ) -> ApiResult<Page<SimplifiedAlbum>> {
        let id = self::artist_id(artist_id)?;
        let page = self
            .request(|| {
                self.rspotify.artist_albums_manual(
                    id.as_ref(),
                    [],
                    Some(rs::Market::FromToken),
                    Some(limit),
                    Some(offset),
                )
            })
            .await?;
        Ok(map::page(&page, map::simplified_album))
    }

    #[tracing::instrument(skip(self))]
    fn artist_albums_stream(&self, artist_id: &str) -> ItemStream<SimplifiedAlbum> {
        let this = self.clone();
        let id = artist_id.to_owned();
        paginate(move |offset, limit| {
            let this = this.clone();
            let id = id.clone();
            async move { this.artist_albums(&id, offset, limit).await }
        })
    }

    #[tracing::instrument(skip(self))]
    async fn artist_top_tracks(&self, artist_id: &str) -> ApiResult<Vec<Track>> {
        let id = self::artist_id(artist_id)?;
        // Deprecated for apps registered after 2024-11-27 (see docs/questions.md).
        #[allow(deprecated)]
        let tracks = self
            .request_deprecated("artist_top_tracks", || {
                self.rspotify
                    .artist_top_tracks(id.as_ref(), Some(rs::Market::FromToken))
            })
            .await?;
        Ok(tracks.iter().map(map::track).collect())
    }

    #[tracing::instrument(skip(self))]
    async fn current_user_top_artists(&self, limit: u32) -> ApiResult<Vec<Artist>> {
        // Not on Spotify's 2024-11-27 deprecation list — works for new apps.
        let page = self
            .request(|| {
                self.rspotify
                    .current_user_top_artists_manual(None, Some(limit), Some(0))
            })
            .await?;
        Ok(page.items.iter().map(map::artist).collect())
    }

    #[tracing::instrument(skip(self))]
    async fn current_user_top_tracks(&self, limit: u32) -> ApiResult<Vec<Track>> {
        // Not on Spotify's 2024-11-27 deprecation list — works for new apps.
        let page = self
            .request(|| {
                self.rspotify
                    .current_user_top_tracks_manual(None, Some(limit), Some(0))
            })
            .await?;
        Ok(page.items.iter().map(map::track).collect())
    }

    #[tracing::instrument(skip(self))]
    async fn search(
        &self,
        query: &str,
        types: &[SearchType],
        limit: u32,
    ) -> ApiResult<SearchResults> {
        let rs_types: Vec<rs::SearchType> = types.iter().copied().map(search_type).collect();
        let result = self
            .request(|| {
                self.rspotify.search_multiple(
                    query,
                    rs_types.iter().copied(),
                    Some(rs::Market::FromToken),
                    None,
                    Some(limit),
                    Some(0),
                )
            })
            .await?;
        Ok(map::search_results(&result))
    }

    #[tracing::instrument(skip(self))]
    async fn featured_playlists(&self, limit: u32) -> ApiResult<Vec<SimplifiedPlaylist>> {
        // Deprecated for apps registered after 2024-11-27 (see docs/questions.md).
        let featured = self
            .request_deprecated("featured_playlists", || {
                self.rspotify
                    .featured_playlists(None, None, None, Some(limit), Some(0))
            })
            .await?;
        Ok(featured
            .playlists
            .items
            .iter()
            .map(map::simplified_playlist)
            .collect())
    }

    #[tracing::instrument(skip(self))]
    async fn browse_categories(&self, limit: u32) -> ApiResult<Vec<Category>> {
        // Deprecated for apps registered after 2024-11-27 (see docs/questions.md).
        #[allow(deprecated)]
        let page = self
            .request_deprecated("browse_categories", || {
                self.rspotify
                    .categories_manual(None, None, Some(limit), Some(0))
            })
            .await?;
        Ok(page.items.iter().map(map::category).collect())
    }

    #[tracing::instrument(skip(self))]
    async fn new_releases(&self, limit: u32) -> ApiResult<Vec<SimplifiedAlbum>> {
        // `rspotify` marks this deprecated; its status for new apps is
        // uncertain — a 403/404 surfaces as `EndpointUnavailable`.
        #[allow(deprecated)]
        let page = self
            .request_deprecated("new_releases", || {
                self.rspotify
                    .new_releases_manual(None, Some(limit), Some(0))
            })
            .await?;
        Ok(page.items.iter().map(map::simplified_album).collect())
    }

    #[tracing::instrument(skip(self))]
    async fn recommendations(
        &self,
        seed_artists: &[String],
        seed_tracks: &[String],
        seed_genres: &[String],
        limit: u32,
    ) -> ApiResult<Vec<Track>> {
        let artists = seed_artists
            .iter()
            .map(|s| artist_id(s))
            .collect::<ApiResult<Vec<_>>>()?;
        let tracks = seed_tracks
            .iter()
            .map(|s| track_id(s))
            .collect::<ApiResult<Vec<_>>>()?;
        let genres: Vec<&str> = seed_genres.iter().map(String::as_str).collect();

        // Deprecated for apps registered after 2024-11-27 — likely to fail
        // with `EndpointUnavailable`; Phase 7 sources recommendations from
        // Last.fm instead. See docs/questions.md.
        let recs = self
            .request_deprecated("recommendations", || {
                self.rspotify.recommendations(
                    [],
                    Some(artists.iter().map(rs::ArtistId::as_ref)),
                    Some(genres.iter().copied()),
                    Some(tracks.iter().map(rs::TrackId::as_ref)),
                    Some(rs::Market::FromToken),
                    Some(limit),
                )
            })
            .await?;
        Ok(recs.tracks.iter().map(map::simplified_to_track).collect())
    }

    #[tracing::instrument(skip(self))]
    async fn devices(&self) -> ApiResult<Vec<Device>> {
        let devices = self.request(|| self.rspotify.device()).await?;
        Ok(devices.iter().map(map::device).collect())
    }

    #[tracing::instrument(skip(self))]
    async fn transfer_playback(&self, device_id: &str, play: bool) -> ApiResult<()> {
        self.request(|| self.rspotify.transfer_playback(device_id, Some(play)))
            .await
    }
}

/// Build an [`ItemStream`] that drives an offset-paginated endpoint.
///
/// `fetch(offset, limit)` returns one [`Page`]; this follows pages until one
/// reports `has_next == false` or comes back empty. A page error is yielded
/// and ends the stream. Each page request already carries its own retry /
/// rate-limit handling inside `fetch`.
fn paginate<T, F, Fut>(fetch: F) -> ItemStream<T>
where
    T: Send + 'static,
    F: Fn(u32, u32) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ApiResult<Page<T>>> + Send,
{
    Box::pin(async_stream::stream! {
        let mut offset = 0u32;
        loop {
            match fetch(offset, STREAM_PAGE_SIZE).await {
                Ok(page) => {
                    let count = page.items.len() as u32;
                    let has_next = page.has_next;
                    for item in page.items {
                        yield Ok(item);
                    }
                    // Stop on an empty page even if `has_next` was set —
                    // Spotify occasionally reports both (rspotify issue #492).
                    if count == 0 || !has_next {
                        break;
                    }
                    offset += count;
                }
                Err(err) => {
                    yield Err(err);
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt as _;

    #[test]
    fn search_type_maps_each_variant() {
        assert_eq!(search_type(SearchType::Track), rs::SearchType::Track);
        assert_eq!(search_type(SearchType::Artist), rs::SearchType::Artist);
        assert_eq!(search_type(SearchType::Album), rs::SearchType::Album);
        assert_eq!(search_type(SearchType::Playlist), rs::SearchType::Playlist);
    }

    #[test]
    fn invalid_ids_map_to_not_found() {
        // Non-base-62 characters and a wrong-type URI are both rejected.
        assert!(matches!(
            track_id("not a valid id!"),
            Err(ApiError::NotFound(_))
        ));
        assert!(matches!(
            artist_id("spotify:track:4y4VO05kYgUTo2bzbox1an"),
            Err(ApiError::NotFound(_))
        ));
    }

    #[test]
    fn a_bare_valid_id_is_accepted() {
        assert!(track_id("4y4VO05kYgUTo2bzbox1an").is_ok());
        // A correctly-typed URI is also accepted.
        assert!(album_id("spotify:album:6IcGNaXFRf5Y1jc7QsE9O2").is_ok());
    }

    #[tokio::test]
    async fn paginate_follows_pages_until_no_next() {
        // Three pages of two items each; the third reports no next page.
        let pages = move |offset: u32, _limit: u32| async move {
            let page = match offset {
                0 => Page {
                    items: vec![1, 2],
                    limit: 2,
                    offset: 0,
                    total: 6,
                    has_next: true,
                },
                2 => Page {
                    items: vec![3, 4],
                    limit: 2,
                    offset: 2,
                    total: 6,
                    has_next: true,
                },
                _ => Page {
                    items: vec![5, 6],
                    limit: 2,
                    offset: 4,
                    total: 6,
                    has_next: false,
                },
            };
            Ok::<_, ApiError>(page)
        };
        let collected: Vec<i32> = paginate(pages)
            .map(|r| r.expect("each page ok"))
            .collect()
            .await;
        assert_eq!(collected, vec![1, 2, 3, 4, 5, 6]);
    }

    #[tokio::test]
    async fn paginate_stops_and_surfaces_an_error() {
        let pages = move |offset: u32, _limit: u32| async move {
            if offset == 0 {
                Ok(Page {
                    items: vec![1],
                    limit: 1,
                    offset: 0,
                    total: 2,
                    has_next: true,
                })
            } else {
                Err(ApiError::Network("boom".to_owned()))
            }
        };
        let results: Vec<ApiResult<i32>> = paginate(pages).collect().await;
        assert_eq!(results.len(), 2);
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(ApiError::Network(_))));
    }
}
