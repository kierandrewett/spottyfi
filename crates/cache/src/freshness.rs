//! The stale-while-revalidate freshness policy.
//!
//! A cached object carries a `last_fetched` Unix timestamp. Within the
//! [`Freshness::window`] it is considered *fresh* and is served without a
//! refresh. Past the window it is *stale*: the cached value is still served
//! immediately (so the UI never blocks), but the caller should trigger a
//! background refresh.

use std::time::Duration;

/// How a cached object's age compares to the freshness window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staleness {
    /// The object is within the freshness window; no refresh needed.
    Fresh,
    /// The object is past the freshness window; serve it but refresh in the
    /// background.
    Stale,
}

impl Staleness {
    /// Whether a background refresh should be triggered.
    #[must_use]
    pub fn should_revalidate(self) -> bool {
        matches!(self, Staleness::Stale)
    }
}

/// The freshness window for a cache.
#[derive(Debug, Clone, Copy)]
pub struct Freshness {
    /// The duration a cached object stays fresh after `last_fetched`.
    window: Duration,
}

/// The default freshness window: one hour.
///
/// Spotify catalogue objects (albums, artists, tracks) change rarely, so an
/// hour keeps the UI snappy without serving badly outdated data.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60 * 60);

impl Default for Freshness {
    fn default() -> Self {
        Self {
            window: DEFAULT_WINDOW,
        }
    }
}

impl Freshness {
    /// Build a freshness policy with an explicit window.
    #[must_use]
    pub fn new(window: Duration) -> Self {
        Self { window }
    }

    /// The freshness window.
    #[must_use]
    pub fn window(&self) -> Duration {
        self.window
    }

    /// Classify an object given its `last_fetched` time and the current time,
    /// both as Unix timestamps in seconds.
    ///
    /// A `last_fetched` in the future (clock skew) is treated as [`Fresh`]; a
    /// negative age cannot make an object stale.
    ///
    /// [`Fresh`]: Staleness::Fresh
    #[must_use]
    pub fn classify(&self, last_fetched: i64, now: i64) -> Staleness {
        let age_secs = now.saturating_sub(last_fetched);
        if age_secs < 0 {
            return Staleness::Fresh;
        }
        let age = Duration::from_secs(age_secs.unsigned_abs());
        if age <= self.window {
            Staleness::Fresh
        } else {
            Staleness::Stale
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_the_window_is_fresh() {
        let freshness = Freshness::new(Duration::from_secs(100));
        assert_eq!(freshness.classify(1_000, 1_050), Staleness::Fresh);
        // Exactly at the window boundary is still fresh.
        assert_eq!(freshness.classify(1_000, 1_100), Staleness::Fresh);
    }

    #[test]
    fn past_the_window_is_stale() {
        let freshness = Freshness::new(Duration::from_secs(100));
        assert_eq!(freshness.classify(1_000, 1_101), Staleness::Stale);
        assert!(freshness.classify(1_000, 5_000).should_revalidate());
    }

    #[test]
    fn future_timestamp_from_clock_skew_is_fresh() {
        let freshness = Freshness::new(Duration::from_secs(100));
        assert_eq!(freshness.classify(2_000, 1_000), Staleness::Fresh);
    }

    #[test]
    fn fresh_does_not_revalidate() {
        assert!(!Staleness::Fresh.should_revalidate());
        assert!(Staleness::Stale.should_revalidate());
    }
}
