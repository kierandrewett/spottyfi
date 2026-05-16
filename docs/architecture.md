# Architecture

Spottyfi is a Cargo workspace of eight crates. The split keeps lifecycles and
runtimes separate, and keeps the UI free of business logic.

```
        ┌───────┐
        │  app  │  binary: eframe loop, dock layout, runtime, wiring
        └───┬───┘
   ┌────────┼─────────┬─────────┬────────┐
   ▼        ▼         ▼         ▼        ▼
┌──────┐ ┌──────┐ ┌──────┐ ┌───────┐ ┌──────┐
│  ui  │ │ state│ │ audio│ │  api  │ │ auth │
└──┬───┘ └──┬───┘ └──┬───┘ └───┬───┘ └──┬───┘
   │        │        │         │        │
   └────────┴────────┴────┬────┴────────┘
                          ▼
                    ┌──────────┐   ┌────────┐
                    │  models  │   │ cache  │
                    └──────────┘   └────────┘
```

## Crate responsibilities

| Crate | Owns | Depends on |
| --- | --- | --- |
| `models` | plain domain types (`Track`, `Album`, …) | nothing |
| `state` | `AppState`, the `Action`/`Event` bus, the dispatcher | `models` |
| `auth` | OAuth PKCE, token refresh, keyring storage | `models` |
| `api` | Spotify Web API client (rspotify), mockable | `models`, `cache` |
| `audio` | librespot `Session`/`Player`, playback state machine | `models`, `auth` |
| `cache` | SQLite metadata cache + on-disk image cache | `models` |
| `ui` | egui widgets, panels, theme — a pure projection of `state` | `models`, `state` |
| `app` | the binary: eframe loop, dock layout, tokio runtime, wiring | everything |

`app` is the only crate that knows about both `audio` and `ui`. `api` is built
behind a trait so the UI can be developed offline against a mock.

## The store

State mutations happen on the runtime thread only. `AppState` lives behind
`Arc<ArcSwap<AppState>>`; each `Action` handler clones the current snapshot,
mutates a builder and swaps. The interior is `Arc`-wrapped so clones are cheap.
The UI reads one snapshot per frame via `load_full()`.

The UI emits **intent** (`Action`s on a `flume` channel), never mutation. This
is the elm/redux shape: the UI is a deterministic projection of the latest
state snapshot. See [`threading.md`](threading.md) for the thread boundary and
[`docking.md`](docking.md) for the dock surface.
