//! Audio engine: a wrapper around `librespot` and the playback state machine.
//!
//! This crate owns the librespot [`Session`] and [`Player`], and exposes an
//! async [`PlaybackController`] that the `app` crate drives. Playback
//! observations flow back through a shared [`PlaybackState`] snapshot, swapped
//! ~10× per second so the UI's transport bar animates smoothly without ever
//! touching librespot. See `docs/threading.md`.
//!
//! # Authentication
//!
//! librespot authenticates directly from the OAuth access token minted by
//! Spottyfi's PKCE flow, via `Credentials::with_access_token`. See
//! `docs/questions.md` #1 for the confirmed details.
//!
//! # Audio backend
//!
//! The output backend is selected at compile time by a Cargo feature; the
//! default is librespot's `rodio` backend (`rodio-backend`), which goes
//! through ALSA on Linux.
//!
//! [`Session`]: librespot::core::session::Session
//! [`Player`]: librespot::playback::player::Player
#![warn(missing_docs)]
// `unwrap`/`expect` are denied in library code but allowed in unit tests,
// per the workspace convention in `PLAN.md`.
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod controller;
mod engine;
mod error;
mod queue;
mod state;

pub use crate::controller::{PlaybackController, SharedPlaybackState, SharedQueueState};
pub use crate::error::{AudioError, AudioResult};
pub use crate::queue::{Queue, QueueState, QueueTrack, RepeatMode};
pub use crate::state::{normalise_uri, parse_playable, PlaybackState, TrackInfo};
