//! Track-change desktop notifications.
//!
//! When enabled, Spottyfi raises a desktop notification each time the playing
//! track changes, via [`notify-rust`](notify_rust). This is **off by default**
//! — it is opt-in from the Settings page's Notifications section.
//!
//! [`TrackChangeNotifier`] is owned by `app` and offered the live
//! [`MediaSnapshot`] each frame; it fires a notification only when the track
//! URI actually changes and the feature is enabled. Posting the notification
//! is handed to a detached thread so a slow notification daemon never stalls
//! the egui frame.

use std::thread;

use super::MediaSnapshot;

/// Watches the playback snapshot and fires a notification on track change.
///
/// Stateful: it remembers the last track it notified for so a re-render with
/// the same track does not re-notify.
#[derive(Default)]
pub struct TrackChangeNotifier {
    /// The URI of the track the last notification was fired for. Empty before
    /// the first notification.
    last_notified_uri: String,
}

impl TrackChangeNotifier {
    /// Build a notifier with no track seen yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer the latest snapshot; fire a notification if the track changed.
    ///
    /// `enabled` is the persisted Settings toggle — when `false` this only
    /// tracks the current URI (so re-enabling does not immediately fire for
    /// the already-playing track) and posts nothing.
    pub fn observe(&mut self, snapshot: &MediaSnapshot, enabled: bool) {
        if !snapshot.has_track {
            // Nothing playing — reset so the next track always notifies.
            self.last_notified_uri.clear();
            return;
        }
        if snapshot.track_uri == self.last_notified_uri {
            return;
        }
        let first_observation = self.last_notified_uri.is_empty();
        self.last_notified_uri = snapshot.track_uri.clone();

        // Do not fire on the very first track we ever see (app start / login)
        // — only on a genuine change — and only when the user opted in.
        if enabled && !first_observation {
            post(snapshot);
        }
    }

    /// Forget the last-seen track (call on logout, so a fresh login's first
    /// track is treated as a first observation, not a change).
    pub fn reset(&mut self) {
        self.last_notified_uri.clear();
    }
}

/// Post one track-change notification on a detached thread.
fn post(snapshot: &MediaSnapshot) {
    let summary = snapshot.title.clone();
    let body = {
        let artists = snapshot.artist_line();
        if snapshot.album.is_empty() {
            artists
        } else if artists.is_empty() {
            snapshot.album.clone()
        } else {
            format!("{artists} · {}", snapshot.album)
        }
    };

    let spawned = thread::Builder::new()
        .name("spottyfi-notify".to_owned())
        .spawn(move || {
            let result = notify_rust::Notification::new()
                .summary(&summary)
                .body(&body)
                .appname("Spottyfi")
                // The reverse-DNS id matches the window app-id so the desktop
                // can attribute the notification to Spottyfi's icon.
                .icon("dev.drewett.spottyfi")
                .show();
            if let Err(err) = result {
                tracing::debug!(%err, "track-change notification failed");
            }
        });
    if let Err(err) = spawned {
        tracing::debug!(%err, "could not spawn notification thread");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn snapshot(uri: &str) -> MediaSnapshot {
        MediaSnapshot {
            has_track: true,
            track_uri: uri.to_owned(),
            title: "Title".to_owned(),
            artists: vec!["Artist".to_owned()],
            album: "Album".to_owned(),
            duration: Duration::from_secs(180),
            ..MediaSnapshot::default()
        }
    }

    #[test]
    fn first_track_is_not_a_change() {
        let mut notifier = TrackChangeNotifier::new();
        // The first observation must not be treated as a change — it only
        // records the URI. (No notification daemon is hit: `enabled` aside,
        // `first_observation` short-circuits `post`.)
        notifier.observe(&snapshot("spotify:track:a"), false);
        assert_eq!(notifier.last_notified_uri, "spotify:track:a");
    }

    #[test]
    fn same_track_does_not_re_notify() {
        let mut notifier = TrackChangeNotifier::new();
        notifier.observe(&snapshot("spotify:track:a"), false);
        notifier.observe(&snapshot("spotify:track:a"), false);
        assert_eq!(notifier.last_notified_uri, "spotify:track:a");
    }

    #[test]
    fn track_change_updates_the_remembered_uri() {
        let mut notifier = TrackChangeNotifier::new();
        notifier.observe(&snapshot("spotify:track:a"), false);
        notifier.observe(&snapshot("spotify:track:b"), false);
        assert_eq!(notifier.last_notified_uri, "spotify:track:b");
    }

    #[test]
    fn no_track_resets_state() {
        let mut notifier = TrackChangeNotifier::new();
        notifier.observe(&snapshot("spotify:track:a"), false);
        notifier.observe(&MediaSnapshot::default(), false);
        assert!(notifier.last_notified_uri.is_empty());
    }

    #[test]
    fn reset_clears_remembered_track() {
        let mut notifier = TrackChangeNotifier::new();
        notifier.observe(&snapshot("spotify:track:a"), false);
        notifier.reset();
        assert!(notifier.last_notified_uri.is_empty());
    }
}
