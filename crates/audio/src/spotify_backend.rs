//! The Spotify [`PlaybackBackend`] — librespot behind the backend trait.
//!
//! Wraps the librespot-backed [`PlaybackController`] so Spotify is driven
//! through the same [`PlaybackBackend`] interface as the OpenSubsonic HTTP
//! player. The controller's methods are `async`; this adapter spawns them on
//! the tokio runtime so the trait stays non-blocking and synchronous.

use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;

use crate::controller::PlaybackController;
use crate::playback::PlaybackBackend;

/// Spotify playback (librespot) behind the [`PlaybackBackend`] trait.
pub struct SpotifyBackend {
    /// The librespot playback controller.
    controller: Arc<PlaybackController>,
    /// The runtime the controller's async methods are spawned on.
    runtime: Handle,
}

impl SpotifyBackend {
    /// Wrap a running [`PlaybackController`] as a backend.
    #[must_use]
    pub fn new(controller: Arc<PlaybackController>, runtime: Handle) -> Self {
        Self {
            controller,
            runtime,
        }
    }
}

impl PlaybackBackend for SpotifyBackend {
    fn name(&self) -> &str {
        "spotify"
    }

    fn load(&self, locator: &str) {
        // `locator` is a `spotify:track:…` URI.
        let controller = Arc::clone(&self.controller);
        let uri = locator.to_owned();
        self.runtime.spawn(async move {
            if let Err(err) = controller.play_uri(&uri).await {
                tracing::warn!(%err, %uri, "spotify backend: play failed");
            }
        });
    }

    fn pause(&self) {
        let controller = Arc::clone(&self.controller);
        self.runtime.spawn(async move {
            let _ = controller.pause().await;
        });
    }

    fn resume(&self) {
        let controller = Arc::clone(&self.controller);
        self.runtime.spawn(async move {
            let _ = controller.resume().await;
        });
    }

    fn stop(&self) {
        // librespot has no discrete "stop"; pausing is the closest no-op-safe
        // equivalent and leaves the session healthy.
        self.pause();
    }

    fn seek(&self, position: Duration) {
        let controller = Arc::clone(&self.controller);
        self.runtime.spawn(async move {
            let _ = controller.seek(position).await;
        });
    }

    fn set_volume(&self, volume: f32) {
        let controller = Arc::clone(&self.controller);
        self.runtime.spawn(async move {
            let _ = controller.set_volume(volume).await;
        });
    }

    fn position(&self) -> Duration {
        self.controller.snapshot().position
    }

    fn is_playing(&self) -> bool {
        self.controller.snapshot().playing
    }

    fn is_finished(&self) -> bool {
        // librespot owns its own queue and advances itself, so the outer
        // queue must not also advance Spotify — this is always `false`.
        false
    }
}
