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

## Phase 5 — Library + page system `[x]`

- [x] `Page` trait + `PageRegistry`; pages keyed by lightweight `Tab` keys
- [x] `Loadable<T>` one-shot promise wrapper (`poll-promise` + tokio runtime)
- [x] Pages: Home, Playlist, Album, Artist, LikedSongs, Library — each loads
      asynchronously and draws a spinner / error / data
- [x] Home replaces the Phase 4 placeholder with a real-data shelf view
- [x] Sidebar lists the user's real playlists (Liked Songs + Library pinned);
      clicking an entry opens its page tab
- [x] Sortable track-table widget in `ui` (`#`, Title, Album, Date added,
      Duration); double-click plays, right-click context menu
- [x] `app` builds a `SpotifyClient` after login and wires `Arc<dyn SpotifyApi>`
      into the shell
- Notes: Liked Songs "Date added" is empty (the `api` `saved_tracks` mapper
  drops `added_at`); tab navigation is open/focus, not strict replace —
  see `docs/questions.md` #8 and #9. "Play next" / "Add to queue" warn and
  defer to the Phase 8 queue.

## Phase 6 — Search `[x]`

- [x] `SearchPage` — a real, registry-backed page replacing the placeholder
      (`Tab::Search` is now a page, not a self-rendered panel)
- [x] In-page search input (no top-bar box); typing re-runs the query
- [x] Debounced ~250ms — query fires only after the user stops typing
- [x] Cancellation — a new query aborts the in-flight task; a stale slow
      response can never overwrite a newer query's results (generation guard)
- [x] In-flight search registered in the `ActivityRegistry` ("Searching…")
- [x] Category tabs: All / Songs / Artists / Albums / Playlists / Podcasts
- [x] **All** tab: Top result card + inline Songs list + horizontal shelves
- [x] Songs reuse the track-row/table widget; artist/album/playlist cards
      via the network image loader; clicking navigates, double-click plays
- [x] `Ctrl/Cmd+K` focuses the search input (opening the tab if needed);
      `Tools ▸ Search` opens + focuses the same way
- [x] Unit tests for debounce, cancellation and result routing
- Note: **Podcasts** is deferred — `api`'s `SearchType` enum has no
  show/episode variant, so the Podcasts tab shows an explanatory note.
  Audiobooks omitted for the same reason. See `docs/questions.md` #10.

## Phase 7 — Browse `[x]`

- [x] `lastfm` module in `api`: `LastfmClient` (chart.getTop*, tag.getTop*,
      artist.getSimilar, track.getSimilar, artist.getTopTracks) with its own
      `thiserror` error; key from `SPOTTYFI_LASTFM_API_KEY`, `from_env`
      returns `NotConfigured` without panicking
- [x] `LastfmResolver` maps Last.fm artist/track names to Spotify objects
      via `SpotifyApi::search` (best-match)
- [x] `current_user_top_artists` / `current_user_top_tracks` + `new_releases`
      added to the `SpotifyApi` trait
- [x] `BrowsePage` — Spotify category grid (rotated art tiles) + Last.fm
      charts shelves
- [x] `CategoryPage` — a Spotify category mapped to a Last.fm tag's top
      tracks/artists, resolved to Spotify objects
- [x] `ChartsPage` — Last.fm global top tracks/artists
- [x] `MadeForYouPage` — recommendations from the user's Spotify top items
      via Last.fm similarity
- [x] `NewReleasesPage` — Spotify `new-releases`; clean note when unavailable
- [x] Sidebar Browse / Charts / New Releases / Discover entries wired to the
      real pages (Discover → Made For You)
- [x] Graceful degradation with no Last.fm key; unit tests for Last.fm JSON
      parsing and the name→Spotify resolver
- Note: discovery is sourced from Last.fm because Spotify's
  Recommendations / Featured Playlists / Category-playlists endpoints are
  dead for new apps — see `docs/questions.md` #7. The maintainer must
  create a free Last.fm API key and set `SPOTTYFI_LASTFM_API_KEY`.

## Phase 8 — Queue + playback context `[x]`

- [x] `audio` queue/context state machine + `PlaybackController` queue methods
      (auto-advance on `EndOfTrack`)
- [x] `QueueState` snapshot bridged into the UI via an `ArcSwap`, alongside the
      playback snapshot
- [x] Context playback from pages: playing a track from a playlist / album /
      artist / search / browse plays that page's full resolved track list as a
      context, so Next/Prev walk it
- [x] Transport bar next/prev buttons + Playback-menu Next/Previous wired to
      the controller
- [x] Track context menu "Play next" / "Add to queue" route to the manual
      queue (replacing the Phase-8 warn stubs)
- [x] `QueuePanel`: Now Playing, "Next from <context>" and the manual queue —
      dense and flat; click an entry to skip to it; drag-to-reorder the manual
      queue with a remove action
- Note: single-track `play_uri` (the Debug panel field) still works as a
  context-free one-off. Pages without a real Spotify context URI (search,
  browse, charts, Liked Songs) use a synthetic `spottyfi:` context URI — the
  list still walks correctly; only the panel's "Next from …" label differs.

## Phase 9 — Caches `[x]`

- [x] `cache` crate: SQLite metadata store (`rusqlite`, bundled) for
      tracks/albums/artists/playlists; JSON-blob rows + `last_fetched`
- [x] `.sql` migration files under `crates/cache/migrations/`, applied by a
      forward-only runner that tracks the version in `PRAGMA user_version`
- [x] `Freshness`/`Staleness` stale-while-revalidate policy (1h window)
- [x] `api`: `MetadataLayer` (in-memory hot cache + persistent SQLite store)
      wires SWR into `album`/`artist`/`playlist` — fresh hit skips the
      network, stale hit serves immediately + spawns a background refresh;
      blocking SQLite calls run on `spawn_blocking`
- [x] `cache` crate: on-disk `ImageCache` keyed by `sha1(url).webp`, a
      size-capped LRU with a 500MB default cap, mtime-ordered eviction
- [x] `ui`: the on-disk image cache slots into the `NetworkImageLoader`
      Phase 9 seam — disk lookup before the network, disk write on a miss,
      all on a worker thread; public surface unchanged
- [x] `--clear-cache` wipes the metadata DB (+ WAL/SHM) and the image cache
      directory on startup
- [x] Unit tests for the migration runner, freshness logic and LRU eviction
- Note: the persistent metadata cache and on-disk image cache both degrade
  gracefully (in-memory-only / network-only, with a warning) if the
  platform cache directory cannot be resolved.

## Phase 10 — Docking power features `[x]`

- [x] Tab pinning — pin/unpin via the right-click menu; a pinned tab keeps no
      close button and is spared by "Close others" / "Close to the right";
      pin state persists with the layout (`DockExtras.pinned`)
- [x] Per-tab browser-style history — each tab has its own back/forward stack;
      replacing a tab's content (plain navigation) pushes history; a fresh
      navigation clears the forward stack
- [x] Back / forward buttons in the menu bar reflect availability for the
      focused tab and are disabled when its history is empty
- [x] Navigation rules — plain click/navigation **replaces** the focused tab;
      `Ctrl/Cmd-click` a sidebar entry or an in-page link **opens a new tab**;
      consistent across the sidebar, page links and search results
- [x] Middle-click a tab closes it
- [x] `Cmd/Ctrl+W` close tab, `Cmd/Ctrl+T` new Home tab,
      `Cmd/Ctrl+Shift+T` reopen last closed (bounded closed-tab stack);
      "Reopen closed tab" also in the Tools menu
- [x] Tab right-click menu: Close, Close others, Close to the right,
      Duplicate, Pin/Unpin
- [x] Predefined layouts in the View menu — Default, Power user (compact
      density + Queue docked right; Lyrics added in Phase 11), Minimal
      (single centre tab); Reset layout still works
- [x] Layout schema extended with `#[serde(default)]` fields so a
      pre-Phase-10 `layout.ron` still loads
- [x] Unit tests for the history stack, the closed-tab stack and the
      pin-aware close logic
- Notes: egui_dock 0.19 supplied the per-tab `context_menu` hook, the
  `on_tab_button` hook (middle-click) and `is_closeable`; pinning, per-tab
  history, the closed-tab stack and predefined layouts are all app-layer
  state in `shell/dock_model.rs` + `shell/nav.rs`, keyed by `Tab`. The
  back/forward buttons live in the menu bar — Phase 4 replaced the Phase-0
  top bar with the menu bar, so there is no separate top bar to host them.

## Phase 11 — Lyrics `[x]`

- [x] `lyrics` module in `api`: `Lyrics` model (synced timestamped lines /
      plain unsynced), an LRC parser and a current-line selector, with its
      own `thiserror` error (`LyricsError`)
- [x] **musixmatch** provider — the legitimate path, behind the off-by-default
      `musixmatch` Cargo feature, keyed by `SPOTTYFI_MUSIXMATCH_KEY`
- [x] **Internal Spotify color-lyrics** provider — undocumented, opt-in only
      via `SPOTTYFI_LYRICS_TOKEN`, clearly commented as against Spotify ToS;
      never on by default
- [x] `LyricsService::from_env` assembles whichever providers are configured;
      with none set, lookups return `NoSourceConfigured` — never panics
- [x] `LyricsPanel` (`Tab::Lyrics`) — a dock panel that re-fetches on track
      change; synced lyrics highlight the current line from the live playback
      position and auto-scroll; click a line to seek; plain lyrics render as a
      static scrollable column; calm empty/loading/unavailable states
- [x] Openable from the View menu; wired into the Power user layout slot
- [x] Unit tests for LRC parsing and current-line selection from a position
- Note: lyrics need a musixmatch key — the maintainer must create a free key
  and build with `--features spottyfi-api/musixmatch`. See `docs/questions.md`
  #12.

## Phase 12 — Platform polish `[x]`

- [x] **MPRIS2** — `org.mpris.MediaPlayer2` + `…Player` D-Bus interface via
      `mpris-server`: publishes title / artist / album / art / position /
      playback status, accepts Play/Pause/Next/Previous/Stop/Seek/Raise/Quit;
      driven from the live `PlaybackState`, commands routed into the existing
      `PlaybackController`. A background task emits `PropertiesChanged` /
      `Seeked` so GNOME/KDE indicators stay in sync.
- [x] **Media keys** — `global-hotkey` registers the XF86Audio* keys plus the
      user's transport hotkeys system-wide, a fallback for WMs that do not
      route media keys through MPRIS. Events pumped on a background thread.
- [x] **System tray** — a `tray-icon` tray (own GTK thread) with a
      Play/Pause / Next / Previous / Show-Hide / Quit menu; the Play/Pause
      label and tooltip track the live playback state.
- [x] **Single-instance** — a `single-instance` lock; a second launch asks the
      running instance to raise its window over MPRIS `Raise`, then exits.
- [x] **Hotkey customisation** — shortcuts are a rebindable, RON-persisted
      `HotkeyMap`; the Settings page's Hotkeys section has a capture-the-next-
      keypress editor with conflict detection and reset-to-defaults.
- [x] **Track-change notifications** — `notify-rust` desktop notification on
      track change, **off by default**, toggled in Settings ▸ Notifications.
- [x] Tray / media-key event channels integrated with the egui loop via a
      `MediaCommand` channel drained each frame; MPRIS runs on the tokio
      runtime.
- [x] CI: `libgtk-3-dev`, `libayatana-appindicator3-dev`, `libdbus-1-dev`
      added to the apt step. `tray-icon` built with `default-features = false`
      + `gtk` so no `libxdo` dev package is needed.
- Notes: most of this needs a real desktop session to verify (D-Bus name
  claim, indicators, tray rendering, media-key grabs, the raise-on-second-
  launch hop). See `docs/questions.md` #5. Windows SMTC / macOS MediaPlayer
  are explicitly out of scope. The Settings page (theme/density/audio/EQ)
  already existed from WS5; Phase 12 only added its Notifications and editable
  Hotkeys sections.

## Phase 13 — Packaging `[ ]`
