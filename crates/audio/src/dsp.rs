//! The 10-band graphic-equaliser DSP.
//!
//! [`Equalizer`] is a bank of ten biquad **peaking** filters, one per ISO
//! octave centre frequency, applied in series to each audio channel. The
//! per-band gains come straight from the persisted `EqualizerSettings`
//! (`app`), pushed in via [`PlaybackController::set_equalizer`].
//!
//! # Bypass
//!
//! When the equaliser is disabled the [`Equalizer::process`] call is a single
//! boolean check and returns immediately — true bypass, no per-sample cost.
//! The custom sink also skips constructing/draining filter state in that case.
//!
//! # Filter design
//!
//! Each band is a Robert Bristow-Johnson "Audio EQ Cookbook" peaking-EQ
//! biquad. The cookbook transfer function is
//!
//! ```text
//!   H(z) = (b0 + b1 z^-1 + b2 z^-2) / (a0 + a1 z^-1 + a2 z^-2)
//! ```
//!
//! with, for a peaking filter at centre frequency `f0`, quality `Q` and
//! linear gain `A = 10^(dB/40)`:
//!
//! ```text
//!   w0    = 2*pi*f0 / fs
//!   alpha = sin(w0) / (2*Q)
//!   b0 = 1 + alpha*A      b1 = -2*cos(w0)     b2 = 1 - alpha*A
//!   a0 = 1 + alpha/A      a1 = -2*cos(w0)     a2 = 1 - alpha/A
//! ```
//!
//! A peaking filter leaves the signal unchanged at 0 dB gain, so a flat EQ is
//! transparent regardless of the bypass flag — the bypass is purely a cost
//! optimisation.

/// The number of equaliser bands. Matches `app`'s `EQ_BAND_COUNT`.
pub const BAND_COUNT: usize = 10;

/// The ISO octave centre frequencies, in hertz, of the ten bands.
///
/// Identical to `app`'s `EQ_BAND_FREQUENCIES_HZ`; duplicated here because the
/// `audio` crate does not depend on `app`.
pub const BAND_FREQUENCIES_HZ: [f32; BAND_COUNT] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
];

/// The shared quality factor for every band.
///
/// `Q ≈ 1.41` gives each peaking filter roughly a one-octave bandwidth, so
/// adjacent bands overlap smoothly — the standard choice for an octave-spaced
/// graphic equaliser. A higher `Q` would leave dips between the bands; a lower
/// one would make neighbouring sliders fight each other.
const BAND_Q: f32 = 1.41;

/// The five coefficients of a single normalised biquad section.
///
/// Stored already divided through by `a0`, so the difference equation is
/// `y = b0*x + b1*x1 + b2*x2 - a1*y1 - a2*y2`.
#[derive(Debug, Clone, Copy)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoeffs {
    /// The identity (pass-through) biquad: `y = x`.
    const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Audio-EQ-cookbook peaking-filter coefficients.
    ///
    /// `freq_hz` is the band centre, `sample_rate_hz` the stream rate (44.1k),
    /// `gain_db` the boost (positive) or cut (negative) at the centre.
    fn peaking(freq_hz: f32, sample_rate_hz: f32, gain_db: f32) -> Self {
        // A flat band is exactly the identity filter — keeps a 0 dB slider
        // perfectly transparent and avoids any rounding drift.
        if gain_db.abs() < f32::EPSILON {
            return Self::IDENTITY;
        }
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * freq_hz / sample_rate_hz;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * BAND_Q);

        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

/// A single biquad section: coefficients plus its two-sample state.
///
/// One [`Biquad`] exists per band **per channel**, because the delay state
/// (`x1/x2/y1/y2`) must not be shared across channels.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    coeffs: BiquadCoeffs,
    /// Previous two input samples (`x[n-1]`, `x[n-2]`).
    x1: f32,
    x2: f32,
    /// Previous two output samples (`y[n-1]`, `y[n-2]`).
    y1: f32,
    y2: f32,
}

impl Biquad {
    /// A new section with the given coefficients and cleared state.
    fn new(coeffs: BiquadCoeffs) -> Self {
        Self {
            coeffs,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Process one sample through the Direct Form I difference equation.
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let c = &self.coeffs;
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// One channel's filter chain: [`BAND_COUNT`] biquads applied in series.
#[derive(Debug, Clone)]
struct ChannelChain {
    bands: [Biquad; BAND_COUNT],
}

impl ChannelChain {
    /// A chain built from a shared coefficient set, state cleared.
    fn new(coeffs: &[BiquadCoeffs; BAND_COUNT]) -> Self {
        Self {
            bands: coeffs.map(Biquad::new),
        }
    }

    /// Push `x` through every band in series and return the filtered sample.
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let mut sample = x;
        for band in &mut self.bands {
            sample = band.process(sample);
        }
        sample
    }
}

/// The 10-band graphic equaliser.
///
/// Holds an independent filter chain per channel so stereo imaging is
/// preserved. Construct one with [`Equalizer::new`]; reconfigure it live with
/// [`Equalizer::set_params`]; run audio through [`Equalizer::process`].
#[derive(Debug, Clone)]
pub struct Equalizer {
    /// The stream sample rate the coefficients were computed for.
    sample_rate_hz: f32,
    /// Whether processing is active. When `false`, [`Equalizer::process`] is a
    /// no-op — the true-bypass fast path.
    enabled: bool,
    /// Per-channel filter chains. Index by channel.
    channels: Vec<ChannelChain>,
    /// The most recent per-band coefficients, so a channel-count change can
    /// rebuild the chains without recomputing the maths.
    coeffs: [BiquadCoeffs; BAND_COUNT],
}

impl Equalizer {
    /// Build a flat (transparent) equaliser for `channel_count` channels at
    /// `sample_rate_hz`. Starts disabled.
    #[must_use]
    pub fn new(sample_rate_hz: u32, channel_count: usize) -> Self {
        let coeffs = [BiquadCoeffs::IDENTITY; BAND_COUNT];
        Self {
            sample_rate_hz: sample_rate_hz as f32,
            enabled: false,
            channels: vec![ChannelChain::new(&coeffs); channel_count.max(1)],
            coeffs,
        }
    }

    /// Reconfigure the equaliser from an on/off flag and ten band gains in dB.
    ///
    /// Recomputes every biquad's coefficients in place; the filter **state**
    /// (delay lines) is preserved so a live gain tweak does not click. Extra
    /// `gains_db` entries beyond [`BAND_COUNT`] are ignored; missing ones are
    /// treated as 0 dB.
    pub fn set_params(&mut self, enabled: bool, gains_db: &[f32]) {
        self.enabled = enabled;
        for (band, coeffs) in self.coeffs.iter_mut().enumerate() {
            let gain_db = gains_db.get(band).copied().unwrap_or(0.0);
            *coeffs =
                BiquadCoeffs::peaking(BAND_FREQUENCIES_HZ[band], self.sample_rate_hz, gain_db);
        }
        for channel in &mut self.channels {
            for (band, biquad) in channel.bands.iter_mut().enumerate() {
                biquad.coeffs = self.coeffs[band];
            }
        }
    }

    /// Ensure the equaliser has exactly `channel_count` channel chains.
    ///
    /// Called by the sink when it first learns the real channel count. New
    /// chains inherit the current coefficients with cleared state.
    fn ensure_channels(&mut self, channel_count: usize) {
        let channel_count = channel_count.max(1);
        if self.channels.len() == channel_count {
            return;
        }
        self.channels
            .resize_with(channel_count, || ChannelChain::new(&self.coeffs));
    }

    /// Equalise an interleaved buffer of `channel_count`-interleaved samples,
    /// in place.
    ///
    /// A true-bypass no-op when the equaliser is disabled. The buffer length
    /// need not be a whole number of frames — a trailing partial frame is
    /// filtered through the channels it covers.
    pub fn process(&mut self, samples: &mut [f32], channel_count: usize) {
        if !self.enabled {
            return;
        }
        let channel_count = channel_count.max(1);
        self.ensure_channels(channel_count);
        for (i, sample) in samples.iter_mut().enumerate() {
            let channel = i % channel_count;
            *sample = self.channels[channel].process(*sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sample rate every Spotify stream decodes to.
    const FS: u32 = 44_100;

    /// Estimate a filter's magnitude response at `freq_hz` by driving it with
    /// a sine wave and measuring the steady-state output amplitude.
    fn measure_gain(eq: &mut Equalizer, freq_hz: f32) -> f32 {
        // Run mono so the single channel chain carries all the state.
        let samples_total = FS as usize * 2; // two seconds — well past settling.
        let mut peak = 0.0_f32;
        for n in 0..samples_total {
            let t = n as f32 / FS as f32;
            let mut buf = [(2.0 * std::f32::consts::PI * freq_hz * t).sin()];
            eq.process(&mut buf, 1);
            // Only measure the steady-state second half.
            if n > samples_total / 2 {
                peak = peak.max(buf[0].abs());
            }
        }
        peak
    }

    /// Linear amplitude ratio to decibels.
    fn to_db(ratio: f32) -> f32 {
        20.0 * ratio.log10()
    }

    #[test]
    fn disabled_equalizer_is_a_true_bypass() {
        let mut eq = Equalizer::new(FS, 2);
        eq.set_params(false, &[12.0; BAND_COUNT]);
        let original = [0.1_f32, -0.4, 0.7, -0.9];
        let mut buf = original;
        eq.process(&mut buf, 2);
        assert_eq!(buf, original, "disabled EQ must not touch the samples");
    }

    #[test]
    fn flat_equalizer_is_transparent() {
        let mut eq = Equalizer::new(FS, 1);
        eq.set_params(true, &[0.0; BAND_COUNT]);
        // Every band at 0 dB collapses to the identity biquad.
        let gain = measure_gain(&mut eq, 1_000.0);
        assert!(
            (to_db(gain)).abs() < 0.1,
            "flat EQ should pass 1 kHz at ~0 dB, got {:.3} dB",
            to_db(gain)
        );
    }

    #[test]
    fn peaking_band_hits_its_target_gain() {
        // Boost only the 1 kHz band by 6 dB and confirm a sine at exactly the
        // band centre comes out ~6 dB louder.
        let mut gains = [0.0_f32; BAND_COUNT];
        let band_1k = 5; // BAND_FREQUENCIES_HZ[5] == 1000.0
        assert_eq!(BAND_FREQUENCIES_HZ[band_1k], 1_000.0);
        gains[band_1k] = 6.0;

        let mut eq = Equalizer::new(FS, 1);
        eq.set_params(true, &gains);
        let gain_db = to_db(measure_gain(&mut eq, 1_000.0));
        assert!(
            (gain_db - 6.0).abs() < 0.5,
            "1 kHz band +6 dB measured {gain_db:.3} dB"
        );
    }

    #[test]
    fn cutting_a_band_attenuates_its_centre() {
        let mut gains = [0.0_f32; BAND_COUNT];
        gains[5] = -9.0; // cut 1 kHz
        let mut eq = Equalizer::new(FS, 1);
        eq.set_params(true, &gains);
        let gain_db = to_db(measure_gain(&mut eq, 1_000.0));
        assert!(
            (gain_db + 9.0).abs() < 0.6,
            "1 kHz band -9 dB measured {gain_db:.3} dB"
        );
    }

    #[test]
    fn a_band_barely_affects_a_distant_frequency() {
        // Boosting 31 Hz should leave 8 kHz essentially untouched.
        let mut gains = [0.0_f32; BAND_COUNT];
        gains[0] = 12.0;
        let mut eq = Equalizer::new(FS, 1);
        eq.set_params(true, &gains);
        let gain_db = to_db(measure_gain(&mut eq, 8_000.0));
        assert!(
            gain_db.abs() < 1.0,
            "31 Hz boost leaked {gain_db:.3} dB into 8 kHz"
        );
    }

    #[test]
    fn identity_biquad_passes_samples_unchanged() {
        let mut biquad = Biquad::new(BiquadCoeffs::IDENTITY);
        for x in [0.0_f32, 1.0, -1.0, 0.33, -0.7] {
            assert!((biquad.process(x) - x).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn peaking_coeffs_are_normalised() {
        // a0 is divided through, so the implicit a0 is 1.0 — sanity-check that
        // a non-trivial filter produced finite, sane coefficients.
        let c = BiquadCoeffs::peaking(1_000.0, 44_100.0, 6.0);
        for v in [c.b0, c.b1, c.b2, c.a1, c.a2] {
            assert!(v.is_finite(), "coefficient not finite: {v}");
        }
    }

    #[test]
    fn channel_count_change_is_handled() {
        let mut eq = Equalizer::new(FS, 2);
        eq.set_params(true, &[3.0; BAND_COUNT]);
        // Process a mono buffer then a stereo buffer — neither should panic.
        let mut mono = [0.5_f32; 8];
        eq.process(&mut mono, 1);
        let mut stereo = [0.5_f32; 8];
        eq.process(&mut stereo, 2);
    }
}
