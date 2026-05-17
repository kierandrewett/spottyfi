//! Conversions from `rspotify` response types onto Spottyfi [`models`] types.
//!
//! Every mapping lives here so the rest of the crate — and the whole rest of
//! the workspace — never touches an `rspotify` type. The conversions are
//! deliberately total: missing-but-deprecated fields collapse to sensible
//! defaults rather than failing.
//!
//! [`models`]: spottyfi_models

#![allow(deprecated)] // rspotify marks Spotify-removed fields deprecated.

use rspotify::model as rs;
use rspotify::model::Id as _;

use spottyfi_models as m;

/// Map an `rspotify` image.
pub fn image(src: &rs::Image) -> m::Image {
    m::Image {
        url: src.url.clone(),
        width: src.width,
        height: src.height,
    }
}

/// Map a slice of `rspotify` images.
fn images(src: &[rs::Image]) -> Vec<m::Image> {
    src.iter().map(image).collect()
}

/// Map a private (signed-in) user.
pub fn private_user(src: &rs::PrivateUser) -> m::User {
    m::User {
        id: m::UserId::new(src.id.id()),
        display_name: src.display_name.clone(),
        images: src.images.as_deref().map(images).unwrap_or_default(),
    }
}

/// Map a public user profile.
pub fn public_user(src: &rs::PublicUser) -> m::User {
    m::User {
        id: m::UserId::new(src.id.id()),
        display_name: src.display_name.clone(),
        images: images(&src.images),
    }
}

/// Map a simplified artist.
pub fn simplified_artist(src: &rs::SimplifiedArtist) -> m::SimplifiedArtist {
    m::SimplifiedArtist {
        id: src.id.as_ref().map(|id| m::ArtistId::new(id.id())),
        name: src.name.clone(),
    }
}

/// Map a full artist.
pub fn artist(src: &rs::FullArtist) -> m::Artist {
    m::Artist {
        id: m::ArtistId::new(src.id.id()),
        name: src.name.clone(),
        images: images(&src.images),
        genres: src.genres.clone(),
        popularity: src.popularity,
    }
}

/// Map a simplified album.
pub fn simplified_album(src: &rs::SimplifiedAlbum) -> m::SimplifiedAlbum {
    m::SimplifiedAlbum {
        id: src.id.as_ref().map(|id| m::AlbumId::new(id.id())),
        name: src.name.clone(),
        images: images(&src.images),
        artists: src.artists.iter().map(simplified_artist).collect(),
        release_date: src.release_date.clone(),
    }
}

/// Map a full album, including its first page of tracks.
pub fn album(src: &rs::FullAlbum) -> m::Album {
    m::Album {
        id: m::AlbumId::new(src.id.id()),
        name: src.name.clone(),
        images: images(&src.images),
        artists: src.artists.iter().map(simplified_artist).collect(),
        release_date: src.release_date.clone(),
        total_tracks: src.tracks.total,
        tracks: page(&src.tracks, simplified_track),
    }
}

/// Map a simplified track.
pub fn simplified_track(src: &rs::SimplifiedTrack) -> m::SimplifiedTrack {
    m::SimplifiedTrack {
        id: src.id.as_ref().map(|id| m::TrackId::new(id.id())),
        name: src.name.clone(),
        artists: src.artists.iter().map(simplified_artist).collect(),
        duration_ms: duration_ms(src.duration),
        explicit: src.explicit,
        track_number: src.track_number,
        disc_number: src.disc_number,
    }
}

/// Map a full track.
pub fn track(src: &rs::FullTrack) -> m::Track {
    m::Track {
        id: src.id.as_ref().map(|id| m::TrackId::new(id.id())),
        name: src.name.clone(),
        artists: src.artists.iter().map(simplified_artist).collect(),
        album: simplified_album(&src.album),
        duration_ms: duration_ms(src.duration),
        explicit: src.explicit,
        popularity: src.popularity,
        track_number: src.track_number,
        is_local: src.is_local,
    }
}

/// Promote a simplified track to a full [`m::Track`].
///
/// Used for the recommendations endpoint, which returns simplified tracks.
/// Fields the simplified shape lacks (`popularity`) default to zero, and the
/// album falls back to an empty placeholder when absent.
pub fn simplified_to_track(src: &rs::SimplifiedTrack) -> m::Track {
    m::Track {
        id: src.id.as_ref().map(|id| m::TrackId::new(id.id())),
        name: src.name.clone(),
        artists: src.artists.iter().map(simplified_artist).collect(),
        album: src
            .album
            .as_ref()
            .map(simplified_album)
            .unwrap_or_else(empty_album),
        duration_ms: duration_ms(src.duration),
        explicit: src.explicit,
        popularity: 0,
        track_number: src.track_number,
        is_local: src.is_local,
    }
}

/// A placeholder simplified album for tracks that arrive without one.
fn empty_album() -> m::SimplifiedAlbum {
    m::SimplifiedAlbum {
        id: None,
        name: String::new(),
        images: Vec::new(),
        artists: Vec::new(),
        release_date: None,
    }
}

/// Map a simplified playlist.
pub fn simplified_playlist(src: &rs::SimplifiedPlaylist) -> m::SimplifiedPlaylist {
    m::SimplifiedPlaylist {
        id: m::PlaylistId::new(src.id.id()),
        name: src.name.clone(),
        description: None,
        images: images(&src.images),
        owner: public_user(&src.owner),
        collaborative: src.collaborative,
        total_tracks: src.items.total,
    }
}

/// Map a full playlist, including its first page of items.
pub fn playlist(src: &rs::FullPlaylist) -> m::Playlist {
    m::Playlist {
        id: m::PlaylistId::new(src.id.id()),
        name: src.name.clone(),
        description: src.description.clone(),
        images: images(&src.images),
        owner: public_user(&src.owner),
        collaborative: src.collaborative,
        total_tracks: src.items.total,
        tracks: page(&src.items, playlist_item),
    }
}

/// Map one playlist entry.
///
/// A non-track item (a podcast episode, or an item `rspotify` could not
/// resolve) maps to a [`m::PlaylistTrack`] with a `None` track, rather than
/// being dropped — the UI still shows the row, greyed out.
pub fn playlist_item(src: &rs::PlaylistItem) -> m::PlaylistTrack {
    let track = match &src.item {
        Some(rs::PlayableItem::Track(t)) => Some(track(t)),
        _ => None,
    };
    m::PlaylistTrack {
        track,
        added_at: src.added_at.map(|dt| dt.to_rfc3339()),
        added_by: src.added_by.as_ref().map(|u| m::UserId::new(u.id.id())),
    }
}

/// Map a saved track (a "Liked Songs" entry), carrying Spotify's `added_at`.
///
/// `GET /me/tracks` returns a `{ added_at, track }` wrapper; both halves are
/// kept so the Liked Songs "Date added" column and its sort work.
pub fn saved_track(src: &rs::SavedTrack) -> m::SavedTrack {
    m::SavedTrack {
        track: track(&src.track),
        added_at: Some(src.added_at.to_rfc3339()),
    }
}

/// Map a saved album.
pub fn saved_album(src: &rs::SavedAlbum) -> m::Album {
    album(&src.album)
}

/// Map a browse category.
pub fn category(src: &rs::Category) -> m::Category {
    m::Category {
        id: src.id.clone(),
        name: src.name.clone(),
        icons: images(&src.icons),
    }
}

/// Map an `rspotify` paging object onto a [`m::Page`], applying `f` to each
/// item.
pub fn page<S, T>(src: &rs::Page<S>, mut f: impl FnMut(&S) -> T) -> m::Page<T>
where
    S: serde::de::DeserializeOwned,
{
    m::Page {
        items: src.items.iter().map(&mut f).collect(),
        limit: src.limit,
        offset: src.offset,
        total: src.total,
        has_next: src.next.is_some(),
    }
}

/// Convert an `rspotify`/`chrono` millisecond duration to a `u32`.
fn duration_ms(d: chrono::Duration) -> u32 {
    d.num_milliseconds().clamp(0, i64::from(u32::MAX)) as u32
}

/// Project an `rspotify` multi-type search result onto [`m::SearchResults`].
pub fn search_results(src: &rs::SearchMultipleResult) -> m::SearchResults {
    m::SearchResults {
        tracks: src
            .tracks
            .as_ref()
            .map(|p| page(p, track))
            .unwrap_or_default(),
        artists: src
            .artists
            .as_ref()
            .map(|p| page(p, artist))
            .unwrap_or_default(),
        albums: src
            .albums
            .as_ref()
            .map(|p| page(p, simplified_album))
            .unwrap_or_default(),
        playlists: src
            .playlists
            .as_ref()
            .map(|p| page(p, simplified_playlist))
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deser<T: serde::de::DeserializeOwned>(json: &str) -> T {
        serde_json::from_str(json).expect("fixture should deserialise")
    }

    #[test]
    fn maps_a_full_track() {
        // Minimal but valid `FullTrack` JSON.
        let json = r#"{
            "album": {
                "album_type": "album",
                "artists": [{"external_urls":{},"href":null,"id":"4tZwfgrHOc3mvqYlEYSvVi","name":"Daft Punk"}],
                "external_urls": {},
                "href": null,
                "id": "4m2880jivSbbyEGAKfITCa",
                "images": [{"url":"https://i.example/cover.jpg","width":640,"height":640}],
                "name": "Random Access Memories",
                "release_date": "2013-05-17",
                "release_date_precision": "day"
            },
            "artists": [{"external_urls":{},"href":null,"id":"4tZwfgrHOc3mvqYlEYSvVi","name":"Daft Punk"}],
            "disc_number": 1,
            "duration_ms": 224000,
            "explicit": false,
            "external_ids": {},
            "external_urls": {},
            "href": null,
            "id": "0DiWol3AO6WpXZgp0goxAV",
            "is_local": false,
            "name": "Get Lucky",
            "preview_url": null,
            "track_number": 8,
            "type": "track"
        }"#;

        let full: rs::FullTrack = deser(json);
        let mapped = track(&full);

        assert_eq!(mapped.name, "Get Lucky");
        assert_eq!(mapped.duration_ms, 224_000);
        assert_eq!(
            mapped.id.as_ref().map(|i| i.0.as_str()),
            Some("0DiWol3AO6WpXZgp0goxAV")
        );
        assert_eq!(mapped.artists.len(), 1);
        assert_eq!(mapped.artists[0].name, "Daft Punk");
        assert_eq!(mapped.album.name, "Random Access Memories");
        assert!(!mapped.is_local);
    }

    #[test]
    fn saved_track_carries_added_at() {
        // A `SavedTrack` is a `{ added_at, track }` wrapper; the mapper must
        // keep the `added_at` the bare `Track` cannot carry.
        let json = r#"{
            "added_at": "2024-03-15T09:30:00Z",
            "track": {
                "album": {
                    "album_type": "album",
                    "artists": [{"external_urls":{},"href":null,"id":"4tZwfgrHOc3mvqYlEYSvVi","name":"Daft Punk"}],
                    "external_urls": {},
                    "href": null,
                    "id": "4m2880jivSbbyEGAKfITCa",
                    "images": [],
                    "name": "Random Access Memories",
                    "release_date": "2013-05-17",
                    "release_date_precision": "day"
                },
                "artists": [{"external_urls":{},"href":null,"id":"4tZwfgrHOc3mvqYlEYSvVi","name":"Daft Punk"}],
                "disc_number": 1,
                "duration_ms": 224000,
                "explicit": false,
                "external_ids": {},
                "external_urls": {},
                "href": null,
                "id": "0DiWol3AO6WpXZgp0goxAV",
                "is_local": false,
                "name": "Get Lucky",
                "preview_url": null,
                "track_number": 8,
                "type": "track"
            }
        }"#;

        let saved: rs::SavedTrack = deser(json);
        let mapped = saved_track(&saved);

        assert_eq!(mapped.track.name, "Get Lucky");
        assert_eq!(
            mapped.added_at.as_deref(),
            Some("2024-03-15T09:30:00+00:00")
        );
    }

    #[test]
    fn maps_a_page_and_its_has_next_flag() {
        let json = r#"{
            "href": "https://api.spotify.com/v1/x",
            "items": [
                {"external_urls":{},"href":null,"id":"4tZwfgrHOc3mvqYlEYSvVi","name":"Daft Punk"}
            ],
            "limit": 20,
            "next": "https://api.spotify.com/v1/x?offset=20",
            "offset": 0,
            "previous": null,
            "total": 57
        }"#;
        let src: rs::Page<rs::SimplifiedArtist> = deser(json);
        let mapped = page(&src, simplified_artist);

        assert_eq!(mapped.total, 57);
        assert_eq!(mapped.offset, 0);
        assert!(mapped.has_next);
        assert_eq!(mapped.items.len(), 1);
        assert_eq!(mapped.items[0].name, "Daft Punk");
    }

    #[test]
    fn page_without_next_link_has_no_further_pages() {
        let json = r#"{
            "href": "https://api.spotify.com/v1/x",
            "items": [],
            "limit": 20,
            "next": null,
            "offset": 40,
            "previous": "https://api.spotify.com/v1/x?offset=20",
            "total": 40
        }"#;
        let src: rs::Page<rs::SimplifiedArtist> = deser(json);
        let mapped = page(&src, simplified_artist);
        assert!(!mapped.has_next);
        assert!(mapped.is_empty());
    }
}
