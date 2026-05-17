# Feedback backlog

A large round of maintainer feedback after running the app (post-Phase-10).
Grouped into workstreams; each workstream is roughly one focused change set.
Worked through sequentially (everything touches the `app` crate, so agents run
one at a time). `[ ]` todo · `[~]` in progress · `[x]` done.

## WS1 — Rendering & scroll quality `[x]`
- [x] Disable smooth scrolling
- [x] Fix "glitching" size issues (layout jitter / resizing glitches)
- [x] Fix scrolling generally
- [x] Smoother text rendering — currently jagged (check `pixels_per_point` /
      display scaling / text AA)
- [x] Smoother image rendering — currently "crispy" (linear texture filtering,
      not nearest)

## WS2 — Transport: custom controls & layout `[~]`
- [x] Custom Spotify-style progress bar component — hover-scrub, drag; reuse the
      same component for the volume control
- [x] Bigger, rounded play/pause button
- [x] Properly centre the seek bar + control cluster
- [ ] Fix the gap below the tab bar / align the tab bar — *needs a maintainer
      screenshot to pin down; a blind agent couldn't locate it precisely*

## WS3 — Audio engine: playback feel `[x]`
- [x] Fix pause latency — press-to-pause has a noticeable delay
- [x] Fade audio in/out on play/pause
- [x] Volume: instant (currently ~1s lag) and logarithmic
- [x] Real bitrate/codec readout in the transport (replace hardcoded
      "Ogg Vorbis xxx")
- [x] Shuffle support
- [x] Repeat: off / repeat-all / repeat-one

## WS4 — Spotify Connect device `[ ]`
- [ ] Register Spottyfi as a Spotify Connect device so plays land in Spotify
      listening history / scrobble. (Significant — librespot-connect / spirc;
      Phase 2 deliberately deferred this.)

## WS5 — Settings & account UI `[~]`
- [x] Proper Settings page — audio settings, local files, equalizer, and other
      power-user options
- [x] Equalizer (real DSP — needs a custom audio backend tapping PCM; large)
- [x] User avatar + name in the **top-right**; that's the entry point for user
      info, Settings, Log out

## WS6 — Library, tables & navigation UX `[x]`
- [x] Sidebar items open in the main pane by default
- [x] Playlist sidebar icons use the real playlist image
- [x] Cache playlist contents so a playlist doesn't reload on every visit
- [x] Album duration shown
- [x] Fix the "Date added" column (currently empty — `api` drops `added_at`)
- [x] Hover an artist name in a table → jump to artist; same for albums
- [x] Make tables look nicer / better
- [x] General docking-UX cleanup — some interactions are confusing and don't
      make sense with the dock model

## WS7 — Waveform & visualisations `[x]`
- [x] Live waveform scrubber like the internal Spotify client
- [x] Live audio visualisations
- (Both need a custom librespot backend that taps the PCM sample stream.)

## Notes on the big ones
- **Connect device, equalizer, waveform/visualisations** are each substantial
  features, not tweaks — equalizer and visualisations both require a custom
  librespot audio backend that intercepts PCM samples before output.
