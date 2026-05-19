//! Desktop platform integration: MPRIS2, media keys, the system tray,
//! single-instance and track-change notifications.
//!
//! Everything here funnels through one type — [`MediaCommand`] — so the four
//! command sources (the MPRIS D-Bus interface, the `global-hotkey` media-key
//! fallback, the tray menu and, indirectly, a second app launch asking the
//! first to raise its window) all route into the same place: `app` drains the
//! command channel each frame and applies them to the playback controller and
//! the window.
//!
//! ## Wiring
//!
//! - [`MediaBridge`] owns the command channel and a shared [`MediaSnapshot`]
//!   of the live playback state. `app` calls [`MediaBridge::publish`] each
//!   frame with the latest [`PlaybackState`] / [`QueueState`]; the MPRIS
//!   server task and the tray reader observe the snapshot.
//! - [`mpris`] runs an `mpris-server` D-Bus server on the tokio runtime. Its
//!   interface implementation reads the snapshot for property getters and
//!   sends [`MediaCommand`]s for Play/Pause/Next/Previous/Stop/Seek.
//! - [`media_keys`] registers the transport hotkeys system-wide via
//!   `global-hotkey` — a fallback for window managers that do not route the
//!   XF86Audio* keys through MPRIS.
//! - [`tray`] owns a `tray-icon` tray with a Play/Pause/Next/Previous/Show-Hide
//!   /Quit menu.
//! - [`single_instance`] holds the process-wide lock; a second launch exits
//!   early after asking the running instance (over MPRIS `Raise`) to surface.
//! - [`notify`] fires an off-by-default desktop notification on track change.
//!
//! All of this is **Linux-first**: MPRIS is Linux-only by definition, and the
//! tray / media-key / notification crates are the cross-platform abstractions
//! the maintainer's Linux/Wayland target needs. Windows SMTC and macOS
//! MediaPlayer are explicitly out of scope for this phase.

pub mod media_keys;
pub mod mpris;
pub mod notify;
pub mod single_instance;
pub mod tray;

use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use spottyfi_audio::{PlaybackState, QueueState, RepeatMode};
use tokio::sync::mpsc;

/// A transport command raised by a desktop integration surface.
///
/// Produced by MPRIS, the media-key fallback and the tray menu; consumed by
/// `app`, which translates each into a [`TransportIntent`](crate::transport::
/// TransportIntent) or a window action.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaCommand {
    /// Toggle between playing and paused.
    PlayPause,
    /// Resume / start playback.
    Play,
    /// Pause playback.
    Pause,
    /// Stop playback (treated as a pause — Spottyfi has no distinct stop).
    Stop,
    /// Skip to the next track.
    Next,
    /// Skip to the previous track.
    Previous,
    /// Seek to an absolute position within the current track.
    SeekTo(Duration),
    /// Seek by a signed offset relative to the current position.
    SeekBy(i64),
    /// Set the output volume to a `0.0..=1.0` fraction.
    SetVolume(f32),
    /// Set shuffle on or off.
    SetShuffle(bool),
    /// Set the repeat mode.
    SetRepeat(RepeatMode),
    /// Bring the application window to the front (MPRIS `Raise`, the tray's
    /// "Show window", or a second-launch request).
    RaiseWindow,
    /// Toggle the window between shown and hidden (the tray's "Show / Hide").
    ToggleWindow,
    /// Quit the application (MPRIS `Quit` or the tray's "Quit").
    Quit,
}

/// An immutable view of the playback state the integration surfaces read.
///
/// A compact projection of [`PlaybackState`] + [`QueueState`] holding only
/// what MPRIS metadata, the tray label and the notification need — kept
/// separate so the integration code never depends on the audio crate's full
/// snapshot shape.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaSnapshot {
    /// The current track's Spotify URI, empty when nothing is loaded.
    pub track_uri: String,
    /// The current track's title.
    pub title: String,
    /// The current track's artists, in billing order.
    pub artists: Vec<String>,
    /// The current track's album name.
    pub album: String,
    /// The current track's cover-art URL, if known.
    pub art_url: Option<String>,
    /// The current track's total duration.
    pub duration: Duration,
    /// The playback position within the current track.
    pub position: Duration,
    /// Whether audio is actively playing.
    pub playing: bool,
    /// Whether a track is loaded at all.
    pub has_track: bool,
    /// The output volume, `0.0..=1.0`.
    pub volume: f32,
    /// Whether the context plays shuffled.
    pub shuffle: bool,
    /// The repeat mode.
    pub repeat: RepeatMode,
    /// Whether there is a track to skip forward to.
    pub can_next: bool,
    /// Whether skipping backward is meaningful.
    pub can_previous: bool,
}

impl MediaSnapshot {
    /// Build a snapshot from the live audio-engine state.
    #[must_use]
    pub fn from_engine(playback: &PlaybackState, queue: &QueueState) -> Self {
        let track = playback.track.as_ref();
        Self {
            track_uri: track.map(|t| t.uri.clone()).unwrap_or_default(),
            title: track.map(|t| t.title.clone()).unwrap_or_default(),
            artists: track.map(|t| t.artists.clone()).unwrap_or_default(),
            album: track.map(|t| t.album.clone()).unwrap_or_default(),
            art_url: track.and_then(|t| t.art_url.clone()),
            duration: track.map(|t| t.duration).unwrap_or_default(),
            position: playback.position,
            playing: playback.playing,
            has_track: track.is_some(),
            volume: playback.volume,
            shuffle: queue.shuffle,
            repeat: queue.repeat,
            // A previous always makes sense while a track is loaded (it seeks
            // to the track start when there is no earlier track); next needs
            // something queued.
            can_next: queue.has_upcoming(),
            can_previous: track.is_some(),
        }
    }

    /// The artists joined into one display string (`"A, B"`).
    #[must_use]
    pub fn artist_line(&self) -> String {
        self.artists.join(", ")
    }

    /// A single-line `"Title — Artists"` label for the tray tooltip.
    #[must_use]
    pub fn now_playing_line(&self) -> String {
        if !self.has_track {
            return "Nothing playing".to_owned();
        }
        let artists = self.artist_line();
        if artists.is_empty() {
            self.title.clone()
        } else {
            format!("{} — {artists}", self.title)
        }
    }
}

/// The shared, hot-swappable [`MediaSnapshot`] read by the integration tasks.
pub type SharedSnapshot = Arc<ArcSwap<MediaSnapshot>>;

/// The hub that connects the desktop integrations to `app`.
///
/// Owns the command receiver `app` drains each frame, the [`MediaSender`] the
/// integrations push commands onto, and the shared snapshot they read.
pub struct MediaBridge {
    /// The receiving end of the command channel, drained by `app`.
    rx: mpsc::UnboundedReceiver<MediaCommand>,
    /// A cloneable sending end handed to each integration.
    sender: MediaSender,
    /// The live playback snapshot, refreshed by `app` and read by the tasks.
    snapshot: SharedSnapshot,
}

impl MediaBridge {
    /// Build a fresh bridge with an empty command channel and snapshot.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            rx,
            sender: MediaSender { tx },
            snapshot: Arc::new(ArcSwap::from_pointee(MediaSnapshot::default())),
        }
    }

    /// A cloneable handle for pushing [`MediaCommand`]s onto the channel.
    #[must_use]
    pub fn sender(&self) -> MediaSender {
        self.sender.clone()
    }

    /// The shared snapshot handle the integration tasks read.
    #[must_use]
    pub fn snapshot(&self) -> SharedSnapshot {
        Arc::clone(&self.snapshot)
    }

    /// Publish the latest engine state for the integration surfaces.
    ///
    /// Called by `app` each frame; cheap — a single `ArcSwap` store when the
    /// projected snapshot actually changed, skipped otherwise.
    pub fn publish(&self, playback: &PlaybackState, queue: &QueueState) {
        let next = MediaSnapshot::from_engine(playback, queue);
        if *self.snapshot.load_full() != next {
            self.snapshot.store(Arc::new(next));
        }
    }

    /// Drain every command queued since the last call.
    ///
    /// `app` calls this once per frame and applies each command.
    pub fn drain(&mut self) -> Vec<MediaCommand> {
        let mut commands = Vec::new();
        while let Ok(command) = self.rx.try_recv() {
            commands.push(command);
        }
        commands
    }
}

impl Default for MediaBridge {
    fn default() -> Self {
        Self::new()
    }
}

/// A cloneable sender the desktop integrations push [`MediaCommand`]s onto.
#[derive(Debug, Clone)]
pub struct MediaSender {
    /// The unbounded channel into `app`'s per-frame command drain.
    tx: mpsc::UnboundedSender<MediaCommand>,
}

impl MediaSender {
    /// Queue a command for `app`. A closed channel (app shutting down) is
    /// silently ignored — the integrations outlive nothing.
    pub fn send(&self, command: MediaCommand) {
        if let Err(err) = self.tx.send(command) {
            tracing::debug!(%err, "media command dropped: app channel closed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spottyfi_audio::{QueueTrack, TrackInfo};

    fn playing_state() -> (PlaybackState, QueueState) {
        let playback = PlaybackState {
            track: Some(TrackInfo {
                uri: "spotify:track:abc".to_owned(),
                title: "Song".to_owned(),
                artists: vec!["A".to_owned(), "B".to_owned()],
                artist_ids: Vec::new(),
                album: "Album".to_owned(),
                art_url: Some("https://i.scdn.co/x".to_owned()),
                duration: Duration::from_secs(200),
            }),
            position: Duration::from_secs(30),
            playing: true,
            volume: 0.8,
            ..PlaybackState::default()
        };
        let queue = QueueState {
            manual: vec![QueueTrack {
                uri: "spotify:track:def".to_owned(),
                title: "Next".to_owned(),
                artists: vec![],
                album: String::new(),
                album_id: None,
                art_url: None,
                duration: Duration::from_secs(100),
            }],
            ..QueueState::default()
        };
        (playback, queue)
    }

    #[test]
    fn snapshot_projects_engine_state() {
        let (playback, queue) = playing_state();
        let snap = MediaSnapshot::from_engine(&playback, &queue);
        assert_eq!(snap.title, "Song");
        assert_eq!(snap.artist_line(), "A, B");
        assert!(snap.playing);
        assert!(snap.has_track);
        assert!(snap.can_next);
        assert!(snap.can_previous);
        assert_eq!(snap.now_playing_line(), "Song — A, B");
    }

    #[test]
    fn empty_snapshot_has_no_track() {
        let snap = MediaSnapshot::default();
        assert!(!snap.has_track);
        assert!(!snap.can_next);
        assert_eq!(snap.now_playing_line(), "Nothing playing");
    }

    #[test]
    fn bridge_round_trips_commands() {
        let mut bridge = MediaBridge::new();
        let sender = bridge.sender();
        sender.send(MediaCommand::PlayPause);
        sender.send(MediaCommand::Next);
        assert_eq!(
            bridge.drain(),
            vec![MediaCommand::PlayPause, MediaCommand::Next]
        );
        // A second drain is empty — commands are consumed once.
        assert!(bridge.drain().is_empty());
    }

    #[test]
    fn bridge_publishes_snapshot() {
        let bridge = MediaBridge::new();
        let shared = bridge.snapshot();
        let (playback, queue) = playing_state();
        bridge.publish(&playback, &queue);
        assert_eq!(shared.load_full().title, "Song");
    }
}
