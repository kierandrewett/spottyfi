# Spottyfi

A native Rust Spotify Premium client with a docking workspace UI.

Spottyfi streams real audio through [librespot] using your own Premium account,
presents a dockable tab/panel workspace (pages open as tabs; art, queue, lyrics
and devices panels drag into splits), and stays keyboard-first and
information-dense — the layout of the leaked internal Spotify imgui tool with
the feature surface of the modern client.

> **Status:** 1.0.0. All roadmap phases (0–13) and the post-roadmap polish
> round are implemented. See [`PLAN.md`](PLAN.md) for the architecture and
> [`TODO.md`](TODO.md) for the phase tracker. Playback needs a Spotify
> **Premium** account and a registered Spotify app id — see
> [`docs/questions.md`](docs/questions.md).

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
| `SPOTTYFI_MUSIXMATCH_KEY` | optional | a [musixmatch API key](https://developer.musixmatch.com/); enables the optional musixmatch lyrics provider. Requires a build with the `musixmatch` Cargo feature (off by default — see below). |

### Lyrics

The Lyrics panel sources time-synced lyrics from [lrclib.net](https://lrclib.net/)
by default — a free, open, community lyrics database that needs **no API key
and no setup**. It works out of the box, so the Lyrics panel is functional in a
default build.

Two further providers are optional:

- **musixmatch** — a commercial API behind the off-by-default `musixmatch`
  Cargo feature. Build with it and set `SPOTTYFI_MUSIXMATCH_KEY`:

  ```sh
  cargo run --features spottyfi-api/musixmatch
  ```

- The **internal Spotify** lyrics endpoint — undocumented and against Spotify's
  Terms of Service; only attempted when `SPOTTYFI_LYRICS_TOKEN` is set. See
  `docs/questions.md`.

The provider is chosen on the Settings page (**Automatic** / lrclib / musixmatch
/ Spotify internal); Automatic tries each available provider, lrclib first.
When several lyrics versions exist, candidates are scored by track duration (and
title/artist/album) so the synced lyrics line up with the recording playing.
Fetched lyrics are cached in the SQLite metadata store, so revisiting a track
does not re-fetch — including "no lyrics found" misses, on a shorter TTL.

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

## Packaging & install

Desktop-integration assets live under [`packaging/`](packaging/): a freedesktop
`.desktop` entry, an AppStream `metainfo.xml`, and the app icon as SVG plus
rasterised PNGs (64/128/256/512). Linux is the supported packaging target;
Windows and macOS are stubbed for later.

### AppImage (Linux)

A self-contained, single-file build. The icon and desktop assets and the
`[package.metadata.appimage]` block in `crates/app/Cargo.toml` are already
wired up:

```sh
cargo install cargo-appimage     # needs `appimagetool` on PATH
cargo appimage --release --package spottyfi-app
```

The result lands in `target/appimage/`. The release workflow
(`.github/workflows/release.yml`) builds this automatically on every `v*` tag.

### Flatpak (Linux)

A manifest is provided at
[`packaging/flatpak/dev.drewett.spottyfi.yml`](packaging/flatpak/dev.drewett.spottyfi.yml).
It targets the `org.freedesktop.Platform` 24.08 runtime with the `rust-stable`
SDK extension. Flatpak builds are offline, so the crate dependencies must first
be vendored into a `cargo-sources.json` with the
[flatpak-builder-tools cargo generator](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo)
(see the comments at the top of the manifest):

```sh
flatpak install flathub org.freedesktop.Platform//24.08 \
                         org.freedesktop.Sdk//24.08 \
                         org.freedesktop.Sdk.Extension.rust-stable//24.08
python3 flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json
flatpak-builder --user --install --force-clean \
    build-dir packaging/flatpak/dev.drewett.spottyfi.yml
```

`cargo-sources.json` is generated, not committed; regenerate it whenever
`Cargo.lock` changes.

### Windows / macOS (later)

Not current targets — only the configuration is stubbed:

- **macOS `.app`** — `[package.metadata.bundle]` in `crates/app/Cargo.toml`
  feeds [`cargo-bundle`](https://github.com/burtonageo/cargo-bundle):
  `cargo install cargo-bundle && cargo bundle --release`.
- **Windows MSI** — `cargo install cargo-wix`, then `cargo wix init` generates
  `wix/main.wxs` and `cargo wix` builds the installer. The `.wxs` file is not
  committed yet.

## Licence

Dual-licensed under MIT or Apache-2.0.
