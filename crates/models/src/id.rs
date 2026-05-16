//! Newtype wrappers for Spotify object identifiers.
//!
//! Each Spotify object is named by a short base-62 id. These newtypes keep ids
//! of different kinds from being confused at compile time, and the
//! [`SpotifyId`] trait derives the canonical `spotify:type:id` URI and the
//! `open.spotify.com` URL from a bare id.

use serde::{Deserialize, Serialize};

/// Behaviour shared by every Spotify id newtype.
pub trait SpotifyId {
    /// The Spotify object kind, as it appears in a URI (`track`, `album`, …).
    const KIND: &'static str;

    /// The bare base-62 id, without any `spotify:` prefix.
    fn id(&self) -> &str;

    /// The canonical Spotify URI, e.g. `spotify:track:4y4VO05kYgUTo2bzbox1an`.
    fn uri(&self) -> String {
        format!("spotify:{}:{}", Self::KIND, self.id())
    }

    /// The shareable `open.spotify.com` URL for this object.
    fn url(&self) -> String {
        format!("https://open.spotify.com/{}/{}", Self::KIND, self.id())
    }
}

/// Defines an id newtype with the boilerplate `SpotifyId` impl.
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            /// Wrap a bare base-62 id.
            #[must_use]
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }
        }

        impl SpotifyId for $name {
            const KIND: &'static str = $kind;

            fn id(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

define_id!(
    /// Identifies a track.
    TrackId,
    "track"
);
define_id!(
    /// Identifies an album.
    AlbumId,
    "album"
);
define_id!(
    /// Identifies an artist.
    ArtistId,
    "artist"
);
define_id!(
    /// Identifies a playlist.
    PlaylistId,
    "playlist"
);
define_id!(
    /// Identifies a user.
    UserId,
    "user"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_and_url_are_derived_from_kind() {
        let id = TrackId::new("4y4VO05kYgUTo2bzbox1an");
        assert_eq!(id.uri(), "spotify:track:4y4VO05kYgUTo2bzbox1an");
        assert_eq!(
            id.url(),
            "https://open.spotify.com/track/4y4VO05kYgUTo2bzbox1an"
        );
    }

    #[test]
    fn album_kind_differs_from_track() {
        assert_eq!(AlbumId::KIND, "album");
        assert_eq!(ArtistId::new("x").uri(), "spotify:artist:x");
    }
}
