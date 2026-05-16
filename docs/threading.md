# Threading model

Two worlds: the egui UI thread and the tokio runtime. They never share mutable
state directly — only snapshots and messages cross the boundary.

## UI thread

- Owns the `egui::Context`; runs at vsync.
- Reads state via `arc-swap` snapshots and read-only `parking_lot::RwLock` reads.
- **Never blocks on I/O.** No `block_on`, no synchronous file/network calls.
- Dispatches user actions onto an unbounded `flume::Sender<Action>`.

If UI code reaches for `block_on`, the crate boundary is drawn in the wrong
place — move the work behind an `Action`.

## Runtime thread(s)

One `tokio` multi-thread runtime, created and owned by `app`. It owns:

- the librespot `Session` + `Player` + Connect device;
- the `api` client;
- the **action consumer** — a single task that pulls `Action`s and dispatches
  them to handlers;
- the **event publisher** — broadcasts `Event`s back toward the UI.

State is mutated only here, by `Action` handlers.

## The bridge

```
  UI thread                         runtime thread
  ─────────                         ──────────────
  emit Action  ──flume::Sender──▶   action consumer
                                         │
                                    handler mutates
                                    AppState (ArcSwap swap)
                                         │
  request_repaint() ◀──Event───────  event publisher
       │
  next frame reads load_full()
```

When an `Event` lands, the runtime calls `egui::Context::request_repaint()` so
the UI wakes and re-reads the latest snapshot.

## Async patterns in the UI

1. **`Promise<T>`** — wrap a `JoinHandle` for one-shot fetches (loading a
   playlist page). Store it on the page state; draw a spinner while pending.
2. **Broadcast subscription** — for continuous streams (playback ticks ~10Hz,
   queue changes). Update the store; the UI redraws on repaint.
