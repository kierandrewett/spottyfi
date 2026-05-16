# UI reference — the target aesthetic

Spottyfi's visual target is the **leaked internal Spotify "imgui" tool**. The
maintainer provided a reference screenshot of it; this document is the
authoritative description of the look. Where it conflicts with the older "UI
shell" section of `PLAN.md`, **this document wins**.

> The reference screenshot is not committed to the repo (it is a leaked Spotify
> internal asset). It lives in the maintainer's local Claude image cache and is
> handed to the agent doing UI work at run time.

## Overall aesthetic

- **Dark and near-black.** Background ~#0c0c0c–#121212. Panels barely lighter.
- **Flat.** No gradients, no drop shadows. The only depth is a faint, heavily
  blurred album-art image bleeding through *behind* the sidebar.
- **Sharp.** **Border radius is 0** — or as close as egui allows — everywhere:
  tabs, buttons, selection highlights, table, scrollbars. This is a Dear ImGui
  application; it has no rounded corners. (The maintainer explicitly called out
  "weird border radii" in the first shell — kill all rounding.)
- **Dense.** Tight row heights, small consistent padding, many rows visible at
  once. Information density beats whitespace.
- **Aligned.** Everything sits on a tight grid; column edges, icon baselines and
  text baselines line up.

## Layout regions (top to bottom)

### 1. Menu bar
A classic application menu bar across the very top: `File  View  Playback
Tools  Help`. Thin, flat. This **replaces** the Phase 4 top bar (which had a
search box + profile menu). Account/profile actions move into a menu; search
becomes a sidebar entry and its own page.

### 2. Tab bar
Directly under the menu bar, spanning the content area (right of the sidebar).
Flat, sharp-cornered tabs. Tab titles are **plain, human names** — `Home`,
the playlist/album/artist name, `Now Playing`, `Queue` — **not** breadcrumb
paths like `spotify/home/`. A small leading icon at the bar's left edge; a
close `✕` per tab. The active tab is a touch lighter; inactive tabs blend into
the bar. Minimal chrome.

### 3. Tree sidebar
The headline change. A **collapsible tree**, not a flat list:

- **Section headers** — uppercase, slightly dimmed, bold, each with a caret
  (`▼`/`▶`) and a thin separator rule beneath. Sections seen: `MAIN`,
  `YOUR LIBRARY`, `PLAYLISTS`. The `PLAYLISTS` header carries a `+` action on
  its right edge.
- **Items** — `MAIN`: Home, Search, Browse, Charts, New Releases, Discover,
  Podcasts. `YOUR LIBRARY`: Made For You, Recently Played, Liked Songs, Your
  Podcasts, Your Albums, Your Artists, Local Files. `PLAYLISTS`: the user's
  playlists (some with a trailing download/offline glyph).
- Each item: a small monochrome **line icon** + label, tight single-line row.
- Hover/selection: a flat, full-width, slightly-lighter highlight bar — sharp
  corners, no inset, no rounding.
- Resizable width; collapsible. Playlist folders should nest as tree children.

### 4. Content area
Per-tab. For a playlist/album page:
- **Hero**: large cover art on the left; to its right a small kicker
  (`PLAYLIST`), a very large title, a description line, then a green **circular
  play/pause** button and a heart (save) button.
- **Track table**: uppercase, dimmed column headers (`TITLE`, `ARTIST`,
  `ALBUM`, date, duration) with **sort carets** and thin vertical column
  separators; a faint header-row background. Each row: a music-note glyph, a
  tiny album thumbnail, then the cell text — dense, single-line.
- **Currently-playing row**: rendered in **Spotify green** with a speaker icon.

### 5. Transport bar
Bottom strip, near-black:
- **Left**: small album-art thumbnail, track title + artist (two lines),
  bitrate in dimmed small text below.
- **Centre**: shuffle / previous / play-pause / next / repeat, over a scrubber
  (the reference shows an audio **waveform** — a plain progress bar is an
  acceptable first cut; waveform is later polish), flanked by elapsed / total.
- **Right**: queue, devices/Connect, settings, and a volume slider.

## Iconography
Monochrome **line icons**, rendered as real **SVGs** (egui_extras + resvg) — no
Unicode-glyph fallbacks. Use one consistent open-source line-icon set (Lucide or
Tabler — permissively licensed). Size and tint icons from the theme.

## Style tokens to enforce
- Corner radius: `0` everywhere.
- Selection/hover highlight: flat, full-bleed, ~8–12% lighter than the surface.
- Spacing: small and uniform; favour density.
- Accent: Spotify green `#1ed760` — used for the play button and the
  now-playing row only, not as a general highlight.
- Text: white primary, ~`#b3b3b3` secondary, dimmer still for section headers.
