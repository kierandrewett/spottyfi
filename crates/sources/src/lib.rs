//! The multi-source music abstraction.
//!
//! Spottyfi treats Spotify, OpenSubsonic servers and Apple Music as
//! interchangeable **sources**. This crate defines the shared vocabulary so
//! the rest of the app never special-cases a backend:
//!
//! * [`identity`] — which source an entity came from ([`SourceRef`]).
//! * [`entity`] — source-neutral [`Track`], [`Album`], [`Artist`].
//! * [`dedup`] — collapse the same entity across sources into one entry.
//! * [`source`] — the [`MusicSource`] trait every backend implements.
//! * [`registry`] — the set of configured sources the app searches at once.
#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

pub mod dedup;
pub mod entity;
pub mod identity;
pub mod registry;
pub mod source;
mod subsonic_source;

pub use dedup::{dedup_albums, dedup_artists, dedup_tracks, Deduped, DedupedTrack};
pub use entity::{Album, Artist, SearchResults, Track};
pub use identity::{SourceId, SourceKind, SourceRef};
pub use registry::SourceRegistry;
pub use source::{MusicSource, SourceError, SourceResult};
pub use subsonic_source::SubsonicSource;
