# Spottyfi — phase tracker

Status legend: `[ ]` not started · `[~]` in progress · `[x]` done.

See `PLAN.md` for the full brief. Each phase ends with a runnable binary.

## Phase 0 — Bootstrap `[x]`

- [x] Cargo workspace with eight crates
- [x] `app` binary opens an empty egui window titled `Spottyfi`
- [x] `tracing` set up; `RUST_LOG=spottyfi=debug` works
- [x] CLI flags scaffolded (`--no-audio`, `--offline`, `--clear-cache`, `--log-level`)
- [x] `cargo build` / `clippy -D warnings` / `fmt --check` / `nextest` all green
- [x] CI: build + clippy + fmt + test on Linux
- [x] Public GitHub repo, regular commits + pushes

## Phase 1 — Authentication `[~]`

- [x] OAuth 2.0 PKCE against `accounts.spotify.com`
- [x] Local HTTP callback server (fixed port `127.0.0.1:8888`)
- [x] Token stored in keyring (`dev.drewett.spottyfi`)
- [x] Auto-refresh background task
- [x] Login screen + logout flow
- [ ] **Live test, blocked on maintainer:** register a Spotify app, set
      `SPOTTYFI_CLIENT_ID`, register redirect `http://127.0.0.1:8888/callback`

## Phase 2 — Audio engine `[~]`

- [x] `audio` wraps librespot 0.8 `Player`/`Session`
- [x] `PlaybackController` async API (play/pause/seek/volume)
- [x] Playback state snapshot, refreshed ~10Hz
- [x] Transport bar wired + debug "play a URI" control
- [ ] **Live test, blocked on maintainer:** sign in with a Premium account and
      play a `spotify:track:` URI

## Phase 3 — Web API client `[x]`

- [x] `api` wraps `rspotify`; `models` domain types; `SpotifyApi` trait + mock
- [x] Core endpoints implemented (rspotify → `models` mapping)
- [x] Rate limiting (`Retry-After` + backoff) + pagination streams + LRU cache
- Note: several discovery endpoints are dead for new apps — see
  `docs/questions.md` #7 (affects Phase 7 Browse).

## Phase 4 — Core UI shell `[x]`

- [x] `ui` crate: two dark themes, bundled Inter/JetBrains Mono fonts,
      network `ImageLoader`, reusable components (headers, art, buttons, chips)
- [x] Top bar (nav + Home + omni-search + View menu + profile menu)
- [x] Resizable, collapsible left sidebar with hardcoded library entries
- [x] `egui_dock` centre with the default Home / Now Playing Art / Queue layout
- [x] Polished bottom transport wired to live `PlaybackState`
- [x] Settings window: theme + density (persisted)
- [x] Dock layout + settings persist to `<config_dir>/layout.ron`; reset action

## Phase 5 — Library + page system `[ ]`
## Phase 6 — Search `[ ]`
## Phase 7 — Browse `[ ]`
## Phase 8 — Queue + playback context `[ ]`
## Phase 9 — Caches `[ ]`
## Phase 10 — Docking power features `[ ]`
## Phase 11 — Lyrics `[ ]`
## Phase 12 — Platform polish `[ ]`
## Phase 13 — Packaging `[ ]`
