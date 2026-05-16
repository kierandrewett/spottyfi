//! The public playback control surface.
//!
//! [`PlaybackController`] is the async API the rest of Spottyfi drives. It owns
//! the librespot [`Engine`] and the queue/context state machine, and exposes
//! intent-style methods — `play_uri`, `play_context`, `next`, `previous`,
//! `enqueue`, `play_next`, `pause`, `resume`, `seek`, `set_volume`.
//!
//! Playback observations flow the other way, through two shared snapshots:
//! the [`PlaybackState`] (track + position + play/pause) and the
//! [`QueueState`] (context + manual queue), both read by the UI each frame.
//!
//! # Why the controller owns the queue
//!
//! librespot's `Player` plays a single track and has no queue of its own — the
//! queue lives in Spotify Connect's `spirc`, which Spottyfi does not use. So
//! the controller owns a [`Queue`] behind a mutex, and a background task
//! subscribed to the player's event stream **auto-advances** it when a track
//! ends. See `queue.rs`.

use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use librespot::playback::player::PlayerEvent;
use std::sync::Mutex;

use crate::engine::Engine;
use crate::error::{AudioError, AudioResult};
use crate::queue::{Queue, QueueState, QueueTrack, RepeatMode};
use crate::state::{self, PlaybackState};

/// Shared, hot-swappable playback state read by the UI each frame.
pub type SharedPlaybackState = Arc<ArcSwap<PlaybackState>>;

/// Shared, hot-swappable queue state read by the UI each frame.
pub type SharedQueueState = Arc<ArcSwap<QueueState>>;

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
    /// The queue/context state machine. Behind a mutex because both the public
    /// control methods and the auto-advance task mutate it.
    queue: Arc<Mutex<Queue>>,
    /// The queue snapshot, shared with the UI's queue panel.
    queue_state: SharedQueueState,
}

impl PlaybackController {
    /// Start the audio engine, authenticating librespot with `access_token`.
    ///
    /// `access_token` is the access-token string from the auth crate's
    /// `Session::token()`. The engine connects a librespot session, builds the
    /// player and mixer, begins publishing [`PlaybackState`] snapshots, and
    /// spawns the queue auto-advance task.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::Connect`] if the librespot session handshake
    /// fails, or [`AudioError::NoBackend`] if no audio output is available.
    #[tracing::instrument(skip_all)]
    pub async fn start(access_token: &str) -> AudioResult<Self> {
        let state: SharedPlaybackState = Arc::new(ArcSwap::from_pointee(PlaybackState::default()));
        let engine = Engine::connect(access_token, Arc::clone(&state)).await?;
        let queue = Arc::new(Mutex::new(Queue::new()));
        let queue_state: SharedQueueState = Arc::new(ArcSwap::from_pointee(QueueState::default()));

        let controller = Self {
            engine,
            state,
            queue,
            queue_state,
        };
        controller.spawn_auto_advance();
        tracing::info!("audio engine started");
        Ok(controller)
    }

    /// The shared playback-state handle the UI reads each frame.
    #[must_use]
    pub fn state(&self) -> SharedPlaybackState {
        Arc::clone(&self.state)
    }

    /// The shared queue-state handle the UI's queue panel reads each frame.
    #[must_use]
    pub fn queue_state(&self) -> SharedQueueState {
        Arc::clone(&self.queue_state)
    }

    /// The current playback-state snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Arc<PlaybackState> {
        self.state.load_full()
    }

    /// Load and play a single track or episode by Spotify URI.
    ///
    /// This is a context-free one-off: it clears any playback context, so
    /// Next/Prev have nothing to walk afterwards. To play through a
    /// playlist/album use [`Self::play_context`].
    ///
    /// `uri` may be a canonical `spotify:track:…` URI or an
    /// `open.spotify.com` URL; both are accepted.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::InvalidUri`] if `uri` cannot be parsed, or
    /// [`AudioError::NotPlayable`] if it refers to an album, artist or
    /// playlist (those need a context — see [`Self::play_context`]).
    #[tracing::instrument(skip(self))]
    pub async fn play_uri(&self, uri: &str) -> AudioResult<()> {
        let parsed = state::parse_playable(uri)?;
        let canonical = state::normalise_uri(uri)?;
        tracing::info!(%uri, "loading single track");
        lock(&self.queue).play_single(QueueTrack {
            uri: canonical,
            title: String::new(),
            artists: Vec::new(),
            album: String::new(),
            art_url: None,
            duration: Duration::ZERO,
        });
        self.publish_queue();
        self.engine.player().load(parsed, true, 0);
        Ok(())
    }

    /// Play a context — a playlist, album or artist — through its track list.
    ///
    /// The caller (`app`) resolves the context's tracks from the Web API and
    /// passes them here; `audio` does not depend on `api`. Playback starts at
    /// `tracks[offset]` and Next/Prev then walk the list.
    ///
    /// `context_uri` is the context's own Spotify URI; `context_name` its
    /// display name (shown in the queue panel's "Next from …" header).
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NotPlayable`] when `tracks` is empty, or
    /// [`AudioError::InvalidUri`] if the chosen track's URI cannot be parsed.
    #[tracing::instrument(skip(self, tracks), fields(tracks = tracks.len()))]
    pub async fn play_context(
        &self,
        context_uri: &str,
        context_name: &str,
        tracks: Vec<QueueTrack>,
        offset: usize,
    ) -> AudioResult<()> {
        let started = lock(&self.queue).play_context(
            context_uri.to_owned(),
            context_name.to_owned(),
            tracks,
            offset,
        );
        let Some(track) = started else {
            return Err(AudioError::NotPlayable(context_uri.to_owned()));
        };
        self.publish_queue();
        self.load_track(&track)?;
        Ok(())
    }

    /// Add a track to the **end** of the manual queue.
    ///
    /// The manual queue plays before the context resumes; this entry is the
    /// last of the manual entries to play.
    #[tracing::instrument(skip(self, track), fields(uri = %track.uri))]
    pub async fn enqueue(&self, track: QueueTrack) {
        lock(&self.queue).enqueue(track);
        self.publish_queue();
    }

    /// Add a track to the **front** of the manual queue — it plays next.
    #[tracing::instrument(skip(self, track), fields(uri = %track.uri))]
    pub async fn play_next(&self, track: QueueTrack) {
        lock(&self.queue).play_next(track);
        self.publish_queue();
    }

    /// Skip to the next track: the manual queue first, then the context.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::InvalidUri`] if the next track's URI cannot be
    /// parsed. A no-op (returns `Ok`) when there is nothing left to play.
    #[tracing::instrument(skip(self))]
    pub async fn next(&self) -> AudioResult<()> {
        let next = lock(&self.queue).advance();
        match next {
            Some(track) => {
                self.publish_queue();
                self.load_track(&track)
            }
            None => {
                tracing::debug!("next: queue exhausted");
                Ok(())
            }
        }
    }

    /// Skip to the previous context track.
    ///
    /// "Previous" walks the context only — it never revisits the manual queue,
    /// matching Spotify. At the start of the context it is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::InvalidUri`] if the previous track's URI cannot
    /// be parsed.
    #[tracing::instrument(skip(self))]
    pub async fn previous(&self) -> AudioResult<()> {
        let prev = lock(&self.queue).previous();
        match prev {
            Some(track) => {
                self.publish_queue();
                self.load_track(&track)
            }
            None => {
                tracing::debug!("previous: at start of context");
                Ok(())
            }
        }
    }

    /// Jump straight to manual-queue entry `index`, dropping the entries before
    /// it. Used by the queue panel when a manual entry is clicked.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::InvalidUri`] if the chosen track's URI cannot be
    /// parsed. A no-op when `index` is out of range.
    #[tracing::instrument(skip(self))]
    pub async fn skip_to_manual(&self, index: usize) -> AudioResult<()> {
        let track = lock(&self.queue).skip_to_manual(index);
        if let Some(track) = track {
            self.publish_queue();
            self.load_track(&track)?;
        }
        Ok(())
    }

    /// Jump straight to context entry `index`. Used by the queue panel when an
    /// upcoming-context entry is clicked.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::InvalidUri`] if the chosen track's URI cannot be
    /// parsed. A no-op when `index` is out of range.
    #[tracing::instrument(skip(self))]
    pub async fn skip_to_context(&self, index: usize) -> AudioResult<()> {
        let track = lock(&self.queue).skip_to_context(index);
        if let Some(track) = track {
            self.publish_queue();
            self.load_track(&track)?;
        }
        Ok(())
    }

    /// Move manual-queue entry `from` to `to` — the drag-to-reorder primitive.
    #[tracing::instrument(skip(self))]
    pub async fn reorder_manual(&self, from: usize, to: usize) {
        lock(&self.queue).reorder_manual(from, to);
        self.publish_queue();
    }

    /// Remove manual-queue entry `index`.
    #[tracing::instrument(skip(self))]
    pub async fn remove_manual(&self, index: usize) {
        lock(&self.queue).remove_manual(index);
        self.publish_queue();
    }

    /// Set the repeat mode.
    #[tracing::instrument(skip(self))]
    pub async fn set_repeat(&self, mode: RepeatMode) {
        lock(&self.queue).set_repeat(mode);
        self.publish_queue();
    }

    /// Pause playback, keeping the current track and position.
    ///
    /// # Errors
    ///
    /// Currently infallible; returns [`AudioResult`] for API symmetry.
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

    /// Load `track` into the librespot player and start it playing.
    fn load_track(&self, track: &QueueTrack) -> AudioResult<()> {
        let parsed = state::parse_playable(&track.uri)?;
        tracing::info!(uri = %track.uri, "loading queued track");
        self.engine.player().load(parsed, true, 0);
        Ok(())
    }

    /// Swap a fresh queue snapshot into the shared `ArcSwap`.
    fn publish_queue(&self) {
        let snapshot = lock(&self.queue).snapshot();
        self.queue_state.store(Arc::new(snapshot));
    }

    /// Spawn the auto-advance task.
    ///
    /// It subscribes its own receiver to the player's event stream (each
    /// `get_player_event_channel` call adds an independent fan-out sender, so
    /// this does not steal events from the engine's own loop) and, on
    /// `EndOfTrack`, advances the queue and loads the next track.
    fn spawn_auto_advance(&self) {
        let mut events = self.engine.player().get_player_event_channel();
        let player = self.engine.player();
        let queue = Arc::clone(&self.queue);
        let queue_state = Arc::clone(&self.queue_state);

        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                if !matches!(event, PlayerEvent::EndOfTrack { .. }) {
                    continue;
                }
                let next = lock(&queue).advance();
                match next {
                    Some(track) => match state::parse_playable(&track.uri) {
                        Ok(parsed) => {
                            tracing::info!(uri = %track.uri, "auto-advancing to next track");
                            player.load(parsed, true, 0);
                        }
                        Err(err) => {
                            tracing::warn!(%err, uri = %track.uri, "auto-advance: unplayable");
                        }
                    },
                    None => tracing::debug!("auto-advance: queue exhausted, stopping"),
                }
                queue_state.store(Arc::new(lock(&queue).snapshot()));
            }
            tracing::debug!("auto-advance task ending: player event channel closed");
        });
    }
}

/// Lock the queue mutex, recovering the guard even if a previous holder
/// panicked. Queue operations never panic, so a poisoned lock would only
/// arise from a bug elsewhere; recovering keeps playback alive rather than
/// cascading the panic into every later control call.
fn lock(queue: &Mutex<Queue>) -> std::sync::MutexGuard<'_, Queue> {
    queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
