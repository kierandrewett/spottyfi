//! The authenticated session: the rspotify client plus the signed-in user's
//! profile, and the auto-refresh background task.

use std::sync::Arc;
use std::time::Duration;

use rspotify::clients::{BaseClient as _, OAuthClient as _};
use rspotify::model::Id as _;
use rspotify::{AuthCodePkceSpotify, Token};

use crate::error::{AuthError, AuthResult};
use crate::storage;

/// How long before a token's expiry the refresh task wakes to renew it.
const REFRESH_LEAD_TIME: Duration = Duration::from_secs(120);

/// The shortest interval the refresh task will ever sleep, so a near-expired
/// or clock-skewed token cannot spin the loop.
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// The signed-in user's public profile, as shown in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserProfile {
    /// The Spotify user id (stable, used as a cache/keyring key elsewhere).
    pub id: String,
    /// The user's display name, if they have set one.
    pub display_name: Option<String>,
    /// URL of the user's avatar image, if any.
    pub avatar_url: Option<String>,
}

/// An authenticated Spotify session.
///
/// Wraps the rspotify client (which holds the live token) and the profile of
/// the user it is authenticated as.
#[derive(Clone)]
pub struct Session {
    client: Arc<AuthCodePkceSpotify>,
    profile: UserProfile,
}

impl Session {
    /// Build a session from an authenticated client by fetching the user's
    /// profile via the `me()` endpoint.
    pub(crate) async fn from_client(client: AuthCodePkceSpotify) -> AuthResult<Self> {
        let profile = fetch_profile(&client).await?;
        Ok(Self {
            client: Arc::new(client),
            profile,
        })
    }

    /// The signed-in user's profile.
    #[must_use]
    pub fn profile(&self) -> &UserProfile {
        &self.profile
    }

    /// The underlying rspotify client, shared for use by `api`/`audio` later.
    #[must_use]
    pub fn client(&self) -> Arc<AuthCodePkceSpotify> {
        Arc::clone(&self.client)
    }

    /// A snapshot of the current OAuth token, if one is present.
    pub async fn token(&self) -> Option<Token> {
        self.client.token.lock().await.ok()?.clone()
    }
}

/// Fetch the current user's profile and project it onto [`UserProfile`].
async fn fetch_profile(client: &AuthCodePkceSpotify) -> AuthResult<UserProfile> {
    let user = client
        .me()
        .await
        .map_err(|err| AuthError::Http(format!("fetching user profile: {err}")))?;

    let avatar_url = user
        .images
        .as_ref()
        .and_then(|images| images.first())
        .map(|image| image.url.clone());

    Ok(UserProfile {
        id: user.id.id().to_owned(),
        display_name: user.display_name.clone(),
        avatar_url,
    })
}

/// Refresh the access token if it has expired (or is about to), and persist
/// the refreshed token back to the keyring.
///
/// Returns `true` when a refresh actually happened.
pub(crate) async fn refresh_if_needed(client: &AuthCodePkceSpotify) -> AuthResult<bool> {
    let expired = match client.token.lock().await {
        Ok(guard) => guard.as_ref().is_none_or(Token::is_expired),
        Err(_) => true,
    };

    if !expired {
        return Ok(false);
    }

    client
        .refresh_token()
        .await
        .map_err(|err| AuthError::TokenExchange(format!("refreshing access token: {err}")))?;

    if let Some(token) = client.token.lock().await.ok().and_then(|g| g.clone()) {
        storage::save_token(&token)?;
    }

    Ok(true)
}

/// How long the refresh task should sleep before the next renewal, derived
/// from the token's `expires_at`.
fn sleep_until_refresh(token: &Token) -> Duration {
    let now = chrono::Utc::now();
    match token.expires_at {
        Some(expires_at) => {
            let until_expiry = expires_at - now;
            // Wake `REFRESH_LEAD_TIME` before expiry, clamped to a sane floor.
            let secs = until_expiry.num_seconds() - REFRESH_LEAD_TIME.as_secs() as i64;
            if secs <= 0 {
                MIN_REFRESH_INTERVAL
            } else {
                Duration::from_secs(secs as u64).max(MIN_REFRESH_INTERVAL)
            }
        }
        None => MIN_REFRESH_INTERVAL,
    }
}

/// How many consecutive refresh failures before the task gives up.
///
/// One failure is almost always a transient network blip — the task retries.
/// Only a *sustained* run of failures means the refresh token is genuinely
/// dead, at which point stopping (the user must log in again) is correct.
const MAX_CONSECUTIVE_REFRESH_FAILURES: u32 = 5;

/// Spawn the background token-refresh task.
///
/// The task loops forever: it sleeps until shortly before the access token
/// expires, refreshes it, re-persists it to the keyring, and repeats. With
/// rspotify's silent auto-refresh disabled (see `build_client`), this is the
/// sole in-session refresher, so it must be resilient — a single failure is
/// retried rather than fatal. It is detached; dropping the returned
/// [`tokio::task::JoinHandle`] lets it run for the lifetime of the process.
#[tracing::instrument(skip_all)]
pub fn spawn_refresh_task(session: &Session) -> tokio::task::JoinHandle<()> {
    let client = session.client();
    tokio::spawn(async move {
        let mut failures: u32 = 0;
        loop {
            let token = client.token.lock().await.ok().and_then(|g| g.clone());
            // After a failure, retry soon rather than waiting for the (stale)
            // expiry-derived interval.
            let nap = if failures > 0 {
                MIN_REFRESH_INTERVAL
            } else {
                token
                    .as_ref()
                    .map_or(MIN_REFRESH_INTERVAL, sleep_until_refresh)
            };

            tracing::debug!(seconds = nap.as_secs(), "token refresh task sleeping");
            tokio::time::sleep(nap).await;

            match refresh_if_needed(&client).await {
                Ok(true) => {
                    failures = 0;
                    tracing::info!("access token refreshed");
                }
                Ok(false) => {
                    failures = 0;
                    tracing::trace!("access token still valid; no refresh");
                }
                Err(err) => {
                    failures += 1;
                    if failures >= MAX_CONSECUTIVE_REFRESH_FAILURES {
                        tracing::error!(%err, failures, "token refresh keeps failing; stopping");
                        break;
                    }
                    tracing::warn!(%err, failures, "token refresh failed; will retry");
                }
            }
        }
    })
}
