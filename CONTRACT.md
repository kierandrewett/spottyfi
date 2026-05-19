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

### F — Apple Music catalog
- [ ] Apple Music catalog API client (developer token), metadata + search
- [ ] Apple Music entries participate in dedup (playable via another source)

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

## Status — honest assessment

Phases **A and B are complete, tested and committed**: a real OpenSubsonic
client and the whole multi-source + de-dup architecture. These are the
foundations and they are solid.

The remaining phases (**C** playback engine, **D** settings/first-run wizard,
**E** search UI + source switching, **F/G** Apple Music) are *app
integration* — wiring the foundations through the audio engine and the egui
frontend. That is a large body of work: a second playback path
(`symphonia` HTTP decode → the cpal sink), source-config persistence, new
settings + wizard UI, and source-aware search/browse/transport screens. It is
**not an overnight deliverable**, and CEF-based Apple Music playback is a
substantial subproject of its own. Tonight delivered the foundations to
spec; the integration is the next, larger push.
