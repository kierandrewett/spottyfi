//! The Apple Music catalog HTTP client.
//!
//! Talks to the documented Apple Music API at `https://api.music.apple.com`.
//! Catalog search and lookup need only a **developer token** (a JWT the app
//! supplies); personalised library calls would additionally need a music-user
//! token, which this catalog-only client does not use.
//!
//! Apple Music audio is FairPlay-protected and cannot be decoded by native
//! code — this client is metadata only. An Apple Music track is played by
//! resolving it, through de-duplication, to a playable source, or (later) via
//! an embedded MusicKit web player.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::time::Duration;

use crate::error::{AppleMusicError, AppleMusicResult};
use crate::model::{Album, Artist, SearchResults, Song};

/// The Apple Music API base URL.
const API_BASE: &str = "https://api.music.apple.com/v1";
/// The pixel size cover-art URL templates are resolved to.
const ART_SIZE: u32 = 512;

/// A client for the Apple Music catalog.
pub struct AppleMusicClient {
    /// The shared `reqwest` HTTP client.
    http: reqwest::Client,
    /// The developer-token JWT, sent as a bearer token on every request.
    developer_token: String,
    /// The catalog storefront (`us`, `gb`, …) — catalogs are region-specific.
    storefront: String,
}

impl AppleMusicClient {
    /// Build a client for a `developer_token` and a `storefront`.
    ///
    /// # Errors
    ///
    /// Returns [`AppleMusicError::Http`] if the HTTP client cannot be built.
    pub fn new(developer_token: String, storefront: String) -> AppleMusicResult<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("Spottyfi/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| AppleMusicError::Http(err.to_string()))?;
        Ok(Self {
            http,
            developer_token,
            storefront,
        })
    }

    /// Issue an authenticated `GET` and decode the JSON body.
    async fn get<T: DeserializeOwned>(
        &self,
        url: &str,
        query: &[(&str, String)],
    ) -> AppleMusicResult<T> {
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.developer_token)
            .query(query)
            .send()
            .await
            .map_err(|err| AppleMusicError::Http(err.to_string()))?;
        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|err| AppleMusicError::Decode(err.to_string()))?;
        if !status.is_success() {
            return Err(AppleMusicError::Api {
                status: status.as_u16(),
                message: api_error_message(&body),
            });
        }
        serde_json::from_value(body).map_err(|err| AppleMusicError::Decode(err.to_string()))
    }

    /// Search the catalog for `term`, capping each list at `limit`.
    ///
    /// # Errors
    ///
    /// Any [`AppleMusicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn search(&self, term: &str, limit: u32) -> AppleMusicResult<SearchResults> {
        let url = format!("{API_BASE}/catalog/{}/search", self.storefront);
        let envelope: SearchEnvelope = self
            .get(
                &url,
                &[
                    ("term", term.to_owned()),
                    ("types", "songs,albums,artists".to_owned()),
                    ("limit", limit.to_string()),
                ],
            )
            .await?;
        let results = envelope.results;
        Ok(SearchResults {
            songs: results.songs.map(into_songs).unwrap_or_default(),
            albums: results.albums.map(into_albums).unwrap_or_default(),
            artists: results.artists.map(into_artists).unwrap_or_default(),
        })
    }

    /// Look up one catalog song by id.
    ///
    /// # Errors
    ///
    /// Any [`AppleMusicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn song(&self, id: &str) -> AppleMusicResult<Option<Song>> {
        let url = format!("{API_BASE}/catalog/{}/songs/{id}", self.storefront);
        let envelope: DataEnvelope<SongAttributes> = self.get(&url, &[]).await?;
        Ok(envelope.data.into_iter().next().map(song_from))
    }

    /// Look up one catalog album by id.
    ///
    /// # Errors
    ///
    /// Any [`AppleMusicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn album(&self, id: &str) -> AppleMusicResult<Option<Album>> {
        let url = format!("{API_BASE}/catalog/{}/albums/{id}", self.storefront);
        let envelope: DataEnvelope<AlbumAttributes> = self.get(&url, &[]).await?;
        Ok(envelope.data.into_iter().next().map(album_from))
    }

    /// Look up one catalog artist by id.
    ///
    /// # Errors
    ///
    /// Any [`AppleMusicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn artist(&self, id: &str) -> AppleMusicResult<Option<Artist>> {
        let url = format!("{API_BASE}/catalog/{}/artists/{id}", self.storefront);
        let envelope: DataEnvelope<ArtistAttributes> = self.get(&url, &[]).await?;
        Ok(envelope.data.into_iter().next().map(artist_from))
    }
}

// --- raw response envelopes -------------------------------------------------

/// The `{ "data": [ … ] }` envelope a lookup returns.
#[derive(Deserialize)]
#[serde(bound(deserialize = "A: serde::de::DeserializeOwned"))]
struct DataEnvelope<A> {
    /// The resources — usually one for a lookup.
    #[serde(default = "Vec::new")]
    data: Vec<Resource<A>>,
}

/// One `{ "id", "type", "attributes" }` resource.
#[derive(Deserialize)]
#[serde(bound(deserialize = "A: serde::de::DeserializeOwned"))]
struct Resource<A> {
    /// The catalog id.
    id: String,
    /// The typed attributes — absent for a sparse resource.
    #[serde(default)]
    attributes: Option<A>,
}

/// The `{ "results": { … } }` envelope a search returns.
#[derive(Deserialize)]
struct SearchEnvelope {
    /// The grouped result lists.
    #[serde(default)]
    results: SearchResultsRaw,
}

/// The per-type result lists inside a search response.
#[derive(Deserialize, Default)]
struct SearchResultsRaw {
    /// Matching songs, if the type was requested and matched.
    #[serde(default)]
    songs: Option<DataEnvelope<SongAttributes>>,
    /// Matching albums.
    #[serde(default)]
    albums: Option<DataEnvelope<AlbumAttributes>>,
    /// Matching artists.
    #[serde(default)]
    artists: Option<DataEnvelope<ArtistAttributes>>,
}

/// Raw song attributes.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SongAttributes {
    /// The song title.
    name: String,
    /// The artist name.
    #[serde(default)]
    artist_name: String,
    /// The album name.
    #[serde(default)]
    album_name: String,
    /// The duration in milliseconds.
    #[serde(default)]
    duration_in_millis: u64,
    /// The cover artwork.
    #[serde(default)]
    artwork: Option<Artwork>,
    /// The ISRC recording code.
    #[serde(default)]
    isrc: Option<String>,
    /// The track number within its album.
    #[serde(default)]
    track_number: Option<u32>,
}

/// Raw album attributes.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlbumAttributes {
    /// The album name.
    name: String,
    /// The album-artist name.
    #[serde(default)]
    artist_name: String,
    /// The cover artwork.
    #[serde(default)]
    artwork: Option<Artwork>,
    /// The track count.
    #[serde(default)]
    track_count: u32,
    /// The release date (`YYYY-MM-DD`).
    #[serde(default)]
    release_date: Option<String>,
}

/// Raw artist attributes.
#[derive(Deserialize)]
struct ArtistAttributes {
    /// The artist name.
    name: String,
    /// The artist artwork.
    #[serde(default)]
    artwork: Option<Artwork>,
}

/// A catalog artwork object — a templated URL plus its native dimensions.
#[derive(Deserialize)]
struct Artwork {
    /// A URL template with `{w}` / `{h}` placeholders.
    #[serde(default)]
    url: String,
}

impl Artwork {
    /// Resolve the URL template to a concrete square size.
    fn resolved(&self) -> Option<String> {
        if self.url.is_empty() {
            return None;
        }
        Some(
            self.url
                .replace("{w}", &ART_SIZE.to_string())
                .replace("{h}", &ART_SIZE.to_string()),
        )
    }
}

// --- raw -> public conversions ---------------------------------------------

/// Map a raw song resource onto the public [`Song`].
fn song_from(resource: Resource<SongAttributes>) -> Song {
    let attributes = resource.attributes;
    Song {
        id: resource.id,
        title: attributes
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or_default(),
        artist_name: attributes
            .as_ref()
            .map(|a| a.artist_name.clone())
            .unwrap_or_default(),
        album_name: attributes
            .as_ref()
            .map(|a| a.album_name.clone())
            .unwrap_or_default(),
        duration: Duration::from_millis(attributes.as_ref().map_or(0, |a| a.duration_in_millis)),
        artwork_url: attributes
            .as_ref()
            .and_then(|a| a.artwork.as_ref())
            .and_then(Artwork::resolved),
        isrc: attributes.as_ref().and_then(|a| a.isrc.clone()),
        track_number: attributes.as_ref().and_then(|a| a.track_number),
    }
}

/// Map a raw album resource onto the public [`Album`].
fn album_from(resource: Resource<AlbumAttributes>) -> Album {
    let attributes = resource.attributes;
    Album {
        id: resource.id,
        name: attributes
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or_default(),
        artist_name: attributes
            .as_ref()
            .map(|a| a.artist_name.clone())
            .unwrap_or_default(),
        artwork_url: attributes
            .as_ref()
            .and_then(|a| a.artwork.as_ref())
            .and_then(Artwork::resolved),
        track_count: attributes.as_ref().map_or(0, |a| a.track_count),
        year: attributes
            .as_ref()
            .and_then(|a| a.release_date.as_deref())
            .and_then(parse_year),
    }
}

/// Map a raw artist resource onto the public [`Artist`].
fn artist_from(resource: Resource<ArtistAttributes>) -> Artist {
    let attributes = resource.attributes;
    Artist {
        id: resource.id,
        name: attributes
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or_default(),
        artwork_url: attributes
            .as_ref()
            .and_then(|a| a.artwork.as_ref())
            .and_then(Artwork::resolved),
    }
}

/// Map a list envelope of song resources onto public songs.
fn into_songs(envelope: DataEnvelope<SongAttributes>) -> Vec<Song> {
    envelope.data.into_iter().map(song_from).collect()
}

/// Map a list envelope of album resources onto public albums.
fn into_albums(envelope: DataEnvelope<AlbumAttributes>) -> Vec<Album> {
    envelope.data.into_iter().map(album_from).collect()
}

/// Map a list envelope of artist resources onto public artists.
fn into_artists(envelope: DataEnvelope<ArtistAttributes>) -> Vec<Artist> {
    envelope.data.into_iter().map(artist_from).collect()
}

/// Extract the year from a `YYYY-MM-DD` (or `YYYY`) release date.
fn parse_year(release_date: &str) -> Option<u32> {
    release_date.get(0..4).and_then(|year| year.parse().ok())
}

/// Pull a readable message out of an Apple Music `{ "errors": [ … ] }` body.
fn api_error_message(body: &serde_json::Value) -> String {
    body.get("errors")
        .and_then(|errors| errors.get(0))
        .and_then(|error| {
            error
                .get("detail")
                .or_else(|| error.get("title"))
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or("unknown error")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_template_resolves_to_a_concrete_size() {
        let artwork = Artwork {
            url: "https://example.com/{w}x{h}bb.jpg".to_owned(),
        };
        assert_eq!(
            artwork.resolved().as_deref(),
            Some("https://example.com/512x512bb.jpg"),
        );
    }

    #[test]
    fn empty_artwork_resolves_to_none() {
        assert!(Artwork { url: String::new() }.resolved().is_none());
    }

    #[test]
    fn year_is_parsed_from_a_release_date() {
        assert_eq!(parse_year("2017-06-23"), Some(2017));
        assert_eq!(parse_year("1999"), Some(1999));
        assert_eq!(parse_year(""), None);
    }

    #[test]
    fn search_response_parses() {
        let json = serde_json::json!({
            "results": {
                "songs": { "data": [{
                    "id": "1", "type": "songs",
                    "attributes": {
                        "name": "Airbag", "artistName": "Radiohead",
                        "albumName": "OK Computer", "durationInMillis": 284000,
                        "isrc": "GBAYE9700001", "trackNumber": 1,
                    },
                }] },
                "albums": { "data": [{
                    "id": "2", "type": "albums",
                    "attributes": {
                        "name": "OK Computer", "artistName": "Radiohead",
                        "trackCount": 12, "releaseDate": "1997-06-16",
                    },
                }] },
            },
        });
        let envelope: SearchEnvelope = serde_json::from_value(json).expect("parses");
        let songs = into_songs(envelope.results.songs.expect("songs present"));
        assert_eq!(songs[0].title, "Airbag");
        assert_eq!(songs[0].isrc.as_deref(), Some("GBAYE9700001"));
        assert_eq!(songs[0].duration, Duration::from_secs(284));
        let albums = into_albums(envelope.results.albums.expect("albums present"));
        assert_eq!(albums[0].year, Some(1997));
    }

    #[test]
    fn api_error_message_reads_the_detail() {
        let body = serde_json::json!({
            "errors": [{ "status": "404", "title": "Not Found", "detail": "no such song" }],
        });
        assert_eq!(api_error_message(&body), "no such song");
    }
}
