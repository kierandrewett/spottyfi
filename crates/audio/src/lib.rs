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
//! through ALSA on Linux. Spottyfi wraps that stock sink in its own
//! [`TappedSink`](crate::sink::TappedSink), which applies the 10-band
//! [`Equalizer`](crate::dsp::Equalizer) DSP and feeds the post-EQ PCM into the
//! [`AudioTap`] the UI reads for waveform/visualisations.
//!
//! [`Session`]: librespot::core::session::Session
//! [`Player`]: librespot::playback::player::Player
#![warn(missing_docs)]
// `unwrap`/`expect` are denied in library code but allowed in unit tests,
// per the workspace convention in `PLAN.md`.
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod config;
mod connect;
mod controller;
mod cpal_sink;
mod dsp;
mod engine;
mod error;
mod http_player;
mod playback;
mod queue;
mod sink;
mod spectrum;
mod spotify_backend;
mod state;
mod tap;
mod waveform;

pub use crate::config::{EngineConfig, StreamQuality};
pub use crate::controller::{PlaybackController, SharedPlaybackState, SharedQueueState};
pub use crate::dsp::BAND_COUNT as EQ_BAND_COUNT;
pub use crate::error::{AudioError, AudioResult};
pub use crate::http_player::HttpAudioPlayer;
pub use crate::playback::PlaybackBackend;
pub use crate::queue::{Queue, QueueState, QueueTrack, RepeatMode};
pub use crate::spectrum::{SpectrumAnalyzer, SpectrumSnapshot, BAND_COUNT as SPECTRUM_BAND_COUNT};
pub use crate::spotify_backend::SpotifyBackend;
pub use crate::state::{normalise_uri, parse_playable, PlaybackState, TrackInfo};
pub use crate::tap::{AudioTap, TapSnapshot, WaveformPoint, SPECTRUM_LEN, WAVEFORM_LEN};
pub use crate::waveform::{TrackWaveform, WaveformAnalyzer, WAVEFORM_RESOLUTION};
