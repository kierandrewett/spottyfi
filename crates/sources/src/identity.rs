//! Source identity — which library an entity came from.

use serde::{Deserialize, Serialize};

/// The kind of backend a source speaks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceKind {
    /// Spotify, streamed via librespot.
    Spotify,
    /// An OpenSubsonic-compatible server (Navidrome, Gonic, Airsonic, …).
    Subsonic,
    /// Apple Music — a catalog source; playback (when wired) is via an
    /// embedded MusicKit web player.
    AppleMusic,
}

impl SourceKind {
    /// A short human-readable label, shown on source badges.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Spotify => "Spotify",
            SourceKind::Subsonic => "Subsonic",
            SourceKind::AppleMusic => "Apple Music",
        }
    }

    /// De-duplication preference: when the same track exists on several
    /// sources, the one with the highest priority is chosen as primary.
    ///
    /// A self-hosted Subsonic library ranks first (the user owns it, no rate
    /// limits, often lossless); Spotify next (full streaming); Apple Music
    /// last (catalog-only until CEF playback lands).
    #[must_use]
    pub fn dedup_priority(self) -> u8 {
        match self {
            SourceKind::Subsonic => 3,
            SourceKind::Spotify => 2,
            SourceKind::AppleMusic => 1,
        }
    }
}

/// A stable id for one *configured* source instance.
///
/// There is one Spotify, but a user may add several Subsonic servers, so the
/// id is a free string (`"spotify"`, `"subsonic:<uuid>"`, …) rather than just
/// the [`SourceKind`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceId(pub String);

impl SourceId {
    /// Borrow the id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A reference to one entity (track, album, artist) within one source.
///
/// This is what makes every entity in the app traceable to its origin — the
/// pair `(source, id)` uniquely locates it, and `kind` drives the UI badge
/// and the playback routing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceRef {
    /// The configured source instance this entity belongs to.
    pub source: SourceId,
    /// The kind of that source.
    pub kind: SourceKind,
    /// The entity's native id *within* that source.
    pub id: String,
}

impl SourceRef {
    /// Build a reference.
    #[must_use]
    pub fn new(source: SourceId, kind: SourceKind, id: impl Into<String>) -> Self {
        Self {
            source,
            kind,
            id: id.into(),
        }
    }
}
