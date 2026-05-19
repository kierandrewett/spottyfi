//! The embedded-browser seam.
//!
//! [`WebEngine`] is the one boundary between Spottyfi and the browser that
//! actually runs MusicKit JS. A production build supplies a CEF (Chromium
//! Embedded Framework) implementation — an off-screen Chromium with the
//! Widevine CDM, the only engine that can satisfy Apple Music's EME/DRM. CEF
//! is a heavy, separately-provisioned dependency, so it is deliberately kept
//! behind this trait rather than wired into the default build.

/// A browser engine that can run the MusicKit web player.
///
/// The host loads [`bootstrap_html`](crate::musickit::bootstrap_html) once,
/// binds the page's `window.spottyfiOnState` callback to push events into the
/// shared [`AppleMusicState`](crate::backend::AppleMusicState), and then
/// evaluates the control scripts this crate builds.
pub trait WebEngine: Send + Sync {
    /// Evaluate `script` in the page's JavaScript context.
    fn eval(&self, script: &str);
}

/// A no-op [`WebEngine`] that logs the scripts it is handed.
///
/// The default when no CEF engine is installed: the Apple Music backend stays
/// constructible and the rest of the app runs unaffected — Apple Music tracks
/// simply de-duplicate onto a playable source instead of playing here.
#[derive(Debug, Default, Clone, Copy)]
pub struct LoggingWebEngine;

impl WebEngine for LoggingWebEngine {
    fn eval(&self, script: &str) {
        tracing::debug!(%script, "apple music: no web engine installed; script dropped");
    }
}
