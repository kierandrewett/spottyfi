//! The Spotify domain types: users, artists, albums, tracks and playlists.
//!
//! Spotify returns objects in two shapes: a *full* object (every field) and a
//! *simplified* object (the subset embedded inside other objects). Both shapes
//! are modelled here so a consumer never has to deal with absent fields that
//! the full object would have carried.
//!
//! Durations are milliseconds (`u32`) and timestamps are kept as the raw
//! RFC 3339 strings Spotify sends, so this crate stays free of a `chrono`
//! dependency.

use serde::{Deserialize, Serialize};

use crate::id::{AlbumId, ArtistId, PlaylistId, TrackId, UserId};
use crate::page::Page;

/// An image (album art, avatar, playlist cover) at one resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    /// The image source URL.
    pub url: String,
    /// The image width in pixels, if Spotify reported it.
    pub width: Option<u32>,
    /// The image height in pixels, if Spotify reported it.
    pub height: Option<u32>,
}

/// The signed-in user, or another user's public profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// The Spotify user id.
    pub id: UserId,
    /// The user's chosen display name, if set.
    pub display_name: Option<String>,
    /// The user's avatar images, largest first where Spotify orders them.
    pub images: Vec<Image>,
}

/// A simplified artist: the shape embedded in tracks and albums.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimplifiedArtist {
    /// The artist id. Absent only for local-file artists.
    pub id: Option<ArtistId>,
    /// The artist's name.
    pub name: String,
}

/// A full artist object, as returned by the artist endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    /// The artist id.
    pub id: ArtistId,
    /// The artist's name.
    pub name: String,
    /// The artist's images.
    pub images: Vec<Image>,
    /// Associated genre tags. Spotify has deprecated this field; it may be
    /// empty for apps registered after 2024-11-27.
    pub genres: Vec<String>,
    /// Artist popularity (0–100). Deprecated by Spotify; may be `0`.
    pub popularity: u32,
}

/// A simplified album: the shape embedded in tracks and search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimplifiedAlbum {
    /// The album id. Absent only for local-file albums.
    pub id: Option<AlbumId>,
    /// The album title.
    pub name: String,
    /// The album's cover art.
    pub images: Vec<Image>,
    /// The album's artists, simplified.
    pub artists: Vec<SimplifiedArtist>,
    /// The release date as Spotify sent it (`YYYY`, `YYYY-MM` or `YYYY-MM-DD`).
    pub release_date: Option<String>,
}

/// A full album object, as returned by the album endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Album {
    /// The album id.
    pub id: AlbumId,
    /// The album title.
    pub name: String,
    /// The album's cover art.
    pub images: Vec<Image>,
    /// The album's artists, simplified.
    pub artists: Vec<SimplifiedArtist>,
    /// The release date as Spotify sent it (`YYYY`, `YYYY-MM` or `YYYY-MM-DD`).
    pub release_date: String,
    /// The total number of tracks on the album.
    pub total_tracks: u32,
    /// The first page of the album's tracks.
    pub tracks: Page<SimplifiedTrack>,
}

/// A simplified track: the shape embedded in an album's track listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimplifiedTrack {
    /// The track id. Absent for local files.
    pub id: Option<TrackId>,
    /// The track title.
    pub name: String,
    /// The track's artists, simplified.
    pub artists: Vec<SimplifiedArtist>,
    /// Track length in milliseconds.
    pub duration_ms: u32,
    /// Whether the track is flagged explicit.
    pub explicit: bool,
    /// The track's 1-based position within its disc.
    pub track_number: u32,
    /// The 1-based disc number this track belongs to.
    pub disc_number: i32,
}

/// A full track object, as returned by search and playlist/saved listings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    /// The track id. Absent for local files.
    pub id: Option<TrackId>,
    /// The track title.
    pub name: String,
    /// The track's artists, simplified.
    pub artists: Vec<SimplifiedArtist>,
    /// The album this track appears on, simplified.
    pub album: SimplifiedAlbum,
    /// Track length in milliseconds.
    pub duration_ms: u32,
    /// Whether the track is flagged explicit.
    pub explicit: bool,
    /// Track popularity (0–100). Deprecated by Spotify; may be `0`.
    pub popularity: u32,
    /// The track's 1-based position within its disc.
    pub track_number: u32,
    /// Whether this is a user's local file rather than a Spotify catalogue
    /// track.
    pub is_local: bool,
}

/// A simplified playlist: the shape returned by the user-playlists listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimplifiedPlaylist {
    /// The playlist id.
    pub id: PlaylistId,
    /// The playlist name.
    pub name: String,
    /// The playlist description, if any.
    pub description: Option<String>,
    /// The playlist's cover images.
    pub images: Vec<Image>,
    /// The playlist's owner.
    pub owner: User,
    /// Whether the playlist is collaborative.
    pub collaborative: bool,
    /// The number of tracks the playlist contains.
    pub total_tracks: u32,
}

/// A full playlist object, as returned by the playlist endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    /// The playlist id.
    pub id: PlaylistId,
    /// The playlist name.
    pub name: String,
    /// The playlist description, if any.
    pub description: Option<String>,
    /// The playlist's cover images.
    pub images: Vec<Image>,
    /// The playlist's owner.
    pub owner: User,
    /// Whether the playlist is collaborative.
    pub collaborative: bool,
    /// The total number of tracks in the playlist.
    pub total_tracks: u32,
    /// The first page of the playlist's tracks.
    pub tracks: Page<PlaylistTrack>,
}

/// One entry in a playlist: a track plus the metadata of how it was added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistTrack {
    /// The track itself. `None` when Spotify cannot resolve the item (an
    /// unavailable track, or a non-track item this client does not model).
    pub track: Option<Track>,
    /// When the track was added, as an RFC 3339 string, if Spotify reported it.
    pub added_at: Option<String>,
    /// The id of the user who added the track, if reported.
    pub added_by: Option<UserId>,
}

/// A saved ("Liked Songs") entry: a track plus the date it was saved.
///
/// Spotify's `GET /me/tracks` response is a `{ added_at, track }` wrapper, not
/// a bare track; this type carries the `added_at` timestamp the bare
/// [`Track`] cannot, so the Liked Songs "Date added" column and its sort work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTrack {
    /// The saved track itself.
    pub track: Track,
    /// When the track was saved, as an RFC 3339 string, if Spotify reported it.
    pub added_at: Option<String>,
}

/// A browse category (a tile on the Browse page).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    /// The category id (used to fetch the category's playlists).
    pub id: String,
    /// The human-readable category name.
    pub name: String,
    /// The category's icon images.
    pub icons: Vec<Image>,
}

/// The combined results of a multi-type search.
///
/// Each field carries one page of results of that kind; a field is empty when
/// the search did not request — or found nothing of — that kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResults {
    /// Matching tracks.
    pub tracks: Page<Track>,
    /// Matching artists.
    pub artists: Page<Artist>,
    /// Matching albums.
    pub albums: Page<SimplifiedAlbum>,
    /// Matching playlists.
    pub playlists: Page<SimplifiedPlaylist>,
}

/// The kind of a Spotify Connect device — drives the device picker's icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceKind {
    /// A desktop or laptop computer.
    Computer,
    /// A phone.
    Smartphone,
    /// A tablet.
    Tablet,
    /// A speaker, including smart speakers.
    Speaker,
    /// A television.
    Tv,
    /// Anything else (receiver, set-top box, console, car, …).
    Other,
}

/// A Spotify Connect playback device (`GET /me/player/devices`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    /// The device's Spotify id. `None` when Spotify omits it for a device
    /// that cannot currently be targeted for a playback transfer.
    pub id: Option<String>,
    /// The human-readable device name.
    pub name: String,
    /// Whether playback is currently happening on this device.
    pub is_active: bool,
    /// Whether Spotify forbids controlling this device through the Web API.
    pub is_restricted: bool,
    /// The device kind, used to pick the picker icon.
    pub kind: DeviceKind,
    /// The device's current volume as a `0..=100` percentage, if known.
    pub volume_percent: Option<u32>,
}

/// A snapshot of Spotify playback happening on another Connect device.
///
/// Fetched from `GET /me/player`; the transport's Connect banner shows it and
/// the transport controls drive it while playback is remote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePlayback {
    /// Whether the remote device is currently playing (vs. paused).
    pub is_playing: bool,
    /// The active device's name.
    pub device_name: String,
    /// The playing track's title (empty when nothing is loaded).
    pub track_title: String,
    /// The playing track's artist line (empty when unknown).
    pub artist: String,
    /// Playback progress into the track, in milliseconds.
    pub progress_ms: u32,
    /// The track's total duration, in milliseconds (`0` when unknown).
    pub duration_ms: u32,
}
