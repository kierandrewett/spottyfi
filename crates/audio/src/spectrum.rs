//! The off-UI-thread spectrum analyser — the data half of WS7's visualiser.
//!
//! # What this is
//!
//! The [`AudioTap`](crate::AudioTap) hands the UI raw, FFT-ready mono samples,
//! but the UI thread must not run an FFT every frame. This module owns a small
//! tokio task that:
//!
//! 1. reads the tap's `spectrum` window once per tick (~60 Hz),
//! 2. windows it, runs one real-input FFT ([`rustfft`]),
//! 3. folds the linear FFT bins into a fixed set of **log-spaced frequency
//!    bands** (so bass and treble each get fair screen width),
//! 4. converts each band to a smoothed, decaying `0.0..=1.0` magnitude, and
//! 5. publishes an immutable [`SpectrumSnapshot`] through an [`ArcSwap`].
//!
//! The UI's visualiser panel reads that snapshot with a single lock-free load
//! per frame, exactly like [`AudioTap::snapshot`](crate::AudioTap::snapshot).
//!
//! # Idle behaviour
//!
//! When playback stalls the tap's `sequence` stops advancing. The analyser
//! notices, feeds the FFT silence, and the per-band smoothing decays every bar
//! toward zero — the visualiser settles to a flat idle baseline rather than
//! freezing on the last loud frame.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;

use crate::tap::{AudioTap, SPECTRUM_LEN};

/// The number of log-spaced frequency bands the analyser publishes.
///
/// A dense bar count — the visualiser draws one thin bar per band, matching the
/// flat, dense aesthetic in `docs/ui-reference.md`.
pub const BAND_COUNT: usize = 64;

/// The lowest frequency the band scale starts at, in hertz.
///
/// Below this is mostly DC / rumble and not musically interesting on a bar
/// display.
const MIN_FREQ_HZ: f32 = 30.0;

/// The highest frequency the band scale reaches, in hertz.
///
/// Comfortably above the top of musical content while staying under Nyquist
/// for a 44.1 kHz stream.
const MAX_FREQ_HZ: f32 = 16_000.0;

/// How fast a band rises toward a louder value, per published frame (`0..1`).
///
/// Near `1.0` so transients punch through promptly.
const ATTACK: f32 = 0.55;

/// How fast a band falls toward a quieter value, per published frame (`0..1`).
///
/// Small, so bars decay smoothly rather than snapping down — the classic
/// spectrum-analyser "falling bar" feel.
const RELEASE: f32 = 0.16;

/// Reference level (full-scale magnitude) the dB conversion is relative to.
const REFERENCE: f32 = 1.0;

/// The dB floor mapped to `0.0`; anything quieter clamps to silence.
const FLOOR_DB: f32 = -72.0;

/// An immutable, cheap-to-clone snapshot of the analysed spectrum.
///
/// This is the visualiser panel's contract: obtain the shared handle once via
/// [`SpectrumAnalyzer::new`], then call [`SpectrumAnalyzer::snapshot`] once per
/// UI frame.
#[derive(Debug, Clone)]
pub struct SpectrumSnapshot {
    /// Monotonically increasing publish counter. Two reads with the same value
    /// observed the same analysis — the panel can hold its repaint cadence to
    /// the analyser's rather than spinning.
    pub sequence: u64,
    /// Per-band magnitudes in `0.0..=1.0`, low frequency first. Always exactly
    /// [`BAND_COUNT`] long. Smoothed and decaying — safe to draw directly as
    /// bar heights.
    pub bands: Vec<f32>,
    /// A downsampled copy of the most recent raw mono waveform in `-1.0..=1.0`,
    /// oldest first — drives the visualiser's optional oscilloscope mode.
    pub scope: Vec<f32>,
    /// Whether the analyser currently sees live audio. `false` once playback
    /// has stalled and the bars have been fed silence.
    pub active: bool,
}

impl SpectrumSnapshot {
    /// An all-zero snapshot — the idle / pre-playback state.
    #[must_use]
    fn silent() -> Self {
        Self {
            sequence: 0,
            bands: vec![0.0; BAND_COUNT],
            scope: Vec::new(),
            active: false,
        }
    }
}

impl Default for SpectrumSnapshot {
    fn default() -> Self {
        Self::silent()
    }
}

/// Shared handle to the spectrum analyser's published output.
///
/// Cloning is cheap (an `Arc` bump). The analyser task holds one clone and
/// publishes; the UI holds another and reads.
#[derive(Clone)]
pub struct SpectrumAnalyzer {
    /// The latest published snapshot, swapped by the analyser task.
    published: Arc<ArcSwap<SpectrumSnapshot>>,
    /// The publish counter, bumped on every store.
    sequence: Arc<AtomicU64>,
}

impl SpectrumAnalyzer {
    /// Create a fresh analyser with a silent initial snapshot.
    #[must_use]
    pub fn new() -> Self {
        Self {
            published: Arc::new(ArcSwap::from_pointee(SpectrumSnapshot::silent())),
            sequence: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Read the most recent spectrum snapshot — a single atomic load.
    #[must_use]
    pub fn snapshot(&self) -> Arc<SpectrumSnapshot> {
        self.published.load_full()
    }

    /// Run the analysis loop until the future is dropped.
    ///
    /// Spawn this on the tokio runtime (see
    /// [`SpectrumAnalyzer::spawn`]). It reads `tap`, runs one FFT per tick and
    /// publishes a smoothed [`SpectrumSnapshot`]; `wake` is invoked after each
    /// publish so a UI can repaint.
    async fn run(self, tap: AudioTap, wake: impl Fn() + Send + 'static) {
        let mut engine = AnalysisEngine::new();
        // ~60 Hz: matches the tap's publish cadence so no frame is missed and
        // none is processed twice needlessly.
        let mut tick = tokio::time::interval(Duration::from_millis(16));
        let mut last_tap_seq = u64::MAX;

        loop {
            tick.tick().await;
            let tap_snapshot = tap.snapshot();
            // The tap's sequence is unchanged ⇒ playback stalled / paused.
            let active = tap_snapshot.sequence != last_tap_seq && tap_snapshot.sequence != 0;
            last_tap_seq = tap_snapshot.sequence;

            let snapshot = engine.analyse(&tap_snapshot.spectrum, active);
            let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
            self.published.store(Arc::new(SpectrumSnapshot {
                sequence,
                ..snapshot
            }));
            wake();
        }
    }

    /// Spawn the analysis loop on `runtime`, returning the read handle.
    ///
    /// The returned [`SpectrumAnalyzer`] shares the published snapshot with the
    /// spawned task; the task lives for the lifetime of the runtime. `wake` is
    /// called after every publish — pass a closure that requests an egui
    /// repaint.
    #[must_use]
    pub fn spawn(
        runtime: &tokio::runtime::Handle,
        tap: AudioTap,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let analyzer = Self::new();
        runtime.spawn(analyzer.clone().run(tap, wake));
        analyzer
    }

    /// Spawn an analysis loop that publishes into an *existing* analyser.
    ///
    /// Use this when the read handle must be created up front (so the UI has a
    /// stable handle) but the analysis task can only start once the audio tap
    /// exists — i.e. when the engine is started or restarted. The new task
    /// publishes into `analyzer`; an earlier task left running simply stops
    /// having any visible effect once this one takes over the snapshot.
    pub fn spawn_into(
        analyzer: &Self,
        runtime: &tokio::runtime::Handle,
        tap: AudioTap,
        wake: impl Fn() + Send + 'static,
    ) {
        runtime.spawn(analyzer.clone().run(tap, wake));
    }
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// The number of waveform points the oscilloscope mode keeps.
const SCOPE_LEN: usize = 256;

/// The stateful FFT + band-folding + smoothing engine.
///
/// Held by the analyser task across ticks so the smoothing has memory. Kept
/// separate from [`SpectrumAnalyzer`] so the pure DSP can be unit-tested
/// without a tokio runtime.
struct AnalysisEngine {
    /// The configured real-input FFT, planned once.
    fft: Arc<dyn rustfft::Fft<f32>>,
    /// A precomputed Hann window, one coefficient per input sample.
    window: Vec<f32>,
    /// Reusable FFT scratch — the windowed, complex-promoted input.
    buffer: Vec<Complex32>,
    /// The previous frame's smoothed band magnitudes, the smoothing memory.
    smoothed: Vec<f32>,
    /// For each output band, the half-open range of FFT bins folded into it.
    band_bins: Vec<(usize, usize)>,
}

impl AnalysisEngine {
    /// Build the engine for a [`SPECTRUM_LEN`]-sample, 44.1 kHz stream.
    fn new() -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(SPECTRUM_LEN);
        Self {
            fft,
            window: hann_window(SPECTRUM_LEN),
            buffer: vec![Complex32::default(); SPECTRUM_LEN],
            smoothed: vec![0.0; BAND_COUNT],
            band_bins: band_bin_ranges(SPECTRUM_LEN, 44_100),
        }
    }

    /// Analyse one window of raw mono samples into a [`SpectrumSnapshot`].
    ///
    /// `active` is whether the tap reported fresh audio; when `false` the FFT
    /// is fed silence so every band decays toward zero.
    fn analyse(&mut self, samples: &[f32], active: bool) -> SpectrumSnapshot {
        // Window the input into the complex scratch buffer. A short or empty
        // tap window (pre-playback) is zero-padded; stale audio while paused is
        // ignored so the bars fall rather than holding a frozen frame.
        for (i, slot) in self.buffer.iter_mut().enumerate() {
            let sample = if active {
                samples.get(i).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            *slot = Complex32::new(sample * self.window[i], 0.0);
        }
        self.fft.process(&mut self.buffer);

        // Fold the linear FFT bins into log-spaced bands and smooth each.
        // FFT magnitudes scale with the window length; normalise by it.
        let norm = 2.0 / SPECTRUM_LEN as f32;
        for (band, &(lo, hi)) in self.band_bins.iter().enumerate() {
            let mut peak = 0.0_f32;
            for bin in lo..hi {
                let mag = self.buffer[bin].norm() * norm;
                peak = peak.max(mag);
            }
            let target = magnitude_to_unit(peak);
            let prev = self.smoothed[band];
            // Asymmetric smoothing: snap up on attack, glide down on release.
            let coeff = if target > prev { ATTACK } else { RELEASE };
            self.smoothed[band] = prev + (target - prev) * coeff;
        }

        let scope = if active {
            downsample(samples, SCOPE_LEN)
        } else {
            Vec::new()
        };

        SpectrumSnapshot {
            sequence: 0,
            bands: self.smoothed.clone(),
            scope,
            active: active && self.smoothed.iter().any(|&b| b > 0.001),
        }
    }
}

/// Build a Hann window of `len` coefficients.
///
/// Tapering the FFT input suppresses spectral leakage so the bars are clean.
fn hann_window(len: usize) -> Vec<f32> {
    if len <= 1 {
        return vec![1.0; len];
    }
    let denom = (len - 1) as f32;
    (0..len)
        .map(|i| {
            let phase = std::f32::consts::PI * i as f32 / denom;
            phase.sin().powi(2)
        })
        .collect()
}

/// Compute, for each of [`BAND_COUNT`] log-spaced bands, the half-open range
/// `[lo, hi)` of FFT bins that fall inside it.
///
/// Bands are spaced geometrically between [`MIN_FREQ_HZ`] and [`MAX_FREQ_HZ`];
/// the bin index for a frequency `f` is `f * fft_len / sample_rate`. Every band
/// is guaranteed to span at least one bin (`hi > lo`) so no band is ever empty.
fn band_bin_ranges(fft_len: usize, sample_rate: u32) -> Vec<(usize, usize)> {
    let nyquist_bin = fft_len / 2;
    let hz_to_bin = |hz: f32| {
        let bin = (hz * fft_len as f32 / sample_rate as f32).round() as usize;
        bin.clamp(1, nyquist_bin)
    };
    let ratio = (MAX_FREQ_HZ / MIN_FREQ_HZ).powf(1.0 / BAND_COUNT as f32);

    let mut ranges = Vec::with_capacity(BAND_COUNT);
    for band in 0..BAND_COUNT {
        let lo_hz = MIN_FREQ_HZ * ratio.powi(band as i32);
        let hi_hz = MIN_FREQ_HZ * ratio.powi(band as i32 + 1);
        let lo = hz_to_bin(lo_hz);
        let hi = hz_to_bin(hi_hz).max(lo + 1).min(nyquist_bin + 1);
        ranges.push((lo, hi));
    }
    ranges
}

/// Map a raw linear FFT magnitude to a `0.0..=1.0` display value.
///
/// The conversion is logarithmic (decibels): hearing is roughly logarithmic,
/// so a dB scale gives a far more readable bar display than a linear one.
/// `0.0` is the [`FLOOR_DB`] noise floor, `1.0` is the [`REFERENCE`] level.
fn magnitude_to_unit(magnitude: f32) -> f32 {
    if magnitude <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * (magnitude / REFERENCE).log10();
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// Downsample `samples` to at most `target` points by peak-per-slice picking.
///
/// Peak picking (not averaging) keeps the oscilloscope trace lively rather than
/// flattening transients.
fn downsample(samples: &[f32], target: usize) -> Vec<f32> {
    if samples.is_empty() || target == 0 {
        return Vec::new();
    }
    if samples.len() <= target {
        return samples.to_vec();
    }
    let slice = samples.len() / target;
    (0..target)
        .map(|i| {
            let start = i * slice;
            let end = (start + slice).min(samples.len());
            samples[start..end]
                .iter()
                .copied()
                .max_by(|a, b| a.abs().total_cmp(&b.abs()))
                .unwrap_or(0.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_analyzer_snapshot_is_silent() {
        let analyzer = SpectrumAnalyzer::new();
        let snap = analyzer.snapshot();
        assert_eq!(snap.sequence, 0);
        assert_eq!(snap.bands.len(), BAND_COUNT);
        assert!(snap.bands.iter().all(|&b| b == 0.0));
        assert!(!snap.active);
    }

    #[test]
    fn hann_window_tapers_to_zero_at_the_edges() {
        let window = hann_window(SPECTRUM_LEN);
        assert_eq!(window.len(), SPECTRUM_LEN);
        assert!(window[0] < 1e-6, "window starts at zero");
        assert!(*window.last().unwrap() < 1e-6, "window ends at zero");
        // Peaks in the middle, at ~1.0.
        assert!((window[SPECTRUM_LEN / 2] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn band_ranges_cover_all_bands_and_are_non_empty() {
        let ranges = band_bin_ranges(SPECTRUM_LEN, 44_100);
        assert_eq!(ranges.len(), BAND_COUNT);
        let nyquist_bin = SPECTRUM_LEN / 2;
        for &(lo, hi) in &ranges {
            assert!(hi > lo, "every band spans at least one bin");
            assert!(lo >= 1, "DC bin is excluded");
            assert!(hi <= nyquist_bin + 1, "bands stay under Nyquist");
        }
    }

    #[test]
    fn band_ranges_are_monotonically_increasing_in_frequency() {
        // A log axis must place each band no lower than the previous one.
        let ranges = band_bin_ranges(SPECTRUM_LEN, 44_100);
        for pair in ranges.windows(2) {
            assert!(
                pair[1].0 >= pair[0].0,
                "band start frequencies do not decrease: {:?} then {:?}",
                pair[0],
                pair[1],
            );
        }
    }

    #[test]
    fn band_ranges_are_log_spaced_not_linear() {
        // On a log axis the high-frequency bands each cover many more linear
        // FFT bins than the low ones — that is the whole point of log spacing.
        let ranges = band_bin_ranges(SPECTRUM_LEN, 44_100);
        let first_width = ranges[0].1 - ranges[0].0;
        let last_width = ranges[BAND_COUNT - 1].1 - ranges[BAND_COUNT - 1].0;
        assert!(
            last_width > first_width,
            "top band ({last_width} bins) should be wider than the bottom ({first_width} bins)",
        );
    }

    #[test]
    fn magnitude_to_unit_maps_floor_and_reference() {
        // Silence is zero; the reference level is full-scale.
        assert_eq!(magnitude_to_unit(0.0), 0.0);
        assert!((magnitude_to_unit(REFERENCE) - 1.0).abs() < 1e-6);
        // The noise floor maps to ~0.0.
        let floor_mag = 10.0_f32.powf(FLOOR_DB / 20.0);
        assert!(magnitude_to_unit(floor_mag) < 1e-4);
        // A mid-level signal lands strictly inside the unit range.
        let mid = magnitude_to_unit(0.1);
        assert!(mid > 0.0 && mid < 1.0, "mid-level mapped to {mid}");
    }

    #[test]
    fn magnitude_to_unit_is_monotonic() {
        // Louder input must never map to a shorter bar.
        let mut prev = magnitude_to_unit(0.0);
        for step in 1..=100 {
            let next = magnitude_to_unit(step as f32 / 100.0);
            assert!(next >= prev, "not monotonic at step {step}");
            prev = next;
        }
    }

    #[test]
    fn analyse_silence_leaves_every_band_at_zero() {
        let mut engine = AnalysisEngine::new();
        let snap = engine.analyse(&[], false);
        assert_eq!(snap.bands.len(), BAND_COUNT);
        assert!(snap.bands.iter().all(|&b| b == 0.0));
        assert!(!snap.active);
        assert!(snap.scope.is_empty());
    }

    #[test]
    fn analyse_detects_a_tone_in_the_expected_band() {
        // Synthesise a 1 kHz sine and confirm the loudest band straddles it.
        let mut engine = AnalysisEngine::new();
        let freq = 1_000.0_f32;
        let samples: Vec<f32> = (0..SPECTRUM_LEN)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / 44_100.0).sin() * 0.8)
            .collect();
        // Two passes so the attack smoothing settles onto the tone.
        engine.analyse(&samples, true);
        let snap = engine.analyse(&samples, true);

        let loudest = snap
            .bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .expect("a loudest band");
        let ranges = band_bin_ranges(SPECTRUM_LEN, 44_100);
        let (lo, hi) = ranges[loudest];
        let lo_hz = lo as f32 * 44_100.0 / SPECTRUM_LEN as f32;
        let hi_hz = hi as f32 * 44_100.0 / SPECTRUM_LEN as f32;
        // The 1 kHz tone should fall within a couple of bands of the peak.
        assert!(
            (lo_hz..=hi_hz).contains(&freq) || (700.0..1400.0).contains(&((lo_hz + hi_hz) / 2.0)),
            "loudest band {loudest} spans {lo_hz:.0}..{hi_hz:.0} Hz, expected ~{freq} Hz",
        );
        assert!(snap.active, "a loud tone is active");
    }

    #[test]
    fn analyse_release_decays_bars_after_audio_stops() {
        // Drive the bars up with a tone, then feed silence and confirm decay.
        let mut engine = AnalysisEngine::new();
        let samples: Vec<f32> = (0..SPECTRUM_LEN)
            .map(|i| (2.0 * std::f32::consts::PI * 1_000.0 * i as f32 / 44_100.0).sin())
            .collect();
        for _ in 0..8 {
            engine.analyse(&samples, true);
        }
        let loud = engine.smoothed.iter().copied().fold(0.0, f32::max);
        assert!(loud > 0.0, "tone should have raised some band");

        let after = engine.analyse(&[], false);
        let decayed = after.bands.iter().copied().fold(0.0, f32::max);
        assert!(decayed < loud, "bars should decay once audio stops");
    }

    #[test]
    fn downsample_picks_peaks_and_caps_length() {
        let samples: Vec<f32> = (0..2048).map(|i| (i as f32 / 2048.0) - 0.5).collect();
        let out = downsample(&samples, 256);
        assert_eq!(out.len(), 256);
        // A short input is returned untouched.
        assert_eq!(downsample(&[0.1, 0.2], 256), vec![0.1, 0.2]);
        assert!(downsample(&[], 256).is_empty());
    }
}
