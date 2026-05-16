# Spottyfi

A native Rust Spotify Premium client with a docking workspace UI.

Spottyfi streams real audio through [librespot] using your own Premium account,
presents a dockable tab/panel workspace (pages open as tabs; art, queue, lyrics
and devices panels drag into splits), and stays keyboard-first and
information-dense — the layout of the leaked internal Spotify imgui tool with
the feature surface of the modern client.

> **Status:** early development. See [`PLAN.md`](PLAN.md) for the full roadmap
> and [`TODO.md`](TODO.md) for the phase tracker. Currently at **Phase 0 —
> bootstrap**: the binary opens an empty window.

## Requirements

- Rust 1.95.0 (pinned via [`rust-toolchain.toml`](rust-toolchain.toml); `rustup`
  picks it up automatically).
- A Spotify **Premium** account (playback control endpoints are Premium-only).
- Linux: ALSA/PipeWire, plus the usual graphics stack (Wayland or X11, Vulkan or
  OpenGL). On Debian/Ubuntu the build needs `pkg-config`, `libasound2-dev`,
  `libssl-dev`, `libxkbcommon-dev` and `libwayland-dev`.

## Build & run

```sh
# Live login needs a Spotify app — set its Client ID first (PKCE, no secret):
export SPOTTYFI_CLIENT_ID=<your-client-id>
# Optional: a free Last.fm API key powers Browse's charts & recommendations.
export SPOTTYFI_LASTFM_API_KEY=<your-lastfm-key>
cargo run
```

`cargo run` launches the app (the workspace defaults to the `app` crate).
Workspace-wide commands still take an explicit `--workspace`. The binary is
named `spottyfi`. Useful flags:

| Flag | Effect |
| --- | --- |
| `--no-audio` | start without the audio engine (UI-only development) |
| `--offline` | suppress network requests; render from cache |
| `--clear-cache` | wipe metadata + image caches on startup |
| `--log-level <level>` | default log level when `RUST_LOG` is unset |

Logging uses [`tracing`]. `RUST_LOG` overrides everything:

```sh
RUST_LOG=spottyfi=debug cargo run
```

### Environment variables

| Variable | Required? | Purpose |
| --- | --- | --- |
| `SPOTTYFI_CLIENT_ID` | for live login | the registered Spotify app's Client ID (PKCE, no secret) |
| `SPOTTYFI_REDIRECT_PORT` | optional | overrides the loopback callback port (default `8888`) |
| `SPOTTYFI_LASTFM_API_KEY` | optional | a free [Last.fm API key](https://www.last.fm/api/account/create); enables Browse's charts and recommendations. Without it, Browse still shows the Spotify category grid and those sections show a "set the key" note. |

## Workspace layout

| Crate | Responsibility |
| --- | --- |
| `app` | binary — eframe app, dock layout, wiring (the only crate that knows both `audio` and `ui`) |
| `audio` | librespot wrapper + playback state machine |
| `api` | Spotify Web API client (built on rspotify) |
| `auth` | OAuth PKCE flow, token refresh, keyring storage |
| `cache` | SQLite metadata cache + on-disk image cache |
| `models` | shared domain types (`Track`, `Album`, …) |
| `state` | app state, event bus, action dispatcher |
| `ui` | egui widgets, panels, theme, components |

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace      # or: cargo test --workspace
```

CI runs the same gate on Linux. See [`docs/`](docs/) for architecture notes.

## Licence

Dual-licensed under MIT or Apache-2.0.
