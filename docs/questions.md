# Open questions

Things to confirm with the maintainer or against upstream **before** relying on
them. Don't guess endpoint shapes or auth flows — add a question here and ask.

## Open

10. **Podcast / audiobook search (Phase 6).** The Search page ships with a
    **Podcasts** category tab, but it currently renders an explanatory note
    instead of results: the `api` crate's `SearchType` enum
    (`Track | Artist | Album | Playlist`) has no `Show`/`Episode`/`Audiobook`
    variant, and `models::SearchResults` has no field for them. Wiring podcast
    search means a small `api` + `models` change: add `SearchType::Show`
    (rspotify already has `rspotify::model::SearchType::Show` and `Episode`),
    a `shows` field on `SearchResults`, and a `Show` domain type. Deferred so
    Phase 6 stays UI-focused. Audiobooks are omitted entirely for now (rspotify
    has no audiobook search type). The page's category tab is in place so this
    is a drop-in once the `api` surface exists.

8. **Liked Songs "Date added" column (Phase 5).** The track table has a
   "Date added" column, populated for playlist pages from each
   `PlaylistTrack.added_at`. The **Liked Songs** page cannot fill it: the
   `SpotifyApi::saved_tracks` method returns plain
   [`Track`](spottyfi_models::Track)s, and the `api` crate's `saved_track`
   mapper drops the `added_at` field that Spotify's `GET /me/tracks`
   response actually carries (it returns `SavedTrack`, a `{ added_at, track }`
   wrapper). The Liked Songs column is therefore empty for now. Resolving it
   means either a new `saved_tracks` return type carrying `added_at`, or a
   parallel `SavedTrack`-style model — a small `api` change deferred so
   Phase 5 stays UI-only. Sort-by-date on that page is consequently a no-op.

9. **Tab navigation: open-vs-replace (Phase 5 / Phase 10).** `docs/docking.md`
   specifies "plain click on a sidebar entry or a link **replaces** the
   focused tab". Phase 5 implements the simpler **open/focus** rule instead:
   clicking a playlist focuses its tab if already open, otherwise adds a new
   tab to the focused leaf. The strict replace-the-focused-tab behaviour, the
   Cmd-click-for-new-tab modifier and per-tab history are the Phase 10
   docking-power-features work; the Phase 5 brief explicitly accepts basic
   open here.

2. **Spotify app registration.** Spottyfi needs a Spotify app registered on the
   developer dashboard (https://developer.spotify.com/dashboard). The maintainer
   must create it and provide the **Client ID** via the `SPOTTYFI_CLIENT_ID`
   environment variable (the PKCE flow has no client secret, so the ID is not
   sensitive). _Blocks live login in Phase 1; the code is built without it._

5. **MPRIS2 + Wayland.** Smoke-test the MPRIS2 D-Bus interface on the target
   NixOS/Wayland setup early (Phase 4), not at Phase 12.

6. **Audio backend on PipeWire.** `rodio` goes through ALSA; on PipeWire it
   works via the ALSA shim. Confirm acceptable, or plan a `pipewire-rs` backend.

7. **Deprecated Spotify Web API endpoints (affects Phase 3 + Phase 7).**
   On **2024-11-27** Spotify restricted a set of Web API endpoints to apps
   that already had *extended quota* before that date. Apps registered
   **after** 2024-11-27 — which Spottyfi's new app will be — get **403/404**
   from:

   - **Recommendations** (`GET /recommendations`)
   - **Get Featured Playlists** (`GET /browse/featured-playlists`)
   - **Get a Category's Playlists** (`GET /browse/categories/{id}/playlists`)
   - **Get an Artist's Related Artists** (`GET /artists/{id}/related-artists`)
   - **Audio Features / Audio Analysis** (`GET /audio-features`, `/audio-analysis`)
   - 30-second `preview_url`s in multi-get responses; algorithmic and
     Spotify-owned editorial playlists.

   Sources: [Spotify dev blog, 2024-11-27](https://developer.spotify.com/blog/2024-11-27-changes-to-the-web-api),
   [TechCrunch](https://techcrunch.com/2024/11/27/spotify-cuts-developer-access-to-several-of-its-recommendation-features/).

   **What Phase 3 did about it.** The `api` crate still implements the
   `SpotifyApi` methods `PLAN.md` lists (`artist_top_tracks`,
   `featured_playlists`, `browse_categories`, `recommendations`), but a
   403/404 from any of them is mapped to a dedicated
   `ApiError::EndpointUnavailable { endpoint }` variant instead of a
   misleading `NotFound` or an empty result. Note `artist_top_tracks` is
   marked `#[deprecated]` in rspotify 0.16 but is *not* on Spotify's
   2024-11-27 list — it is included defensively in case access is uneven.

   **Open decisions for the maintainer / Phase 7 (Browse):**
   - **Recommendations → a third-party source (proposal — not yet decided).**
     With Spotify's `/recommendations` dead for new apps, Phase 7's Browse
     surface needs another source of suggestions. One candidate is the
     **Last.fm API** (`track.getSimilar`, `artist.getSimilar`,
     `artist.getTopTracks`, `chart.getTopTracks`, `tag.getTopArtists`), which
     would need a free **Last.fm API key**
     (https://www.last.fm/api/account/create) — e.g. a `SPOTTYFI_LASTFM_API_KEY`
     env var mirroring `SPOTTYFI_CLIENT_ID`. Last.fm returns artist/track
     *names*, not Spotify ids, so Phase 7 would resolve them back via
     `api.search(...)`, in a thin `lastfm` module. **Maintainer to decide
     before Phase 7** — Last.fm, another source, or dropping recommendations.
   - **Featured Playlists / Browse categories** have no Last.fm equivalent.
     Phase 7's `BrowsePage` will likely fall back to *new releases*
     (`GET /browse/new-releases` — also `#[deprecated]` in rspotify but not
     on the 2024-11-27 kill list, so its real status needs a live check once
     the app is registered) and to Last.fm charts/tags for the genre grid.
   - **Confirm once the app exists.** All of the above is the *documented*
     behaviour; the only certain test is to register the app and hit each
     endpoint. The `EndpointUnavailable` plumbing means that test is a
     no-risk smoke check rather than something that can crash the client.

## Resolved

- **`egui_dock` state serialisation** (was #3) — `egui_dock 0.19.1` is the
  release built against `egui 0.34` (0.19.0/0.19.1 target `egui ^0.34`; the
  newer line had not bumped past it). Its `serde` feature derives
  `Serialize`/`Deserialize` on `DockState` (`#[cfg_attr(feature = "serde", …)]`
  on the type), so the whole layout round-trips through RON. Phase 4 enables
  the feature and persists the layout to `<config_dir>/layout.ron`.
- **egui image-loading lifecycle** (was #4) — egui ships no network image
  loader. Phase 4 establishes one consistent approach: a custom
  `egui::load::ImageLoader` (`spottyfi-ui`'s `NetworkImageLoader`) that fetches
  `http(s)` URLs with `ehttp` and decodes them with the `image` crate. Once
  installed (after `egui_extras::install_image_loaders`), `egui::Image::from_uri(url)`
  resolves remote album art and avatars everywhere. The loader's in-memory
  cache is the seam for the Phase 9 on-disk `sha1(url).webp` cache — only the
  `fetch` function does network I/O. `image` needs the `jpeg`+`png` features
  (already enabled in the workspace) for Spotify's `i.scdn.co` art.
- **librespot auth flow** (was #1) — **`Credentials::with_access_token` works
  directly** with the OAuth access token from Spottyfi's PKCE flow. No separate
  dealer/keymaster handshake or token exchange is needed. Confirmed against the
  librespot 0.8.0 source: `with_access_token` sets `auth_type` to
  `AUTHENTICATION_SPOTIFY_TOKEN`, and `Session::connect` forwards that auth type
  and the raw token bytes straight into the access-point handshake
  (`librespot-core/src/connection/mod.rs`). The 0.8.0 `examples/play.rs` does
  exactly this. _Caveats:_ (1) a token-authed session **cannot use keymaster** —
  `session.token_provider()` will not mint fresh tokens from inside librespot,
  so token refresh must stay owned by the `auth` crate (it already is); the
  audio engine is simply restarted with a fresh token if the session drops.
  (2) librespot needs the `streaming` scope, which Spottyfi's PKCE flow already
  requests. (3) Playback requires a **Spotify Premium** account — librespot
  rejects free accounts at the AP handshake.
- **Roadmap pinned `librespot = "0.6"`; the implemented version is `0.8`.** The
  API moved: `SpotifyId` is now split into `SpotifyId` (the raw id) and
  `SpotifyUri` (the typed enum, used by `Player::load` and `PlayerEvent`).
  `Player::new` takes a `VolumeGetter` and a sink-builder closure. The new
  `PlayerEvent::PositionChanged` (gated on `PlayerConfig::position_update_interval`)
  feeds the ~10Hz progress updates.
- **`vergen-gitcl 1.0.8` dependency conflict.** librespot-core's build script
  pulls `vergen-gitcl 1.0.8`, which has inconsistent constraints: it requires
  `vergen-lib ^0.1.6` directly but also `vergen ^9.0.6`, and `vergen 9.1.0`
  needs `vergen-lib 9.1.0` — two incompatible `vergen-lib` versions in one
  build. `Cargo.lock` pins `vergen` to `9.0.6` (which pairs with `vergen-lib
  0.1.6`) to resolve it. The lockfile is committed, so this holds; if a future
  `cargo update` reintroduces the break, re-pin with
  `cargo update vergen --precise 9.0.6`.
- **Product name** — `Spottyfi` (placeholder, may change).
- **Nix flake** — dropped; use `rustup` with the pinned `rust-toolchain.toml`.
- **Redirect URI** — fixed loopback `http://127.0.0.1:8888/callback` (port
  configurable via `SPOTTYFI_REDIRECT_PORT`). A random port can't work: Spotify
  requires the registered redirect URI to match exactly, port included.
- **`keyring` version** — staying on `keyring 4` + `keyring-core 1` (maintainer's
  call), not the `keyring 3` originally pinned in `PLAN.md`. 4.x is a different
  crate shape: store registration (`keyring::use_native_store`) is split from the
  `Entry` API (`keyring-core`), all OS backends are bundled unconditionally, and
  it pulls a heavier dependency tree (`turso`, vendored OpenSSL on Linux). `PLAN.md`
  has been updated to match.
