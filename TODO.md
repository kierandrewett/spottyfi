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

## Phase 9 — Caches `[ ]`
## Phase 10 — Docking power features `[ ]`
## Phase 11 — Lyrics `[ ]`
## Phase 12 — Platform polish `[ ]`
## Phase 13 — Packaging `[ ]`
