//! The eframe application.
//!
//! Phase 1 wires authentication: the app owns a multi-thread tokio runtime,
//! drives the OAuth flow through an [`AuthController`], and renders the login
//! / logged-in screens. The dock surface, sidebar and transport bar arrive in
//! Phase 4.

use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::runtime::Runtime;

use crate::auth_controller::AuthController;
use crate::avatar::{self, SharedAvatar};
use crate::ui::{self, AuthIntent};

/// Top-level Spottyfi application state held by eframe.
pub struct SpottyfiApp {
    /// The tokio runtime that owns every async flow. Kept alive for the
    /// lifetime of the app; dropped (and shut down) when the window closes.
    _runtime: Runtime,
    /// Drives login / restore / logout and holds the auth state snapshot.
    auth: AuthController,
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
    /// tasks can request repaints, and spawns the startup session-restore.
    pub fn new(cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        tracing::debug!("constructing SpottyfiApp");

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("spottyfi-rt")
            .build()?;

        let auth = AuthController::new(runtime.handle().clone(), cc.egui_ctx.clone());
        // Startup: attempt to restore a session from the keyring.
        auth.spawn_restore();

        Ok(Self {
            _runtime: runtime,
            auth,
            avatar_image: Arc::new(ArcSwap::from_pointee(None)),
            avatar_texture: None,
            avatar_requested: false,
        })
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
}

impl eframe::App for SpottyfiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.ensure_avatar(&ctx);
        self.ensure_avatar_texture(&ctx);

        let state = self.auth.state();
        let intent = ui::auth_screen(ui, &state, self.avatar_texture.as_ref());

        match intent {
            Some(AuthIntent::Login) => self.auth.spawn_login(),
            Some(AuthIntent::Logout) => {
                self.auth.spawn_logout();
                // Drop the avatar so a future login fetches a fresh one.
                self.avatar_texture = None;
                self.avatar_requested = false;
                self.avatar_image.store(Arc::new(None));
            }
            None => {}
        }
    }
}
