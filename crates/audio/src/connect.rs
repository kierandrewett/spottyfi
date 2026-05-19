//! The Spotify Connect device integration (WS4).
//!
//! Spottyfi registers as a real Spotify Connect device so that local plays
//! land in the account's listening history and scrobble. The integration is
//! built on librespot 0.8's [`Spirc`] — the `librespot-connect` event loop
//! that maintains the Connect device's state with Spotify's servers.
//!
//! # How it relates to Spottyfi's own queue
//!
//! `Spirc` is itself a playback controller: it normally owns the queue, the
//! context and next/prev. Spottyfi already has its own authoritative queue
//! ([`crate::queue`], Phase 8), and rewriting that to hand control to `Spirc`
//! is out of scope (see `docs/questions.md`).
//!
//! Instead this integration uses `Spirc` in a deliberately narrow way:
//!
//! * `Spirc` owns the librespot [`Player`] (it must, to report state), and
//!   performs the single `Session::connect` handshake.
//! * Spottyfi's queue stays authoritative. When the queue picks the next
//!   track, the controller asks `Spirc` to *load that one track* via
//!   [`ConnectDevice::load_track`] — a one-track [`LoadRequest::from_tracks`],
//!   the same shape as the Web API's "play these URIs" call. `Spirc` then
//!   drives the `Player`, tracks play/pause/position from the player events,
//!   and reports the now-playing state to Spotify.
//! * Pause / resume / seek / volume stay as direct `Player`/mixer calls in
//!   [`crate::engine`]; `Spirc` observes the resulting [`PlayerEvent`]s and
//!   folds them into the state it reports. No command needs to be duplicated.
//!
//! The net effect: the device is visible to the Spotify account, every track
//! Spottyfi plays is reported as a play (history + scrobble), and Phase 8's
//! queue is untouched. The documented limitation is that remote control from
//! another device — transferring playback *to* Spottyfi, or driving its queue
//! from the phone app — is not wired; see `docs/questions.md`.
//!
//! [`Spirc`]: librespot::connect::Spirc
//! [`Player`]: librespot::playback::player::Player
//! [`PlayerEvent`]: librespot::playback::player::PlayerEvent

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use librespot::connect::{ConnectConfig, LoadRequest, LoadRequestOptions, Spirc};
use librespot::core::authentication::Credentials;
use librespot::core::config::DeviceType;
use librespot::core::session::Session;
use librespot::playback::mixer::Mixer;
use librespot::playback::player::Player;

use crate::error::{AudioError, AudioResult};

/// The device name Spottyfi registers under in the Spotify Connect picker.
const DEVICE_NAME: &str = "Spottyfi";

/// A running Spotify Connect device backed by librespot's [`Spirc`].
///
/// Construct one with [`ConnectDevice::start`]. Dropping it leaves the
/// background `Spirc` task running until [`ConnectDevice::shutdown`] is called
/// or the session ends; the engine calls `shutdown` from its `Drop`.
pub(crate) struct ConnectDevice {
    /// The `Spirc` control handle, behind an `Arc` so a cheap cloneable
    /// [`ConnectLoader`] can be handed to the queue's auto-advance task.
    /// Commands are buffered until the device has finished registering with
    /// Spotify's dealer, so calls made immediately after construction are not
    /// lost.
    spirc: Arc<Spirc>,
    /// Whether Spottyfi has become the *active* Connect device yet.
    ///
    /// The device registers at startup so it is visible in other devices'
    /// pickers, but it does **not** seize playback until the user actually
    /// plays something — the first [`load_track`] activates it. Shared with
    /// every [`ConnectLoader`] so the auto-advance task sees the same flag.
    activated: Arc<AtomicBool>,
}

/// A cheap, cloneable handle that loads tracks through the Connect device.
///
/// Handed to the controller's auto-advance task so it can drive playback of
/// the next queue entry without holding the whole [`ConnectDevice`].
#[derive(Clone)]
pub(crate) struct ConnectLoader {
    /// Shared `Spirc` handle; see [`ConnectDevice::spirc`].
    spirc: Arc<Spirc>,
    /// Shared activation flag; see [`ConnectDevice::activated`].
    activated: Arc<AtomicBool>,
}

impl ConnectLoader {
    /// Load and start a single track by canonical Spotify URI.
    ///
    /// See [`ConnectDevice::load_track`] — this is the same operation exposed
    /// for the background auto-advance task.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::Connect`] if the Connect device has shut down.
    pub(crate) fn load_track(&self, uri: &str, position_ms: u32) -> AudioResult<()> {
        load_track(&self.spirc, &self.activated, uri, position_ms)
    }
}

/// Issue a one-track [`LoadRequest`] to `spirc`.
///
/// On the very first call this also makes Spottyfi the *active* Connect
/// device: activation is deferred from startup to the first actual play, so
/// launching Spottyfi never yanks playback off whatever device is already
/// playing. Shared by [`ConnectDevice::load_track`] and
/// [`ConnectLoader::load_track`].
fn load_track(
    spirc: &Spirc,
    activated: &AtomicBool,
    uri: &str,
    position_ms: u32,
) -> AudioResult<()> {
    // Claim the one-time activation. `compare_exchange` lets exactly one
    // caller win the race; if `activate()` then fails we release the claim so
    // a later play retries rather than skipping activation forever.
    if activated
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        tracing::info!("activating spottyfi as the active connect device on first play");
        if let Err(err) = spirc.activate() {
            activated.store(false, Ordering::SeqCst);
            return Err(AudioError::Connect(err.to_string()));
        }
    }
    let options = LoadRequestOptions {
        start_playing: true,
        seek_to: position_ms,
        ..LoadRequestOptions::default()
    };
    let request = LoadRequest::from_tracks(vec![uri.to_owned()], options);
    spirc
        .load(request)
        .map_err(|err| AudioError::Connect(err.to_string()))
}

impl ConnectDevice {
    /// Register Spottyfi as a Connect device and start the `Spirc` event loop.
    ///
    /// This performs the **single** `Session::connect` handshake for the whole
    /// engine: [`Spirc::new`] connects the session internally, *after*
    /// registering its dealer listeners (the order librespot requires). The
    /// caller must therefore hand over a *not-yet-connected* [`Session`].
    ///
    /// `player` and `mixer` are shared with [`crate::engine::Engine`]; `Spirc`
    /// drives the player when it loads a track, and the engine continues to
    /// observe the same player-event stream.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::Connect`] if the session handshake or the Connect
    /// device registration fails (a rejected/expired token, or no network).
    #[tracing::instrument(skip_all)]
    pub(crate) async fn start(
        session: Session,
        access_token: &str,
        player: Arc<Player>,
        mixer: Arc<dyn Mixer>,
        initial_volume: u16,
    ) -> AudioResult<Self> {
        let config = ConnectConfig {
            name: DEVICE_NAME.to_owned(),
            // Spottyfi is a desktop application, so it presents as a computer
            // in the Connect picker rather than librespot's default speaker.
            device_type: DeviceType::Computer,
            is_group: false,
            initial_volume,
            // Spottyfi mirrors Spotify's volume into librespot's mixer itself;
            // leaving remote volume enabled lets another device's slider move
            // this device's mixer, which `Spirc` reflects via `VolumeChanged`.
            disable_volume: false,
            ..ConnectConfig::default()
        };

        let credentials = Credentials::with_access_token(access_token);
        let (spirc, spirc_task) = Spirc::new(config, session, credentials, player, mixer)
            .await
            .map_err(|err| AudioError::Connect(err.to_string()))?;
        let spirc = Arc::new(spirc);

        // The `Spirc` event loop must run for the device to stay registered
        // and to keep reporting state. It ends when `shutdown` is called or
        // the session drops.
        tokio::spawn(async move {
            spirc_task.await;
            tracing::debug!("spirc task ended; connect device deregistered");
        });

        // Deliberately *not* activated here: the device registers so it is
        // visible to the account, but stays inactive so launching Spottyfi
        // does not seize playback from another device. The first `load_track`
        // activates it (a `Load` is ignored by `Spirc` while inactive).
        tracing::info!(
            device = DEVICE_NAME,
            "registered as a spotify connect device (inactive until first play)"
        );
        Ok(Self {
            spirc,
            activated: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Load a single track or episode by canonical Spotify URI through `Spirc`.
    ///
    /// This is how Spottyfi's queue drives playback: the queue decides *which*
    /// track, and this hands that one URI to `Spirc` as a one-track context.
    /// `Spirc` loads it into the player, starts playback, and reports the
    /// now-playing state to Spotify — which is what makes the play land in the
    /// account's listening history and scrobble.
    ///
    /// `position_ms` seeks within the track on load (normally `0`).
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::Connect`] if the command channel to the `Spirc`
    /// task is closed (the device has shut down).
    pub(crate) fn load_track(&self, uri: &str, position_ms: u32) -> AudioResult<()> {
        load_track(&self.spirc, &self.activated, uri, position_ms)
    }

    /// A cheap cloneable handle for loading tracks from a background task.
    pub(crate) fn loader(&self) -> ConnectLoader {
        ConnectLoader {
            spirc: Arc::clone(&self.spirc),
            activated: Arc::clone(&self.activated),
        }
    }

    /// Shut the Connect device down: pause playback, deregister from Spotify
    /// and end the `Spirc` event loop.
    pub(crate) fn shutdown(&self) {
        if let Err(err) = self.spirc.shutdown() {
            tracing::warn!(%err, "failed to shut down connect device");
        }
    }
}
