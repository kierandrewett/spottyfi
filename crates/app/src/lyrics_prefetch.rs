//! Background lyrics prefetch.
//!
//! The Lyrics panel only fetches lyrics while it is open and the track
//! changes. This watches the playing track regardless and warms the lyrics
//! cache on every track change — so opening the panel is instant instead of a
//! network wait.

use spottyfi_api::lyrics::{LyricsService, TrackRef};
use spottyfi_audio::TrackInfo;
use tokio::runtime::Handle;

/// Warms the lyrics cache for the currently-playing track.
pub struct LyricsPrefetcher {
    /// The lyrics source layer — a clone of the shell's `LyricsService`,
    /// sharing its cache.
    service: LyricsService,
    /// Runtime handle used to spawn the prefetch fetch.
    runtime: Handle,
    /// The URI lyrics were last prefetched for, so a fetch fires once per
    /// track change rather than every frame.
    last_uri: Option<String>,
}

impl LyricsPrefetcher {
    /// Build a prefetcher over a clone of the shell's lyrics service.
    #[must_use]
    pub fn new(service: LyricsService, runtime: Handle) -> Self {
        Self {
            service,
            runtime,
            last_uri: None,
        }
    }

    /// Observe the playing track; on a change, prefetch its lyrics.
    ///
    /// Call once per frame with the live playing track. The fetch is spawned
    /// on the runtime and its result discarded — it runs only to populate the
    /// lyrics cache the Lyrics panel later reads.
    pub fn observe(&mut self, track: Option<&TrackInfo>) {
        let uri = track.map(|t| t.uri.clone());
        if uri == self.last_uri {
            return;
        }
        self.last_uri = uri;
        let Some(track) = track else {
            return;
        };
        let track = TrackRef {
            uri: track.uri.clone(),
            title: track.title.clone(),
            artist: track.artists.first().cloned().unwrap_or_default(),
            album: track.album.clone(),
            duration: track.duration,
        };
        let service = self.service.clone();
        self.runtime.spawn(async move {
            // Result discarded: this runs purely to warm the lyrics cache.
            let _ = service.lyrics(&track).await;
        });
    }
}
