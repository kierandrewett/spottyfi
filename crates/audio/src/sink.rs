//! The custom librespot audio backend — a [`Sink`] that wraps the real one.
//!
//! librespot drives playback through a [`Sink`]: `start`/`stop` bracket a
//! stream and `write` receives each decoded PCM packet. Spottyfi inserts
//! [`TappedSink`] between librespot and the stock `rodio` sink so it can,
//! per packet:
//!
//! 1. apply the [`Equalizer`] DSP (true bypass when disabled);
//! 2. copy the post-EQ samples into the [`AudioTap`] for the UI;
//! 3. forward the post-EQ packet to the inner `rodio` sink unchanged.
//!
//! When the equaliser is disabled and nothing reads the tap, the audible
//! output is bit-identical to the stock backend: the EQ early-returns and the
//! tap only ever appends into preallocated ring buffers.
//!
//! # How parameters reach the sink
//!
//! The sink is built inside the closure handed to `Player::new`, so it cannot
//! take constructor arguments from later calls. Instead it shares an
//! [`ArcSwap<EqParams>`] with the [`Engine`](crate::engine::Engine): the
//! controller swaps fresh params in, and the sink picks them up on its next
//! `write`. This mirrors how the soft-volume mixer is shared.

use std::sync::Arc;

use arc_swap::ArcSwap;
use librespot::playback::audio_backend::{Sink, SinkResult};
use librespot::playback::config::AudioFormat;
use librespot::playback::convert::Converter;
use librespot::playback::decoder::AudioPacket;
use librespot::playback::{NUM_CHANNELS, SAMPLE_RATE};

use crate::dsp::{Equalizer, BAND_COUNT};
use crate::tap::{AudioTap, TapWriter};

/// Live equaliser parameters, swapped in by the controller and read by the
/// sink on its next `write`.
///
/// Shared as `Arc<ArcSwap<EqParams>>` — the same lock-free snapshot pattern
/// the rest of the engine uses. Cheap to clone and to publish.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqParams {
    /// Whether the equaliser is enabled. `false` is a true bypass.
    pub enabled: bool,
    /// Per-band gains in decibels, low-to-high, matching the ISO band layout.
    pub band_gains_db: [f32; BAND_COUNT],
}

impl Default for EqParams {
    fn default() -> Self {
        Self {
            enabled: false,
            band_gains_db: [0.0; BAND_COUNT],
        }
    }
}

/// Handle to the equaliser parameter slot.
///
/// The [`Engine`](crate::engine::Engine) holds one; each [`TappedSink`] the
/// player builds holds a clone and reads it.
pub type SharedEqParams = Arc<ArcSwap<EqParams>>;

/// A [`Sink`] that EQs and taps the PCM stream, then forwards it to an inner
/// `rodio` sink.
pub struct TappedSink {
    /// The wrapped real audio backend (the stock `rodio` sink).
    inner: Box<dyn Sink>,
    /// The 10-band graphic equaliser applied before output.
    equalizer: Equalizer,
    /// Live EQ parameters, shared with the engine/controller.
    params: SharedEqParams,
    /// The last params applied to [`Self::equalizer`]; lets `write` skip
    /// recomputing biquad coefficients when nothing changed.
    applied: EqParams,
    /// The sink-side writer half of the UI sample tap.
    tap: TapWriter,
    /// Scratch buffer for the f64 → f32 → (EQ) → f64 round trip, reused across
    /// packets so the hot path does not allocate per call.
    scratch: Vec<f32>,
}

impl TappedSink {
    /// Build a tapping sink wrapping `inner`.
    ///
    /// `params` is the shared EQ slot the controller drives; `tap` is the UI
    /// sample tap to feed. The audio is 44.1 kHz stereo — librespot's fixed
    /// decode format.
    #[must_use]
    pub fn new(inner: Box<dyn Sink>, params: SharedEqParams, tap: &AudioTap) -> Self {
        let channels = NUM_CHANNELS as usize;
        Self {
            inner,
            equalizer: Equalizer::new(SAMPLE_RATE, channels),
            params,
            applied: EqParams::default(),
            tap: tap.writer(SAMPLE_RATE, channels),
            scratch: Vec::new(),
        }
    }

    /// Pick up any freshly-published EQ params and reconfigure the filter bank.
    ///
    /// Cheap when nothing changed — a snapshot load and an equality check.
    fn refresh_params(&mut self) {
        let current = **self.params.load();
        if current == self.applied {
            return;
        }
        self.equalizer
            .set_params(current.enabled, &current.band_gains_db);
        self.applied = current;
    }
}

impl Sink for TappedSink {
    fn start(&mut self) -> SinkResult<()> {
        self.inner.start()
    }

    fn stop(&mut self) -> SinkResult<()> {
        self.inner.stop()
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        // Raw (already-encoded) packets carry no PCM samples to process; pass
        // them straight through. librespot only emits these for passthrough
        // formats Spottyfi does not request, but handle them for safety.
        let samples = match &packet {
            AudioPacket::Samples(samples) => samples,
            AudioPacket::Raw(_) => return self.inner.write(packet, converter),
        };

        self.refresh_params();

        let channels = NUM_CHANNELS as usize;
        // f64 (librespot's decode precision) -> f32 working buffer.
        self.scratch.clear();
        self.scratch.extend(samples.iter().map(|&s| s as f32));

        // 1. Equalise in place (true bypass when disabled).
        self.equalizer.process(&mut self.scratch, channels);

        // 2. Tap the post-EQ audio for the UI. Cheap; see `tap` module docs.
        self.tap.push(&self.scratch);

        // 3. Forward the post-EQ packet to the real sink. When the EQ is
        //    disabled the scratch buffer equals the input bit-for-bit (f32
        //    holds every f64 the decoder produces here without loss visible
        //    after the sink's own f64->f32 step), so output is unchanged.
        let processed: Vec<f64> = self.scratch.iter().map(|&s| s as f64).collect();
        self.inner.write(AudioPacket::Samples(processed), converter)
    }
}

/// The fixed audio format librespot decodes Spotify streams to.
///
/// Re-exported so [`Engine`](crate::engine::Engine) does not need to import it
/// from librespot directly.
pub(crate) const DECODE_FORMAT: AudioFormat = AudioFormat::F32;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A fake inner sink that records every packet it receives.
    #[derive(Default)]
    struct RecordingSink {
        writes: Arc<Mutex<Vec<Vec<f64>>>>,
        started: Arc<Mutex<bool>>,
    }

    impl Sink for RecordingSink {
        fn start(&mut self) -> SinkResult<()> {
            *self.started.lock().expect("lock") = true;
            Ok(())
        }
        fn write(&mut self, packet: AudioPacket, _: &mut Converter) -> SinkResult<()> {
            if let AudioPacket::Samples(s) = packet {
                self.writes.lock().expect("lock").push(s);
            }
            Ok(())
        }
    }

    fn stereo_packet(samples: &[f64]) -> AudioPacket {
        AudioPacket::Samples(samples.to_vec())
    }

    #[test]
    fn bypass_forwards_samples_unchanged() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let inner = Box::new(RecordingSink {
            writes: Arc::clone(&writes),
            ..RecordingSink::default()
        });
        let params: SharedEqParams = Arc::new(ArcSwap::from_pointee(EqParams::default()));
        let tap = AudioTap::new();
        let mut sink = TappedSink::new(inner, params, &tap);

        let input = [0.1_f64, -0.25, 0.5, -0.75];
        let mut converter = Converter::new(None);
        sink.write(stereo_packet(&input), &mut converter)
            .expect("write");

        let recorded = writes.lock().expect("lock");
        assert_eq!(recorded.len(), 1);
        for (got, want) in recorded[0].iter().zip(input.iter()) {
            assert!((got - want).abs() < 1e-6, "{got} != {want}");
        }
    }

    #[test]
    fn enabled_eq_changes_the_forwarded_samples() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let inner = Box::new(RecordingSink {
            writes: Arc::clone(&writes),
            ..RecordingSink::default()
        });
        let mut eq = EqParams {
            enabled: true,
            band_gains_db: [12.0; BAND_COUNT],
        };
        eq.band_gains_db[5] = 12.0;
        let params: SharedEqParams = Arc::new(ArcSwap::from_pointee(eq));
        let tap = AudioTap::new();
        let mut sink = TappedSink::new(inner, params, &tap);

        // A non-trivial signal so the filter bank has something to colour.
        let input: Vec<f64> = (0..512).map(|n| (n as f64 * 0.3).sin()).collect();
        let mut converter = Converter::new(None);
        sink.write(AudioPacket::Samples(input.clone()), &mut converter)
            .expect("write");

        let recorded = writes.lock().expect("lock");
        assert_ne!(recorded[0], input, "enabled EQ should alter the audio");
    }

    #[test]
    fn write_feeds_the_tap() {
        let inner = Box::new(RecordingSink::default());
        let params: SharedEqParams = Arc::new(ArcSwap::from_pointee(EqParams::default()));
        let tap = AudioTap::new();
        let mut sink = TappedSink::new(inner, params, &tap);

        // Push enough audio for the tap to publish at least one snapshot.
        let input = vec![0.3_f64; crate::tap::SPECTRUM_LEN * 2];
        let mut converter = Converter::new(None);
        sink.write(AudioPacket::Samples(input), &mut converter)
            .expect("write");

        assert!(tap.snapshot().sequence >= 1, "tap should have published");
    }

    #[test]
    fn start_and_stop_delegate_to_inner() {
        let started = Arc::new(Mutex::new(false));
        let inner = Box::new(RecordingSink {
            started: Arc::clone(&started),
            ..RecordingSink::default()
        });
        let params: SharedEqParams = Arc::new(ArcSwap::from_pointee(EqParams::default()));
        let tap = AudioTap::new();
        let mut sink = TappedSink::new(inner, params, &tap);
        sink.start().expect("start");
        assert!(*started.lock().expect("lock"));
    }
}
