//! An [OpenSubsonic](https://opensubsonic.netlify.app/docs/) API client.
//!
//! Spottyfi uses this to treat any Subsonic-compatible server (Navidrome,
//! Gonic, Airsonic, Lyrion's media server, the reference server, …) as a
//! first-class music source alongside Spotify.
//!
//! One [`SubsonicClient`] is bound to one server. It speaks the salt-and-token
//! auth every server accepts, unwraps the `subsonic-response` envelope and
//! returns the permissive [`model`] types. Audio is plain HTTP — call
//! [`SubsonicClient::stream_url`] and hand the URL to the player.
#![warn(missing_docs)]
// `unwrap`/`expect` are denied in library code but allowed in unit tests,
// matching the workspace convention.
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod client;
mod error;
mod model;

pub use client::{SubsonicClient, SubsonicConfig};
pub use error::{SubsonicError, SubsonicResult};
pub use model::{
    Album, AlbumList, AlbumListKind, Artist, ArtistIndexEntry, ArtistsIndex, Playlist,
    PlaylistList, SearchResult, Song, Starred,
};
