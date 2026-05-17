//! Background cache-warming after login.
//!
//! Every page serves from the stale-while-revalidate metadata cache, but a
//! *cold* cache still makes the first visit wait on the network. This warms
//! the cache in the background right after login — so opening a playlist is
//! instant rather than a spinner — by performing exactly the fetches a later
//! page open would, just ahead of time.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use spottyfi_api::SpotifyApi;
use spottyfi_models::SpotifyId as _;
use tokio::runtime::Handle;

/// Politeness delay between prefetch fetches, so warming the cache neither
/// contends with the user's own foreground requests nor trips rate limits.
const PREFETCH_GAP: Duration = Duration::from_millis(250);

/// Spawn the post-login cache-warming task.
///
/// Walks the user's playlists and resolves each one's full track listing
/// through [`SpotifyApi::playlist_tracks_all`], whose stale-while-revalidate
/// caching means the later page open is served from cache with no network
/// wait. Best-effort: a failed warm is logged and skipped.
pub fn spawn(api: Arc<dyn SpotifyApi>, runtime: &Handle) {
    runtime.spawn(async move {
        tracing::debug!("prefetch: warming the metadata cache");
        let mut playlists = api.user_playlists_stream();
        let mut warmed = 0_usize;
        while let Some(item) = playlists.next().await {
            let Ok(playlist) = item else { continue };
            let id = playlist.id.id().to_owned();
            // Warm both fetches a playlist page open performs: the playlist
            // metadata (header, name) and its full track listing.
            if let Err(err) = api.playlist(&id).await {
                tracing::debug!(%err, %id, "prefetch: playlist metadata warm failed");
            }
            if let Err(err) = api.playlist_tracks_all(&id).await {
                tracing::debug!(%err, %id, "prefetch: playlist tracks warm failed");
            } else {
                warmed += 1;
            }
            tokio::time::sleep(PREFETCH_GAP).await;
        }
        tracing::debug!(warmed, "prefetch: cache warming complete");
    });
}
