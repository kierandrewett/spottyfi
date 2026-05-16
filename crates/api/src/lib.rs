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
//! Cacheable GETs (`album`, `artist`, `playlist`) go through the
//! [`MetadataLayer`]: a tiny in-memory hot cache ([`ObjectCache`]) in front of
//! the persistent SQLite [`MetadataCache`](spottyfi_cache::MetadataCache) from
//! the `cache` crate, which is the source of truth. The layer implements
//! **stale-while-revalidate** — a cached object is returned immediately, and
//! when it is older than the freshness window a background refresh is spawned.
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
//!
//! ## Last.fm
//!
//! Because Spotify's discovery endpoints are dead for new apps, Phase 7's
//! Browse surface sources charts and recommendations from Last.fm — see the
//! [`lastfm`] module. [`lastfm::LastfmClient`] talks to the Last.fm API;
//! [`lastfm::LastfmResolver`] maps the artist/track *names* Last.fm returns
//! back onto Spotify objects via [`SpotifyApi::search`]. Last.fm needs a free
//! API key in `SPOTTYFI_LASTFM_API_KEY`; with none set the client returns
//! [`lastfm::LastfmError::NotConfigured`] and Browse degrades gracefully.
#![warn(missing_docs)]
// `unwrap`/`expect` are denied in library code but allowed in unit tests,
// per the workspace convention in `PLAN.md`.
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod cache;
mod client;
mod error;
pub mod lastfm;
mod map;
mod metadata;
mod retry;
mod traits;

pub use crate::cache::ObjectCache;
pub use crate::client::SpotifyClient;
pub use crate::error::{ApiError, ApiResult};
pub use crate::metadata::MetadataLayer;
pub use crate::retry::RetryPolicy;
pub use crate::traits::{ItemStream, MockSpotifyApi, SearchType, SpotifyApi};
