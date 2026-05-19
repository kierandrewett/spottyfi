# Multi-source music — contract of work

Turning Spottyfi from a Spotify-only client into a general-purpose music app
(à la Lyrion) with a Spotify-style frontend. Multiple libraries, one seamless
experience, source-tagged everywhere, de-duplicated.

## Honest scope notes

- **OpenSubsonic** and **Spotify** are full sources (metadata + playback).
- **Apple Music** cannot be played by native code — its audio is FairPlay-DRM.
  The agreed route is an embedded Chromium (CEF) running Apple's official
  MusicKit JS, exactly as the Cider client does. CEF integration is a large
  subproject (hundreds of MB of Chromium, a helper-process model, fiddly Rust
  bindings); it is **architected and scaffolded** here, with the Apple Music
  *catalog* API (metadata, search) wired first. Playback via CEF is the last
  and largest phase.
- This is a multi-week rearchitecture. Work lands in small, tested, committed
  increments; this file is the running source of truth.

## Phases & tasks

### A — OpenSubsonic client crate `crates/subsonic` ✅
- [x] Crate scaffold, error type, response-envelope handling
- [x] Auth (salt + token), `ping`
- [x] Models: song / album / artist / playlist / search result
- [x] Endpoints: `search3`, `getArtists`, `getArtist`, `getAlbum`,
      `getAlbumList2`, `getPlaylists`, `getPlaylist`, `getStarred2`
- [x] `stream` + `getCoverArt` URL builders, `scrobble`, `star`/`unstar`
- [x] Unit tests (auth, envelope, error, model parsing)

### B — Multi-source architecture ✅ (Spotify adapter pending)
- [x] `SourceId` / `SourceKind` / `SourceRef` — a source tag on every entity
- [x] Source-neutral `Track` / `Album` / `Artist` / `SearchResults`
- [x] `MusicSource` trait (search, browse, stream URL) + `can_play` capability
- [x] `SourceRegistry` — concurrent `search_all` across all sources
- [x] OpenSubsonic behind the trait (`SubsonicSource`)
- [ ] Spotify adapted behind the trait — deferred (the existing Spotify `api`
      is large; wiring it behind `MusicSource` is its own step)

### C — OpenSubsonic playback
- [x] Shared `CpalOutput` output stage extracted from `CpalSink`
- [x] HTTP-stream audio player: fetch → `symphonia` decode → `CpalOutput`
      (`crate::http_player::HttpAudioPlayer`) — play/pause/resume/stop/seek/
      volume/position, FLAC + MP3 + Ogg-Vorbis
- [ ] Engine routes by source: librespot for Spotify, HTTP player for Subsonic
- [ ] Transport/queue source-aware

### D — Sources in settings & first run
- [ ] First-run wizard: optionally set up Spotify, add Subsonic servers
- [ ] Settings: add / edit / remove / test sources
- [ ] Persisted source config

### E — Cross-library search, dedup & source switching
- [ ] Aggregated search across all sources
- [ ] Dedup tracks / albums / artists across sources
- [ ] "Best available source" selection + per-track source switch in the player
- [ ] Source badge in the UI everywhere

### F — Apple Music catalog ✅
- [x] Apple Music catalog API client (`crates/applemusic`) — developer-token
      auth, search + song/album/artist lookup
- [x] `AppleMusicSource` behind the `MusicSource` trait (catalog-only)
- [x] Apple Music entries participate in dedup — ISRC bridges them to a
      playable Spotify/Subsonic copy

### G — Apple Music playback via CEF
- [ ] CEF integration scaffold (offscreen browser, helper process)
- [ ] MusicKit JS bridge (developer + user token, load/play/pause/seek/volume)
- [ ] Wire as an Apple Music playback backend

### H — Polish
- [ ] Source badges, empty states, error surfaces
- [ ] opencode (GPT) validation pass per phase
- [ ] Docs / README update

## Progress log

- **Phase A done.** `crates/subsonic` — a complete, tested OpenSubsonic
  client: salt+token auth, envelope/error handling, all browse + search +
  library endpoints, signed stream/cover-art URLs, scrobble & star. 7 unit
  tests, clippy clean.
- **Phase B done** (bar the Spotify adapter). `crates/sources` — the
  multi-source layer: `SourceRef` tags every entity; source-neutral
  `Track`/`Album`/`Artist`; the `MusicSource` trait + `SourceRegistry` with
  concurrent `search_all`; cross-source de-duplication (MusicBrainz-id or
  fuzzy title/artist key, noise-aware) collapsing the same song to one entry
  with ranked alternatives; the OpenSubsonic adapter. 8 unit tests.
- **opencode validation pass** run against the OpenSubsonic spec; six
  findings fixed (per-request salt, `error_for_status`, empty-library
  decode, case-insensitive MBID match, no-artist dedup safety, Unicode-aware
  normalisation).
- **Phase C player done.** Extracted a shared `CpalOutput` stage from
  `CpalSink`; built `HttpAudioPlayer` — fetch + `symphonia` decode (FLAC /
  MP3 / Ogg-Vorbis) + resample → `CpalOutput`, on its own thread, with
  play/pause/resume/stop/seek/volume/position. Subsonic audio is playable.
- **Phase F done.** `crates/applemusic` — Apple Music catalog client;
  `AppleMusicSource` (catalog-only); ISRC added as a dedup key so Apple
  Music search hits resolve onto a playable Spotify/Subsonic copy.
- **Backend trait.** `audio::PlaybackBackend` — the one interface every
  player (librespot / HTTP / future MusicKit) presents, so the transport
  drives "the player" without knowing the backend. `HttpAudioPlayer`
  implements it.
- **App build/run verified.** `spottyfi` builds and launches cleanly with
  every new crate in the workspace (clean startup logs, no panic, UI
  reached). A screenshot could not be captured — this GNOME Wayland session
  blocks programmatic screenshots from the agent sandbox (`gnome-screenshot`
  no-ops, the GNOME Shell D-Bus call returns `AccessDenied`, `grim` reports
  the compositor lacks the capture protocol).

## Status — honest assessment

The **whole foundation stack is complete, tested and committed**:

- **A** — OpenSubsonic client (`crates/subsonic`).
- **B** — multi-source abstraction + cross-source de-duplication
  (`crates/sources`).
- **C (player half)** — `HttpAudioPlayer`: fetch → `symphonia` decode →
  the shared `CpalOutput`. Subsonic audio is fully playable as a unit.

Every layer a multi-source app needs — talk to the server, model sources
uniformly, de-duplicate, decode and play non-Spotify audio — exists, is unit
tested, and was validated against the OpenSubsonic spec by an opencode pass.

**What remains is app integration** and it is the large majority of the
effort: the `app` crate is still Spotify-shaped end to end. Wiring the
foundations in means a source-aware engine/controller/queue, a `SourceRegistry`
in the app state, source-config persistence, the first-run wizard and the
settings UI, and source-aware search/browse/transport screens with badges and
in-player source switching — plus the Apple Music catalog client and the CEF
playback subproject. That is a multi-day push, not an overnight one. It is
**not** started here, deliberately: a half-wired `app` crate would break the
build for no gain. The foundations are landed clean so the integration can
proceed crate by crate.
