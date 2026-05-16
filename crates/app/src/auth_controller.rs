//! Bridges the `auth` crate's async flows to the egui UI thread.
//!
//! The UI never blocks: it reads an [`AuthState`] snapshot from an
//! [`ArcSwap`] every frame and dispatches login/logout as detached tokio
//! tasks. Each task swaps in a new state and calls `request_repaint` so the
//! UI wakes to render it.

use std::sync::Arc;

use arc_swap::ArcSwap;
use spottyfi_auth::{AuthConfig, AuthError, AuthState, Session};
use tokio::runtime::Handle;

/// Shared, hot-swappable authentication state read by the UI each frame.
pub type SharedAuthState = Arc<ArcSwap<AuthState>>;

/// Owns the auth state snapshot and the live [`Session`] (once authenticated),
/// and spawns the `auth` flows onto the tokio runtime.
pub struct AuthController {
    /// Runtime handle used to spawn the async auth flows.
    runtime: Handle,
    /// egui context, so background tasks can wake the UI.
    egui_ctx: egui::Context,
    /// The state snapshot the UI projects.
    state: SharedAuthState,
    /// The live session, set on the runtime once login/restore succeeds.
    session: Arc<ArcSwap<Option<Session>>>,
    /// Resolved OAuth config, or the error explaining why it is unavailable.
    config: Result<AuthConfig, Arc<AuthError>>,
}

impl AuthController {
    /// Build the controller, capturing the runtime handle and egui context.
    ///
    /// The OAuth config is read from the environment here; a missing Client ID
    /// is not fatal — it surfaces as a [`AuthState::Failed`] on the login
    /// screen so the app still runs.
    pub fn new(runtime: Handle, egui_ctx: egui::Context) -> Self {
        let config = AuthConfig::from_env().map_err(Arc::new);
        if let Err(err) = &config {
            tracing::warn!(%err, "OAuth config unavailable");
        }

        Self {
            runtime,
            egui_ctx,
            state: Arc::new(ArcSwap::from_pointee(AuthState::LoggedOut)),
            session: Arc::new(ArcSwap::from_pointee(None)),
            config,
        }
    }

    /// The current auth state snapshot.
    pub fn state(&self) -> Arc<AuthState> {
        self.state.load_full()
    }

    /// The live session, if authenticated.
    pub fn session(&self) -> Option<Session> {
        self.session.load_full().as_ref().clone()
    }

    /// Set a new state snapshot and wake the UI.
    fn publish(state: &SharedAuthState, egui_ctx: &egui::Context, next: AuthState) {
        state.store(Arc::new(next));
        egui_ctx.request_repaint();
    }

    /// On startup, try to restore a session from a stored keyring token.
    pub fn spawn_restore(&self) {
        let Ok(config) = self.config.clone() else {
            // No Client ID: cannot restore. Surface it as a login-screen error.
            Self::publish(
                &self.state,
                &self.egui_ctx,
                AuthState::Failed(self.config_error_message()),
            );
            return;
        };

        let state = Arc::clone(&self.state);
        let session_slot = Arc::clone(&self.session);
        let egui_ctx = self.egui_ctx.clone();

        Self::publish(&state, &egui_ctx, AuthState::Restoring);

        self.runtime.spawn(async move {
            match spottyfi_auth::restore(&config).await {
                Ok(Some(session)) => {
                    let profile = session.profile().clone();
                    spottyfi_auth::spawn_refresh_task(&session);
                    session_slot.store(Arc::new(Some(session)));
                    Self::publish(&state, &egui_ctx, AuthState::LoggedIn(profile));
                }
                Ok(None) => {
                    Self::publish(&state, &egui_ctx, AuthState::LoggedOut);
                }
                Err(err) => {
                    tracing::warn!(%err, "session restore failed");
                    Self::publish(&state, &egui_ctx, AuthState::Failed(err.to_string()));
                }
            }
        });
    }

    /// Run the full browser OAuth login.
    pub fn spawn_login(&self) {
        let Ok(config) = self.config.clone() else {
            Self::publish(
                &self.state,
                &self.egui_ctx,
                AuthState::Failed(self.config_error_message()),
            );
            return;
        };

        let state = Arc::clone(&self.state);
        let session_slot = Arc::clone(&self.session);
        let egui_ctx = self.egui_ctx.clone();

        Self::publish(&state, &egui_ctx, AuthState::Authorizing);

        self.runtime.spawn(async move {
            match spottyfi_auth::login(&config).await {
                Ok(session) => {
                    let profile = session.profile().clone();
                    spottyfi_auth::spawn_refresh_task(&session);
                    session_slot.store(Arc::new(Some(session)));
                    Self::publish(&state, &egui_ctx, AuthState::LoggedIn(profile));
                }
                Err(err) => {
                    tracing::warn!(%err, "login failed");
                    Self::publish(&state, &egui_ctx, AuthState::Failed(err.to_string()));
                }
            }
        });
    }

    /// Log out: clear the keyring token and return to the login screen.
    pub fn spawn_logout(&self) {
        let state = Arc::clone(&self.state);
        let session_slot = Arc::clone(&self.session);
        let egui_ctx = self.egui_ctx.clone();

        self.runtime.spawn(async move {
            if let Err(err) = spottyfi_auth::logout().await {
                tracing::warn!(%err, "logout encountered an error");
            }
            session_slot.store(Arc::new(None));
            Self::publish(&state, &egui_ctx, AuthState::LoggedOut);
        });
    }

    /// The message shown when the OAuth config could not be built.
    fn config_error_message(&self) -> String {
        match &self.config {
            Ok(_) => String::new(),
            Err(err) => err.to_string(),
        }
    }
}
