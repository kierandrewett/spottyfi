//! The playback state snapshot the UI projects, and Spotify URI parsing.

use std::time::Duration;

use librespot::core::SpotifyUri;

use crate::error::AudioError;

/// Metadata about the track currently loaded in the player.
///
/// Populated from librespot's `PlayerEvent::TrackChanged`, which carries an
/// `AudioItem` with everything the transport bar needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackInfo {
    /// The track's Spotify URI (`spotify:track:...`).
    pub uri: String,
    /// Track title.
    pub title: String,
    /// Contributing artist names, in billing order.
    pub artists: Vec<String>,
    /// Base-62 Spotify ids for [`Self::artists`], in the same order.
    ///
    /// Either empty (ids unavailable — e.g. a local file) or exactly the same
    /// length as `artists`, so `artists[i]` always pairs with `artist_ids[i]`.
    pub artist_ids: Vec<String>,
    /// Album name, if known.
    pub album: String,
    /// URL of the album cover art at the largest available size, if any.
    pub art_url: Option<String>,
    /// Total track duration.
    pub duration: Duration,
}

impl TrackInfo {
    /// The artists joined into a single display string (`"A, B"`).
    #[must_use]
    pub fn artist_line(&self) -> String {
        self.artists.join(", ")
    }
}

/// A snapshot of the audio engine's playback state, read by the UI each frame.
///
/// The engine swaps a fresh `PlaybackState` into an `ArcSwap` roughly ten times
/// a second while playing, so the transport bar's progress scrubber animates
/// smoothly without the UI thread ever touching librespot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaybackState {
    /// The currently loaded track, or `None` when nothing has been played yet.
    pub track: Option<TrackInfo>,
    /// Playback position within the current track.
    pub position: Duration,
    /// Whether audio is actively playing (as opposed to paused or stopped).
    pub playing: bool,
    /// Whether the player is buffering — loading a track before playback.
    pub buffering: bool,
    /// Output volume, normalised to `0.0..=1.0`.
    pub volume: f32,
    /// The configured stream bitrate in kilobits per second (96 / 160 / 320).
    ///
    /// Zero before the engine has connected; the transport shows it verbatim.
    pub bitrate_kbps: u16,
    /// The decoder codec librespot is using (e.g. `"Ogg Vorbis"`).
    ///
    /// Empty before the engine has connected.
    pub codec: String,
}

impl PlaybackState {
    /// The transport's codec/bitrate readout (e.g. `"Ogg Vorbis 320 kbps"`).
    ///
    /// Returns `None` before the engine has connected and reported a bitrate.
    #[must_use]
    pub fn codec_line(&self) -> Option<String> {
        if self.bitrate_kbps == 0 || self.codec.is_empty() {
            return None;
        }
        Some(format!("{} {} kbps", self.codec, self.bitrate_kbps))
    }

    /// Playback progress through the current track as a `0.0..=1.0` fraction.
    ///
    /// Returns `0.0` when no track is loaded or the duration is unknown.
    #[must_use]
    pub fn progress_fraction(&self) -> f32 {
        match &self.track {
            Some(track) if !track.duration.is_zero() => {
                (self.position.as_secs_f32() / track.duration.as_secs_f32()).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }
}

/// Normalise a user-supplied Spotify reference to a canonical `spotify:` URI.
///
/// Accepts both the canonical URI form (`spotify:track:ID`) and the
/// `open.spotify.com` web URL form (`https://open.spotify.com/track/ID?si=…`).
/// The returned string is always in canonical URI form.
///
/// # Errors
///
/// Returns [`AudioError::InvalidUri`] if the input is neither form, or refers
/// to no recognisable Spotify item.
pub fn normalise_uri(input: &str) -> Result<String, AudioError> {
    let trimmed = input.trim();

    if trimmed.starts_with("spotify:") {
        // Validate by round-tripping through librespot's parser.
        SpotifyUri::from_uri(trimmed).map_err(|err| AudioError::InvalidUri(err.to_string()))?;
        return Ok(trimmed.to_owned());
    }

    // `open.spotify.com/<kind>/<id>` — strip scheme, host and query string.
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let path = without_scheme
        .strip_prefix("open.spotify.com/")
        .ok_or_else(|| AudioError::InvalidUri(input.to_owned()))?;
    // Drop any query string or fragment.
    let path = path
        .split(['?', '#'])
        .next()
        .unwrap_or(path)
        .trim_end_matches('/');

    // The path may carry a locale segment: `/intl-de/track/ID`.
    let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.first().is_some_and(|s| s.starts_with("intl-")) {
        segments.remove(0);
    }
    let [kind, id] = segments.as_slice() else {
        return Err(AudioError::InvalidUri(input.to_owned()));
    };

    let uri = format!("spotify:{kind}:{id}");
    SpotifyUri::from_uri(&uri).map_err(|err| AudioError::InvalidUri(err.to_string()))?;
    Ok(uri)
}

/// Parse a user-supplied reference into a playable librespot [`SpotifyUri`].
///
/// # Errors
///
/// Returns [`AudioError::InvalidUri`] if the reference cannot be parsed, or
/// [`AudioError::NotPlayable`] if it refers to a non-playable item such as an
/// album, artist or playlist (those need a queue — Phase 8).
pub fn parse_playable(input: &str) -> Result<SpotifyUri, AudioError> {
    let uri = normalise_uri(input)?;
    let parsed =
        SpotifyUri::from_uri(&uri).map_err(|err| AudioError::InvalidUri(err.to_string()))?;
    if parsed.is_playable() {
        Ok(parsed)
    } else {
        Err(AudioError::NotPlayable(uri))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK_ID: &str = "4uLU6hMCjMI75M1A2tKUQC";

    #[test]
    fn accepts_canonical_track_uri() {
        let uri = normalise_uri(&format!("spotify:track:{TRACK_ID}")).expect("valid uri");
        assert_eq!(uri, format!("spotify:track:{TRACK_ID}"));
    }

    #[test]
    fn trims_whitespace() {
        let uri = normalise_uri(&format!("  spotify:track:{TRACK_ID}  ")).expect("valid uri");
        assert_eq!(uri, format!("spotify:track:{TRACK_ID}"));
    }

    #[test]
    fn converts_open_url_to_uri() {
        let uri = normalise_uri(&format!(
            "https://open.spotify.com/track/{TRACK_ID}?si=abc123"
        ))
        .expect("valid uri");
        assert_eq!(uri, format!("spotify:track:{TRACK_ID}"));
    }

    #[test]
    fn converts_open_url_with_locale_segment() {
        let uri = normalise_uri(&format!(
            "https://open.spotify.com/intl-de/track/{TRACK_ID}"
        ))
        .expect("valid uri");
        assert_eq!(uri, format!("spotify:track:{TRACK_ID}"));
    }

    #[test]
    fn rejects_non_spotify_input() {
        assert!(matches!(
            normalise_uri("https://example.com/track/abc"),
            Err(AudioError::InvalidUri(_))
        ));
        assert!(matches!(
            normalise_uri("not a uri at all"),
            Err(AudioError::InvalidUri(_))
        ));
    }

    #[test]
    fn parse_playable_accepts_a_track() {
        let parsed = parse_playable(&format!("spotify:track:{TRACK_ID}")).expect("playable");
        assert!(parsed.is_playable());
    }

    #[test]
    fn parse_playable_rejects_a_playlist() {
        // A playlist URI parses but is not directly playable without a queue.
        let result = parse_playable("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M");
        assert!(matches!(result, Err(AudioError::NotPlayable(_))));
    }

    #[test]
    fn progress_fraction_is_clamped() {
        let mut state = PlaybackState {
            track: Some(TrackInfo {
                duration: Duration::from_secs(100),
                ..TrackInfo::default()
            }),
            position: Duration::from_secs(50),
            ..PlaybackState::default()
        };
        assert!((state.progress_fraction() - 0.5).abs() < f32::EPSILON);

        state.position = Duration::from_secs(500);
        assert!((state.progress_fraction() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn progress_fraction_handles_no_track() {
        assert_eq!(PlaybackState::default().progress_fraction(), 0.0);
    }
}
