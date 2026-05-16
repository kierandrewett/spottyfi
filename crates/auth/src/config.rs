//! OAuth configuration: the Spotify Client ID, the loopback redirect port and
//! the set of scopes Spottyfi requests.

use crate::error::{AuthError, AuthResult};

/// Environment variable holding the Spotify app Client ID (required).
pub const ENV_CLIENT_ID: &str = "SPOTTYFI_CLIENT_ID";

/// Environment variable holding the loopback callback port (optional).
pub const ENV_REDIRECT_PORT: &str = "SPOTTYFI_REDIRECT_PORT";

/// Default loopback port for the OAuth callback server.
pub const DEFAULT_REDIRECT_PORT: u16 = 8888;

/// The full set of OAuth scopes Spottyfi requests, as defined in `PLAN.md`
/// Phase 1 and `docs/auth.md`.
///
/// These cover playback control, library and playlist access, follow
/// management and the `streaming`/`app-remote-control` scopes needed for
/// audio in later phases.
pub const SCOPES: &[&str] = &[
    "user-read-private",
    "user-read-email",
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-currently-playing",
    "playlist-read-private",
    "playlist-read-collaborative",
    "playlist-modify-private",
    "playlist-modify-public",
    "user-library-read",
    "user-library-modify",
    "user-top-read",
    "user-read-recently-played",
    "streaming",
    "app-remote-control",
    "user-follow-read",
    "user-follow-modify",
];

/// Configuration for the OAuth 2.0 PKCE flow.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// The Spotify app Client ID. PKCE has no client secret, so this value is
    /// not sensitive.
    pub client_id: String,
    /// The TCP port the local callback server binds to on `127.0.0.1`.
    pub redirect_port: u16,
}

impl AuthConfig {
    /// Build a config explicitly from a Client ID and a redirect port.
    #[must_use]
    pub fn new(client_id: String, redirect_port: u16) -> Self {
        Self {
            client_id,
            redirect_port,
        }
    }

    /// Build a config from the environment.
    ///
    /// Reads `SPOTTYFI_CLIENT_ID` (required) and `SPOTTYFI_REDIRECT_PORT`
    /// (optional, defaults to [`DEFAULT_REDIRECT_PORT`]).
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::MissingClientId`] if the Client ID is unset or
    /// empty, or [`AuthError::InvalidRedirectPort`] if the port variable is
    /// set but not a valid `u16`.
    pub fn from_env() -> AuthResult<Self> {
        let client_id = std::env::var(ENV_CLIENT_ID)
            .ok()
            .filter(|id| !id.trim().is_empty())
            .ok_or(AuthError::MissingClientId)?;

        let redirect_port = match std::env::var(ENV_REDIRECT_PORT) {
            Ok(raw) => raw
                .trim()
                .parse::<u16>()
                .map_err(|_| AuthError::InvalidRedirectPort(raw))?,
            Err(_) => DEFAULT_REDIRECT_PORT,
        };

        Ok(Self {
            client_id,
            redirect_port,
        })
    }

    /// The loopback redirect URI registered with the Spotify app.
    ///
    /// Must match the value on the developer dashboard **exactly**.
    #[must_use]
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.redirect_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialise environment-mutating tests so they cannot race each other.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn clear_env() {
        std::env::remove_var(ENV_CLIENT_ID);
        std::env::remove_var(ENV_REDIRECT_PORT);
    }

    #[test]
    fn from_env_requires_client_id() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        assert!(matches!(
            AuthConfig::from_env(),
            Err(AuthError::MissingClientId)
        ));
    }

    #[test]
    fn from_env_rejects_blank_client_id() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var(ENV_CLIENT_ID, "   ");
        let result = AuthConfig::from_env();
        clear_env();
        assert!(matches!(result, Err(AuthError::MissingClientId)));
    }

    #[test]
    fn from_env_defaults_the_port() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var(ENV_CLIENT_ID, "abc123");
        let config = AuthConfig::from_env().expect("config should build");
        clear_env();
        assert_eq!(config.client_id, "abc123");
        assert_eq!(config.redirect_port, DEFAULT_REDIRECT_PORT);
    }

    #[test]
    fn from_env_reads_a_custom_port() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var(ENV_CLIENT_ID, "abc123");
        std::env::set_var(ENV_REDIRECT_PORT, "9001");
        let config = AuthConfig::from_env().expect("config should build");
        clear_env();
        assert_eq!(config.redirect_port, 9001);
    }

    #[test]
    fn from_env_rejects_a_bad_port() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var(ENV_CLIENT_ID, "abc123");
        std::env::set_var(ENV_REDIRECT_PORT, "not-a-port");
        let result = AuthConfig::from_env();
        clear_env();
        assert!(matches!(result, Err(AuthError::InvalidRedirectPort(_))));
    }

    #[test]
    fn redirect_uri_is_well_formed() {
        let config = AuthConfig::new("id".to_owned(), 8888);
        assert_eq!(config.redirect_uri(), "http://127.0.0.1:8888/callback");
    }

    #[test]
    fn scopes_match_the_plan() {
        // The streaming scopes are load-bearing for Phase 2 audio.
        assert!(SCOPES.contains(&"streaming"));
        assert!(SCOPES.contains(&"app-remote-control"));
        assert_eq!(SCOPES.len(), 17);
    }
}
