//! The OpenSubsonic HTTP client.
//!
//! One [`SubsonicClient`] talks to one server. Requests are plain `GET`s to
//! `{server}/rest/{endpoint}` carrying the [salt-and-token auth] every
//! Subsonic-compatible server accepts, and every JSON reply is unwrapped from
//! its `subsonic-response` envelope. See the [OpenSubsonic spec].
//!
//! [salt-and-token auth]: https://opensubsonic.netlify.app/docs/#authentication
//! [OpenSubsonic spec]: https://opensubsonic.netlify.app/docs/

use md5::{Digest, Md5};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::error::{SubsonicError, SubsonicResult};
use crate::model::{
    Album, AlbumList, AlbumListKind, Artist, ArtistsIndex, Playlist, PlaylistList, SearchResult,
    Starred,
};

/// The Subsonic API protocol version Spottyfi requests.
const API_VERSION: &str = "1.16.1";
/// The client name reported to the server (shown in its session list).
const CLIENT_NAME: &str = "Spottyfi";

/// Connection details for one OpenSubsonic server.
#[derive(Clone)]
pub struct SubsonicConfig {
    /// The server's base URL, e.g. `https://music.example.com` — with or
    /// without a trailing slash, but *without* the `/rest` suffix.
    pub base_url: String,
    /// The Subsonic account username.
    pub username: String,
    /// The account password. Used only to derive the auth token; never sent.
    pub password: String,
}

/// A client for a single OpenSubsonic server.
pub struct SubsonicClient {
    /// The shared `reqwest` HTTP client.
    http: reqwest::Client,
    /// The normalised base URL (no trailing slash).
    base: String,
    /// The account username.
    username: String,
    /// The account password. Retained so a fresh random salt — and therefore
    /// a fresh token — can be generated per request, as the spec recommends.
    password: String,
}

impl SubsonicClient {
    /// Build a client for `config`.
    ///
    /// No network request is made; call [`SubsonicClient::ping`] to verify the
    /// server is reachable and the credentials are valid.
    ///
    /// # Errors
    ///
    /// Returns [`SubsonicError::BadUrl`] for an empty URL, or
    /// [`SubsonicError::Http`] if the HTTP client cannot be built.
    pub fn new(config: SubsonicConfig) -> SubsonicResult<Self> {
        let base = config.base_url.trim().trim_end_matches('/').to_owned();
        if base.is_empty() {
            return Err(SubsonicError::BadUrl("the server URL is empty".to_owned()));
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("Spottyfi/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| SubsonicError::Http(err.to_string()))?;
        Ok(Self {
            http,
            base,
            username: config.username,
            password: config.password,
        })
    }

    /// A fresh `(salt, token)` pair — `token = md5(password + salt)`.
    ///
    /// Generated per request: the spec recommends a new random salt for every
    /// call rather than a reused one.
    fn credentials(&self) -> (String, String) {
        let salt = random_salt();
        let token = hex::encode(Md5::digest(format!("{}{salt}", self.password).as_bytes()));
        (salt, token)
    }

    /// The authentication query parameters every request carries.
    fn auth_params(&self) -> Vec<(&'static str, String)> {
        let (salt, token) = self.credentials();
        vec![
            ("u", self.username.clone()),
            ("t", token),
            ("s", salt),
            ("v", API_VERSION.to_owned()),
            ("c", CLIENT_NAME.to_owned()),
            ("f", "json".to_owned()),
        ]
    }

    /// Issue a request and return the unwrapped `subsonic-response` object,
    /// turning a `status: "failed"` reply into a [`SubsonicError::Api`].
    async fn request_raw(
        &self,
        endpoint: &str,
        extra: &[(&str, String)],
    ) -> SubsonicResult<serde_json::Value> {
        let url = format!("{}/rest/{endpoint}", self.base);
        let mut params = self.auth_params();
        params.extend(extra.iter().map(|(key, value)| (*key, value.clone())));
        let response = self
            .http
            .get(&url)
            .query(&params)
            .send()
            .await
            .map_err(|err| SubsonicError::Http(err.to_string()))?
            // A non-2xx reply is an HTTP-level failure (a reverse proxy, a
            // crashed server) — surface it as such rather than letting an
            // HTML error page fail later as a confusing decode error.
            .error_for_status()
            .map_err(|err| SubsonicError::Http(err.to_string()))?;
        let envelope: Envelope = response
            .json()
            .await
            .map_err(|err| SubsonicError::Decode(err.to_string()))?;
        check_status(envelope.response)
    }

    /// Issue a request and deserialise the named payload field.
    async fn request<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        extra: &[(&str, String)],
        payload_key: &str,
    ) -> SubsonicResult<T> {
        let body = self.request_raw(endpoint, extra).await?;
        // A missing payload key defaults to an empty object, not null: list
        // endpoints (`getArtists`, …) legitimately omit the key when the
        // library is empty, and every list model's fields are `default`, so
        // `{}` decodes to "no results" rather than a spurious decode error.
        let payload = body
            .get(payload_key)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        serde_json::from_value(payload).map_err(|err| SubsonicError::Decode(err.to_string()))
    }

    /// Check connectivity and credentials (`ping`).
    ///
    /// # Errors
    ///
    /// Any [`SubsonicError`] — a network failure, or [`SubsonicError::Api`]
    /// with code `40` when the username or password is wrong.
    #[tracing::instrument(skip(self))]
    pub async fn ping(&self) -> SubsonicResult<()> {
        self.request_raw("ping", &[]).await.map(|_| ())
    }

    /// Search artists, albums and songs by `query` (`search3`).
    ///
    /// # Errors
    ///
    /// Any [`SubsonicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn search(
        &self,
        query: &str,
        artist_count: u32,
        album_count: u32,
        song_count: u32,
    ) -> SubsonicResult<SearchResult> {
        self.request(
            "search3",
            &[
                ("query", query.to_owned()),
                ("artistCount", artist_count.to_string()),
                ("albumCount", album_count.to_string()),
                ("songCount", song_count.to_string()),
            ],
            "searchResult3",
        )
        .await
    }

    /// Fetch every artist in the library (`getArtists`), flattening the
    /// server's per-letter index buckets into one list.
    ///
    /// # Errors
    ///
    /// Any [`SubsonicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn artists(&self) -> SubsonicResult<Vec<Artist>> {
        let index: ArtistsIndex = self.request("getArtists", &[], "artists").await?;
        Ok(index
            .index
            .into_iter()
            .flat_map(|bucket| bucket.artist)
            .collect())
    }

    /// Fetch one artist and its albums (`getArtist`).
    ///
    /// # Errors
    ///
    /// Any [`SubsonicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn artist(&self, id: &str) -> SubsonicResult<Artist> {
        self.request("getArtist", &[("id", id.to_owned())], "artist")
            .await
    }

    /// Fetch one album and its songs (`getAlbum`).
    ///
    /// # Errors
    ///
    /// Any [`SubsonicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn album(&self, id: &str) -> SubsonicResult<Album> {
        self.request("getAlbum", &[("id", id.to_owned())], "album")
            .await
    }

    /// Fetch a list of albums (`getAlbumList2`) — newest, most-played, random…
    ///
    /// # Errors
    ///
    /// Any [`SubsonicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn album_list(
        &self,
        kind: AlbumListKind,
        size: u32,
        offset: u32,
    ) -> SubsonicResult<Vec<Album>> {
        let list: AlbumList = self
            .request(
                "getAlbumList2",
                &[
                    ("type", kind.as_param().to_owned()),
                    ("size", size.to_string()),
                    ("offset", offset.to_string()),
                ],
                "albumList2",
            )
            .await?;
        Ok(list.album)
    }

    /// Fetch the user's playlists (`getPlaylists`).
    ///
    /// # Errors
    ///
    /// Any [`SubsonicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn playlists(&self) -> SubsonicResult<Vec<Playlist>> {
        let list: PlaylistList = self.request("getPlaylists", &[], "playlists").await?;
        Ok(list.playlist)
    }

    /// Fetch one playlist and its songs (`getPlaylist`).
    ///
    /// # Errors
    ///
    /// Any [`SubsonicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn playlist(&self, id: &str) -> SubsonicResult<Playlist> {
        self.request("getPlaylist", &[("id", id.to_owned())], "playlist")
            .await
    }

    /// Fetch the user's starred artists, albums and songs (`getStarred2`).
    ///
    /// # Errors
    ///
    /// Any [`SubsonicError`] from the request or decode.
    #[tracing::instrument(skip(self))]
    pub async fn starred(&self) -> SubsonicResult<Starred> {
        self.request("getStarred2", &[], "starred2").await
    }

    /// Report a play to the server (`scrobble`).
    ///
    /// `submission` is `true` for a completed play, `false` for a "now
    /// playing" notification.
    ///
    /// # Errors
    ///
    /// Any [`SubsonicError`] from the request.
    #[tracing::instrument(skip(self))]
    pub async fn scrobble(&self, id: &str, submission: bool) -> SubsonicResult<()> {
        self.request_raw(
            "scrobble",
            &[
                ("id", id.to_owned()),
                ("submission", submission.to_string()),
            ],
        )
        .await
        .map(|_| ())
    }

    /// Star or unstar an item (`star` / `unstar`).
    ///
    /// # Errors
    ///
    /// Any [`SubsonicError`] from the request.
    #[tracing::instrument(skip(self))]
    pub async fn set_starred(&self, id: &str, starred: bool) -> SubsonicResult<()> {
        let endpoint = if starred { "star" } else { "unstar" };
        self.request_raw(endpoint, &[("id", id.to_owned())])
            .await
            .map(|_| ())
    }

    /// The fully-signed URL that streams song `id` (`stream`).
    ///
    /// The audio player fetches this URL directly; it carries the auth
    /// parameters so no further handshake is needed.
    ///
    /// # Errors
    ///
    /// [`SubsonicError::BadUrl`] if the base URL cannot be parsed.
    pub fn stream_url(&self, id: &str) -> SubsonicResult<String> {
        self.signed_url("stream", &[("id", id)])
    }

    /// The fully-signed URL for cover art `id` (`getCoverArt`), optionally
    /// constrained to a maximum `size` in pixels.
    ///
    /// # Errors
    ///
    /// [`SubsonicError::BadUrl`] if the base URL cannot be parsed.
    pub fn cover_art_url(&self, id: &str, size: Option<u32>) -> SubsonicResult<String> {
        let size = size.map(|pixels| pixels.to_string());
        let mut extra: Vec<(&str, &str)> = vec![("id", id)];
        if let Some(size) = size.as_deref() {
            extra.push(("size", size));
        }
        self.signed_url("getCoverArt", &extra)
    }

    /// Build a signed `{base}/rest/{endpoint}?…` URL with the auth parameters
    /// and `extra` query pairs.
    fn signed_url(&self, endpoint: &str, extra: &[(&str, &str)]) -> SubsonicResult<String> {
        let (salt, token) = self.credentials();
        let mut params: Vec<(&str, String)> = vec![
            ("u", self.username.clone()),
            ("t", token),
            ("s", salt),
            ("v", API_VERSION.to_owned()),
            ("c", CLIENT_NAME.to_owned()),
            ("f", "json".to_owned()),
        ];
        params.extend(extra.iter().map(|(key, value)| (*key, (*value).to_owned())));
        let url =
            reqwest::Url::parse_with_params(&format!("{}/rest/{endpoint}", self.base), &params)
                .map_err(|err| SubsonicError::BadUrl(err.to_string()))?;
        Ok(url.to_string())
    }
}

/// The outer `{ "subsonic-response": { … } }` envelope every reply carries.
#[derive(Deserialize)]
struct Envelope {
    /// The response object itself.
    #[serde(rename = "subsonic-response")]
    response: serde_json::Value,
}

/// Turn a `subsonic-response` body into either its content or a typed error.
///
/// Split out as a free function so the failure-path handling is unit-testable
/// without a live server.
fn check_status(body: serde_json::Value) -> SubsonicResult<serde_json::Value> {
    if body.get("status").and_then(serde_json::Value::as_str) == Some("failed") {
        let error = body.get("error");
        let code = error
            .and_then(|err| err.get("code"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        let message = error
            .and_then(|err| err.get("message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown error")
            .to_owned();
        return Err(SubsonicError::Api { code, message });
    }
    Ok(body)
}

/// A random 32-hex-digit salt for the auth token.
fn random_salt() -> String {
    format!("{:032x}", rand::random::<u128>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_normalises_a_trailing_slash() {
        let client = SubsonicClient::new(SubsonicConfig {
            base_url: "https://music.example.com/".to_owned(),
            username: "kieran".to_owned(),
            password: "hunter2".to_owned(),
        })
        .expect("client builds");
        assert_eq!(client.base, "https://music.example.com");
    }

    #[test]
    fn empty_url_is_rejected() {
        let result = SubsonicClient::new(SubsonicConfig {
            base_url: "   ".to_owned(),
            username: "kieran".to_owned(),
            password: "hunter2".to_owned(),
        });
        assert!(matches!(result, Err(SubsonicError::BadUrl(_))));
    }

    #[test]
    fn random_salt_is_32_hex_digits() {
        let salt = random_salt();
        assert_eq!(salt.len(), 32);
        assert!(salt.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn failed_status_becomes_an_api_error() {
        let body = serde_json::json!({
            "status": "failed",
            "version": "1.16.1",
            "error": { "code": 40, "message": "Wrong username or password." },
        });
        let result = check_status(body);
        match result {
            Err(SubsonicError::Api { code, message }) => {
                assert_eq!(code, 40);
                assert_eq!(message, "Wrong username or password.");
            }
            other => panic!("expected an API error, got {other:?}"),
        }
    }

    #[test]
    fn ok_status_passes_the_body_through() {
        let body = serde_json::json!({ "status": "ok", "version": "1.16.1" });
        assert!(check_status(body).is_ok());
    }

    #[test]
    fn signed_stream_url_carries_auth_and_id() {
        let client = SubsonicClient::new(SubsonicConfig {
            base_url: "https://music.example.com".to_owned(),
            username: "kieran".to_owned(),
            password: "hunter2".to_owned(),
        })
        .expect("client builds");
        let url = client.stream_url("song-42").expect("url builds");
        assert!(url.starts_with("https://music.example.com/rest/stream?"));
        assert!(url.contains("id=song-42"));
        assert!(url.contains("u=kieran"));
        assert!(url.contains("c=Spottyfi"));
        assert!(url.contains("t=") && url.contains("s="));
    }

    #[test]
    fn search_result_parses_a_typical_payload() {
        let json = serde_json::json!({
            "artist": [{ "id": "ar-1", "name": "Radiohead" }],
            "album": [{ "id": "al-1", "name": "OK Computer", "artist": "Radiohead" }],
            "song": [{
                "id": "so-1", "title": "Airbag", "album": "OK Computer",
                "artist": "Radiohead", "duration": 284, "suffix": "flac",
            }],
        });
        let result: SearchResult = serde_json::from_value(json).expect("parses");
        assert_eq!(result.artist.len(), 1);
        assert_eq!(result.album[0].name, "OK Computer");
        assert_eq!(result.song[0].duration, Some(284));
    }
}
