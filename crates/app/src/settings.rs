//! The persisted user-settings model.
//!
//! [`AppSettings`] is the power-user configuration shown on the Settings page
//! and serialised, with the dock layout, into `<config_dir>/layout.ron` (see
//! [`crate::shell::PersistedShell`]). It groups four sections:
//!
//! - [`AudioSettings`] — streaming quality, normalisation, crossfade. These are
//!   **start-time** settings: librespot bakes bitrate and normalisation into
//!   its `PlayerConfig` when the session connects, so a change takes effect on
//!   the next engine start (a logout/login). The Settings page says so.
//! - [`EqualizerSettings`] — a 10-band graphic EQ. The on/off flag and band
//!   gains are persisted and drive the live DSP in the custom audio backend
//!   (WS7a): a bank of biquad peaking filters in `spottyfi-audio`. Unlike the
//!   audio settings the equaliser applies live — no engine restart.
//! - [`LocalFilesSettings`] — the list of local-music folders. The folder list
//!   persists here; scanning and playback of local files are out of scope.
//!
//! Appearance (theme / density) and dock layout stay on [`PersistedShell`]
//! directly — they predate this module and the Settings page reads them there.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use spottyfi_audio::{EngineConfig, StreamQuality};

/// The streaming bitrate tier requested from Spotify.
///
/// Mirrors [`spottyfi_audio::StreamQuality`] but is `serde`-derived so it can
/// live in the persisted config; [`StreamTier::to_audio`] converts to the
/// audio crate's enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StreamTier {
    /// 96 kbps — least bandwidth.
    Low,
    /// 160 kbps — the normal tier.
    Normal,
    /// 320 kbps — highest quality; the default.
    #[default]
    High,
}

impl StreamTier {
    /// Every tier, low-to-high, for a settings selector.
    #[must_use]
    pub fn all() -> [StreamTier; 3] {
        [StreamTier::Low, StreamTier::Normal, StreamTier::High]
    }

    /// A human-readable label including the bitrate.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            StreamTier::Low => "Low — 96 kbps",
            StreamTier::Normal => "Normal — 160 kbps",
            StreamTier::High => "High — 320 kbps",
        }
    }

    /// Convert to the audio crate's [`StreamQuality`].
    #[must_use]
    pub fn to_audio(self) -> StreamQuality {
        match self {
            StreamTier::Low => StreamQuality::Low,
            StreamTier::Normal => StreamQuality::Normal,
            StreamTier::High => StreamQuality::High,
        }
    }
}

/// Playback / audio engine settings.
///
/// Quality and normalisation are start-time settings (see the module docs);
/// crossfade is a UI-level preference with no engine support yet — librespot
/// has no crossfade, so the value is persisted for a future custom backend.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct AudioSettings {
    /// The streaming bitrate tier.
    pub quality: StreamTier,
    /// Whether librespot volume normalisation is enabled.
    pub normalisation: bool,
    /// Crossfade duration between tracks, in seconds (`0.0` disables it).
    ///
    /// Persisted for a future custom backend — librespot itself has no
    /// crossfade, so this currently does not affect playback.
    pub crossfade_seconds: f32,
}

impl AudioSettings {
    /// The largest crossfade the slider offers, in seconds.
    pub const MAX_CROSSFADE_SECONDS: f32 = 12.0;

    /// The start-time [`EngineConfig`] these settings map to.
    #[must_use]
    pub fn engine_config(&self) -> EngineConfig {
        EngineConfig {
            quality: self.quality.to_audio(),
            normalisation: self.normalisation,
        }
    }
}

/// The number of bands in the graphic equalizer.
pub const EQ_BAND_COUNT: usize = 10;

/// The centre frequencies, in hertz, of the [`EQ_BAND_COUNT`] EQ bands.
///
/// A standard ISO octave-spaced 10-band layout, the same set hardware graphic
/// equalizers use.
pub const EQ_BAND_FREQUENCIES_HZ: [u32; EQ_BAND_COUNT] =
    [31, 62, 125, 250, 500, 1_000, 2_000, 4_000, 8_000, 16_000];

/// The gain limit, in decibels, each EQ band can be cut or boosted by.
pub const EQ_GAIN_LIMIT_DB: f32 = 12.0;

/// A named equalizer preset: a fixed set of per-band gains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqPreset {
    /// Every band at 0 dB.
    Flat,
    /// Boosted lows and highs — a smiling response curve.
    BassBoost,
    /// Lifted presence and treble for clearer vocals.
    Vocal,
    /// Gently lifted treble.
    Treble,
}

impl EqPreset {
    /// Every preset, in menu order.
    #[must_use]
    pub fn all() -> [EqPreset; 4] {
        [
            EqPreset::Flat,
            EqPreset::BassBoost,
            EqPreset::Vocal,
            EqPreset::Treble,
        ]
    }

    /// A human-readable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            EqPreset::Flat => "Flat",
            EqPreset::BassBoost => "Bass boost",
            EqPreset::Vocal => "Vocal",
            EqPreset::Treble => "Treble",
        }
    }

    /// This preset's per-band gains, in decibels, low-to-high.
    #[must_use]
    pub fn band_gains_db(self) -> [f32; EQ_BAND_COUNT] {
        match self {
            EqPreset::Flat => [0.0; EQ_BAND_COUNT],
            EqPreset::BassBoost => [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 1.0, 2.5, 4.0, 5.0],
            EqPreset::Vocal => [-2.0, -1.5, 0.0, 1.5, 3.0, 3.5, 3.0, 1.5, 0.0, -1.0],
            EqPreset::Treble => [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.5, 4.0, 5.0, 6.0],
        }
    }
}

/// Graphic-equalizer settings: an on/off toggle and the per-band gains.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct EqualizerSettings {
    /// Whether the equalizer is enabled.
    pub enabled: bool,
    /// The per-band gains, in decibels, ordered low-to-high to match
    /// [`EQ_BAND_FREQUENCIES_HZ`]. Each is clamped to ±[`EQ_GAIN_LIMIT_DB`].
    pub band_gains_db: [f32; EQ_BAND_COUNT],
}

impl EqualizerSettings {
    /// Apply a preset's gain curve to the bands.
    pub fn apply_preset(&mut self, preset: EqPreset) {
        self.band_gains_db = preset.band_gains_db();
    }

    /// Clamp every band gain into the legal ±[`EQ_GAIN_LIMIT_DB`] range.
    ///
    /// Called after loading a persisted config so a hand-edited or
    /// out-of-range value can never drive the DSP stage.
    pub fn clamp_bands(&mut self) {
        for gain in &mut self.band_gains_db {
            *gain = gain.clamp(-EQ_GAIN_LIMIT_DB, EQ_GAIN_LIMIT_DB);
        }
    }
}

/// The list of local-music folders the user has registered.
///
/// Only the folder list is in scope here; scanning the folders and playing
/// their contents is a later workstream. Paths are stored verbatim.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalFilesSettings {
    /// The registered local-music folder paths.
    pub folders: Vec<PathBuf>,
}

impl LocalFilesSettings {
    /// Add `folder` to the list if it is not already present.
    ///
    /// Returns `true` when the folder was newly added.
    pub fn add_folder(&mut self, folder: PathBuf) -> bool {
        if self.folders.contains(&folder) {
            return false;
        }
        self.folders.push(folder);
        true
    }

    /// Remove the folder at `index`, if it is in range.
    pub fn remove_folder(&mut self, index: usize) {
        if index < self.folders.len() {
            self.folders.remove(index);
        }
    }
}

/// Desktop-integration settings (Phase 12).
///
/// Currently a single toggle: whether a desktop notification fires on every
/// track change. **Off by default** — notifications are opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NotificationSettings {
    /// Whether to raise a desktop notification when the playing track changes.
    #[serde(default)]
    pub track_change: bool,
}

/// Which lyrics provider the Lyrics panel uses.
///
/// Re-exports [`spottyfi_api::lyrics::LyricsProvider`] under a settings name;
/// the api enum is already a `serde` type, so it persists directly.
pub type LyricsProvider = spottyfi_api::lyrics::LyricsProvider;

/// Lyrics settings: the chosen provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LyricsSettings {
    /// Which provider to source lyrics from.
    ///
    /// [`LyricsProvider::Auto`] (the default) tries every configured provider,
    /// lrclib first.
    #[serde(default)]
    pub provider: LyricsProvider,
}

/// Every selectable lyrics provider, in menu order, with a display label.
///
/// Used to build the Settings page's provider selector.
#[must_use]
pub fn lyrics_provider_choices() -> [(LyricsProvider, &'static str); 4] {
    [
        (LyricsProvider::Auto, "Automatic (recommended)"),
        (LyricsProvider::Lrclib, "lrclib.net"),
        (LyricsProvider::Musixmatch, "Musixmatch"),
        (LyricsProvider::SpotifyInternal, "Spotify (internal)"),
    ]
}

/// A human-readable label for one [`LyricsProvider`].
#[must_use]
pub fn lyrics_provider_label(provider: LyricsProvider) -> &'static str {
    lyrics_provider_choices()
        .into_iter()
        .find(|(candidate, _)| *candidate == provider)
        .map_or("Automatic (recommended)", |(_, label)| label)
}

/// The full persisted user-settings model surfaced on the Settings page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    /// Playback / audio engine settings.
    #[serde(default)]
    pub audio: AudioSettings,
    /// Graphic-equalizer settings.
    #[serde(default)]
    pub equalizer: EqualizerSettings,
    /// Registered local-music folders.
    #[serde(default)]
    pub local_files: LocalFilesSettings,
    /// Desktop-integration / notification settings.
    #[serde(default)]
    pub notifications: NotificationSettings,
    /// Lyrics settings — the chosen lyrics provider.
    #[serde(default)]
    pub lyrics: LyricsSettings,
    /// The user-rebindable keyboard shortcuts.
    #[serde(default)]
    pub hotkeys: crate::hotkeys::HotkeyMap,
}

impl AppSettings {
    /// Normalise a freshly loaded config so out-of-range persisted values can
    /// never reach the engine or the (future) EQ DSP.
    pub fn sanitise(&mut self) {
        self.equalizer.clamp_bands();
        self.audio.crossfade_seconds = self
            .audio
            .crossfade_seconds
            .clamp(0.0, AudioSettings::MAX_CROSSFADE_SECONDS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let settings = AppSettings::default();
        assert_eq!(settings.audio.quality, StreamTier::High);
        assert!(!settings.audio.normalisation);
        assert_eq!(settings.audio.crossfade_seconds, 0.0);
        assert!(!settings.equalizer.enabled);
        assert_eq!(settings.equalizer.band_gains_db, [0.0; EQ_BAND_COUNT]);
        assert!(settings.local_files.folders.is_empty());
        // Track-change notifications are opt-in — off by default.
        assert!(!settings.notifications.track_change);
        // Lyrics default to the Automatic provider order (lrclib first).
        assert_eq!(settings.lyrics.provider, LyricsProvider::Auto);
        // The hotkey map starts at its first-launch defaults.
        assert_eq!(settings.hotkeys, crate::hotkeys::HotkeyMap::default());
    }

    #[test]
    fn lyrics_provider_choice_round_trips_through_ron() {
        let settings = AppSettings {
            lyrics: LyricsSettings {
                provider: LyricsProvider::Lrclib,
            },
            ..AppSettings::default()
        };
        let text = ron::ser::to_string(&settings).expect("serialise settings");
        let restored: AppSettings = ron::from_str(&text).expect("deserialise settings");
        assert_eq!(restored.lyrics.provider, LyricsProvider::Lrclib);
    }

    #[test]
    fn every_lyrics_provider_has_a_label() {
        for (provider, label) in lyrics_provider_choices() {
            assert!(!label.is_empty());
            assert_eq!(lyrics_provider_label(provider), label);
        }
    }

    #[test]
    fn settings_round_trip_through_ron() {
        let mut settings = AppSettings {
            audio: AudioSettings {
                quality: StreamTier::Normal,
                normalisation: true,
                crossfade_seconds: 4.5,
            },
            equalizer: EqualizerSettings {
                enabled: true,
                ..EqualizerSettings::default()
            },
            ..AppSettings::default()
        };
        settings.equalizer.apply_preset(EqPreset::BassBoost);
        settings
            .local_files
            .add_folder(PathBuf::from("/music/library"));

        let text = ron::ser::to_string(&settings).expect("serialise settings");
        let restored: AppSettings = ron::from_str(&text).expect("deserialise settings");
        assert_eq!(settings, restored);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // A pre-WS5 config has no `settings` block at all; an empty struct
        // literal must still deserialise via the `#[serde(default)]` fields.
        let restored: AppSettings = ron::from_str("()").expect("deserialise empty settings");
        assert_eq!(restored, AppSettings::default());
    }

    #[test]
    fn sanitise_clamps_out_of_range_values() {
        let mut settings = AppSettings::default();
        settings.equalizer.band_gains_db[0] = 999.0;
        settings.equalizer.band_gains_db[1] = -999.0;
        settings.audio.crossfade_seconds = 100.0;
        settings.sanitise();
        assert_eq!(settings.equalizer.band_gains_db[0], EQ_GAIN_LIMIT_DB);
        assert_eq!(settings.equalizer.band_gains_db[1], -EQ_GAIN_LIMIT_DB);
        assert_eq!(
            settings.audio.crossfade_seconds,
            AudioSettings::MAX_CROSSFADE_SECONDS
        );
    }

    #[test]
    fn presets_have_full_band_curves() {
        for preset in EqPreset::all() {
            assert_eq!(preset.band_gains_db().len(), EQ_BAND_COUNT);
        }
        assert_eq!(EqPreset::Flat.band_gains_db(), [0.0; EQ_BAND_COUNT]);
    }

    #[test]
    fn local_folders_dedupe() {
        let mut local = LocalFilesSettings::default();
        assert!(local.add_folder(PathBuf::from("/a")));
        assert!(!local.add_folder(PathBuf::from("/a")));
        assert!(local.add_folder(PathBuf::from("/b")));
        assert_eq!(local.folders.len(), 2);
        local.remove_folder(0);
        assert_eq!(local.folders, vec![PathBuf::from("/b")]);
    }

    #[test]
    fn engine_config_maps_quality_and_normalisation() {
        let audio = AudioSettings {
            quality: StreamTier::Low,
            normalisation: true,
            ..AudioSettings::default()
        };
        let cfg = audio.engine_config();
        assert_eq!(cfg.quality, StreamQuality::Low);
        assert!(cfg.normalisation);
    }
}
