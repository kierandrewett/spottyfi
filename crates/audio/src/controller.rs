//! The public playback control surface.
//!
//! [`PlaybackController`] is the async API the rest of Spottyfi drives. It owns
//! the librespot [`Engine`] and exposes intent-style methods — `play_uri`,
//! `pause`, `resume`, `seek`, `set_volume` — that map onto librespot's player.
//!
//! Playback observations flow the other way, through the shared
//! [`PlaybackState`] snapshot returned by [`PlaybackController::state`].

use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;

use crate::engine::Engine;
use crate::error::{AudioError, AudioResult};
use crate::state::{self, PlaybackState};

/// Shared, hot-swappable playback state read by the UI each frame.
pub type SharedPlaybackState = Arc<ArcSwap<PlaybackState>>;

/// The audio engine's control surface.
///
/// Construct one with [`PlaybackController::start`], which connects a librespot
/// session from an OAuth access token. Dropping the controller stops playback
/// and shuts the engine's background tasks down.
pub struct PlaybackController {
    /// The running librespot engine.
    engine: Engine,
    /// The playback snapshot, shared with the UI.
    state: SharedPlaybackState,
}

impl PlaybackController {
    /// Start the audio engine, authenticating librespot with `access_token`.
    ///
    /// `access_token` is the access-token string from the auth crate's
    /// `Session::token()`. The engine connects a librespot session, builds the
    /// player and mixer, and begins publishing [`PlaybackState`] snapshots.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::Connect`] if the librespot session handshake
    /// fails, or [`AudioError::NoBackend`] if no audio output is available.
    #[tracing::instrument(skip_all)]
    pub async fn start(access_token: &str) -> AudioResult<Self> {
        let state: SharedPlaybackState = Arc::new(ArcSwap::from_pointee(PlaybackState::default()));
        let engine = Engine::connect(access_token, Arc::clone(&state)).await?;
        tracing::info!("audio engine started");
        Ok(Self { engine, state })
    }

    /// The shared playback-state handle the UI reads each frame.
    #[must_use]
    pub fn state(&self) -> SharedPlaybackState {
        Arc::clone(&self.state)
    }

    /// The current playback-state snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Arc<PlaybackState> {
        self.state.load_full()
    }

    /// Load and play a single track or episode by Spotify URI.
    ///
    /// `uri` may be a canonical `spotify:track:…` URI or an
    /// `open.spotify.com` URL; both are accepted. Playback starts immediately
    /// from the beginning of the track.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::InvalidUri`] if `uri` cannot be parsed, or
    /// [`AudioError::NotPlayable`] if it refers to an album, artist or
    /// playlist (those need a queue — Phase 8; see [`Self::play_context`]).
    #[tracing::instrument(skip(self))]
    pub async fn play_uri(&self, uri: &str) -> AudioResult<()> {
        let parsed = state::parse_playable(uri)?;
        tracing::info!(%uri, "loading track");
        self.engine.player().load(parsed, true, 0);
        Ok(())
    }

    /// Pause playback, keeping the current track and position.
    ///
    /// # Errors
    ///
    /// Currently infallible; returns [`AudioResult`] for API symmetry and so a
    /// future device-handoff implementation can report failures.
    #[tracing::instrument(skip(self))]
    pub async fn pause(&self) -> AudioResult<()> {
        self.engine.player().pause();
        Ok(())
    }

    /// Resume playback of the current track.
    ///
    /// # Errors
    ///
    /// Currently infallible; see [`Self::pause`].
    #[tracing::instrument(skip(self))]
    pub async fn resume(&self) -> AudioResult<()> {
        self.engine.player().play();
        Ok(())
    }

    /// Seek to `position` within the current track.
    ///
    /// # Errors
    ///
    /// Currently infallible; see [`Self::pause`].
    #[tracing::instrument(skip(self))]
    pub async fn seek(&self, position: Duration) -> AudioResult<()> {
        let position_ms = u32::try_from(position.as_millis()).unwrap_or(u32::MAX);
        self.engine.player().seek(position_ms);
        Ok(())
    }

    /// Set the output volume from a `0.0..=1.0` fraction.
    ///
    /// Values outside the range are clamped.
    ///
    /// # Errors
    ///
    /// Currently infallible; see [`Self::pause`].
    #[tracing::instrument(skip(self))]
    pub async fn set_volume(&self, volume: f32) -> AudioResult<()> {
        self.engine.set_volume(volume);
        Ok(())
    }

    /// Play a context (playlist, album or artist) starting at an optional
    /// offset.
    ///
    /// # Errors
    ///
    /// Always returns [`AudioError::NotRunning`] for now: contexts need a play
    /// queue, which arrives in Phase 8.
    #[tracing::instrument(skip(self))]
    pub async fn play_context(&self, _uri: &str, _offset: Option<usize>) -> AudioResult<()> {
        // TODO(phase-8): resolve the context to a track list and feed the
        // queue; librespot's single `Player::load` only handles one track.
        tracing::warn!("play_context is not implemented until Phase 8 (queue)");
        Err(AudioError::NotRunning)
    }

    /// Skip to the next track in the queue.
    ///
    /// # Errors
    ///
    /// Always returns [`AudioError::NotRunning`] for now: there is no queue
    /// until Phase 8.
    #[tracing::instrument(skip(self))]
    pub async fn next(&self) -> AudioResult<()> {
        // TODO(phase-8): advance the play queue.
        tracing::warn!("next is not implemented until Phase 8 (queue)");
        Err(AudioError::NotRunning)
    }

    /// Skip to the previous track in the queue.
    ///
    /// # Errors
    ///
    /// Always returns [`AudioError::NotRunning`] for now: there is no queue
    /// until Phase 8.
    #[tracing::instrument(skip(self))]
    pub async fn previous(&self) -> AudioResult<()> {
        // TODO(phase-8): step back through the play queue.
        tracing::warn!("previous is not implemented until Phase 8 (queue)");
        Err(AudioError::NotRunning)
    }
}
