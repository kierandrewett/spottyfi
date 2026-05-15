{
  # Spottyfi — reproducible development shell (Phase 0).
  #
  # This flake provides `devShells.default`: a Nix dev shell with the pinned
  # Rust toolchain (from ./rust-toolchain.toml), the native libraries needed to
  # build and run the egui/eframe UI and the librespot audio backend, plus
  # extra cargo dev tooling.
  #
  # NOTE: a `packages.default` build of the client itself (via crane or naersk)
  # is intentionally deferred to Phase 13. This flake only ships a dev shell.

  description = "Spottyfi — native Spotify client (dev shell)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Rust toolchain pinned by the repo's rust-toolchain.toml
        # (channel 1.95.0 with rustfmt, clippy, rust-analyzer).
        rustToolchain =
          pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        # Native libraries required to build and run the app:
        # egui/eframe needs wayland/X11/vulkan/GL, librespot needs alsa/openssl.
        buildInputs = with pkgs; [
          alsa-lib
          openssl
          libxkbcommon
          wayland
          vulkan-loader
          libGL
          xorg.libX11
          xorg.libXcursor
          xorg.libXrandr
          xorg.libXi
        ];

        nativeBuildInputs = with pkgs; [
          pkg-config
        ];

        devTools = with pkgs; [
          cargo-nextest
          cargo-deny
          cargo-machete
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          inherit buildInputs nativeBuildInputs;

          packages = [ rustToolchain ] ++ devTools;

          # The egui window dlopens wayland/vulkan/GL at runtime, so the
          # library paths must be visible via LD_LIBRARY_PATH.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;

          RUST_BACKTRACE = "1";
        };
      });
}
