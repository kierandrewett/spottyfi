//! The MusicKit JS control protocol.
//!
//! Apple Music audio is FairPlay-DRM protected, so the only sanctioned way to
//! play a full track outside Apple's own apps is Apple's official
//! [MusicKit JS](https://js-cdn.music.apple.com/musickit/v3/docs/) player
//! running inside a real browser engine (the same approach the Cider client
//! takes). Nothing here is reverse-engineered: these are calls against the
//! documented MusicKit JS v3 API.
//!
//! This module is pure — it only *builds* the JavaScript. A
//! [`WebEngine`](crate::engine::WebEngine) evaluates it inside the embedded
//! browser. Keeping it pure makes the protocol unit-testable with no runtime.

use std::time::Duration;

/// The MusicKit JS v3 library URL.
const MUSICKIT_JS: &str = "https://js-cdn.music.apple.com/musickit/v3/musickit.js";

/// Build the bootstrap document the embedded browser loads once.
///
/// It pulls in MusicKit JS, configures it with the developer token, exposes
/// the player instance as `window.spottyfi.music`, and forwards playback
/// events to `window.spottyfiOnState` — the hook the [`WebEngine`] host binds
/// to push state back into [`AppleMusicState`](crate::backend::AppleMusicState).
#[must_use]
pub fn bootstrap_html(developer_token: &str) -> String {
    let token = js_string(developer_token);
    format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><script src="{MUSICKIT_JS}" async></script></head>
<body><script>
document.addEventListener('musickitloaded', async () => {{
  await MusicKit.configure({{
    developerToken: {token},
    app: {{ name: 'Spottyfi', build: '{build}' }},
  }});
  const music = MusicKit.getInstance();
  window.spottyfi = {{ music }};
  const report = () => {{
    if (!window.spottyfiOnState) return;
    window.spottyfiOnState(JSON.stringify({{
      position_ms: Math.round((music.currentPlaybackTime || 0) * 1000),
      playing: music.isPlaying === true,
      finished: music.playbackState === MusicKit.PlaybackStates.completed,
    }}));
  }};
  music.addEventListener('playbackTimeDidChange', report);
  music.addEventListener('playbackStateDidChange', report);
}});
</script></body>
</html>"#,
        build = env!("CARGO_PKG_VERSION"),
    )
}

/// Script to authorize the user (opens Apple's sign-in) — needed once before
/// any playback, and only for the user's own library; the catalog is reachable
/// with the developer token alone.
#[must_use]
pub fn authorize_script() -> String {
    "window.spottyfi.music.authorize();".to_owned()
}

/// Script to load a catalog song by id and start playing it.
#[must_use]
pub fn load_song_script(song_id: &str) -> String {
    let id = js_string(song_id);
    format!(
        "(async () => {{ const m = window.spottyfi.music; \
         await m.setQueue({{ song: {id} }}); await m.play(); }})();"
    )
}

/// Script to resume playback.
#[must_use]
pub fn play_script() -> String {
    "window.spottyfi.music.play();".to_owned()
}

/// Script to pause playback.
#[must_use]
pub fn pause_script() -> String {
    "window.spottyfi.music.pause();".to_owned()
}

/// Script to stop playback.
#[must_use]
pub fn stop_script() -> String {
    "window.spottyfi.music.stop();".to_owned()
}

/// Script to seek to `position` within the current track.
#[must_use]
pub fn seek_script(position: Duration) -> String {
    format!(
        "window.spottyfi.music.seekToTime({});",
        position.as_secs_f64(),
    )
}

/// Script to set the output volume from a `0.0..=1.0` fraction.
#[must_use]
pub fn volume_script(volume: f32) -> String {
    format!("window.spottyfi.music.volume = {};", volume.clamp(0.0, 1.0),)
}

/// Escape a string as a single-quoted JavaScript string literal.
///
/// Developer tokens and catalog ids are normally safe characters, but the
/// values still cross into a JS context, so they are escaped rather than
/// trusted.
fn js_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        match ch {
            '\'' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_embeds_token_and_musickit() {
        let html = bootstrap_html("dev-token-123");
        assert!(html.contains(MUSICKIT_JS));
        assert!(html.contains("'dev-token-123'"));
        assert!(html.contains("MusicKit.configure"));
        assert!(html.contains("spottyfiOnState"));
    }

    #[test]
    fn load_song_script_quotes_the_id() {
        let script = load_song_script("1440913170");
        assert!(script.contains("setQueue({ song: '1440913170' })"));
        assert!(script.contains("m.play()"));
    }

    #[test]
    fn seek_script_uses_seconds() {
        assert_eq!(
            seek_script(Duration::from_millis(90_500)),
            "window.spottyfi.music.seekToTime(90.5);",
        );
    }

    #[test]
    fn volume_script_clamps() {
        assert_eq!(volume_script(2.0), "window.spottyfi.music.volume = 1;");
        assert_eq!(volume_script(-1.0), "window.spottyfi.music.volume = 0;");
    }

    #[test]
    fn js_string_escapes_quotes_and_backslashes() {
        assert_eq!(js_string("a'b\\c"), r"'a\'b\\c'");
    }
}
