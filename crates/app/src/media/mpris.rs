//! The MPRIS2 D-Bus interface (Linux).
//!
//! Spottyfi publishes the standard `org.mpris.MediaPlayer2` +
//! `org.mpris.MediaPlayer2.Player` interfaces via the
//! [`mpris-server`](mpris_server) crate, so GNOME/KDE media indicators, the
//! lock-screen widget and `playerctl` can see the now-playing track and drive
//! transport.
//!
//! The interface implementation is a pure adapter:
//!
//! - **Property getters** (`Metadata`, `PlaybackStatus`, `Volume`, …) read the
//!   shared [`MediaSnapshot`].
//! - **Methods** (`Play`, `Pause`, `Next`, `Seek`, `Raise`, `Quit`, …) push a
//!   [`MediaCommand`] onto the [`MediaSender`]; `app` applies it to the real
//!   playback controller.
//!
//! A background task started by [`spawn`] watches the snapshot and emits
//! `PropertiesChanged` / `Seeked` so indicators stay in sync. The whole thing
//! runs on the app's tokio runtime — `mpris-server` is async (zbus).

use std::time::Duration;

use mpris_server::zbus::fdo;
use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Property, Server, Signal, Time, TrackId};
use tokio::runtime::Handle;

use super::{MediaCommand, MediaSender, MediaSnapshot, SharedSnapshot};

/// The D-Bus bus-name suffix — the server binds `org.mpris.MediaPlayer2.spottyfi`.
const BUS_SUFFIX: &str = "spottyfi";

/// The freedesktop `.desktop` entry name (without the `.desktop` suffix),
/// matching the window's app-id so indicators can pick up the icon.
const DESKTOP_ENTRY: &str = "dev.drewett.spottyfi";

/// A synthetic, stable D-Bus object path for the current track.
///
/// MPRIS requires a `mpris:trackid` on every `Metadata`; Spotify URIs are not
/// valid object paths, so a fixed sentinel path is used. Indicators only use
/// it to correlate `SetPosition` calls, which Spottyfi maps to a plain seek.
const TRACK_OBJECT_PATH: &str = "/dev/drewett/spottyfi/track/current";

/// The MPRIS interface implementation: a thin adapter over the [`MediaSnapshot`]
/// and the [`MediaSender`].
struct MprisAdapter {
    /// The live playback snapshot, read by every property getter.
    snapshot: SharedSnapshot,
    /// The command sink — methods push transport / window commands here.
    sender: MediaSender,
}

impl MprisAdapter {
    /// The current snapshot, loaded once per getter call.
    fn snap(&self) -> std::sync::Arc<MediaSnapshot> {
        self.snapshot.load_full()
    }
}

/// Build the `mpris:trackid` object path.
fn track_id() -> TrackId {
    // The path is a compile-time constant known to be valid, but `TrackId`'s
    // constructor is fallible; fall back to the MPRIS "no track" sentinel
    // rather than panicking (the crate forbids `unwrap`/`expect`).
    TrackId::try_from(TRACK_OBJECT_PATH).unwrap_or(TrackId::NO_TRACK)
}

/// Build an MPRIS [`Metadata`] block from a [`MediaSnapshot`].
fn metadata_of(snap: &MediaSnapshot) -> Metadata {
    let mut builder = Metadata::builder().trackid(track_id());
    if snap.has_track {
        builder = builder
            .title(snap.title.clone())
            .album(snap.album.clone())
            .artist(snap.artists.clone())
            .length(Time::from_micros(snap.duration.as_micros() as i64));
        if let Some(art) = &snap.art_url {
            builder = builder.art_url(art.clone());
        }
    }
    builder.build()
}

/// The MPRIS `PlaybackStatus` for a snapshot.
fn status_of(snap: &MediaSnapshot) -> PlaybackStatus {
    if !snap.has_track {
        PlaybackStatus::Stopped
    } else if snap.playing {
        PlaybackStatus::Playing
    } else {
        PlaybackStatus::Paused
    }
}

impl mpris_server::RootInterface for MprisAdapter {
    async fn raise(&self) -> fdo::Result<()> {
        self.sender.send(MediaCommand::RaiseWindow);
        Ok(())
    }

    async fn quit(&self) -> fdo::Result<()> {
        self.sender.send(MediaCommand::Quit);
        Ok(())
    }

    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn identity(&self) -> fdo::Result<String> {
        Ok("Spottyfi".to_owned())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok(DESKTOP_ENTRY.to_owned())
    }

    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        // Spottyfi does not accept `OpenUri`, so it advertises no schemes.
        Ok(Vec::new())
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

impl mpris_server::PlayerInterface for MprisAdapter {
    async fn next(&self) -> fdo::Result<()> {
        self.sender.send(MediaCommand::Next);
        Ok(())
    }

    async fn previous(&self) -> fdo::Result<()> {
        self.sender.send(MediaCommand::Previous);
        Ok(())
    }

    async fn pause(&self) -> fdo::Result<()> {
        self.sender.send(MediaCommand::Pause);
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        self.sender.send(MediaCommand::PlayPause);
        Ok(())
    }

    async fn stop(&self) -> fdo::Result<()> {
        self.sender.send(MediaCommand::Stop);
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        self.sender.send(MediaCommand::Play);
        Ok(())
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        self.sender.send(MediaCommand::SeekBy(offset.as_micros()));
        Ok(())
    }

    async fn set_position(&self, _track_id: TrackId, position: Time) -> fdo::Result<()> {
        let micros = position.as_micros().max(0) as u64;
        self.sender
            .send(MediaCommand::SeekTo(Duration::from_micros(micros)));
        Ok(())
    }

    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        // Not supported — `supported_uri_schemes` advertises an empty list.
        Ok(())
    }

    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(status_of(&self.snap()))
    }

    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(match self.snap().repeat {
            spottyfi_audio::RepeatMode::Off => LoopStatus::None,
            spottyfi_audio::RepeatMode::Context => LoopStatus::Playlist,
            spottyfi_audio::RepeatMode::Track => LoopStatus::Track,
        })
    }

    async fn set_loop_status(&self, loop_status: LoopStatus) -> mpris_server::zbus::Result<()> {
        let mode = match loop_status {
            LoopStatus::None => spottyfi_audio::RepeatMode::Off,
            LoopStatus::Playlist => spottyfi_audio::RepeatMode::Context,
            LoopStatus::Track => spottyfi_audio::RepeatMode::Track,
        };
        self.sender.send(MediaCommand::SetRepeat(mode));
        Ok(())
    }

    async fn rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn set_rate(&self, _rate: f64) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(self.snap().shuffle)
    }

    async fn set_shuffle(&self, shuffle: bool) -> mpris_server::zbus::Result<()> {
        self.sender.send(MediaCommand::SetShuffle(shuffle));
        Ok(())
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(metadata_of(&self.snap()))
    }

    async fn volume(&self) -> fdo::Result<f64> {
        Ok(f64::from(self.snap().volume))
    }

    async fn set_volume(&self, volume: f64) -> mpris_server::zbus::Result<()> {
        self.sender
            .send(MediaCommand::SetVolume(volume.clamp(0.0, 1.0) as f32));
        Ok(())
    }

    async fn position(&self) -> fdo::Result<Time> {
        Ok(Time::from_micros(self.snap().position.as_micros() as i64))
    }

    async fn minimum_rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn maximum_rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(self.snap().can_next)
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(self.snap().can_previous)
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(self.snap().has_track)
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(self.snap().has_track)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(self.snap().has_track)
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

/// Start the MPRIS server and its property-sync task on `runtime`.
///
/// Best-effort: any failure to claim the D-Bus name (no session bus, name
/// already taken) is logged and swallowed — MPRIS is a nicety, not a
/// requirement, and the rest of the app runs regardless.
pub fn spawn(runtime: &Handle, snapshot: SharedSnapshot, sender: MediaSender) {
    runtime.spawn(async move {
        let adapter = MprisAdapter {
            snapshot: snapshot.clone(),
            sender,
        };
        let server = match Server::new(BUS_SUFFIX, adapter).await {
            Ok(server) => server,
            Err(err) => {
                tracing::info!(%err, "MPRIS2 unavailable; desktop media controls disabled");
                return;
            }
        };
        tracing::info!("MPRIS2 interface published on org.mpris.MediaPlayer2.spottyfi");
        sync_loop(server, snapshot).await;
    });
}

/// Watch the snapshot and emit `PropertiesChanged` / `Seeked` on change.
///
/// Polls a touch faster than the engine's ~10Hz swap so indicators never lag
/// a control action by more than a frame.
async fn sync_loop(server: Server<MprisAdapter>, snapshot: SharedSnapshot) {
    let mut last = snapshot.load_full();
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    loop {
        tick.tick().await;
        let current = snapshot.load_full();
        if std::sync::Arc::ptr_eq(&current, &last) {
            continue;
        }

        let mut properties: Vec<Property> = Vec::new();
        if status_of(&current) != status_of(&last) {
            properties.push(Property::PlaybackStatus(status_of(&current)));
        }
        // Track identity changing is the trigger for fresh metadata.
        if current.track_uri != last.track_uri
            || current.title != last.title
            || current.duration != last.duration
        {
            properties.push(Property::Metadata(metadata_of(&current)));
        }
        if (current.volume - last.volume).abs() > f32::EPSILON {
            properties.push(Property::Volume(f64::from(current.volume)));
        }
        if current.shuffle != last.shuffle {
            properties.push(Property::Shuffle(current.shuffle));
        }
        if current.can_next != last.can_next {
            properties.push(Property::CanGoNext(current.can_next));
        }
        if current.can_previous != last.can_previous {
            properties.push(Property::CanGoPrevious(current.can_previous));
        }
        if current.has_track != last.has_track {
            properties.push(Property::CanPlay(current.has_track));
            properties.push(Property::CanPause(current.has_track));
            properties.push(Property::CanSeek(current.has_track));
        }

        if !properties.is_empty() {
            if let Err(err) = server.properties_changed(properties).await {
                tracing::debug!(%err, "MPRIS PropertiesChanged emit failed");
            }
        }

        // A position jump that is not explained by normal playback (a seek,
        // or a track change starting mid-track) gets an explicit `Seeked`.
        let track_changed = current.track_uri != last.track_uri;
        if !track_changed && position_jumped(&last, &current) {
            let signal = Signal::Seeked {
                position: Time::from_micros(current.position.as_micros() as i64),
            };
            if let Err(err) = server.emit(signal).await {
                tracing::debug!(%err, "MPRIS Seeked emit failed");
            }
        }

        last = current;
    }
}

/// Whether the position moved in a way normal playback cannot explain.
///
/// Between two 200ms polls the position should advance by at most a small
/// margin while playing, or not at all while paused; anything outside that is
/// a seek the MPRIS spec wants reported with a `Seeked` signal.
fn position_jumped(last: &MediaSnapshot, current: &MediaSnapshot) -> bool {
    let last_ms = last.position.as_millis() as i128;
    let current_ms = current.position.as_millis() as i128;
    let delta = current_ms - last_ms;
    // Allow up to ~600ms of forward drift (poll jitter + a frame) before
    // calling it a seek; any backward move is always a seek.
    !(-50..=600).contains(&delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reflects_playback() {
        let mut snap = MediaSnapshot::default();
        assert_eq!(status_of(&snap), PlaybackStatus::Stopped);
        snap.has_track = true;
        assert_eq!(status_of(&snap), PlaybackStatus::Paused);
        snap.playing = true;
        assert_eq!(status_of(&snap), PlaybackStatus::Playing);
    }

    #[test]
    fn metadata_carries_track_fields() {
        let snap = MediaSnapshot {
            has_track: true,
            title: "Song".to_owned(),
            album: "Album".to_owned(),
            artists: vec!["A".to_owned()],
            duration: Duration::from_secs(120),
            ..MediaSnapshot::default()
        };
        let meta = metadata_of(&snap);
        assert_eq!(meta.title(), Some("Song"));
        assert_eq!(meta.album(), Some("Album"));
        assert_eq!(meta.length(), Some(Time::from_secs(120)));
    }

    #[test]
    fn position_jump_detection() {
        let base = MediaSnapshot {
            position: Duration::from_secs(10),
            ..MediaSnapshot::default()
        };
        // Normal forward drift between polls — not a seek.
        let drift = MediaSnapshot {
            position: Duration::from_millis(10_200),
            ..MediaSnapshot::default()
        };
        assert!(!position_jumped(&base, &drift));
        // A forward jump — a seek.
        let forward = MediaSnapshot {
            position: Duration::from_secs(40),
            ..MediaSnapshot::default()
        };
        assert!(position_jumped(&base, &forward));
        // Any backward move — a seek.
        let backward = MediaSnapshot {
            position: Duration::from_secs(5),
            ..MediaSnapshot::default()
        };
        assert!(position_jumped(&base, &backward));
    }
}
