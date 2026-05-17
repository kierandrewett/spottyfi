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
use spottyfi_audio::{
    EngineConfig, PlaybackController, PlaybackState, QueueState, QueueTrack, SharedPlaybackState,
    SharedQueueState, SpectrumAnalyzer, TrackWaveform,
};
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
    /// The queue-state snapshot the UI's queue panel projects.
    queue_state: SharedQueueState,
    /// The live controller, set once the engine has started. Held behind an
    /// `Arc` so a clone can be moved into a spawned transport-command task.
    controller: Arc<ArcSwap<Option<Arc<PlaybackController>>>>,
    /// The engine lifecycle status, shown in the debug control.
    status: Arc<ArcSwap<EngineStatus>>,
    /// When true, the engine is never started (the `--no-audio` flag).
    audio_disabled: bool,
    /// Whether a start has already been kicked off for the current session.
    start_requested: bool,
    /// The live session, kept so the engine can be restarted in place when the
    /// audio settings change (librespot bakes them in at connect time).
    session: Option<Session>,
    /// The engine config the running engine was started with.
    engine_config: EngineConfig,
    /// The most recent equaliser settings, applied to the engine the moment it
    /// becomes ready. Held here so a start (or restart) always begins with the
    /// user's persisted EQ rather than the engine default (a flat bypass).
    equalizer: (bool, [f32; spottyfi_audio::EQ_BAND_COUNT]),
    /// The off-thread spectrum analyser the visualiser panel reads. Created
    /// once up front so its handle is stable; its analysis task is (re)spawned
    /// against the live tap each time the engine starts.
    spectrum: SpectrumAnalyzer,
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
            queue_state: Arc::new(ArcSwap::from_pointee(QueueState::default())),
            controller: Arc::new(ArcSwap::from_pointee(None)),
            status: Arc::new(ArcSwap::from_pointee(EngineStatus::Idle)),
            audio_disabled,
            start_requested: false,
            session: None,
            engine_config: EngineConfig::default(),
            equalizer: (false, [0.0; spottyfi_audio::EQ_BAND_COUNT]),
            spectrum: SpectrumAnalyzer::new(),
        }
    }

    /// The latest background full-song waveform, once the engine is running.
    ///
    /// The transport's seek bar draws this; `None` before the engine starts.
    /// The UI matches `TrackWaveform::uri` against the playing track before
    /// using the envelope, so a waveform still being decoded is ignored.
    pub fn waveform(&self) -> Option<Arc<TrackWaveform>> {
        let controller = self.controller.load_full();
        controller
            .as_ref()
            .as_ref()
            .map(|controller| controller.waveform_analyzer().current())
    }

    /// The shared spectrum-analyser handle the visualiser panel reads.
    ///
    /// The handle is stable for the controller's lifetime; the analysis task
    /// behind it is (re)spawned when the engine starts.
    pub fn spectrum(&self) -> SpectrumAnalyzer {
        self.spectrum.clone()
    }

    /// The current playback-state snapshot.
    pub fn state(&self) -> Arc<PlaybackState> {
        self.state.load_full()
    }

    /// The current queue-state snapshot, read by the queue panel each frame.
    pub fn queue_state(&self) -> Arc<QueueState> {
        self.queue_state.load_full()
    }

    /// The current engine lifecycle status.
    pub fn status(&self) -> Arc<EngineStatus> {
        self.status.load_full()
    }

    /// Start the audio engine for `session`, if not already started.
    ///
    /// Called once per login. Does nothing when `--no-audio` is set, or when a
    /// start is already in flight. The engine authenticates librespot with the
    /// session's current OAuth access token and applies `config` (stream
    /// quality, normalisation) — those are baked into librespot's
    /// `PlayerConfig` at connect time. `equalizer` is the persisted EQ to
    /// apply the moment the engine is ready; it can still be changed live
    /// afterwards via [`Self::set_equalizer`].
    pub fn ensure_started(
        &mut self,
        session: &Session,
        config: EngineConfig,
        equalizer: (bool, [f32; spottyfi_audio::EQ_BAND_COUNT]),
    ) {
        if self.audio_disabled || self.start_requested {
            return;
        }
        self.session = Some(session.clone());
        self.engine_config = config;
        self.equalizer = equalizer;
        self.start_requested = true;
        self.spawn_start(session.clone(), config);
    }

    /// Restart the running engine so a changed [`EngineConfig`] takes effect.
    ///
    /// librespot bakes the stream quality and normalisation into its
    /// `PlayerConfig` when the session connects, so a settings change can only
    /// be applied by reconnecting. A no-op when nothing changed, `--no-audio`
    /// is set, or no session has been seen yet. The current track is not
    /// resumed — the engine starts idle, as after a fresh login.
    pub fn restart_with(&mut self, config: EngineConfig) {
        if self.audio_disabled || config == self.engine_config {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        tracing::info!("restarting audio engine to apply new audio settings");
        self.engine_config = config;
        self.controller.store(Arc::new(None));
        self.state.store(Arc::new(PlaybackState::default()));
        self.queue_state.store(Arc::new(QueueState::default()));
        self.spawn_start(session, config);
    }

    /// Spawn the engine-start task: connect librespot, bridge the snapshots
    /// and publish the lifecycle status. Shared by first start and restart.
    fn spawn_start(&self, session: Session, config: EngineConfig) {
        let runtime = self.runtime.clone();
        let egui_ctx = self.egui_ctx.clone();
        let controller_slot = Arc::clone(&self.controller);
        let status = Arc::clone(&self.status);
        let state_slot = Arc::clone(&self.state);
        let queue_slot = Arc::clone(&self.queue_state);
        let spectrum = self.spectrum.clone();
        let (eq_enabled, eq_gains) = self.equalizer;

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

            match PlaybackController::start(&token.access_token, config).await {
                Ok(controller) => {
                    // Apply the persisted equaliser before the engine goes
                    // live, so the first track plays with the user's EQ.
                    if let Err(err) = controller.set_equalizer(eq_enabled, eq_gains).await {
                        tracing::warn!(%err, "applying startup equaliser failed");
                    }
                    // Spawn the off-thread spectrum analyser against the
                    // engine's post-EQ tap — the visualiser panel reads it.
                    {
                        let ctx = egui_ctx.clone();
                        SpectrumAnalyzer::spawn_into(
                            &spectrum,
                            &runtime,
                            controller.audio_tap(),
                            move || ctx.request_repaint(),
                        );
                    }
                    // Mirror the engine's playback and queue state into our
                    // shared slots, waking the UI whenever either changes.
                    Self::bridge_snapshot(&runtime, &egui_ctx, controller.state(), state_slot);
                    Self::bridge_snapshot(
                        &runtime,
                        &egui_ctx,
                        controller.queue_state(),
                        queue_slot,
                    );
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
        self.queue_state.store(Arc::new(QueueState::default()));
        self.status.store(Arc::new(EngineStatus::Idle));
        self.start_requested = false;
        self.session = None;
    }

    /// Play a single track by Spotify URI or `open.spotify.com` URL.
    pub fn play_uri(&self, uri: String) {
        self.dispatch(move |controller| async move {
            if let Err(err) = controller.play_uri(&uri).await {
                tracing::warn!(%err, %uri, "play_uri failed");
            }
        });
    }

    /// Play a context — a playlist/album's full resolved track list — starting
    /// at `offset`, so Next/Prev walk the list.
    pub fn play_context(&self, uri: String, name: String, tracks: Vec<QueueTrack>, offset: usize) {
        self.dispatch(move |controller| async move {
            if let Err(err) = controller.play_context(&uri, &name, tracks, offset).await {
                tracing::warn!(%err, %uri, "play_context failed");
            }
        });
    }

    /// Skip to the next track (manual queue first, then the context).
    pub fn next(&self) {
        self.dispatch(|controller| async move {
            if let Err(err) = controller.next().await {
                tracing::warn!(%err, "next failed");
            }
        });
    }

    /// Skip to the previous context track.
    pub fn previous(&self) {
        self.dispatch(|controller| async move {
            if let Err(err) = controller.previous().await {
                tracing::warn!(%err, "previous failed");
            }
        });
    }

    /// Add a track to the front of the manual queue (play it next).
    pub fn play_next(&self, track: QueueTrack) {
        self.dispatch(move |controller| async move {
            controller.play_next(track).await;
        });
    }

    /// Add a track to the end of the manual queue.
    pub fn enqueue(&self, track: QueueTrack) {
        self.dispatch(move |controller| async move {
            controller.enqueue(track).await;
        });
    }

    /// Jump to manual-queue entry `index` — a click in the queue panel.
    pub fn skip_to_manual(&self, index: usize) {
        self.dispatch(move |controller| async move {
            if let Err(err) = controller.skip_to_manual(index).await {
                tracing::warn!(%err, index, "skip_to_manual failed");
            }
        });
    }

    /// Jump to context entry `index` — a click in the queue panel's "Next
    /// from …" section. `index` is an absolute index into the context's
    /// track list, which the panel derives from `QueueState::context_index`.
    pub fn skip_to_context(&self, index: usize) {
        self.dispatch(move |controller| async move {
            if let Err(err) = controller.skip_to_context(index).await {
                tracing::warn!(%err, index, "skip_to_context failed");
            }
        });
    }

    /// Move manual-queue entry `from` to `to` — the drag-to-reorder primitive.
    pub fn reorder_manual(&self, from: usize, to: usize) {
        self.dispatch(move |controller| async move {
            controller.reorder_manual(from, to).await;
        });
    }

    /// Remove manual-queue entry `index`.
    pub fn remove_manual(&self, index: usize) {
        self.dispatch(move |controller| async move {
            controller.remove_manual(index).await;
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

    /// Set shuffle on or off. The currently-playing track is preserved.
    pub fn set_shuffle(&self, shuffle: bool) {
        self.dispatch(move |controller| async move {
            controller.set_shuffle(shuffle).await;
        });
    }

    /// Set the repeat mode (off / repeat-all / repeat-one).
    pub fn set_repeat(&self, mode: spottyfi_audio::RepeatMode) {
        self.dispatch(move |controller| async move {
            controller.set_repeat(mode).await;
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

    /// Push the 10-band equaliser configuration to the audio engine.
    ///
    /// Unlike the start-time audio settings, the equaliser applies live: the
    /// custom audio backend's DSP picks the new gains up on its next decoded
    /// packet. Called on engine start and whenever the EQ settings change.
    ///
    /// The settings are also remembered so a later engine (re)start re-applies
    /// them — if the engine is not yet running the live push is a no-op but the
    /// stored value is replayed by [`Self::spawn_start`] once it is ready.
    pub fn set_equalizer(
        &mut self,
        enabled: bool,
        band_gains_db: [f32; spottyfi_audio::EQ_BAND_COUNT],
    ) {
        self.equalizer = (enabled, band_gains_db);
        self.dispatch(move |controller| async move {
            if let Err(err) = controller.set_equalizer(enabled, band_gains_db).await {
                tracing::warn!(%err, "set_equalizer failed");
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

    /// Spawn a task that mirrors an engine-owned `ArcSwap` snapshot into our
    /// shared slot and requests a repaint whenever the snapshot changes.
    ///
    /// Used for both the playback state (swapped ~10Hz) and the queue state
    /// (swapped on every queue mutation and on auto-advance).
    fn bridge_snapshot<T: Send + Sync + 'static>(
        runtime: &Handle,
        egui_ctx: &egui::Context,
        engine_state: Arc<ArcSwap<T>>,
        target: Arc<ArcSwap<T>>,
    ) {
        let egui_ctx = egui_ctx.clone();
        runtime.spawn(async move {
            let mut last = engine_state.load_full();
            // Poll at ~60Hz — comfortably faster than the engine's ~30Hz
            // position swap — so the UI mirror never aliases against the
            // source rate and the scrubber tracks playback smoothly.
            let mut tick = tokio::time::interval(Duration::from_millis(16));
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
