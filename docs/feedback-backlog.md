# Feedback backlog

A large round of maintainer feedback after running the app (post-Phase-10).
Grouped into workstreams; each workstream is roughly one focused change set.
Worked through sequentially (everything touches the `app` crate, so agents run
one at a time). `[ ]` todo · `[~]` in progress · `[x]` done.

## WS1 — Rendering & scroll quality `[ ]`
- [ ] Disable smooth scrolling
- [ ] Fix "glitching" size issues (layout jitter / resizing glitches)
- [ ] Fix scrolling generally
- [ ] Smoother text rendering — currently jagged (check `pixels_per_point` /
      display scaling / text AA)
- [ ] Smoother image rendering — currently "crispy" (linear texture filtering,
      not nearest)

## WS2 — Transport: custom controls & layout `[ ]`
- [ ] Custom Spotify-style progress bar component — hover-scrub, drag; reuse the
      same component for the volume control
- [ ] Bigger, rounded play/pause button
- [ ] Properly centre the seek bar + control cluster (still off)
- [ ] Fix the gap below the tab bar / align the tab bar

## WS3 — Audio engine: playback feel `[ ]`
- [ ] Fix pause latency — press-to-pause has a noticeable delay
- [ ] Fade audio in/out on play/pause
- [ ] Volume: instant (currently ~1s lag) and logarithmic
- [ ] Real bitrate/codec readout in the transport (replace hardcoded
      "Ogg Vorbis xxx")
- [ ] Shuffle support
- [ ] Repeat: off / repeat-all / repeat-one

## WS4 — Spotify Connect device `[ ]`
- [ ] Register Spottyfi as a Spotify Connect device so plays land in Spotify
      listening history / scrobble. (Significant — librespot-connect / spirc;
      Phase 2 deliberately deferred this.)

## WS5 — Settings & account UI `[ ]`
- [ ] Proper Settings page — audio settings, local files, equalizer, and other
      power-user options
- [ ] Equalizer (real DSP — needs a custom audio backend tapping PCM; large)
- [ ] User avatar + name in the **top-right**; that's the entry point for user
      info, Settings, Log out

## WS6 — Library, tables & navigation UX `[ ]`
- [ ] Sidebar items open in the main pane by default
- [ ] Playlist sidebar icons use the real playlist image
- [ ] Cache playlist contents so a playlist doesn't reload on every visit
- [ ] Album duration shown
- [ ] Fix the "Date added" column (currently empty — `api` drops `added_at`)
- [ ] Hover an artist name in a table → jump to artist; same for albums
- [ ] Make tables look nicer / better
- [ ] General docking-UX cleanup — some interactions are confusing and don't
      make sense with the dock model

## WS7 — Waveform & visualisations `[ ]`
- [ ] Live waveform scrubber like the internal Spotify client
- [ ] Live audio visualisations
- (Both need a custom librespot backend that taps the PCM sample stream.)

## Notes on the big ones
- **Connect device, equalizer, waveform/visualisations** are each substantial
  features, not tweaks — equalizer and visualisations both require a custom
  librespot audio backend that intercepts PCM samples before output.
