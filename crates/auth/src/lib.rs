//! Authentication: OAuth 2.0 PKCE flow, token refresh and keyring storage.
//!
//! Handles the browser-based login dance against `accounts.spotify.com`, stores
//! the OAuth token in the platform keyring under service `dev.drewett.spottyfi`,
//! and keeps a fresh access token available for both `api` and `audio`. See
//! `docs/auth.md`.
//!
//! ## Flow overview
//!
//! - [`login`] runs the full browser PKCE dance and persists the token.
//! - [`restore`] reloads a previously stored token, refreshing it if expired.
//! - [`logout`] wipes the stored token.
//! - [`Session`] is the authenticated handle; [`spawn_refresh_task`] keeps its
//!   access token fresh in the background.
#![warn(missing_docs)]

mod callback;
mod config;
mod error;
mod session;
mod storage;

use rspotify::clients::OAuthClient as _;
use rspotify::{AuthCodePkceSpotify, Config as RspotifyConfig, Credentials, OAuth};

pub use crate::callback::CALLBACK_TIMEOUT;
pub use crate::config::{
    AuthConfig, DEFAULT_REDIRECT_PORT, ENV_CLIENT_ID, ENV_REDIRECT_PORT, SCOPES,
};
pub use crate::error::{AuthError, AuthResult};
pub use crate::session::{spawn_refresh_task, Session, UserProfile};
pub use crate::storage::{KEYRING_SERVICE, TOKEN_ACCOUNT};

/// The lifecycle state of authentication, observed by the UI.
#[derive(Debug, Clone, Default)]
pub enum AuthState {
    /// No session; the login screen is shown.
    #[default]
    LoggedOut,
    /// A stored token is being reloaded and validated at startup.
    Restoring,
    /// The browser login dance is in progress.
    Authorizing,
    /// Authenticated; carries the signed-in user's profile.
    LoggedIn(UserProfile),
    /// Authentication failed; carries a human-readable message.
    Failed(String),
}

/// Build an rspotify PKCE client for the given config.
///
/// rspotify's own file-based token cache is disabled (`token_cached = false`)
/// because Spottyfi persists tokens itself, in the platform keyring.
fn build_client(config: &AuthConfig) -> AuthCodePkceSpotify {
    let creds = Credentials::new_pkce(&config.client_id);

    let oauth = OAuth {
        redirect_uri: config.redirect_uri(),
        scopes: SCOPES.iter().map(|s| (*s).to_owned()).collect(),
        ..OAuth::default()
    };

    let rspotify_config = RspotifyConfig {
        // Spottyfi owns token persistence; rspotify must not write its own
        // `.spotify_token_cache.json`.
        token_cached: false,
        // Refresh the access token automatically when a request finds it stale.
        token_refreshing: true,
        ..RspotifyConfig::default()
    };

    AuthCodePkceSpotify::with_config(creds, oauth, rspotify_config)
}

/// Restore a session from a token previously stored in the keyring.
///
/// Returns `Ok(None)` when no token is stored (a fresh install, or after
/// [`logout`]). When a token is found it is loaded into a client, refreshed if
/// it has expired, re-persisted, and used to fetch the user's profile.
///
/// # Errors
///
/// Returns an [`AuthError`] if the keyring read fails, the stored token cannot
/// be deserialised, a needed refresh fails, or the profile fetch fails.
#[tracing::instrument(skip_all)]
pub async fn restore(config: &AuthConfig) -> AuthResult<Option<Session>> {
    let Some(token) = storage::load_token()? else {
        tracing::debug!("no stored token; nothing to restore");
        return Ok(None);
    };

    let client = build_client(config);
    *client
        .token
        .lock()
        .await
        .map_err(|err| AuthError::Task(format!("token mutex poisoned: {err:?}")))? = Some(token);

    // Renew up front if the token is already stale, so the first API call
    // (the profile fetch) is guaranteed a valid access token.
    session::refresh_if_needed(&client).await?;

    let session = Session::from_client(client).await?;
    tracing::info!(user = %session.profile().id, "session restored from keyring");
    Ok(Some(session))
}

/// Run the full OAuth 2.0 PKCE browser login.
///
/// Builds the authorize URL with a random `state`, opens the system browser,
/// runs the local callback server, exchanges the returned code for a token,
/// persists that token to the keyring, and fetches the user's profile.
///
/// # Errors
///
/// Returns an [`AuthError`] for any failure in the dance: a browser that
/// won't open, a callback timeout, a CSRF `state` mismatch, a failed token
/// exchange, or a failed profile fetch.
#[tracing::instrument(skip_all)]
pub async fn login(config: &AuthConfig) -> AuthResult<Session> {
    let mut client = build_client(config);

    // `get_authorize_url` also generates and stores the PKCE verifier on the
    // client, which `request_token` later needs.
    let authorize_url = client
        .get_authorize_url(None)
        .map_err(|err| AuthError::TokenExchange(format!("building authorize URL: {err}")))?;

    let expected_state = client.oauth.state.clone();
    let port = config.redirect_port;

    tracing::info!("opening the system browser for Spotify login");
    webbrowser::open(&authorize_url).map_err(|err| AuthError::Browser(err.to_string()))?;

    // tiny_http is blocking; run it off the async runtime's worker threads.
    let code =
        tokio::task::spawn_blocking(move || callback::run_callback_server(port, &expected_state))
            .await
            .map_err(|err| AuthError::Task(format!("callback task panicked: {err}")))??;

    tracing::debug!("authorization code received; exchanging for a token");
    client
        .request_token(&code)
        .await
        .map_err(|err| AuthError::TokenExchange(err.to_string()))?;

    let token = client
        .token
        .lock()
        .await
        .map_err(|err| AuthError::Task(format!("token mutex poisoned: {err:?}")))?
        .clone()
        .ok_or_else(|| AuthError::TokenExchange("token missing after exchange".to_owned()))?;
    storage::save_token(&token)?;

    let session = Session::from_client(client).await?;
    tracing::info!(user = %session.profile().id, "login complete");
    Ok(session)
}

/// Log out: delete the stored OAuth token from the keyring.
///
/// The caller is responsible for any additional teardown (clearing caches,
/// returning the UI to the login screen).
///
/// # Errors
///
/// Returns [`AuthError::Keyring`] if the keyring delete fails.
#[tracing::instrument]
pub async fn logout() -> AuthResult<()> {
    storage::delete_token()?;
    tracing::info!("logged out; keyring token cleared");
    Ok(())
}
