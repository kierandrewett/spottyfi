//! The librespot-backed playback engine.
//!
//! [`Engine`] owns the librespot [`Session`], [`Player`] and software mixer.
//! It runs two background tasks on the tokio runtime:
//!
//! * the **event loop** — consumes [`PlayerEvent`]s and updates the shared
//!   [`PlaybackState`] (track changes, play/pause, buffering, volume);
//! * the **position poller** — wakes ~10× per second while playing and swaps a
//!   fresh state snapshot so the UI's progress scrubber animates smoothly.
//!
//! All of this runs on the runtime thread; the UI only ever reads the
//! [`ArcSwap`] snapshot. See `docs/threading.md`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use librespot::core::config::SessionConfig;
use librespot::core::session::Session;
use librespot::core::{FileId, SpotifyId};
use librespot::metadata::audio::{AudioFileFormat, AudioItem, UniqueFields};
use librespot::playback::audio_backend::Sink;
use librespot::playback::config::{Bitrate, PlayerConfig, VolumeCtrl};
use librespot::playback::mixer::softmixer::SoftMixer;
use librespot::playback::mixer::{Mixer, MixerConfig, NoOpVolume};
use librespot::playback::player::{Player, PlayerEvent};
use tokio::runtime::Handle;

use crate::config::EngineConfig;
use crate::connect::ConnectDevice;
use crate::error::{AudioError, AudioResult};
use crate::sink::{EqParams, SharedEqParams, TappedSink};
use crate::state::{PlaybackState, TrackInfo};
use crate::tap::AudioTap;
use crate::waveform::WaveformAnalyzer;

/// How often librespot is asked to emit a `PositionChanged` event.
///
/// These coarse 10Hz events re-anchor the locally-interpolated position; the
/// poller below ticks much faster and interpolates between them.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How often the position poller refreshes the published snapshot.
///
/// ~30Hz. The poller interpolates the play head from the last anchor, so a
/// fast tick is what makes the transport scrubber glide smoothly instead of
/// jumping in 100ms steps — the coarse rate was the visible playback "jitter".
const POSITION_POLL_INTERVAL: Duration = Duration::from_millis(33);

/// librespot's full-scale mixer volume (`u16::MAX`).
const MAX_VOLUME: u16 = u16::MAX;

/// The codec librespot decodes — Spotify streams Ogg Vorbis at every tier.
pub(crate) const CODEC_NAME: &str = "Ogg Vorbis";

/// How long after a pause is requested librespot's decoder is actually
/// stopped.
///
/// The output gain is dropped to silence the instant pause is pressed and
/// glides down inside the audio callback; this short grace period lets that
/// ramp finish so the decoder stop is inaudible.
const PAUSE_RAMP: Duration = Duration::from_millis(20);

/// The perceptual loudness exponent applied to the `0.0..=1.0` UI volume.
///
/// The human ear's loudness response is roughly logarithmic, so a linear
/// fader feels coarse at the quiet end and barely audible across its top
/// half. Raising the fraction to this power expands the quiet end and
/// compresses the loud end, giving an even *perceived* sweep — the same
/// "ideal" perceptual law librespot's own `Log` volume curve targets, but
/// applied here so the mapping is explicit and unit-tested. The mixer is
/// therefore driven with a plain `Linear` control to avoid mapping twice.
const VOLUME_CURVE_EXP: f32 = 3.0;

/// Map a `0.0..=1.0` UI volume through the perceptual curve.
///
/// Quiet values are spread over more of the fader's travel; `0.0` and `1.0`
/// map exactly to silence and full scale.
fn perceptual_volume(fraction: f32) -> f32 {
    fraction.clamp(0.0, 1.0).powf(VOLUME_CURVE_EXP)
}

/// Convert a `0.0..=1.0` UI volume into librespot's `u16` mixer scale,
/// applying the perceptual curve on the way.
fn volume_to_u16(volume: f32) -> u16 {
    (perceptual_volume(volume) * f32::from(MAX_VOLUME)).round() as u16
}

/// Convert librespot's `u16` mixer volume back into a `0.0..=1.0` UI fraction,
/// inverting the perceptual curve so a round trip is stable.
fn volume_from_u16(volume: u16) -> f32 {
    let mapped = f32::from(volume) / f32::from(MAX_VOLUME);
    mapped.clamp(0.0, 1.0).powf(1.0 / VOLUME_CURVE_EXP)
}

/// The kilobits-per-second figure for a librespot [`Bitrate`] tier.
fn bitrate_kbps(bitrate: Bitrate) -> u16 {
    match bitrate {
        Bitrate::Bitrate96 => 96,
        Bitrate::Bitrate160 => 160,
        Bitrate::Bitrate320 => 320,
    }
}

/// Map a Spottyfi [`StreamQuality`] to the librespot [`Bitrate`] tier.
fn bitrate_for(quality: crate::config::StreamQuality) -> Bitrate {
    match quality {
        crate::config::StreamQuality::Low => Bitrate::Bitrate96,
        crate::config::StreamQuality::Normal => Bitrate::Bitrate160,
        crate::config::StreamQuality::High => Bitrate::Bitrate320,
    }
}

/// The running librespot engine: session, player, mixer and shared state.
pub(crate) struct Engine {
    /// The librespot player. Cheap to clone (`Arc` inside).
    player: Arc<Player>,
    /// The software mixer. librespot itself applies no volume (`NoOpVolume`)
    /// — all gain is the cpal callback's — so this exists purely as the
    /// volume state `Spirc` reads and writes for the Connect device.
    mixer: Arc<dyn Mixer>,
    /// The Spotify Connect device. `Spirc` owns the player for the purpose of
    /// loading tracks and reporting state; track loads route through it so
    /// plays land in Spotify's listening history. See [`crate::connect`].
    connect: ConnectDevice,
    /// The published playback snapshot, read by the UI each frame.
    state: Arc<ArcSwap<PlaybackState>>,
    /// The real-time control surface shared with [`CpalSink`](crate::cpal_sink):
    /// output gain, the pause flag and the seek-flush flag. The engine writes
    /// the gain and flush; the audio callback acts on them within a few ms.
    controls: crate::cpal_sink::SinkControls,
    /// The user's chosen volume as a `0.0..=1.0` fraction. The actual output
    /// gain is `perceptual_volume(base_volume) * crossfade_gain`; keeping the
    /// fraction separate means the UI slider never twitches during a fade.
    base_volume: Arc<AtomicU32>,
    /// The current crossfade level (`f32` bits, `0.0..=1.0`), `1.0` outside a
    /// track transition. The poller writes it; it folds into the output gain.
    crossfade_gain: Arc<AtomicU32>,
    /// `false` while paused — the engine then leaves the output gain at
    /// silence and the poller's crossfade ramp does not touch it.
    output_active: Arc<AtomicBool>,
    /// Track-transition crossfade duration in seconds (`f32` bits), `0.0`
    /// when disabled. Read every poller tick to fade a track's head in and
    /// its tail out; live-adjustable via [`Engine::set_crossfade`].
    crossfade_secs: Arc<AtomicU32>,
    /// Live equaliser parameters, shared with every [`TappedSink`] the player
    /// builds. The controller swaps fresh params in; the sink picks them up on
    /// its next decoded packet.
    eq_params: SharedEqParams,
    /// The post-EQ sample tap the UI reads for visualisations (WS7b).
    tap: AudioTap,
    /// The background full-song waveform analyser. Triggered on every track
    /// change; the UI reads its published envelope for the seek bar.
    waveform: WaveformAnalyzer,
    /// A session handle kept solely so the waveform analyser can open its own
    /// [`AudioFile`](librespot::audio::AudioFile) for an independent decode.
    analysis_session: Session,
}

impl Engine {
    /// Connect a librespot session with the given OAuth access token and build
    /// the player and mixer.
    ///
    /// The access token is the one minted by Spottyfi's PKCE flow; librespot
    /// accepts it directly via [`Credentials::with_access_token`] (see
    /// `docs/questions.md` #1).
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::Connect`] if the session handshake fails (a
    /// rejected/expired token, or no network), or [`AudioError::NoBackend`] if
    /// no audio output backend is available.
    pub(crate) async fn connect(
        access_token: &str,
        config: EngineConfig,
        state: Arc<ArcSwap<PlaybackState>>,
    ) -> AudioResult<Self> {
        let session_config = SessionConfig::default();
        let bitrate = bitrate_for(config.quality);
        let player_config = PlayerConfig {
            // Opt into periodic `PositionChanged` events; combined with the
            // poller this keeps the published position accurate.
            position_update_interval: Some(POLL_INTERVAL),
            // The user-chosen tier; surfaced verbatim in the transport readout.
            bitrate,
            // librespot's volume normalisation, toggled from Settings.
            normalisation: config.normalisation,
            ..PlayerConfig::default()
        };

        // Build the session but do **not** connect it here: the Spotify
        // Connect device (`Spirc`) performs the single `Session::connect`
        // handshake itself, after registering its dealer listeners — the
        // order librespot 0.8 requires. See `crate::connect`.
        let session = Session::new(session_config, None);

        // Drive the mixer with a plain linear control: Spottyfi applies its own
        // perceptual curve in `volume_to_u16`, so the mixer must not map again.
        let mixer_config = MixerConfig {
            volume_ctrl: VolumeCtrl::Linear,
            ..MixerConfig::default()
        };
        let mixer = SoftMixer::open(mixer_config)
            .map(|m| Arc::new(m) as Arc<dyn Mixer>)
            .map_err(|err| AudioError::Connect(err.to_string()))?;

        let eq_params: SharedEqParams = Arc::new(ArcSwap::from_pointee(EqParams::default()));
        let tap = AudioTap::new();
        let sink_params = Arc::clone(&eq_params);
        let sink_tap = tap.clone();

        // The real-time control surface shared with the cpal sink. Its initial
        // gain is the mixer's starting volume run through the perceptual curve.
        let initial_volume = volume_from_u16(mixer.volume());
        let controls = crate::cpal_sink::SinkControls::new(perceptual_volume(initial_volume));
        let sink_controls = controls.clone();

        // `Spirc` needs its own handle to the same session; `Session` is an
        // `Arc` inside, so the clone is cheap and shares one connection. A
        // further clone is kept for the background waveform analyser, which
        // opens its own audio-file streams independently of playback.
        let connect_session = session.clone();
        let analysis_session = connect_session.clone();
        // Spottyfi's own `cpal` output sink — low, fixed latency and instant
        // pause — wrapped per-stream by `TappedSink` so the EQ DSP and the UI
        // sample tap sit between librespot and the real output. The sink is
        // built on librespot's player thread (`cpal::Stream` is `!Send`); a
        // device-open failure degrades to a silent `NullSink` rather than
        // panicking, so login still succeeds. librespot itself applies no
        // volume (`NoOpVolume`) — all gain is the cpal callback's, so a
        // volume change is heard within one device buffer rather than behind
        // the ring.
        let player = Player::new(player_config, session, Box::new(NoOpVolume), move || {
            let inner: Box<dyn Sink> = match crate::cpal_sink::CpalSink::open(sink_controls.clone())
            {
                Ok(sink) => Box::new(sink),
                Err(err) => {
                    tracing::error!(%err, "audio output device unavailable; playing silently");
                    Box::new(crate::cpal_sink::NullSink)
                }
            };
            Box::new(TappedSink::new(inner, Arc::clone(&sink_params), &sink_tap))
        });

        // Register the Spotify Connect device. This connects the session and
        // becomes the active device, so track loads route through it and
        // plays land in Spotify's listening history. `Spirc` shares the
        // player and mixer the engine just built.
        let connect = ConnectDevice::start(
            connect_session,
            access_token,
            Arc::clone(&player),
            Arc::clone(&mixer),
            mixer.volume(),
        )
        .await?;
        tracing::info!("librespot session connected via spotify connect");

        // Publish the mixer's starting volume so the UI's slider is correct.
        let initial = PlaybackState {
            volume: initial_volume,
            bitrate_kbps: bitrate_kbps(bitrate),
            codec: CODEC_NAME.to_owned(),
            ..PlaybackState::default()
        };
        state.store(Arc::new(initial));

        let engine = Self {
            player,
            mixer,
            connect,
            state,
            controls,
            base_volume: Arc::new(AtomicU32::new(initial_volume.to_bits())),
            crossfade_gain: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            output_active: Arc::new(AtomicBool::new(true)),
            crossfade_secs: Arc::new(AtomicU32::new(0.0f32.to_bits())),
            eq_params,
            tap,
            waveform: WaveformAnalyzer::new(),
            analysis_session,
        };
        engine.spawn_event_loop();
        Ok(engine)
    }

    /// The librespot player handle.
    pub(crate) fn player(&self) -> Arc<Player> {
        Arc::clone(&self.player)
    }

    /// Load and start playing a single track by canonical Spotify URI.
    ///
    /// The load is routed through the Spotify Connect device (`Spirc`) rather
    /// than calling `Player::load` directly: `Spirc` then drives the player,
    /// reports the now-playing state to Spotify and the play lands in the
    /// account's listening history. See [`crate::connect`].
    ///
    /// `position_ms` seeks within the track on load (normally `0`).
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::Connect`] if the Connect device has shut down.
    pub(crate) fn load(&self, uri: &str, position_ms: u32) -> AudioResult<()> {
        self.connect.load_track(uri, position_ms)
    }

    /// A cheap cloneable handle for loading tracks through the Connect device
    /// from a background task (the queue auto-advance loop).
    pub(crate) fn connect_loader(&self) -> crate::connect::ConnectLoader {
        self.connect.loader()
    }

    /// The post-EQ sample tap the UI reads for visualisations (WS7b).
    pub(crate) fn tap(&self) -> AudioTap {
        self.tap.clone()
    }

    /// The background full-song waveform analyser the seek bar reads.
    pub(crate) fn waveform(&self) -> WaveformAnalyzer {
        self.waveform.clone()
    }

    /// Update the equaliser configuration.
    ///
    /// Publishes fresh [`EqParams`] into the shared slot; the active
    /// [`TappedSink`] picks them up on its next decoded packet (single-digit
    /// milliseconds). When `enabled` is `false` the EQ is a true bypass.
    pub(crate) fn set_equalizer(
        &self,
        enabled: bool,
        band_gains_db: [f32; crate::dsp::BAND_COUNT],
    ) {
        self.eq_params.store(Arc::new(EqParams {
            enabled,
            band_gains_db,
        }));
    }

    /// Set the output volume from a `0.0..=1.0` fraction.
    ///
    /// Applies immediately: the audio callback reads the new gain and glides
    /// to it within one device buffer (a few ms), the short ramp keeping the
    /// change click-free. The `SoftMixer` is also updated so the Connect
    /// device reports a consistent volume to Spotify.
    pub(crate) fn set_volume(&self, volume: f32) {
        let clamped = volume.clamp(0.0, 1.0);
        self.base_volume.store(clamped.to_bits(), Ordering::SeqCst);
        self.mixer.set_volume(volume_to_u16(clamped));
        self.refresh_gain();
        // The mixer does not emit an event; publish the change ourselves.
        publish_with(&self.state, |s| s.volume = clamped);
    }

    /// Recompute and publish the output gain from the base volume and the
    /// current crossfade level. A no-op while paused — pause/resume own the
    /// gain then.
    fn refresh_gain(&self) {
        refresh_output_gain(
            &self.output_active,
            &self.base_volume,
            &self.crossfade_gain,
            &self.controls.gain,
        );
    }

    /// Set the track-transition crossfade duration, in seconds (`0.0` disables
    /// it). Applies live — the position poller picks the new value up on its
    /// next tick.
    ///
    /// Disabling crossfade resets the crossfade level to full in case a tail
    /// fade-out had left it attenuated.
    pub(crate) fn set_crossfade(&self, seconds: f32) {
        let seconds = seconds.max(0.0);
        self.crossfade_secs
            .store(seconds.to_bits(), Ordering::SeqCst);
        if seconds <= 0.0 {
            self.crossfade_gain
                .store(1.0f32.to_bits(), Ordering::SeqCst);
            self.refresh_gain();
        }
    }

    /// Pause playback.
    ///
    /// The output gain is dropped to silence immediately — the audio callback
    /// glides there in a few ms — and librespot's decoder is stopped a short
    /// [`PAUSE_RAMP`] later, once that ramp has finished, so the pause is
    /// instant *and* click-free. The sink then freezes its ring buffer, so a
    /// resume replays the buffered audio with no refill gap.
    pub(crate) fn pause(&self) {
        self.output_active.store(false, Ordering::SeqCst);
        self.controls.gain.store(0.0f32.to_bits(), Ordering::SeqCst);
        let player = Arc::clone(&self.player);
        tokio::spawn(async move {
            tokio::time::sleep(PAUSE_RAMP).await;
            player.pause();
        });
    }

    /// Resume playback.
    ///
    /// Restarts librespot's decoder (the sink unfreezes its still-full ring,
    /// so audio is immediate) and ramps the output gain back up from silence.
    pub(crate) fn resume(&self) {
        self.output_active.store(true, Ordering::SeqCst);
        self.player.play();
        self.refresh_gain();
    }

    /// Seek within the current track.
    ///
    /// Raises the sink's flush flag so the ~150ms of buffered pre-seek audio
    /// is dropped rather than played before the seek lands.
    pub(crate) fn seek(&self, position_ms: u32) {
        self.controls.flush.store(true, Ordering::SeqCst);
        self.player.seek(position_ms);
    }

    /// Spawn the player-event loop and the position poller.
    fn spawn_event_loop(&self) {
        let mut events = self.player.get_player_event_channel();
        let state = Arc::clone(&self.state);
        let player = Arc::clone(&self.player);
        let waveform = self.waveform.clone();
        let session = self.analysis_session.clone();
        let mixer = Arc::clone(&self.mixer);
        let base_volume = Arc::clone(&self.base_volume);
        let crossfade_gain = Arc::clone(&self.crossfade_gain);
        let crossfade_secs = Arc::clone(&self.crossfade_secs);
        let output_active = Arc::clone(&self.output_active);
        let output_gain = Arc::clone(&self.controls.gain);

        tokio::spawn(async move {
            // The poller ticks continuously; it only mutates the snapshot
            // while a track is actually playing.
            let mut poll = tokio::time::interval(POSITION_POLL_INTERVAL);
            // A locally tracked position, advanced between librespot's own
            // (coarser) position events for a smooth scrubber.
            let mut anchor: Option<PositionAnchor> = None;
            // The last mixer volume the poller saw, to detect a Connect
            // (remote) volume change and mirror it into the output gain.
            let mut last_mixer_volume = mixer.volume();

            loop {
                tokio::select! {
                    event = events.recv() => {
                        let Some(event) = event else {
                            tracing::debug!("player event channel closed; engine loop ending");
                            break;
                        };
                        // A new track: kick off background full-song waveform
                        // analysis before folding the event into the state.
                        if let PlayerEvent::TrackChanged { audio_item } = &event {
                            trigger_waveform(&waveform, &session, audio_item);
                        }
                        handle_event(&state, &mut anchor, event);
                    }
                    _ = poll.tick() => {
                        if let Some(anchor) = anchor.as_ref() {
                            tick_position(&state, anchor);
                        }
                        poll_connect_volume(
                            &state,
                            &mixer,
                            &mut last_mixer_volume,
                            &base_volume,
                            &crossfade_gain,
                            &output_active,
                            &output_gain,
                        );
                        apply_crossfade(
                            &state,
                            &crossfade_secs,
                            &base_volume,
                            &crossfade_gain,
                            &output_active,
                            &output_gain,
                        );
                    }
                }
            }
            // Stop referencing the player only when the loop ends.
            drop(player);
        });
    }
}

impl Drop for Engine {
    /// Deregister the Spotify Connect device when the engine is torn down
    /// (logout, or an engine restart for a changed [`EngineConfig`]). This
    /// pauses playback and ends the `Spirc` task so the device disappears
    /// from the account's device list promptly.
    fn drop(&mut self) {
        self.connect.shutdown();
    }
}

/// A reference point for interpolating playback position between events.
struct PositionAnchor {
    /// Position reported by librespot at [`PositionAnchor::at`].
    reported: Duration,
    /// When [`PositionAnchor::reported`] was captured.
    at: Instant,
    /// Whether playback was advancing when the anchor was set.
    playing: bool,
}

impl PositionAnchor {
    /// The interpolated position right now.
    fn now(&self) -> Duration {
        if self.playing {
            self.reported + self.at.elapsed()
        } else {
            self.reported
        }
    }
}

/// Swap a fresh snapshot built by mutating a clone of the current one.
fn publish_with(state: &ArcSwap<PlaybackState>, mutate: impl FnOnce(&mut PlaybackState)) {
    let mut next = (**state.load()).clone();
    mutate(&mut next);
    state.store(Arc::new(next));
}

/// Advance the published position from the anchor, clamped to the duration.
fn tick_position(state: &ArcSwap<PlaybackState>, anchor: &PositionAnchor) {
    let position = anchor.now();
    let current = state.load();
    let clamped = current
        .track
        .as_ref()
        .map_or(position, |t| position.min(t.duration));
    if current.position != clamped {
        publish_with(state, |s| s.position = clamped);
    }
}

/// Recompute the output gain from the base volume and crossfade level and
/// publish it to the audio callback.
///
/// A no-op while `active` is `false` (paused) — pause/resume own the gain
/// then, and the poller must not lift it back off silence.
fn refresh_output_gain(
    active: &AtomicBool,
    base_volume: &AtomicU32,
    crossfade_gain: &AtomicU32,
    output_gain: &AtomicU32,
) {
    if !active.load(Ordering::SeqCst) {
        return;
    }
    let base = f32::from_bits(base_volume.load(Ordering::SeqCst));
    let crossfade = f32::from_bits(crossfade_gain.load(Ordering::SeqCst));
    let gain = perceptual_volume(base) * crossfade;
    output_gain.store(gain.to_bits(), Ordering::SeqCst);
}

/// Mirror a Connect (remote) volume change into the local volume state.
///
/// `Spirc` drives the [`SoftMixer`] when the volume is changed from another
/// Spotify client; the engine otherwise owns volume itself. When the mixer
/// value changes without a local `set_volume`, it was a remote change — adopt
/// it as the new base volume, refresh the gain and publish it for the UI.
fn poll_connect_volume(
    state: &ArcSwap<PlaybackState>,
    mixer: &Arc<dyn Mixer>,
    last_seen: &mut u16,
    base_volume: &AtomicU32,
    crossfade_gain: &AtomicU32,
    output_active: &AtomicBool,
    output_gain: &AtomicU32,
) {
    let current = mixer.volume();
    if current == *last_seen {
        return;
    }
    *last_seen = current;
    let fraction = volume_from_u16(current);
    base_volume.store(fraction.to_bits(), Ordering::SeqCst);
    refresh_output_gain(output_active, base_volume, crossfade_gain, output_gain);
    publish_with(state, |s| s.volume = fraction);
}

/// Apply the track-transition crossfade by scaling the output gain.
///
/// Ramps a track's first `crossfade` seconds *in* and its last `crossfade`
/// seconds *out*: the level factor is `min(position/N, remaining/N)` clamped
/// to `0.0..=1.0`. Because librespot transitions are gapless, the outgoing
/// tail fade and the incoming head fade meet at the track seam, giving a
/// smooth transition with no hard cut.
///
/// A no-op when crossfade is disabled, when paused, or when the track
/// duration is unknown.
fn apply_crossfade(
    state: &ArcSwap<PlaybackState>,
    crossfade_secs: &AtomicU32,
    base_volume: &AtomicU32,
    crossfade_gain: &AtomicU32,
    output_active: &AtomicBool,
    output_gain: &AtomicU32,
) {
    let crossfade = f32::from_bits(crossfade_secs.load(Ordering::SeqCst));
    if crossfade <= 0.0 {
        return;
    }
    let snapshot = state.load();
    if !snapshot.playing {
        return;
    }
    let Some(track) = snapshot.track.as_ref() else {
        return;
    };
    let total = track.duration.as_secs_f32();
    if total <= 0.0 {
        return;
    }
    let factor = crossfade_factor(snapshot.position.as_secs_f32(), total, crossfade);
    crossfade_gain.store(factor.to_bits(), Ordering::SeqCst);
    refresh_output_gain(output_active, base_volume, crossfade_gain, output_gain);
}

/// The crossfade volume factor (`0.0..=1.0`) for a play head at `position`
/// seconds within a `total`-second track, given a `crossfade`-second ramp.
///
/// Ramps from `0.0` at the track edges to `1.0` once `crossfade` seconds in
/// (and back down over the final `crossfade` seconds). A track shorter than
/// `2 * crossfade` never reaches full volume — its fade-in and fade-out
/// overlap, which is the intended behaviour.
///
/// The ramp follows an **equal-power** (quarter-sine) curve rather than a
/// straight line: a track holds near full volume for longer and then drops
/// quickly through the seam, so the transition spends far less time sagging
/// at low volume than a linear fade would.
fn crossfade_factor(position: f32, total: f32, crossfade: f32) -> f32 {
    if crossfade <= 0.0 {
        return 1.0;
    }
    let fade_in = (position / crossfade).clamp(0.0, 1.0);
    let fade_out = ((total - position) / crossfade).clamp(0.0, 1.0);
    equal_power(fade_in.min(fade_out))
}

/// Shape a `0.0..=1.0` linear ramp position into an equal-power gain with a
/// quarter-sine curve: `0 -> 0`, `1 -> 1`, and a gentle knee in between.
fn equal_power(linear: f32) -> f32 {
    (linear.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).sin()
}

/// Kick off background full-song waveform analysis for a just-changed track.
///
/// Best-effort: a track whose id cannot be resolved is simply skipped — the
/// seek bar then keeps its plain capsule.
fn trigger_waveform(analyzer: &WaveformAnalyzer, session: &Session, item: &AudioItem) {
    let track_id = match SpotifyId::try_from(&item.track_id) {
        Ok(id) => id,
        Err(err) => {
            tracing::debug!(%err, "waveform: track id unavailable; skipping analysis");
            return;
        }
    };
    let files: HashMap<AudioFileFormat, FileId> = item
        .files
        .iter()
        .map(|(&format, &id)| (format, id))
        .collect();
    analyzer.analyze(
        &Handle::current(),
        session.clone(),
        track_id,
        item.uri.clone(),
        files,
    );
}

/// Apply a single [`PlayerEvent`] to the shared state and position anchor.
fn handle_event(
    state: &ArcSwap<PlaybackState>,
    anchor: &mut Option<PositionAnchor>,
    event: PlayerEvent,
) {
    match event {
        PlayerEvent::TrackChanged { audio_item } => {
            let track = track_info(&audio_item);
            publish_with(state, |s| {
                s.track = Some(track);
                s.position = Duration::ZERO;
                s.buffering = false;
            });
        }
        PlayerEvent::Loading { position_ms, .. } => {
            *anchor = Some(PositionAnchor {
                reported: Duration::from_millis(u64::from(position_ms)),
                at: Instant::now(),
                playing: false,
            });
            publish_with(state, |s| {
                s.buffering = true;
                s.position = Duration::from_millis(u64::from(position_ms));
            });
        }
        PlayerEvent::Playing { position_ms, .. } => {
            *anchor = Some(PositionAnchor {
                reported: Duration::from_millis(u64::from(position_ms)),
                at: Instant::now(),
                playing: true,
            });
            publish_with(state, |s| {
                s.playing = true;
                s.buffering = false;
                s.position = Duration::from_millis(u64::from(position_ms));
            });
        }
        PlayerEvent::Paused { position_ms, .. } => {
            *anchor = Some(PositionAnchor {
                reported: Duration::from_millis(u64::from(position_ms)),
                at: Instant::now(),
                playing: false,
            });
            publish_with(state, |s| {
                s.playing = false;
                s.buffering = false;
                s.position = Duration::from_millis(u64::from(position_ms));
            });
        }
        PlayerEvent::PositionChanged { position_ms, .. }
        | PlayerEvent::PositionCorrection { position_ms, .. }
        | PlayerEvent::Seeked { position_ms, .. } => {
            let playing = anchor.as_ref().is_some_and(|a| a.playing);
            *anchor = Some(PositionAnchor {
                reported: Duration::from_millis(u64::from(position_ms)),
                at: Instant::now(),
                playing,
            });
            publish_with(state, |s| {
                s.position = Duration::from_millis(u64::from(position_ms));
            });
        }
        PlayerEvent::Stopped { .. } | PlayerEvent::EndOfTrack { .. } => {
            *anchor = None;
            publish_with(state, |s| {
                s.playing = false;
                s.buffering = false;
            });
        }
        PlayerEvent::Unavailable { track_id, .. } => {
            tracing::warn!(?track_id, "track unavailable for playback");
            *anchor = None;
            publish_with(state, |s| {
                s.playing = false;
                s.buffering = false;
            });
        }
        PlayerEvent::VolumeChanged { volume } => {
            publish_with(state, |s| s.volume = volume_from_u16(volume));
        }
        // Remaining events (preload hints, Connect session bookkeeping,
        // shuffle/repeat) are not relevant to local Phase 2 playback.
        other => tracing::trace!(?other, "ignored player event"),
    }
}

/// Project a librespot [`AudioItem`] onto Spottyfi's [`TrackInfo`].
fn track_info(item: &AudioItem) -> TrackInfo {
    // Pick the highest-resolution cover by pixel area.
    let art_url = item
        .covers
        .iter()
        .max_by_key(|cover| i64::from(cover.width) * i64::from(cover.height))
        .map(|cover| cover.url.clone());

    let (artists, artist_ids, album) = match &item.unique_fields {
        UniqueFields::Track { artists, album, .. } => {
            let names: Vec<String> = artists.0.iter().map(|a| a.name.clone()).collect();
            // Resolve each artist URI to a base-62 id. Keep the ids only if
            // every one resolved, so `artists[i]` and `artist_ids[i]` stay
            // aligned (a partial list would mislabel the links).
            let ids: Vec<String> = artists
                .0
                .iter()
                .filter_map(|a| {
                    SpotifyId::try_from(&a.id)
                        .ok()
                        .and_then(|id| id.to_base62().ok())
                })
                .collect();
            let ids = if ids.len() == names.len() {
                ids
            } else {
                Vec::new()
            };
            (names, ids, album.clone())
        }
        UniqueFields::Episode { show_name, .. } => {
            (vec![show_name.clone()], Vec::new(), String::new())
        }
        UniqueFields::Local { artists, album, .. } => (
            artists.clone().map(|a| vec![a]).unwrap_or_default(),
            Vec::new(),
            album.clone().unwrap_or_default(),
        ),
    };

    TrackInfo {
        uri: item.uri.clone(),
        title: item.name.clone(),
        artists,
        artist_ids,
        album,
        art_url,
        duration: Duration::from_millis(u64::from(item.duration_ms)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_round_trips() {
        for v in [0.0_f32, 0.1, 0.25, 0.5, 0.75, 1.0] {
            let back = volume_from_u16(volume_to_u16(v));
            assert!((back - v).abs() < 0.01, "{v} -> {back}");
        }
    }

    #[test]
    fn volume_is_clamped() {
        assert_eq!(volume_to_u16(2.0), MAX_VOLUME);
        assert_eq!(volume_to_u16(-1.0), 0);
    }

    #[test]
    fn volume_endpoints_are_exact() {
        assert_eq!(volume_to_u16(0.0), 0);
        assert_eq!(volume_to_u16(1.0), MAX_VOLUME);
    }

    #[test]
    fn volume_curve_is_perceptual() {
        // The perceptual curve devotes more fader travel to quiet levels:
        // the mixer value at the half-way point sits well below the linear
        // half, expanding fine control across the quiet end.
        let half = volume_to_u16(0.5);
        let linear_half = f32::from(MAX_VOLUME) * 0.5;
        assert!(
            f32::from(half) < linear_half * 0.5,
            "0.5 mapped to {half}, expected well below {linear_half}"
        );
        // The curve is monotonically increasing.
        let mut last = 0;
        for step in 1..=20 {
            let v = volume_to_u16(step as f32 / 20.0);
            assert!(v >= last, "curve not monotonic at step {step}");
            last = v;
        }
    }

    #[test]
    fn bitrate_kbps_matches_tier() {
        assert_eq!(bitrate_kbps(Bitrate::Bitrate96), 96);
        assert_eq!(bitrate_kbps(Bitrate::Bitrate160), 160);
        assert_eq!(bitrate_kbps(Bitrate::Bitrate320), 320);
    }

    #[test]
    fn crossfade_disabled_is_full_volume() {
        assert_eq!(crossfade_factor(50.0, 200.0, 0.0), 1.0);
    }

    #[test]
    fn crossfade_fades_track_edges() {
        // 6s crossfade on a 200s track.
        // Start of the track: silent, ramping in.
        assert_eq!(crossfade_factor(0.0, 200.0, 6.0), 0.0);
        // Halfway through the ramp the equal-power curve sits above 0.5.
        let mid = crossfade_factor(3.0, 200.0, 6.0);
        assert!((mid - 0.707).abs() < 0.01, "equal-power midpoint: {mid}");
        // Once past the ramp-in window: full volume.
        assert!((crossfade_factor(60.0, 200.0, 6.0) - 1.0).abs() < 1e-6);
        // Final seconds: ramping back out to silence at the seam.
        let tail = crossfade_factor(197.0, 200.0, 6.0);
        assert!((tail - 0.707).abs() < 0.01, "equal-power tail: {tail}");
        assert_eq!(crossfade_factor(200.0, 200.0, 6.0), 0.0);
    }

    #[test]
    fn crossfade_on_a_short_track_never_reaches_full_volume() {
        // A 4s track with a 6s crossfade: fade-in and fade-out overlap, so the
        // factor peaks below 1.0 at the midpoint.
        let peak = crossfade_factor(2.0, 4.0, 6.0);
        assert!(peak > 0.0 && peak < 1.0, "peak factor was {peak}");
    }
}
