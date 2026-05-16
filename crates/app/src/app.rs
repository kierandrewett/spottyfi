//! The eframe application.
//!
//! Phase 2 adds the audio engine: after login the app starts the librespot
//! engine on its tokio runtime, holds playback state behind an `ArcSwap`, and
//! renders a functional bottom transport bar plus a debug "play a URI"
//! control. The dock surface and sidebar still arrive in Phase 4.

use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::runtime::Runtime;

use crate::auth_controller::AuthController;
use crate::avatar::{self, SharedAvatar};
use crate::playback_controller::PlaybackControllerHandle;
use crate::transport::{self, TransportIntent, TransportUiState};
use crate::ui::{self, AuthIntent};

/// Top-level Spottyfi application state held by eframe.
pub struct SpottyfiApp {
    /// The tokio runtime that owns every async flow. Kept alive for the
    /// lifetime of the app; dropped (and shut down) when the window closes.
    _runtime: Runtime,
    /// Drives login / restore / logout and holds the auth state snapshot.
    auth: AuthController,
    /// Drives the audio engine and holds the playback state snapshot.
    playback: PlaybackControllerHandle,
    /// Per-frame UI state for the transport widgets (scrub drag, debug field).
    transport_ui: TransportUiState,
    /// The decoded avatar image, populated by a background task.
    avatar_image: SharedAvatar,
    /// The uploaded avatar texture, created once from `avatar_image`.
    avatar_texture: Option<egui::TextureHandle>,
    /// Whether an avatar fetch has already been kicked off for the session.
    avatar_requested: bool,
}

impl SpottyfiApp {
    /// Build the app from eframe's creation context.
    ///
    /// Creates the tokio runtime, captures the egui context so background
    /// tasks can request repaints, spawns the startup session-restore, and
    /// prepares the (not-yet-started) audio engine. `no_audio` reflects the
    /// `--no-audio` CLI flag.
    pub fn new(cc: &eframe::CreationContext<'_>, no_audio: bool) -> anyhow::Result<Self> {
        tracing::debug!("constructing SpottyfiApp");

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("spottyfi-rt")
            .build()?;

        let auth = AuthController::new(runtime.handle().clone(), cc.egui_ctx.clone());
        // Startup: attempt to restore a session from the keyring.
        auth.spawn_restore();

        let playback =
            PlaybackControllerHandle::new(runtime.handle().clone(), cc.egui_ctx.clone(), no_audio);

        Ok(Self {
            _runtime: runtime,
            auth,
            playback,
            transport_ui: TransportUiState::default(),
            avatar_image: Arc::new(ArcSwap::from_pointee(None)),
            avatar_texture: None,
            avatar_requested: false,
        })
    }

    /// Start the audio engine the first time we see a logged-in session.
    fn ensure_audio(&mut self) {
        if let Some(session) = self.auth.session() {
            self.playback.ensure_started(&session);
        }
    }

    /// Kick off a background avatar fetch the first time we see a logged-in
    /// session that has an avatar URL. Best-effort and non-blocking.
    fn ensure_avatar(&mut self, ctx: &egui::Context) {
        if self.avatar_requested {
            return;
        }
        let Some(session) = self.auth.session() else {
            return;
        };
        let Some(url) = session.profile().avatar_url.clone() else {
            return;
        };

        self.avatar_requested = true;
        avatar::spawn_fetch(
            self._runtime.handle(),
            ctx.clone(),
            url,
            Arc::clone(&self.avatar_image),
        );
    }

    /// Upload the decoded avatar to a texture once it is available.
    fn ensure_avatar_texture(&mut self, ctx: &egui::Context) {
        if self.avatar_texture.is_some() {
            return;
        }
        if let Some(image) = self.avatar_image.load_full().as_ref() {
            let texture =
                ctx.load_texture("user-avatar", image.clone(), egui::TextureOptions::LINEAR);
            self.avatar_texture = Some(texture);
        }
    }

    /// Apply a transport intent by dispatching it onto the playback engine.
    fn apply_transport_intent(&self, intent: TransportIntent) {
        match intent {
            TransportIntent::TogglePlayPause => self.playback.toggle_play_pause(),
            TransportIntent::Seek(position) => self.playback.seek(position),
            TransportIntent::SetVolume(volume) => self.playback.set_volume(volume),
            TransportIntent::PlayUri(uri) => self.playback.play_uri(uri),
        }
    }
}

impl eframe::App for SpottyfiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.ensure_avatar(&ctx);
        self.ensure_avatar_texture(&ctx);
        self.ensure_audio();

        let auth_state = self.auth.state();
        let logged_in = matches!(*auth_state, spottyfi_auth::AuthState::LoggedIn(_));

        // The transport bar is shown only once logged in: it needs a session
        // (and, ideally, a running engine) to be meaningful. It is a bottom
        // panel, so it must be added before the central auth screen.
        let mut transport_intent = None;
        if logged_in {
            let playback = self.playback.state();
            transport_intent = transport::transport_bar(ui, &mut self.transport_ui, &playback);
        }

        // The engine status `Arc` must outlive the `auth_screen` call, so
        // bind it here rather than borrowing a temporary.
        let engine_status = self.playback.status();
        let auth_intent = ui::auth_screen(
            ui,
            &auth_state,
            self.avatar_texture.as_ref(),
            logged_in.then(|| ui::DebugControls {
                ui_state: &mut self.transport_ui,
                engine: &engine_status,
            }),
        );

        if let Some(intent) = transport_intent {
            self.apply_transport_intent(intent);
        }

        match auth_intent {
            Some(AuthIntent::Login) => self.auth.spawn_login(),
            Some(AuthIntent::Logout) => {
                self.auth.spawn_logout();
                self.playback.shutdown();
                // Drop the avatar so a future login fetches a fresh one.
                self.avatar_texture = None;
                self.avatar_requested = false;
                self.avatar_image.store(Arc::new(None));
            }
            Some(AuthIntent::PlayDebug(intent)) => self.apply_transport_intent(intent),
            None => {}
        }
    }
}
