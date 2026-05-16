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

- [ ] OAuth 2.0 PKCE against `accounts.spotify.com`
- [ ] Local HTTP callback server (fixed port `127.0.0.1:8888`)
- [ ] Token stored in keyring (`dev.drewett.spottyfi`)
- [ ] Auto-refresh background task
- [ ] Login screen + logout flow
- [ ] **Blocked on maintainer:** register a Spotify app, set `SPOTTYFI_CLIENT_ID`,
      register redirect `http://127.0.0.1:8888/callback`

## Phase 2 — Audio engine `[ ]`

- [ ] `audio` wraps librespot `Player`/`Session`
- [ ] `PlaybackController` async API
- [ ] `PlaybackChanged` events ~10Hz
- [ ] Transport bar wired

## Phase 3 — Web API client `[ ]`

- [ ] `api` wraps `rspotify`
- [ ] Core endpoints implemented
- [ ] Rate limiting + pagination

## Phase 4 — Core UI shell `[ ]`

- [ ] Top bar, sidebar, `egui_dock` centre, bottom transport
- [ ] Theme applied; layout persists

## Phase 5 — Library + page system `[ ]`
## Phase 6 — Search `[ ]`
## Phase 7 — Browse `[ ]`
## Phase 8 — Queue + playback context `[ ]`
## Phase 9 — Caches `[ ]`
## Phase 10 — Docking power features `[ ]`
## Phase 11 — Lyrics `[ ]`
## Phase 12 — Platform polish `[ ]`
## Phase 13 — Packaging `[ ]`
