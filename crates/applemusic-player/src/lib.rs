//! Apple Music playback for Spottyfi, via an embedded MusicKit web player.
//!
//! Apple Music audio is FairPlay-DRM protected: no native code can decode it.
//! The sanctioned route — the one the Cider client uses — is Apple's official
//! [MusicKit JS](https://js-cdn.music.apple.com/musickit/v3/docs/) player
//! running inside a real browser engine, which handles the DRM itself.
//!
//! This crate is the Spottyfi side of that:
//!
//! * [`musickit`] — builds the MusicKit JS control protocol (pure, tested).
//! * [`WebEngine`] — the seam to the browser that runs it.
//! * [`AppleMusicBackend`] — a [`PlaybackBackend`](spottyfi_audio::PlaybackBackend)
//!   so the transport drives Apple Music exactly like every other backend.
//!
//! # The CEF runtime
//!
//! The production [`WebEngine`] is an off-screen Chromium (CEF) with the
//! Widevine CDM — the only engine that satisfies Apple Music's EME/DRM. CEF is
//! a large, separately-provisioned dependency (its own ~1 GB binary
//! distribution, Widevine provisioning, a multi-process model), so it is kept
//! **behind the [`WebEngine`] trait** rather than wired into the default
//! build: dropping in a CEF engine is one trait implementation, and until
//! then [`LoggingWebEngine`] keeps everything else building and running.
#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

mod backend;
mod engine;
pub mod musickit;

pub use backend::{AppleMusicBackend, AppleMusicState};
pub use engine::{LoggingWebEngine, WebEngine};
