//! Spotify Web API client.
//!
//! Wraps `rspotify` and exposes typed, cache-aware methods for the rest of the
//! app. The public surface is the [`SpotifyApi`] trait, so the UI can be
//! developed offline against the `mockall`-generated [`MockSpotifyApi`].
//!
//! ## Layers
//!
//! - [`SpotifyApi`] — the mockable trait every consumer depends on.
//! - [`SpotifyClient`] — the real implementation, built from an
//!   [`auth::Session`](spottyfi_auth::Session).
//! - Responses are mapped from `rspotify` types onto [`spottyfi_models`] types
//!   so no `rspotify` type escapes this crate.
//!
//! ## Rate limiting & retries
//!
//! On HTTP 429 the client honours the `Retry-After` header; transient
//! transport failures are retried with capped exponential backoff plus full
//! jitter. See [`RetryPolicy`].
//!
//! ## Caching
//!
//! Cacheable GETs (currently `album` and `artist`) sit behind an in-memory
//! [`ObjectCache`]. This is a Phase 3 seam — the on-disk `cache` crate
//! (Phase 9) replaces it.
//!
//! ## Deprecated endpoints
//!
//! Spotify restricted several Web API endpoints to apps that held extended
//! quota before 2024-11-27 — Recommendations, Featured Playlists, a Category's
//! playlists, Related Artists, and Audio Features/Analysis. A newly registered
//! app gets 403/404 from these, so `featured_playlists` and `recommendations`
//! are expected to fail for Spottyfi; `artist_top_tracks` and
//! `browse_categories` are *not* on that list and should work. Any 403/404 is
//! surfaced as [`ApiError::EndpointUnavailable`] rather than a misleading
//! `NotFound` or empty result. See `docs/questions.md`.
#![warn(missing_docs)]
// `unwrap`/`expect` are denied in library code but allowed in unit tests,
// per the workspace convention in `PLAN.md`.
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod cache;
mod client;
mod error;
mod map;
mod retry;
mod traits;

pub use crate::cache::ObjectCache;
pub use crate::client::SpotifyClient;
pub use crate::error::{ApiError, ApiResult};
pub use crate::retry::RetryPolicy;
pub use crate::traits::{ItemStream, MockSpotifyApi, SearchType, SpotifyApi};
