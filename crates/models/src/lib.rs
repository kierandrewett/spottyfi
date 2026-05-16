//! Shared domain types for Spottyfi.
//!
//! This crate holds the plain data types (`Track`, `Album`, `Artist`,
//! `Playlist`, …) that every other crate exchanges. It depends on nothing else
//! in the workspace so it can be freely imported anywhere.
//!
//! ## Why a separate crate?
//!
//! The `api` crate maps `rspotify`'s response types onto these types. Keeping
//! the domain types here means `ui`, `state` and the rest of the workspace
//! never see an `rspotify` type — the Web API client is an implementation
//! detail behind the `api` crate's trait.
//!
//! ## Identifiers
//!
//! Spotify objects are identified by a short base-62 id and a `spotify:…` URI.
//! The id newtypes ([`TrackId`], [`AlbumId`], [`ArtistId`], [`PlaylistId`],
//! [`UserId`]) wrap the bare id string; each can render its canonical URI and
//! `open.spotify.com` URL via the [`SpotifyId`] trait.
#![warn(missing_docs)]

mod id;
mod page;
mod types;

pub use crate::id::{AlbumId, ArtistId, PlaylistId, SpotifyId, TrackId, UserId};
pub use crate::page::Page;
pub use crate::types::{
    Album, Artist, Category, Image, Playlist, PlaylistTrack, SearchResults, SimplifiedAlbum,
    SimplifiedArtist, SimplifiedPlaylist, SimplifiedTrack, Track, User,
};
