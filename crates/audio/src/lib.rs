//! Audio engine: a wrapper around `librespot` and the playback state machine.
//!
//! Owns the librespot `Session` and `Player`, exposes an async
//! `PlaybackController`, and emits playback ticks back to the app. See
//! `docs/threading.md`.
//!
//! Phase 0: placeholder. The engine arrives in Phase 2.
#![warn(missing_docs)]
