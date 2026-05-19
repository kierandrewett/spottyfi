//! The [`MusicSource`] trait — the uniform interface every backend implements.

use async_trait::async_trait;
use thiserror::Error;

use crate::entity::{Album, SearchResults, Track};
use crate::identity::{SourceId, SourceKind};

/// An error raised by a music source.
#[derive(Debug, Error)]
pub enum SourceError {
    /// The source could not be reached (network, auth, server down).
    #[error("source unavailable: {0}")]
    Unavailable(String),
    /// The requested entity does not exist on this source.
    #[error("not found on this source")]
    NotFound,
    /// Any other source-specific failure.
    #[error("source error: {0}")]
    Other(String),
}

/// Convenience alias for results from a music source.
pub type SourceResult<T> = Result<T, SourceError>;

/// A music backend Spottyfi can search, browse and play.
///
/// Each configured source — Spotify, an OpenSubsonic server — implements
/// this. The app holds them in a [`SourceRegistry`](crate::registry::SourceRegistry)
/// and treats them interchangeably; only [`MusicSource::can_play`] and the
/// playback routing distinguish them.
#[async_trait]
pub trait MusicSource: Send + Sync {
    /// The configured id of this source instance.
    fn id(&self) -> &SourceId;

    /// The kind of backend this is.
    fn kind(&self) -> SourceKind;

    /// A human-readable name for the source (a server name, `"Spotify"`, …).
    fn display_name(&self) -> &str;

    /// Whether this source can actually play audio.
    ///
    /// `false` would mark a catalog-only source — searchable and able to
    /// de-duplicate against a playable source, but unable to stream itself.
    /// Both current sources (Spotify, Subsonic) return `true`.
    fn can_play(&self) -> bool;

    /// A streamable HTTP URL for a track, when the source plays over plain
    /// HTTP (Subsonic). `None` for sources with a non-URL playback path
    /// (Spotify, played through librespot) — the engine routes those itself.
    fn stream_url(&self, track_id: &str) -> Option<String>;

    /// A cover-art URL for an art id, when the source exposes one.
    fn cover_art_url(&self, art_id: &str) -> Option<String>;

    /// Search the source for `query`, capping each entity list at `limit`.
    ///
    /// # Errors
    ///
    /// [`SourceError`] when the source cannot be reached or the reply is bad.
    async fn search(&self, query: &str, limit: u32) -> SourceResult<SearchResults>;

    /// Fetch the tracks of one album.
    ///
    /// # Errors
    ///
    /// [`SourceError`] when the source cannot be reached or the reply is bad.
    async fn album_tracks(&self, album_id: &str) -> SourceResult<Vec<Track>>;

    /// Fetch the albums of one artist.
    ///
    /// # Errors
    ///
    /// [`SourceError`] when the source cannot be reached or the reply is bad.
    async fn artist_albums(&self, artist_id: &str) -> SourceResult<Vec<Album>>;
}
