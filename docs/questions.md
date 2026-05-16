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

7. **`keyring` 4.x is a different crate from `keyring` 3.x.** PLAN.md pins
   `keyring = "3"`; Phase 1 was instructed to use `keyring = "4"`. In the 4.x
   line the `keyring` crate was restructured: it no longer exposes the `Entry`
   API, and it has no `[features]` for selecting backends. It is now a thin
   *store-registration* crate (`keyring::use_native_store(...)`), and the
   credential `Entry` API lives in the separate `keyring-core` crate. Spottyfi
   therefore depends on **both** `keyring` (to register the platform store at
   startup) and `keyring-core` (for `Entry::new/get/set/delete`). All OS
   backends are bundled unconditionally in 4.x, so no per-OS feature flags are
   needed — it is cross-platform out of the box. On Linux, Spottyfi calls
   `use_native_store(true)` to select the Secret Service rather than the kernel
   keyutils store, so tokens survive a reboot.
   _Decision needed: keep `keyring 4` + `keyring-core 1`, or pin back to
   `keyring 3` (single crate, classic `Entry` API). Phase 1 went with 4.x as
   instructed; PLAN.md's `keyring = "3"` line should be updated to match
   whichever is chosen._

8. **`keyring` 4.x pulls in a large dependency tree.** Its `db-keystore`
   fallback drags in `turso` (a SQLite reimplementation) and, on Linux, the
   Secret Service backend vendors OpenSSL. This noticeably increases first-build
   time. If undesirable, `keyring 3` avoids it. _Tied to question 7._

## Resolved

- **Product name** — `Spottyfi` (placeholder, may change).
- **Nix flake** — dropped; use `rustup` with the pinned `rust-toolchain.toml`.
- **Redirect URI** — fixed loopback `http://127.0.0.1:8888/callback` (port
  configurable via `SPOTTYFI_REDIRECT_PORT`). A random port can't work: Spotify
  requires the registered redirect URI to match exactly, port included.
