//! Bridges the `audio` crate's playback engine to the egui UI thread.
//!
//! Mirrors [`auth_controller`](crate::auth_controller): the UI reads a
//! [`PlaybackState`] snapshot from an [`ArcSwap`] every frame and dispatches
//! transport commands as detached tokio tasks. The audio engine itself swaps
//! fresh state in ~10× per second and calls `request_repaint` so the
//! transport bar animates.

use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use spottyfi_audio::{PlaybackController, PlaybackState, SharedPlaybackState};
use spottyfi_auth::Session;
use tokio::runtime::Handle;

/// Lifecycle of the audio engine, observed by the UI.
#[derive(Debug, Clone, Default)]
pub enum EngineStatus {
    /// No engine — either pre-login, or `--no-audio` was passed.
    #[default]
    Idle,
    /// The librespot session is connecting.
    Starting,
    /// The engine is connected and ready for playback.
    Ready,
    /// The engine failed to start; carries a human-readable message.
    Failed(String),
}

/// Owns the audio engine and the playback-state snapshot, and spawns transport
/// commands onto the tokio runtime.
pub struct PlaybackControllerHandle {
    /// Runtime handle used to spawn async playback commands.
    runtime: Handle,
    /// egui context, so the engine and command tasks can wake the UI.
    egui_ctx: egui::Context,
    /// The playback-state snapshot the UI projects.
    state: SharedPlaybackState,
    /// The live controller, set once the engine has started. Held behind an
    /// `Arc` so a clone can be moved into a spawned transport-command task.
    controller: Arc<ArcSwap<Option<Arc<PlaybackController>>>>,
    /// The engine lifecycle status, shown in the debug control.
    status: Arc<ArcSwap<EngineStatus>>,
    /// When true, the engine is never started (the `--no-audio` flag).
    audio_disabled: bool,
    /// Whether a start has already been kicked off for the current session.
    start_requested: bool,
}

impl PlaybackControllerHandle {
    /// Build the controller, capturing the runtime handle and egui context.
    ///
    /// `audio_disabled` reflects the `--no-audio` CLI flag; when set, the
    /// engine is never started and transport commands are no-ops.
    pub fn new(runtime: Handle, egui_ctx: egui::Context, audio_disabled: bool) -> Self {
        Self {
            runtime,
            egui_ctx,
            state: Arc::new(ArcSwap::from_pointee(PlaybackState::default())),
            controller: Arc::new(ArcSwap::from_pointee(None)),
            status: Arc::new(ArcSwap::from_pointee(EngineStatus::Idle)),
            audio_disabled,
            start_requested: false,
        }
    }

    /// The current playback-state snapshot.
    pub fn state(&self) -> Arc<PlaybackState> {
        self.state.load_full()
    }

    /// The current engine lifecycle status.
    pub fn status(&self) -> Arc<EngineStatus> {
        self.status.load_full()
    }

    /// Start the audio engine for `session`, if not already started.
    ///
    /// Called once per login. Does nothing when `--no-audio` is set, or when a
    /// start is already in flight. The engine authenticates librespot with the
    /// session's current OAuth access token.
    pub fn ensure_started(&mut self, session: &Session) {
        if self.audio_disabled || self.start_requested {
            return;
        }
        self.start_requested = true;

        let session = session.clone();
        let runtime = self.runtime.clone();
        let egui_ctx = self.egui_ctx.clone();
        let controller_slot = Arc::clone(&self.controller);
        let status = Arc::clone(&self.status);
        let state_slot = Arc::clone(&self.state);

        Self::publish_status(&status, &egui_ctx, EngineStatus::Starting);

        self.runtime.spawn(async move {
            let Some(token) = session.token().await else {
                Self::publish_status(
                    &status,
                    &egui_ctx,
                    EngineStatus::Failed("no access token on session".to_owned()),
                );
                return;
            };

            match PlaybackController::start(&token.access_token).await {
                Ok(controller) => {
                    // Mirror the engine's state into our shared slot, and wake
                    // the UI whenever it changes.
                    Self::bridge_state(&runtime, &egui_ctx, controller.state(), state_slot);
                    controller_slot.store(Arc::new(Some(Arc::new(controller))));
                    Self::publish_status(&status, &egui_ctx, EngineStatus::Ready);
                }
                Err(err) => {
                    tracing::warn!(%err, "audio engine failed to start");
                    Self::publish_status(&status, &egui_ctx, EngineStatus::Failed(err.to_string()));
                }
            }
        });
    }

    /// Tear the engine down (e.g. on logout) so a future login starts fresh.
    pub fn shutdown(&mut self) {
        self.controller.store(Arc::new(None));
        self.state.store(Arc::new(PlaybackState::default()));
        self.status.store(Arc::new(EngineStatus::Idle));
        self.start_requested = false;
    }

    /// Play a single track by Spotify URI or `open.spotify.com` URL.
    pub fn play_uri(&self, uri: String) {
        self.dispatch(move |controller| async move {
            if let Err(err) = controller.play_uri(&uri).await {
                tracing::warn!(%err, %uri, "play_uri failed");
            }
        });
    }

    /// Toggle between playing and paused based on the current snapshot.
    pub fn toggle_play_pause(&self) {
        let playing = self.state().playing;
        self.dispatch(move |controller| async move {
            let result = if playing {
                controller.pause().await
            } else {
                controller.resume().await
            };
            if let Err(err) = result {
                tracing::warn!(%err, "play/pause toggle failed");
            }
        });
    }

    /// Seek to `position` within the current track.
    pub fn seek(&self, position: Duration) {
        self.dispatch(move |controller| async move {
            if let Err(err) = controller.seek(position).await {
                tracing::warn!(%err, "seek failed");
            }
        });
    }

    /// Set the output volume from a `0.0..=1.0` fraction.
    pub fn set_volume(&self, volume: f32) {
        self.dispatch(move |controller| async move {
            if let Err(err) = controller.set_volume(volume).await {
                tracing::warn!(%err, "set_volume failed");
            }
        });
    }

    /// Spawn `body` against the live controller, if the engine is running.
    ///
    /// The controller `Arc` is cloned into the spawned task, so the task owns
    /// a stable handle and a concurrent [`Self::shutdown`] cannot invalidate
    /// a command already in flight.
    fn dispatch<F, Fut>(&self, body: F)
    where
        F: FnOnce(Arc<PlaybackController>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let Some(controller) = self.controller.load_full().as_ref().clone() else {
            tracing::debug!("transport command ignored: engine not running");
            return;
        };
        self.runtime.spawn(body(controller));
    }

    /// Publish a new engine status and wake the UI.
    fn publish_status(
        status: &ArcSwap<EngineStatus>,
        egui_ctx: &egui::Context,
        next: EngineStatus,
    ) {
        status.store(Arc::new(next));
        egui_ctx.request_repaint();
    }

    /// Spawn a task that mirrors the engine's state into our shared slot and
    /// requests a repaint whenever the snapshot changes.
    fn bridge_state(
        runtime: &Handle,
        egui_ctx: &egui::Context,
        engine_state: SharedPlaybackState,
        target: SharedPlaybackState,
    ) {
        let egui_ctx = egui_ctx.clone();
        runtime.spawn(async move {
            let mut last = engine_state.load_full();
            // The engine swaps its snapshot ~10Hz; poll a touch faster so the
            // UI never lags a frame behind a control action.
            let mut tick = tokio::time::interval(Duration::from_millis(80));
            loop {
                tick.tick().await;
                let current = engine_state.load_full();
                if !Arc::ptr_eq(&current, &last) {
                    last = Arc::clone(&current);
                    target.store(current);
                    egui_ctx.request_repaint();
                }
            }
        });
    }
}
