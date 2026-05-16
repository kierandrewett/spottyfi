//! Keyring-backed persistence for the OAuth token.
//!
//! The rspotify [`Token`] is serialised to JSON and stored in the platform
//! credential store (Secret Service on Linux, Keychain on macOS, Credential
//! Manager on Windows) under service `dev.drewett.spottyfi`.
//!
//! A stable account name ([`TOKEN_ACCOUNT`]) is used so the token can be
//! loaded on the next launch *before* the Spotify user id is known.

use std::sync::Once;

use keyring_core::Entry;
use rspotify::Token;

use crate::error::{AuthError, AuthResult};

/// Keyring service name for all Spottyfi credentials.
pub const KEYRING_SERVICE: &str = "dev.drewett.spottyfi";

/// Stable keyring account name under which the OAuth token is stored.
pub const TOKEN_ACCOUNT: &str = "oauth-token";

/// Ensures the platform keyring store is registered exactly once.
static KEYRING_INIT: Once = Once::new();

/// Register the platform-native keyring store as the process default.
///
/// `keyring` 4.x requires a store to be selected before [`Entry`] can be used.
/// On Linux this selects the Secret Service (rather than the kernel keyutils
/// store) so tokens survive a reboot. Safe to call repeatedly.
fn init_keyring() {
    KEYRING_INIT.call_once(|| {
        // `true` => prefer the Secret Service over kernel keyutils on Linux.
        if let Err(err) = keyring::use_native_store(true) {
            tracing::warn!(%err, "could not register the native keyring store");
        }
    });
}

/// Build an [`Entry`] for the OAuth token.
fn token_entry() -> AuthResult<Entry> {
    init_keyring();
    Entry::new(KEYRING_SERVICE, TOKEN_ACCOUNT).map_err(AuthError::Keyring)
}

/// Persist `token` to the platform keyring as JSON, replacing any prior value.
///
/// # Errors
///
/// Returns [`AuthError::Serde`] if the token cannot be serialised, or
/// [`AuthError::Keyring`] if the credential store rejects the write.
pub fn save_token(token: &Token) -> AuthResult<()> {
    let json = serde_json::to_string(token)?;
    token_entry()?
        .set_password(&json)
        .map_err(AuthError::Keyring)?;
    tracing::debug!("OAuth token saved to the keyring");
    Ok(())
}

/// Load the OAuth token from the platform keyring.
///
/// Returns `Ok(None)` when no token has been stored yet.
///
/// # Errors
///
/// Returns [`AuthError::Keyring`] on a store-access failure, or
/// [`AuthError::Serde`] if the stored value is not valid token JSON.
pub fn load_token() -> AuthResult<Option<Token>> {
    let entry = token_entry()?;
    match entry.get_password() {
        Ok(json) => {
            let token: Token = serde_json::from_str(&json)?;
            tracing::debug!("OAuth token loaded from the keyring");
            Ok(Some(token))
        }
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(err) => Err(AuthError::Keyring(err)),
    }
}

/// Delete the OAuth token from the platform keyring.
///
/// A missing entry is treated as success — deleting nothing is still "deleted".
///
/// # Errors
///
/// Returns [`AuthError::Keyring`] on a store-access failure.
pub fn delete_token() -> AuthResult<()> {
    let entry = token_entry()?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => {
            tracing::debug!("OAuth token cleared from the keyring");
            Ok(())
        }
        Err(err) => Err(AuthError::Keyring(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn sample_token() -> Token {
        Token {
            access_token: "access-abc".to_owned(),
            expires_in: chrono::Duration::seconds(3600),
            expires_at: Some(chrono::Utc::now()),
            refresh_token: Some("refresh-xyz".to_owned()),
            scopes: HashSet::from(["user-read-private".to_owned()]),
        }
    }

    #[test]
    fn token_json_round_trips() {
        // Pure (de)serialisation — no keyring involved, always runs.
        let token = sample_token();
        let json = serde_json::to_string(&token).expect("serialises");
        let restored: Token = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(token.access_token, restored.access_token);
        assert_eq!(token.refresh_token, restored.refresh_token);
        assert_eq!(token.scopes, restored.scopes);
    }

    /// Round-trips a token through the real platform keyring.
    ///
    /// Ignored by default: needs an unlocked Secret Service / Keychain, which
    /// is not available in headless CI.
    #[test]
    #[ignore = "requires a real platform credential store"]
    fn keyring_save_load_delete_round_trip() {
        let token = sample_token();
        save_token(&token).expect("save");
        let loaded = load_token().expect("load").expect("token present");
        assert_eq!(loaded.access_token, token.access_token);
        delete_token().expect("delete");
        assert!(load_token().expect("load after delete").is_none());
    }
}
