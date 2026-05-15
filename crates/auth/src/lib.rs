//! Authentication: OAuth 2.0 PKCE flow, token refresh and keyring storage.
//!
//! Handles the browser-based login dance against `accounts.spotify.com`, stores
//! the refresh token in the platform keyring, and keeps a fresh access token
//! available for both `api` and `audio`. See `docs/auth.md`.
//!
//! Phase 0: placeholder. The flow arrives in Phase 1.
#![warn(missing_docs)]
