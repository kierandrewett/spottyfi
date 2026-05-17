//! Engine-level audio configuration applied when the librespot session starts.
//!
//! librespot builds its [`PlayerConfig`](librespot::playback::config::PlayerConfig)
//! once, at connect time — bitrate and normalisation are baked in then. So
//! these settings are **start-time** configuration: changing them takes effect
//! the next time the engine connects (a logout/login, or an explicit engine
//! restart), not live. The UI surfaces that caveat.

/// The streaming bitrate Spottyfi requests from Spotify.
///
/// Maps to librespot's three `Bitrate` tiers; Spotify streams Ogg Vorbis at
/// every tier, so this only changes the quality/bandwidth trade-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamQuality {
    /// 96 kbps — the low tier, least bandwidth.
    Low,
    /// 160 kbps — the normal tier.
    Normal,
    /// 320 kbps — the high tier; what Spottyfi has always shipped.
    #[default]
    High,
}

impl StreamQuality {
    /// The bitrate in kilobits per second.
    #[must_use]
    pub fn kbps(self) -> u16 {
        match self {
            StreamQuality::Low => 96,
            StreamQuality::Normal => 160,
            StreamQuality::High => 320,
        }
    }
}

/// Start-time configuration for the librespot engine.
///
/// Constructed by `app` from the persisted user settings and handed to
/// [`PlaybackController::start`](crate::PlaybackController::start). All fields
/// are applied when the librespot `Player` is built and cannot change live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EngineConfig {
    /// The streaming bitrate to request.
    pub quality: StreamQuality,
    /// Whether librespot's volume normalisation is enabled.
    pub normalisation: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_tiers_map_to_kbps() {
        assert_eq!(StreamQuality::Low.kbps(), 96);
        assert_eq!(StreamQuality::Normal.kbps(), 160);
        assert_eq!(StreamQuality::High.kbps(), 320);
    }

    #[test]
    fn default_engine_config_is_high_no_normalisation() {
        let cfg = EngineConfig::default();
        assert_eq!(cfg.quality, StreamQuality::High);
        assert!(!cfg.normalisation);
    }
}
