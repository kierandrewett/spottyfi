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

use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use librespot::core::authentication::Credentials;
use librespot::core::config::SessionConfig;
use librespot::core::session::Session;
use librespot::metadata::audio::{AudioItem, UniqueFields};
use librespot::playback::audio_backend;
use librespot::playback::config::{AudioFormat, PlayerConfig};
use librespot::playback::mixer::softmixer::SoftMixer;
use librespot::playback::mixer::{Mixer, MixerConfig};
use librespot::playback::player::{Player, PlayerEvent};

use crate::error::{AudioError, AudioResult};
use crate::state::{PlaybackState, TrackInfo};

/// How often the position poller refreshes the published snapshot.
///
/// 100ms gives the ~10Hz update rate the transport bar needs for a smooth
/// scrubber, as specified in `PLAN.md` Phase 2.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// librespot's full-scale mixer volume (`u16::MAX`).
const MAX_VOLUME: u16 = u16::MAX;

/// Convert a `0.0..=1.0` UI volume into librespot's `u16` mixer scale.
fn volume_to_u16(volume: f32) -> u16 {
    (volume.clamp(0.0, 1.0) * f32::from(MAX_VOLUME)).round() as u16
}

/// Convert librespot's `u16` mixer volume back into a `0.0..=1.0` fraction.
fn volume_from_u16(volume: u16) -> f32 {
    f32::from(volume) / f32::from(MAX_VOLUME)
}

/// The running librespot engine: session, player, mixer and shared state.
pub(crate) struct Engine {
    /// The librespot player. Cheap to clone (`Arc` inside).
    player: Arc<Player>,
    /// The software mixer driving output volume.
    mixer: Arc<dyn Mixer>,
    /// The published playback snapshot, read by the UI each frame.
    state: Arc<ArcSwap<PlaybackState>>,
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
        state: Arc<ArcSwap<PlaybackState>>,
    ) -> AudioResult<Self> {
        let session_config = SessionConfig::default();
        let player_config = PlayerConfig {
            // Opt into periodic `PositionChanged` events; combined with the
            // poller this keeps the published position accurate.
            position_update_interval: Some(POLL_INTERVAL),
            ..PlayerConfig::default()
        };

        let session = Session::new(session_config, None);
        let credentials = Credentials::with_access_token(access_token);
        session
            .connect(credentials, false)
            .await
            .map_err(|err| AudioError::Connect(err.to_string()))?;
        tracing::info!("librespot session connected");

        let mixer = SoftMixer::open(MixerConfig::default())
            .map(|m| Arc::new(m) as Arc<dyn Mixer>)
            .map_err(|err| AudioError::Connect(err.to_string()))?;

        let backend = audio_backend::find(None).ok_or(AudioError::NoBackend)?;
        let audio_format = AudioFormat::default();
        let soft_volume = mixer.get_soft_volume();
        let player = Player::new(player_config, session, soft_volume, move || {
            backend(None, audio_format)
        });

        // Publish the mixer's starting volume so the UI's slider is correct.
        let initial = PlaybackState {
            volume: volume_from_u16(mixer.volume()),
            ..PlaybackState::default()
        };
        state.store(Arc::new(initial));

        let engine = Self {
            player,
            mixer,
            state,
        };
        engine.spawn_event_loop();
        Ok(engine)
    }

    /// The librespot player handle.
    pub(crate) fn player(&self) -> Arc<Player> {
        Arc::clone(&self.player)
    }

    /// Set the output volume from a `0.0..=1.0` fraction.
    pub(crate) fn set_volume(&self, volume: f32) {
        self.mixer.set_volume(volume_to_u16(volume));
        // The mixer does not emit an event; publish the change ourselves.
        publish_with(&self.state, |s| s.volume = volume.clamp(0.0, 1.0));
    }

    /// Spawn the player-event loop and the position poller.
    fn spawn_event_loop(&self) {
        let mut events = self.player.get_player_event_channel();
        let state = Arc::clone(&self.state);
        let player = Arc::clone(&self.player);

        tokio::spawn(async move {
            // The poller ticks continuously; it only mutates the snapshot
            // while a track is actually playing.
            let mut poll = tokio::time::interval(POLL_INTERVAL);
            // A locally tracked position, advanced between librespot's own
            // (coarser) position events for a smooth scrubber.
            let mut anchor: Option<PositionAnchor> = None;

            loop {
                tokio::select! {
                    event = events.recv() => {
                        let Some(event) = event else {
                            tracing::debug!("player event channel closed; engine loop ending");
                            break;
                        };
                        handle_event(&state, &mut anchor, event);
                    }
                    _ = poll.tick() => {
                        if let Some(anchor) = anchor.as_ref() {
                            tick_position(&state, anchor);
                        }
                    }
                }
            }
            // Stop referencing the player only when the loop ends.
            drop(player);
        });
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

    let (artists, album) = match &item.unique_fields {
        UniqueFields::Track { artists, album, .. } => (
            artists.0.iter().map(|a| a.name.clone()).collect(),
            album.clone(),
        ),
        UniqueFields::Episode { show_name, .. } => (vec![show_name.clone()], String::new()),
        UniqueFields::Local { artists, album, .. } => (
            artists.clone().map(|a| vec![a]).unwrap_or_default(),
            album.clone().unwrap_or_default(),
        ),
    };

    TrackInfo {
        uri: item.uri.clone(),
        title: item.name.clone(),
        artists,
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
        for v in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            let back = volume_from_u16(volume_to_u16(v));
            assert!((back - v).abs() < 0.001, "{v} -> {back}");
        }
    }

    #[test]
    fn volume_is_clamped() {
        assert_eq!(volume_to_u16(2.0), MAX_VOLUME);
        assert_eq!(volume_to_u16(-1.0), 0);
    }
}
