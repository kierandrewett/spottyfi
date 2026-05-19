//! The backend-agnostic playback trait.
//!
//! Spottyfi plays from several backends — librespot for Spotify, the
//! [`HttpAudioPlayer`](crate::http_player::HttpAudioPlayer) for OpenSubsonic.
//! [`PlaybackBackend`] is the one interface they all present, so the
//! transport, the queue and the UI drive "the player" without ever knowing
//! which backend is behind it.
//!
//! Routing a track to its backend (by source kind) and resolving a
//! backend-specific locator happens one layer up; a backend itself only ever
//! sees the `locator` string it understands.

use std::time::Duration;

/// A music playback backend.
///
/// Every method is non-blocking: a backend does its fetching/decoding on its
/// own thread or task, and exposes progress through cheap atomic reads.
pub trait PlaybackBackend: Send + Sync {
    /// A short human-readable name for the backend (for logs and diagnostics).
    fn name(&self) -> &str;

    /// Load and start playing the track identified by `locator`.
    ///
    /// `locator` is whatever *this* backend understands — an HTTP stream URL
    /// for OpenSubsonic, a `spotify:track:…` URI for librespot. The caller
    /// resolves it from the track's source before calling.
    fn load(&self, locator: &str);

    /// Pause playback, keeping the current position.
    fn pause(&self);

    /// Resume playback from the current position.
    fn resume(&self);

    /// Stop playback and discard the current track.
    fn stop(&self);

    /// Seek within the current track.
    fn seek(&self, position: Duration);

    /// Set the output volume from a `0.0..=1.0` fraction.
    fn set_volume(&self, volume: f32);

    /// The current play position.
    fn position(&self) -> Duration;

    /// Whether a track is actively playing right now.
    fn is_playing(&self) -> bool;

    /// Whether the current track has played through to its end — the signal
    /// the queue uses to advance to the next track.
    fn is_finished(&self) -> bool;
}

impl PlaybackBackend for crate::http_player::HttpAudioPlayer {
    fn name(&self) -> &str {
        "http"
    }

    fn load(&self, locator: &str) {
        self.load(locator.to_owned());
    }

    fn pause(&self) {
        self.pause();
    }

    fn resume(&self) {
        self.resume();
    }

    fn stop(&self) {
        self.stop();
    }

    fn seek(&self, position: Duration) {
        self.seek(position);
    }

    fn set_volume(&self, volume: f32) {
        self.set_volume(volume);
    }

    fn position(&self) -> Duration {
        self.position()
    }

    fn is_playing(&self) -> bool {
        self.is_playing()
    }

    fn is_finished(&self) -> bool {
        self.is_finished()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake backend, proving the trait is object-safe and usable behind a
    /// `dyn` pointer with no knowledge of the concrete type.
    #[derive(Default)]
    struct FakeBackend {
        loaded: std::sync::Mutex<Option<String>>,
    }

    impl PlaybackBackend for FakeBackend {
        fn name(&self) -> &str {
            "fake"
        }
        fn load(&self, locator: &str) {
            *self.loaded.lock().expect("lock") = Some(locator.to_owned());
        }
        fn pause(&self) {}
        fn resume(&self) {}
        fn stop(&self) {}
        fn seek(&self, _: Duration) {}
        fn set_volume(&self, _: f32) {}
        fn position(&self) -> Duration {
            Duration::ZERO
        }
        fn is_playing(&self) -> bool {
            false
        }
        fn is_finished(&self) -> bool {
            false
        }
    }

    #[test]
    fn backend_is_object_safe_and_drivable_through_dyn() {
        let backend: Box<dyn PlaybackBackend> = Box::new(FakeBackend::default());
        backend.load("spotify:track:abc");
        backend.set_volume(0.5);
        assert_eq!(backend.name(), "fake");
        assert!(!backend.is_playing());
    }
}
