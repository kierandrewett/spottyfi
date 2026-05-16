# Spottyfi — build plan

A power-user Spotify desktop client in Rust. The render loop is egui (immediate
mode), audio is librespot, and the layout is dockable tabs/panels like the
internal Spotify imgui tool. The visual brief: take the information density and
tabbed dock surface of the leaked internal client, but cover the actual feature
surface of the modern official client (sidebar library, search, browse, lyrics,
queue, devices, profile).

## Deviations from the original brief

- **Name** is **Spottyfi** (placeholder until something better sticks).
- **No Nix flake.** The original brief specified a `flake.nix` dev shell and Nix
  packaging; this has been dropped at the maintainer's request. Use `rustup`
  with the pinned `rust-toolchain.toml`. Packaging (Phase 13) targets AppImage /
  MSI / `.app` only.

## Goal in one paragraph

A native Rust Spotify Premium client with three non-negotiables: (1) it streams
real audio via librespot using the user's own Premium account; (2) it presents a
docking workspace — pages open as tabs, panels (art, queue, lyrics, devices) can
be dragged into splits, with sane defaults that look like the modern Spotify UI;
(3) it stays keyboard-first and information-dense like the imgui internal tool.
Targets Linux first, Windows and macOS as a downstream concern.

## Repo layout

Cargo workspace under `crates/`. One binary, the rest libraries.

```
spottyfi/
├── Cargo.toml                  # workspace
├── rust-toolchain.toml         # pinned toolchain
├── README.md
├── PLAN.md                     # this file
├── TODO.md                     # phase tracker
├── docs/
│   ├── architecture.md
│   ├── auth.md                 # OAuth dance + librespot session
│   ├── threading.md            # tokio <-> egui boundary
│   ├── docking.md
│   └── questions.md            # open questions for the maintainer
└── crates/
    ├── app/                    # binary; eframe app, dock layout, wiring
    ├── audio/                  # librespot wrapper, playback state machine
    ├── api/                    # Spotify Web API client (built on rspotify)
    ├── auth/                   # OAuth PKCE, token refresh, keyring storage
    ├── cache/                  # sqlite metadata cache + on-disk image cache
    ├── models/                 # shared domain types (Track, Album, …)
    ├── state/                  # app state, event bus, action dispatcher
    └── ui/                     # egui widgets, panels, theme, components
```

Crate package names are prefixed `spottyfi-` (e.g. `spottyfi-app`); the binary
is `spottyfi`. The `spottyfi` prefix means `RUST_LOG=spottyfi=debug` covers
every workspace crate.

Why split this way: `audio` and `api` have different runtimes and lifecycles,
and you want to mock `api` for offline UI work. `state` owning the event bus
keeps `ui` free of business logic. `app` is the only crate that knows about both
`audio` and `ui`.

## Tech stack

### Core

- **librespot** (`0.6` or a pinned git commit) — audio + Spotify Connect.
- **rspotify** (`0.13`) — Web API. PKCE flow; share access tokens with librespot
  where possible.
- **eframe / egui** (`0.34`+) — render loop.
- **egui_dock** — dockable tabs and splits. The headline UX feature; do not defer.
- **egui_extras** — `TableBuilder`, image loaders.
- **tokio** — single multi-thread runtime owned by `app`, passed into `audio`/`api`.

### Plumbing

- **reqwest** with `rustls-tls`.
- **serde** + **serde_json**.
- **flume** (preferred) or `tokio::sync` channels between async tasks and the UI.
- **parking_lot** — locks for shared state read by the UI thread.
- **arc-swap** — hot-swappable state snapshots the UI reads each frame.
- **keyring** (`4`) + **keyring-core** (`1`) — refresh tokens in Secret Service /
  keychain / wincred. The 4.x line splits store registration (`keyring`) from the
  `Entry` API (`keyring-core`); see `docs/questions.md`.
- **directories-next** — XDG paths.
- **rusqlite** with `bundled` — metadata + lyrics cache; `.sql` migrations.
- **image** + **webp** — decode album art for egui.
- **tracing** + **tracing-subscriber** — structured logging.
- **thiserror** in libraries, **anyhow** in `app`.
- **clap** — CLI flags (`--no-audio`, `--offline`, `--clear-cache`, `--log-level`).

### Platform integration

- **mpris-server** — MPRIS2 D-Bus on Linux (media keys, indicators).
- **global-hotkey** — fallback for keys not caught by the WM.
- **single-instance** — second launch focuses the running window.
- **tray-icon** — system tray.
- Windows SMTC via **windows-rs** (later); macOS MediaPlayer via **objc2** (much later).

### Build & dev

- **cargo-nextest** for tests.
- **cargo-deny** for licence / advisory checks.
- **cargo-machete** to catch unused deps.
- Pre-commit / CI gate: `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo nextest run`.
- CI: GitHub Actions; build matrix Linux/Windows/macOS later, full test on Linux.

## Architecture

### Threading model

- **UI thread**: owns the egui context; runs at vsync. Reads state via
  `arc-swap` snapshots and read-only `RwLock` reads. Never blocks on I/O.
  Dispatches user actions onto an unbounded `flume::Sender<Action>`.
- **Runtime thread(s)**: one multi-thread `tokio` runtime. Owns the librespot
  `Session`/`Player`/Connect device, the `api` client, the action consumer task,
  and the event publisher.
- **Bridge**: when an `Event` lands, call `egui::Context::request_repaint()`.

### Action / Event model

State mutations only happen on the runtime thread. The UI emits **intent**,
never mutation. `Action` is the inbound command enum (Login, PlayContext,
TogglePlayPause, Seek, Search, OpenPage, …); `Event` is the outbound notification
enum (AuthChanged, PlaybackChanged, LibraryUpdated, PageLoaded, Error, …). Same
shape as a redux/elm store — the UI is a pure projection.

### State store

`AppState` lives behind `Arc<ArcSwap<AppState>>`. Each `Action` handler clones
the current state, mutates a builder, and swaps. Cheap because the mutable
interior is all `Arc`-wrapped. The UI reads `state.load_full()` once per frame.

### Async + egui

- **Promise<T>** wrappers around `JoinHandle` for one-shot fetches; draw a
  spinner while pending.
- **Broadcast subscription** for continuous streams (playback ticks).
- No `block_on` in UI code.

## UI shell

Fixed regions outside the dock:

- **Top bar (28px)**: window controls, back/forward, home, omnisearch
  (Cmd/Ctrl+K focuses), profile menu, notifications.
- **Left sidebar (240–320px, resizable)**: Your Library — pinned playlists,
  scrollable list, filter chips. Collapsible to icon-only.
- **Bottom transport (76px)**: now-playing art + title/artist, centred transport
  controls + scrubber, right side lyrics/queue/devices toggles + volume.
- **Centre: dock surface** — `egui_dock::DockArea`.

### Dock surface

Tab kinds: **Page tabs** (Home, Browse, Search, Playlist, Album, Artist, Genre,
Liked Songs, Made For You, Audiobook) and **Panel tabs** (Now Playing Art,
Queue, Lyrics, Devices, Friend Activity, Debug — docked right by default but
movable). Layout serialises to `config_dir/layout.ron` on shutdown; reset-to-
default in the menu. Cmd-click opens in a new tab; plain click replaces the
focused tab (configurable).

### Theme

Near-black base (#121212), card grey (#181818), elevated (#1f1f1f), accent green
(#1ed760), white/muted text. Ship an alternate teal-green/lilac theme too. Inter
Variable for UI, JetBrains Mono for the debug panel. Density toggle: comfortable
vs compact.

## Phased roadmap

Each phase ends with a working binary. Don't merge a phase that doesn't run.
Branch per phase (`phase-<n>-<slug>`); small commits; run
`cargo fmt && cargo clippy -- -D warnings && cargo nextest run` before each
commit; stop at the end of a phase for review.

### Phase 0 — Bootstrap
Empty workspace that builds, runs, and shows a blank egui window. Eight crates,
CI (build + clippy + fmt on Linux), `tracing` set up so `RUST_LOG=spottyfi=debug`
works. Deliverable: `cargo run -p spottyfi-app` opens a window titled `Spottyfi`.

### Phase 1 — Authentication
OAuth 2.0 PKCE against `accounts.spotify.com`; local HTTP callback server; token
in keyring under service `dev.drewett.spottyfi`; auto-refresh task; login screen;
logout wipes keyring + cache. Confirm librespot's current auth path
(`Credentials::with_access_token` vs dealer/keymaster) before implementing.

### Phase 2 — Audio engine
`audio` wraps librespot `Player`/`Session`. Backend `rodio` (Linux/macOS) /
`cpal` (Windows) behind a feature flag. `PlaybackController` async API; emits
`PlaybackChanged` ~10Hz. Transport bar wired. Local playback only.

### Phase 3 — Web API client
`api` wraps `rspotify::AuthCodeSpotify`. Core endpoints (user, playlists, tracks,
albums, artists, search, browse, recommendations). Respect `Retry-After`,
exponential backoff. Pagination as streams. In-memory LRU cache for now.

### Phase 4 — Core UI shell
Top bar, sidebar (hardcoded items), `egui_dock` centre with one Home tab, bottom
transport showing real playback. Theme applied; layout persists across restarts.

### Phase 5 — Library + page system
`Page` trait + `PageRegistry`. Pages: Home, Playlist, Album, Artist, LikedSongs,
Library. Sidebar lists real playlists. `PlaylistPage` uses `TableBuilder` with
sortable columns. Track row widget with double-click play + context menu.

### Phase 6 — Search
Debounced 250ms search, cancellable in-flight requests. `SearchPage` with All /
Songs / Artists / Albums / Playlists / Podcasts tabs and a Top Result card.

### Phase 7 — Browse
`BrowsePage` genre/category grid, `CategoryPage`, `MadeForYouPage`. Spotify's
discovery endpoints (Recommendations, Featured Playlists, a Category's
playlists) are dead for newly-registered apps, so discovery is sourced from
the **Last.fm API** instead — a `lastfm` module in the `api` crate, keyed by
`SPOTTYFI_LASTFM_API_KEY`, with a resolver that maps Last.fm names back to
Spotify objects. Browse degrades gracefully when no key is set. See
`docs/questions.md` #7.

### Phase 8 — Queue + playback context
`QueuePanel` with Now Playing, "Next from <context>", and the manual queue.
Drag-to-reorder. Playing from a playlist sets the play context.

### Phase 9 — Caches
SQLite metadata cache (stale-while-revalidate); on-disk image cache keyed by
`sha1(url).webp`, LRU-evicted at a 500MB cap; `--clear-cache` wipes both.

### Phase 10 — Docking power features
Tab pinning, per-tab history, Cmd-click new tab, middle-click close,
Cmd+W/T/Shift+T, tab context menu, pre-defined layouts.

### Phase 11 — Lyrics
musixmatch behind a feature flag (off by default); internal Spotify lyrics
endpoint as an undocumented opt-in via `SPOTTYFI_LYRICS_TOKEN`. `LyricsPanel`
renders LRC with the current line highlighted; click a line to seek.

### Phase 12 — Platform polish
MPRIS2, media keys, tray icon, single-instance, settings page, hotkey
customisation, optional track-change notifications.

### Phase 13 — Packaging
AppImage via `cargo-appimage`. Windows MSI via `cargo-wix` (later). macOS `.app`
via `cargo-bundle` (later).

### Out of scope
Music videos, podcasts beyond basic playback, social / Friend Activity, Jam,
DJ/AI features, mobile.

## Conventions

- Rust edition 2021; MSRV pinned in `rust-toolchain.toml`.
- One `thiserror` error type per library crate; `app` collapses to `anyhow`.
- No `unwrap`/`expect` in library code outside `tests/` — `clippy::unwrap_used`
  and `clippy::expect_used` warn locally, deny in CI for non-test code.
- All public items documented; missing-docs warning on lib crates.
- `#[tracing::instrument]` on every public async fn in `api` and `audio`.
- Unit tests next to code; integration tests under `crates/<crate>/tests/`. Mock
  the API client with a trait + `mockall`.
- Conventional Commits (`feat(audio): …`, `fix(api): …`).

## Known unknowns — research before relying on them

1. Current librespot auth flow (it has moved twice recently).
2. Spotify Web API client registration + exact redirect URI.
3. `egui_dock` `DockState` `Serialize`/`Deserialize` support in the chosen version.
4. egui image-loading lifecycle (`install_image_loaders`, network loader).
5. MPRIS2 + Wayland on the target NixOS setup.
6. Audio backend behaviour on PipeWire (via the ALSA shim).

See `docs/questions.md` for the live list.
