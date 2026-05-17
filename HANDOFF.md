# Handoff — continuing Spottyfi

Paste the prompt at the bottom into Claude Code on your main PC. Everything
above it is the context that prompt refers to.

---

## What this is

**Spottyfi** is a native Rust Spotify client — librespot audio, an `egui` +
`egui_dock` docking workspace, keyboard-first and information-dense (the look of
the leaked internal Spotify imgui tool). Public repo:
`github.com/kierandrewett/spottyfi`.

## Current state

The full `PLAN.md` roadmap (**Phases 0–13**) is implemented, plus a round of
maintainer feedback (`docs/feedback-backlog.md`, **WS1–WS9**). `main` is green
(build + `clippy -D warnings` + fmt + tests pass) and CI runs that gate on every
push.

It is **functionally complete but only lightly verified on a real desktop** —
it was built largely in a headless environment, so expect visual rough edges
and runtime issues that only show up when you actually run it. The first job is
to run it and fix what's wrong.

What's in: OAuth PKCE login, librespot 0.8 streaming, the dock shell (tree
sidebar, menu bar, tabs, transport), Home/Playlist/Album/Artist/Search/Browse
pages, queue + shuffle/repeat, 10-band equalizer, waveform + spectrum
visualiser, synced lyrics (lrclib default), SQLite + image caching, Spotify
Connect device registration, MPRIS2/tray/media-keys/notifications, a settings
page, and Linux packaging (AppImage/Flatpak).

## First steps on the new machine

```sh
git clone https://github.com/kierandrewett/spottyfi
cd spottyfi
cargo build --workspace          # first build is slow (librespot is heavy)
export SPOTTYFI_CLIENT_ID=979a1119766143e689929da242f06a60
cargo run                        # opens the app → "Sign in with Spotify"
```

- Needs a Spotify **Premium** account for playback (librespot rejects free).
- Optional env keys: `SPOTTYFI_LASTFM_API_KEY` (Browse charts/recs),
  `SPOTTYFI_MUSIXMATCH_KEY` (extra lyrics source; also needs
  `--features spottyfi-api/musixmatch`). Lyrics work out of the box via lrclib.
- System build deps (Debian/Ubuntu): `pkg-config libasound2-dev libssl-dev`
  `libxkbcommon-dev libwayland-dev libgtk-3-dev libayatana-appindicator3-dev`
  `libdbus-1-dev`. CI's `.github/workflows/ci.yml` has the authoritative list.
- If a build hits a stale-incremental linker error (`undefined hidden symbol`),
  `cargo clean` and rebuild — it's a known rustc/rust-lld incremental bug.

## Layout & docs

- `PLAN.md` — the original roadmap and architecture (authoritative reference).
- `docs/architecture.md`, `threading.md`, `auth.md`, `docking.md` — design.
- `docs/ui-reference.md` — the target visual aesthetic (overrides PLAN's UI
  shell section).
- `docs/feedback-backlog.md` — the WS1–WS9 feedback round and its status.
- `docs/questions.md` — resolved decisions + any open questions.
- Cargo workspace: `crates/{app,audio,api,auth,cache,models,state,ui}`. `app` is
  the binary (`spottyfi`); it's the only crate that knows both `audio` and `ui`.

## Working conventions (followed so far — keep them)

- **Commit straight to `main`** — no feature branches, no PRs.
- **Conventional Commits** (`feat(app): …`, `fix(ui): …`); no vague messages.
- **Quality gate before every commit / push** — `main` must stay green:
  `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings &&`
  `cargo build --workspace && cargo test --workspace`.
- `thiserror` per library crate, `anyhow` in `app`; no `unwrap`/`expect` in
  library code outside tests; `#[tracing::instrument]` on public async fns in
  `api`/`audio`; doc all public items.
- UI never blocks on I/O; state crosses the tokio↔egui boundary as `ArcSwap`
  snapshots; the UI emits intent, never mutates state directly.
- **No Nix.** Use `rustup` with the pinned `rust-toolchain.toml`.
- If git commits fail to sign (no GPG key on the new machine), disable signing
  for this repo: `git config commit.gpgsign false`.

## What's outstanding / good next work

1. **Run it and fix what's broken or ugly** — the priority. It was built
   headless; many agent reports flagged "needs the maintainer's eyes".
2. **Tab-bar gap** — the one un-done feedback item (`docs/feedback-backlog.md`
   WS2): an unwanted gap below the dock tab bar. A blind agent couldn't locate
   it — find it with the app in front of you and fix it.
3. **Live-verify the integrations** — playback, Connect device showing in
   Spotify history/scrobbles, MPRIS via `playerctl`, the tray, media keys,
   lyrics matching, the equalizer audibly working.
4. **Smoke-test packaging** — actually run `cargo appimage` and a
   `flatpak-builder` build (`docs` / `packaging/` have the commands).
5. Out-of-scope-but-maybe: real branding/icon, the tab-bar lone-leaf chrome
   (needs an `egui_dock` patch), local-file library, podcasts.

---

## The prompt to paste into Claude Code

> I'm continuing **Spottyfi**, a native Rust Spotify client (egui/eframe +
> librespot). Read `HANDOFF.md`, then `PLAN.md`, `docs/feedback-backlog.md` and
> `docs/questions.md` — they're the full context. The roadmap (Phases 0–13) and
> a feedback round (WS1–WS9) are done and on `main`; the code builds green but
> has only been lightly verified on a real desktop.
>
> First, confirm the build is green (`cargo build --workspace`, clippy, fmt,
> tests). Then I'll run the app and give you feedback — your job is to fix what
> I find. Work directly on `main` in small Conventional commits, run the full
> quality gate before each commit, and keep `main` green. Don't use Nix. When a
> task is large, feel free to delegate to subagents. Ask before guessing at
> external API behaviour.
>
> Start by reading the docs above and running the quality gate, then wait for
> my feedback.
