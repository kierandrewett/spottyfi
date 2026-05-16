# Open questions

Things to confirm with the maintainer or against upstream **before** relying on
them. Don't guess endpoint shapes or auth flows — add a question here and ask.

## Open

1. **librespot auth flow.** Does `Credentials::with_access_token` work with the
   token from our PKCE flow, or is a separate dealer/keymaster handshake / token
   exchange required? The librespot auth path has moved twice in the last year.
   _Blocks: Phase 1, Phase 2._

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

- **Product name** — `Spottyfi` (placeholder, may change).
- **Nix flake** — dropped; use `rustup` with the pinned `rust-toolchain.toml`.
- **Redirect URI** — fixed loopback `http://127.0.0.1:8888/callback` (port
  configurable via `SPOTTYFI_REDIRECT_PORT`). A random port can't work: Spotify
  requires the registered redirect URI to match exactly, port included.
