# Open questions

Things to confirm with the maintainer or against upstream **before** relying on
them. Don't guess endpoint shapes or auth flows — add a question here and ask.

## Open

2. **Spotify app registration.** Spottyfi needs a Spotify app registered on the
   developer dashboard (https://developer.spotify.com/dashboard). The maintainer
   must create it and provide the **Client ID** via the `SPOTTYFI_CLIENT_ID`
   environment variable (the PKCE flow has no client secret, so the ID is not
   sensitive). _Blocks live login in Phase 1; the code is built without it._

3. **`egui_dock` state serialisation.** Confirm the pinned `egui_dock` version
   derives `Serialize`/`Deserialize` on `DockState`. _Blocks: Phase 4 layout
   persistence._

4. **egui image-loading lifecycle.** Validate `egui_extras::install_image_loaders`
   plus the required `image` crate features, and whether web URLs need a custom
   network loader. _Blocks: Phase 4._

5. **MPRIS2 + Wayland.** Smoke-test the MPRIS2 D-Bus interface on the target
   NixOS/Wayland setup early (Phase 4), not at Phase 12.

6. **Audio backend on PipeWire.** `rodio` goes through ALSA; on PipeWire it
   works via the ALSA shim. Confirm acceptable, or plan a `pipewire-rs` backend.

## Resolved

- **librespot auth flow** (was #1) — **`Credentials::with_access_token` works
  directly** with the OAuth access token from Spottyfi's PKCE flow. No separate
  dealer/keymaster handshake or token exchange is needed. Confirmed against the
  librespot 0.8.0 source: `with_access_token` sets `auth_type` to
  `AUTHENTICATION_SPOTIFY_TOKEN`, and `Session::connect` forwards that auth type
  and the raw token bytes straight into the access-point handshake
  (`librespot-core/src/connection/mod.rs`). The 0.8.0 `examples/play.rs` does
  exactly this. _Caveats:_ (1) a token-authed session **cannot use keymaster** —
  `session.token_provider()` will not mint fresh tokens from inside librespot,
  so token refresh must stay owned by the `auth` crate (it already is); the
  audio engine is simply restarted with a fresh token if the session drops.
  (2) librespot needs the `streaming` scope, which Spottyfi's PKCE flow already
  requests. (3) Playback requires a **Spotify Premium** account — librespot
  rejects free accounts at the AP handshake.
- **Roadmap pinned `librespot = "0.6"`; the implemented version is `0.8`.** The
  API moved: `SpotifyId` is now split into `SpotifyId` (the raw id) and
  `SpotifyUri` (the typed enum, used by `Player::load` and `PlayerEvent`).
  `Player::new` takes a `VolumeGetter` and a sink-builder closure. The new
  `PlayerEvent::PositionChanged` (gated on `PlayerConfig::position_update_interval`)
  feeds the ~10Hz progress updates.
- **`vergen-gitcl 1.0.8` dependency conflict.** librespot-core's build script
  pulls `vergen-gitcl 1.0.8`, which has inconsistent constraints: it requires
  `vergen-lib ^0.1.6` directly but also `vergen ^9.0.6`, and `vergen 9.1.0`
  needs `vergen-lib 9.1.0` — two incompatible `vergen-lib` versions in one
  build. `Cargo.lock` pins `vergen` to `9.0.6` (which pairs with `vergen-lib
  0.1.6`) to resolve it. The lockfile is committed, so this holds; if a future
  `cargo update` reintroduces the break, re-pin with
  `cargo update vergen --precise 9.0.6`.
- **Product name** — `Spottyfi` (placeholder, may change).
- **Nix flake** — dropped; use `rustup` with the pinned `rust-toolchain.toml`.
- **Redirect URI** — fixed loopback `http://127.0.0.1:8888/callback` (port
  configurable via `SPOTTYFI_REDIRECT_PORT`). A random port can't work: Spotify
  requires the registered redirect URI to match exactly, port included.
- **`keyring` version** — staying on `keyring 4` + `keyring-core 1` (maintainer's
  call), not the `keyring 3` originally pinned in `PLAN.md`. 4.x is a different
  crate shape: store registration (`keyring::use_native_store`) is split from the
  `Entry` API (`keyring-core`), all OS backends are bundled unconditionally, and
  it pulls a heavier dependency tree (`turso`, vendored OpenSSL on Linux). `PLAN.md`
  has been updated to match.
