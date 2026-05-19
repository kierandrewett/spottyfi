# Multi-source music — contract of work

Turning Spottyfi from a Spotify-only client into a general-purpose music app
(à la Lyrion) with a Spotify-style frontend. Multiple libraries, one seamless
experience, source-tagged everywhere, de-duplicated.

## Scope

- **OpenSubsonic** and **Spotify** are the two sources — both full sources
  (metadata + playback).
- Apple Music was evaluated and **dropped**: its audio is FairPlay-DRM, so it
  could only ever be a catalog source played through an embedded Chromium
  (CEF) — not worth the weight. All Apple Music code has been removed.
- Work lands in small, tested, committed increments; this file is the
  running source of truth.

## Phases & tasks

### A — OpenSubsonic client crate `crates/subsonic` ✅
- [x] Crate scaffold, error type, response-envelope handling
- [x] Auth (salt + token), `ping`
- [x] Models: song / album / artist / playlist / search result
- [x] Endpoints: `search3`, `getArtists`, `getArtist`, `getAlbum`,
      `getAlbumList2`, `getPlaylists`, `getPlaylist`, `getStarred2`
- [x] `stream` + `getCoverArt` URL builders, `scrobble`, `star`/`unstar`
- [x] Unit tests (auth, envelope, error, model parsing)

### B — Multi-source architecture ✅
- [x] `SourceId` / `SourceKind` / `SourceRef` — a source tag on every entity
- [x] Source-neutral `Track` / `Album` / `Artist` / `SearchResults`
- [x] `MusicSource` trait (search, browse, stream URL) + `can_play` capability
- [x] `SourceRegistry` — concurrent `search_all` across all sources
- [x] OpenSubsonic and Spotify behind the `MusicSource` trait
      (`SubsonicSource` / `SpotifySource`)
- [x] `PlaybackBackend` trait — librespot (`SpotifyBackend`) and HTTP
      (`HttpAudioPlayer`) both behind it

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

### F — Polish
- [ ] Source badges, empty states, error surfaces
- [ ] opencode (GPT) validation pass per phase
- [ ] Docs / README update

## Progress log

- **Phase A done.** `crates/subsonic` — a complete, tested OpenSubsonic
  client: salt+token auth, envelope/error handling, all browse + search +
  library endpoints, signed stream/cover-art URLs, scrobble & star.
- **Phase B done.** `crates/sources` — the multi-source layer: `SourceRef`
  tags every entity; source-neutral `Track`/`Album`/`Artist`; the
  `MusicSource` trait + `SourceRegistry` with concurrent `search_all`;
  cross-source de-duplication (recording-id then fuzzy, noise-aware); the
  OpenSubsonic and Spotify adapters. `PlaybackBackend` trait with the
  librespot and HTTP backends behind it.
- **Phase C player done.** Extracted a shared `CpalOutput` stage from
  `CpalSink`; built `HttpAudioPlayer` — fetch + `symphonia` decode (FLAC /
  MP3 / Ogg-Vorbis) + resample → `CpalOutput`, on its own thread, with
  play/pause/resume/stop/seek/volume/position. Subsonic audio is playable.
- **Two opencode (GPT) review passes** run — the OpenSubsonic-spec pass and
  the 5-agent pass; the actionable findings were fixed (per-request salt,
  `error_for_status`, empty-library decode, the HTTP-player paused-ring
  deadlock, the dedup `unique_key` collision, …).
- **Apple Music removed.** Evaluated and dropped — the `applemusic` and
  `applemusic-player` crates and the `AppleMusicSource` are gone; ISRC is
  kept as a general-purpose dedup key.

## Status — honest assessment

The **foundation stack is complete, tested and committed**: the OpenSubsonic
client, the multi-source abstraction + de-duplication, the `MusicSource` and
`PlaybackBackend` traits with adapters for both backends, and the HTTP
playback player.

**What remains is app integration** — the `app` crate is still Spotify-shaped
end to end. Wiring the foundations in means a source-aware
engine/controller/queue, a `SourceRegistry` in the app state, source-config
persistence, the first-run wizard and settings UI, and source-aware
search/browse/transport screens with badges and in-player source switching.
That is a multi-day push; it proceeds crate by crate from the clean
foundations.
