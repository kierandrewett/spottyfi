//! The Apple Music [`PlaybackBackend`].

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use spottyfi_audio::PlaybackBackend;

use crate::engine::WebEngine;
use crate::musickit;

/// Playback state pushed in from the MusicKit web player.
///
/// The [`WebEngine`] host binds the page's `window.spottyfiOnState` callback
/// and writes these atomics from MusicKit's `playbackTimeDidChange` /
/// `playbackStateDidChange` events; [`AppleMusicBackend`] reads them.
#[derive(Debug, Default)]
pub struct AppleMusicState {
    /// The current playback position, in milliseconds.
    pub position_ms: AtomicU64,
    /// Whether a track is playing right now.
    pub playing: AtomicBool,
    /// Whether the current track has played through to its end.
    pub finished: AtomicBool,
}

/// Apple Music playback through an embedded MusicKit web player.
///
/// Implements [`PlaybackBackend`] so the transport drives Apple Music exactly
/// as it drives librespot and the HTTP player — the backend turns each call
/// into a MusicKit JS control script and hands it to the [`WebEngine`].
pub struct AppleMusicBackend {
    /// The browser engine running MusicKit JS.
    engine: Arc<dyn WebEngine>,
    /// Playback state, written by the engine host, read here.
    state: Arc<AppleMusicState>,
}

impl AppleMusicBackend {
    /// Build a backend over a web engine and its shared state.
    ///
    /// The caller is expected to have loaded
    /// [`bootstrap_html`](crate::musickit::bootstrap_html) into `engine` and
    /// wired its state callback into `state`.
    #[must_use]
    pub fn new(engine: Arc<dyn WebEngine>, state: Arc<AppleMusicState>) -> Self {
        Self { engine, state }
    }

    /// Authorize the Apple Music user (opens Apple's sign-in in the engine).
    pub fn authorize(&self) {
        self.engine.eval(&musickit::authorize_script());
    }
}

impl PlaybackBackend for AppleMusicBackend {
    fn name(&self) -> &str {
        "apple-music"
    }

    fn load(&self, locator: &str) {
        // `locator` is an Apple Music catalog song id.
        self.engine.eval(&musickit::load_song_script(locator));
    }

    fn pause(&self) {
        self.engine.eval(&musickit::pause_script());
    }

    fn resume(&self) {
        self.engine.eval(&musickit::play_script());
    }

    fn stop(&self) {
        self.engine.eval(&musickit::stop_script());
    }

    fn seek(&self, position: Duration) {
        self.engine.eval(&musickit::seek_script(position));
    }

    fn set_volume(&self, volume: f32) {
        self.engine.eval(&musickit::volume_script(volume));
    }

    fn position(&self) -> Duration {
        Duration::from_millis(self.state.position_ms.load(Ordering::Relaxed))
    }

    fn is_playing(&self) -> bool {
        self.state.playing.load(Ordering::Relaxed)
    }

    fn is_finished(&self) -> bool {
        self.state.finished.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::WebEngine;
    use std::sync::Mutex;

    /// A web engine that records the scripts it is handed.
    #[derive(Default)]
    struct RecordingEngine {
        scripts: Mutex<Vec<String>>,
    }

    impl WebEngine for RecordingEngine {
        fn eval(&self, script: &str) {
            self.scripts.lock().expect("lock").push(script.to_owned());
        }
    }

    #[test]
    fn backend_drives_the_engine_with_musickit_scripts() {
        let engine = Arc::new(RecordingEngine::default());
        let state = Arc::new(AppleMusicState::default());
        let backend = AppleMusicBackend::new(engine.clone(), state);

        backend.load("1440913170");
        backend.pause();
        backend.set_volume(0.5);

        let scripts = engine.scripts.lock().expect("lock");
        assert!(scripts[0].contains("setQueue({ song: '1440913170' })"));
        assert!(scripts[1].contains("pause()"));
        assert!(scripts[2].contains("volume = 0.5"));
    }

    #[test]
    fn position_and_flags_reflect_the_shared_state() {
        let state = Arc::new(AppleMusicState::default());
        let backend = AppleMusicBackend::new(
            Arc::new(crate::engine::LoggingWebEngine),
            Arc::clone(&state),
        );
        state.position_ms.store(45_000, Ordering::Relaxed);
        state.playing.store(true, Ordering::Relaxed);
        assert_eq!(backend.position(), Duration::from_secs(45));
        assert!(backend.is_playing());
        assert!(!backend.is_finished());
    }
}
