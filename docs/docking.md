# Docking

The centre of the window is an `egui_dock::DockArea`. Everything that is not the
top bar, the left sidebar or the bottom transport lives in the dock.

## Tab kinds

- **Page tabs** — navigable content: Home, Browse, Search, Playlist, Album,
  Artist, Genre, Liked Songs, Made For You, Audiobook. Each `Page` is a typed
  enum variant that renders into a `Tab` impl.
- **Panel tabs** — auxiliary surfaces: Now Playing Art, Queue, Lyrics, Devices,
  Friend Activity, Debug. Docked on the right by default but freely movable.

## Default layout (first launch)

```
┌────────────┬───────────────────────────┬──────────────┐
│            │  [Home] [ … page tabs … ] │ [Art][Queue] │
│  sidebar   │                           │              │
│ (not in    │       page content        │  Now Playing │
│  the dock) │                           │  Art over    │
│            │                           │  Queue       │
└────────────┴───────────────────────────┴──────────────┘
```

Left column = sidebar (outside the dock). Centre tab group = Home. Right column
= Now Playing Art over Queue.

## Persistence

The dock layout serialises to `<config_dir>/layout.ron` on shutdown and is
restored on launch. A **Reset to default** action lives in the View menu.

> Confirm the chosen `egui_dock` version derives `Serialize`/`Deserialize` on
> `DockState` — some 0.x releases did not. Tracked in `questions.md`.

## Navigation rules

- Plain click on a sidebar entry or a link **replaces** the focused tab.
- Cmd/Ctrl-click **opens in a new tab**.
- Middle-click a tab closes it.
- `Cmd+W` close, `Cmd+T` new Home tab, `Cmd+Shift+T` reopen last closed.
- Tab right-click menu: Close, Close others, Close to right, Duplicate, Pin.

The replace-vs-new-tab default is configurable. Power-user features (pinning,
per-tab history, predefined layouts) arrive in Phase 10.
