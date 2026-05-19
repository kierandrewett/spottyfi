//! An [Apple Music catalog API](https://developer.apple.com/documentation/applemusicapi)
//! client.
//!
//! Spottyfi uses this to make Apple Music a searchable, browsable source.
//! Apple Music audio is FairPlay-DRM protected, so this client is **catalog
//! metadata only** — an Apple Music track becomes playable by de-duplicating
//! it onto a playable source (Subsonic, Spotify) via a shared identifier such
//! as the ISRC, or, later, through an embedded MusicKit web player.
//!
//! The catalog needs only a developer-token JWT, supplied by the caller.
#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod client;
mod error;
mod model;

pub use client::AppleMusicClient;
pub use error::{AppleMusicError, AppleMusicResult};
pub use model::{Album, Artist, SearchResults, Song};
