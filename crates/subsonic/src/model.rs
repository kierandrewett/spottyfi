//! Deserialised OpenSubsonic response types.
//!
//! These mirror the [OpenSubsonic data model](https://opensubsonic.netlify.app/docs/responses/).
//! Every field beyond the id is `#[serde(default)]`: real-world servers (Navidrome,
//! Gonic, Airsonic, the reference server, …) each omit a different subset, so a
//! permissive shape keeps the client working everywhere.

use serde::Deserialize;

/// A song (the Subsonic `Child` type, used for tracks).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    /// The server-assigned id, stable for streaming and starring.
    pub id: String,
    /// The track title.
    #[serde(default)]
    pub title: String,
    /// The album name, if the server reports it.
    #[serde(default)]
    pub album: Option<String>,
    /// The (primary) artist name.
    #[serde(default)]
    pub artist: Option<String>,
    /// The album this song belongs to, for navigation.
    #[serde(default)]
    pub album_id: Option<String>,
    /// The artist this song belongs to, for navigation.
    #[serde(default)]
    pub artist_id: Option<String>,
    /// The track number within its album.
    #[serde(default)]
    pub track: Option<u32>,
    /// The disc number within a multi-disc album.
    #[serde(default)]
    pub disc_number: Option<u32>,
    /// The release year.
    #[serde(default)]
    pub year: Option<u32>,
    /// The primary genre name.
    #[serde(default)]
    pub genre: Option<String>,
    /// The cover-art id, passed to `getCoverArt`.
    #[serde(default)]
    pub cover_art: Option<String>,
    /// The track duration in whole seconds.
    #[serde(default)]
    pub duration: Option<u32>,
    /// The stream bit rate in kbps, when transcoding/known.
    #[serde(default)]
    pub bit_rate: Option<u32>,
    /// The original file's MIME type (e.g. `audio/flac`).
    #[serde(default)]
    pub content_type: Option<String>,
    /// The original file extension (e.g. `flac`, `mp3`).
    #[serde(default)]
    pub suffix: Option<String>,
    /// The file size in bytes.
    #[serde(default)]
    pub size: Option<u64>,
    /// The MusicBrainz recording id — an OpenSubsonic extension, used for
    /// high-confidence cross-source de-duplication when present.
    #[serde(default)]
    pub music_brainz_id: Option<String>,
    /// An ISO-8601 timestamp; present and non-empty when the song is starred.
    #[serde(default)]
    pub starred: Option<String>,
}

/// An artist (the Subsonic `ArtistID3` type).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    /// The server-assigned artist id.
    pub id: String,
    /// The artist name.
    #[serde(default)]
    pub name: String,
    /// The cover-art id for the artist.
    #[serde(default)]
    pub cover_art: Option<String>,
    /// A direct artist-image URL, when the server provides one.
    #[serde(default)]
    pub artist_image_url: Option<String>,
    /// How many albums the artist has.
    #[serde(default)]
    pub album_count: Option<u32>,
    /// The artist's albums — populated by `getArtist`, empty elsewhere.
    #[serde(default)]
    pub album: Vec<Album>,
    /// The MusicBrainz artist id (OpenSubsonic extension).
    #[serde(default)]
    pub music_brainz_id: Option<String>,
}

/// An album (the Subsonic `AlbumID3` type).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    /// The server-assigned album id.
    pub id: String,
    /// The album name.
    #[serde(default)]
    pub name: String,
    /// The album-artist name.
    #[serde(default)]
    pub artist: Option<String>,
    /// The album-artist id, for navigation.
    #[serde(default)]
    pub artist_id: Option<String>,
    /// The cover-art id, passed to `getCoverArt`.
    #[serde(default)]
    pub cover_art: Option<String>,
    /// How many songs the album has.
    #[serde(default)]
    pub song_count: Option<u32>,
    /// The total album duration in whole seconds.
    #[serde(default)]
    pub duration: Option<u32>,
    /// The release year.
    #[serde(default)]
    pub year: Option<u32>,
    /// The primary genre name.
    #[serde(default)]
    pub genre: Option<String>,
    /// The album's songs — populated by `getAlbum`, empty elsewhere.
    #[serde(default)]
    pub song: Vec<Song>,
    /// The MusicBrainz release id (OpenSubsonic extension).
    #[serde(default)]
    pub music_brainz_id: Option<String>,
}

/// A playlist (the Subsonic `Playlist` type).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    /// The server-assigned playlist id.
    pub id: String,
    /// The playlist name.
    #[serde(default)]
    pub name: String,
    /// A free-text description.
    #[serde(default)]
    pub comment: Option<String>,
    /// The owning user's name.
    #[serde(default)]
    pub owner: Option<String>,
    /// How many songs the playlist has.
    #[serde(default)]
    pub song_count: Option<u32>,
    /// The total playlist duration in whole seconds.
    #[serde(default)]
    pub duration: Option<u32>,
    /// The cover-art id, passed to `getCoverArt`.
    #[serde(default)]
    pub cover_art: Option<String>,
    /// The playlist's songs — populated by `getPlaylist`, empty elsewhere.
    #[serde(default)]
    pub entry: Vec<Song>,
}

/// The result of a `search3` query.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// Matching artists.
    #[serde(default)]
    pub artist: Vec<Artist>,
    /// Matching albums.
    #[serde(default)]
    pub album: Vec<Album>,
    /// Matching songs.
    #[serde(default)]
    pub song: Vec<Song>,
}

/// The `artists` payload of `getArtists` — artists grouped by index letter.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArtistsIndex {
    /// The index buckets (`A`, `B`, …).
    #[serde(default)]
    pub index: Vec<ArtistIndexEntry>,
}

/// One index bucket within [`ArtistsIndex`].
#[derive(Debug, Clone, Deserialize)]
pub struct ArtistIndexEntry {
    /// The index letter or label.
    #[serde(default)]
    pub name: String,
    /// The artists filed under this letter.
    #[serde(default)]
    pub artist: Vec<Artist>,
}

/// The `albumList2` payload of `getAlbumList2`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlbumList {
    /// The albums in the requested list.
    #[serde(default)]
    pub album: Vec<Album>,
}

/// The `playlists` payload of `getPlaylists`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlaylistList {
    /// The user's playlists.
    #[serde(default)]
    pub playlist: Vec<Playlist>,
}

/// The `starred2` payload of `getStarred2` — the user's starred library.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Starred {
    /// Starred artists.
    #[serde(default)]
    pub artist: Vec<Artist>,
    /// Starred albums.
    #[serde(default)]
    pub album: Vec<Album>,
    /// Starred songs.
    #[serde(default)]
    pub song: Vec<Song>,
}

/// Which list `getAlbumList2` should return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumListKind {
    /// Most recently added albums.
    Newest,
    /// Most frequently played albums.
    Frequent,
    /// Most recently played albums.
    Recent,
    /// A random selection.
    Random,
    /// Alphabetical by album name.
    AlphabeticalByName,
}

impl AlbumListKind {
    /// The Subsonic `type` query value for this list.
    #[must_use]
    pub fn as_param(self) -> &'static str {
        match self {
            AlbumListKind::Newest => "newest",
            AlbumListKind::Frequent => "frequent",
            AlbumListKind::Recent => "recent",
            AlbumListKind::Random => "random",
            AlbumListKind::AlphabeticalByName => "alphabeticalByName",
        }
    }
}
