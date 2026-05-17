//! The post-EQ sample tap — the contract WS7b (waveform + visualisations UI)
//! consumes.
//!
//! # What the tap is
//!
//! While audio plays, the custom sink ([`crate::sink`]) pushes every decoded,
//! EQ'd packet into an [`AudioTap`]. The tap maintains, lock-free on the hot
//! path, two rolling products and republishes them as an immutable
//! [`TapSnapshot`] roughly 60× a second:
//!
//! 1. a **waveform envelope** — a downsampled rolling window of recent peak
//!    amplitudes (mono-summed), ready to draw straight as a scope/scrubber;
//! 2. a **spectrum window** — the most recent raw mono samples, enough for the
//!    UI to run an FFT for a bar/visualiser display.
//!
//! # How the UI reads it
//!
//! The tap follows the same `Arc<ArcSwap<…>>` snapshot pattern as
//! [`PlaybackState`](crate::PlaybackState): obtain the shared handle once with
//! [`PlaybackController::audio_tap`], then call [`AudioTap::snapshot`] once per
//! UI frame. `snapshot` is a single atomic load — it never blocks, never
//! allocates, and is safe to call from the egui thread every frame. A
//! `TapSnapshot` is cheap to clone (`Arc` inside) and immutable.
//!
//! # Why it does not stutter playback
//!
//! The audio `write` path only ever:
//!
//! * appends into two fixed-capacity ring buffers (no allocation), and
//! * **occasionally** (≈ every 16 ms of audio) builds one fresh snapshot and
//!   does a single `ArcSwap::store`.
//!
//! The per-snapshot `Vec`s are small (see [`WAVEFORM_LEN`] / [`SPECTRUM_LEN`])
//! and the publish is rate-limited by sample count, so the cost per `write` is
//! a few hundred copies amortised — far below the ~37 ms of audio a typical
//! packet carries. No locks are taken on the audio thread.
//!
//! # Silence
//!
//! When playback stops the sink does not write, so the published snapshot
//! simply goes stale. WS7b should treat a snapshot whose `sequence` has not
//! advanced as "no new audio" and decay its visualisation toward zero.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwap;

/// Number of amplitude points in the published waveform envelope.
///
/// 2048 points at the publish rate covers a generous on-screen history for a
/// waveform scrubber while staying tiny to clone.
pub const WAVEFORM_LEN: usize = 2048;

/// Number of raw mono samples in the published spectrum window.
///
/// A power of two so WS7b can FFT it directly without zero-padding. 2048
/// samples at 44.1 kHz is ~46 ms — fine frequency resolution (~21 Hz/bin) for
/// a spectrum display.
pub const SPECTRUM_LEN: usize = 2048;

/// How many audio samples (mono-summed frames) the tap accumulates between
/// snapshot publications.
///
/// 735 frames ≈ 16.7 ms at 44.1 kHz — one 60 Hz UI frame's worth, so the UI
/// sees a fresh snapshot every frame without the sink publishing more often
/// than the UI can consume.
const PUBLISH_INTERVAL_FRAMES: usize = 735;

/// One amplitude point of the waveform envelope: the peak magnitude over a
/// short slice of audio. Always in `0.0..=1.0` for normalised audio.
pub type WaveformPoint = f32;

/// An immutable, cheap-to-clone snapshot of recent post-EQ audio.
///
/// This is the WS7b contract. Obtain one per UI frame via
/// [`AudioTap::snapshot`]. Both vectors are oldest-sample-first.
#[derive(Debug, Clone, Default)]
pub struct TapSnapshot {
    /// Monotonically increasing publish counter.
    ///
    /// Bumped once per published snapshot. Two reads with the same `sequence`
    /// observed the same audio — WS7b can use this to detect a stalled stream
    /// (paused/stopped playback) and decay its visualisation.
    pub sequence: u64,
    /// The stream sample rate in hertz (44 100 for Spotify) — the basis for
    /// turning [`spectrum`](Self::spectrum) FFT bins into frequencies.
    pub sample_rate_hz: u32,
    /// A rolling window of recent peak amplitudes, oldest first, each the peak
    /// magnitude of a short audio slice. Up to [`WAVEFORM_LEN`] points; draw
    /// directly as a waveform/scope. Empty before any audio has played.
    pub waveform: Vec<WaveformPoint>,
    /// The most recent raw mono (channel-averaged) samples, oldest first, in
    /// `-1.0..=1.0`. Up to [`SPECTRUM_LEN`] samples — feed straight into an
    /// FFT for a spectrum display. Empty before any audio has played.
    pub spectrum: Vec<f32>,
}

impl TapSnapshot {
    /// The peak amplitude across the waveform window, or `0.0` when empty.
    ///
    /// A convenience for a simple level meter that does not need the full
    /// envelope.
    #[must_use]
    pub fn peak_amplitude(&self) -> f32 {
        self.waveform.iter().copied().fold(0.0, f32::max)
    }
}

/// Shared handle to the audio tap.
///
/// Cloning is cheap (an `Arc` bump). The sink holds one clone and writes; the
/// UI holds another and reads. See the module docs.
#[derive(Clone)]
pub struct AudioTap {
    inner: Arc<TapInner>,
}

/// The tap's shared interior.
struct TapInner {
    /// The latest published snapshot, swapped ~60×/second by the sink.
    published: ArcSwap<TapSnapshot>,
    /// The publish counter, bumped on every store.
    sequence: AtomicU64,
}

impl AudioTap {
    /// Create a fresh tap with an empty initial snapshot.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(TapInner {
                published: ArcSwap::from_pointee(TapSnapshot::default()),
                sequence: AtomicU64::new(0),
            }),
        }
    }

    /// Read the most recent snapshot. A single atomic load — never blocks,
    /// never allocates. Call once per UI frame.
    #[must_use]
    pub fn snapshot(&self) -> Arc<TapSnapshot> {
        self.inner.published.load_full()
    }

    /// Build a [`TapWriter`] for the sink to feed.
    ///
    /// `sample_rate_hz` and `channel_count` describe the PCM the sink will
    /// push. The writer owns the ring buffers; only it touches the audio-hot
    /// path.
    #[must_use]
    pub(crate) fn writer(&self, sample_rate_hz: u32, channel_count: usize) -> TapWriter {
        TapWriter {
            inner: Arc::clone(&self.inner),
            sample_rate_hz,
            channel_count: channel_count.max(1),
            waveform: RingBuffer::new(WAVEFORM_LEN),
            spectrum: RingBuffer::new(SPECTRUM_LEN),
            frames_since_publish: 0,
            slice_peak: 0.0,
            slice_frames: 0,
        }
    }
}

impl Default for AudioTap {
    fn default() -> Self {
        Self::new()
    }
}

/// How many mono frames each waveform-envelope point summarises.
///
/// The envelope is a peak-per-slice downsample of the audio; this sets the
/// slice width. ~64 frames ≈ 1.5 ms — fine enough for a smooth scrubber.
const WAVEFORM_SLICE_FRAMES: usize = 64;

/// The sink-side writer half of the tap.
///
/// Lives entirely on the audio thread. [`TapWriter::push`] is the hot-path
/// entry point; it appends to ring buffers and, at most once per
/// [`PUBLISH_INTERVAL_FRAMES`], publishes a snapshot.
pub(crate) struct TapWriter {
    /// Shared interior — the publish target.
    inner: Arc<TapInner>,
    /// Stream sample rate, copied into every snapshot.
    sample_rate_hz: u32,
    /// Interleaved channel count of the PCM `push` receives.
    channel_count: usize,
    /// Rolling downsampled amplitude envelope.
    waveform: RingBuffer,
    /// Rolling raw mono sample window for the spectrum.
    spectrum: RingBuffer,
    /// Mono frames accumulated since the last publish.
    frames_since_publish: usize,
    /// Running peak magnitude of the in-progress waveform slice.
    slice_peak: f32,
    /// Mono frames accumulated into the in-progress waveform slice.
    slice_frames: usize,
}

impl TapWriter {
    /// Feed one interleaved PCM buffer (post-EQ) into the tap.
    ///
    /// Cheap and allocation-free except for the amortised ~60 Hz snapshot
    /// publish. Safe to call from the audio `write` path.
    pub(crate) fn push(&mut self, samples: &[f32]) {
        let channels = self.channel_count;
        // Walk whole interleaved frames; average channels to mono.
        for frame in samples.chunks_exact(channels) {
            let mono = frame.iter().copied().sum::<f32>() / channels as f32;
            self.spectrum.push(mono);

            // Accumulate the waveform envelope as a peak-per-slice downsample.
            self.slice_peak = self.slice_peak.max(mono.abs());
            self.slice_frames += 1;
            if self.slice_frames >= WAVEFORM_SLICE_FRAMES {
                self.waveform.push(self.slice_peak);
                self.slice_peak = 0.0;
                self.slice_frames = 0;
            }

            self.frames_since_publish += 1;
        }

        if self.frames_since_publish >= PUBLISH_INTERVAL_FRAMES {
            self.publish();
            self.frames_since_publish = 0;
        }
    }

    /// Build a fresh [`TapSnapshot`] from the ring buffers and store it.
    fn publish(&self) {
        let sequence = self.inner.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let snapshot = TapSnapshot {
            sequence,
            sample_rate_hz: self.sample_rate_hz,
            waveform: self.waveform.to_vec(),
            spectrum: self.spectrum.to_vec(),
        };
        self.inner.published.store(Arc::new(snapshot));
    }
}

/// A fixed-capacity, oldest-overwriting ring buffer of `f32` samples.
///
/// Allocated once at construction; [`RingBuffer::push`] never allocates.
struct RingBuffer {
    /// Backing storage, length == capacity once filled.
    data: Vec<f32>,
    /// Capacity (the window length).
    capacity: usize,
    /// Index of the next write — also the oldest element once full.
    head: usize,
    /// Whether the buffer has wrapped at least once.
    filled: bool,
}

impl RingBuffer {
    /// A new empty ring buffer holding up to `capacity` samples.
    fn new(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            capacity,
            head: 0,
            filled: false,
        }
    }

    /// Append `value`, overwriting the oldest sample once full.
    #[inline]
    fn push(&mut self, value: f32) {
        if self.data.len() < self.capacity {
            self.data.push(value);
        } else {
            self.data[self.head] = value;
            self.head += 1;
            if self.head == self.capacity {
                self.head = 0;
                self.filled = true;
            }
        }
    }

    /// Copy the contents out oldest-first into a fresh `Vec`.
    fn to_vec(&self) -> Vec<f32> {
        if !self.filled && self.head == 0 {
            // Not yet wrapped — `data` is already in order.
            return self.data.clone();
        }
        let mut out = Vec::with_capacity(self.data.len());
        out.extend_from_slice(&self.data[self.head..]);
        out.extend_from_slice(&self.data[..self.head]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_tap_snapshot_is_empty() {
        let tap = AudioTap::new();
        let snap = tap.snapshot();
        assert_eq!(snap.sequence, 0);
        assert!(snap.waveform.is_empty());
        assert!(snap.spectrum.is_empty());
    }

    #[test]
    fn ring_buffer_keeps_newest_in_order() {
        let mut ring = RingBuffer::new(4);
        for v in [1.0, 2.0, 3.0] {
            ring.push(v);
        }
        assert_eq!(ring.to_vec(), vec![1.0, 2.0, 3.0]);
        // Overflow: oldest drops, order preserved.
        for v in [4.0, 5.0, 6.0] {
            ring.push(v);
        }
        assert_eq!(ring.to_vec(), vec![3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn push_publishes_a_snapshot_after_the_interval() {
        let tap = AudioTap::new();
        let mut writer = tap.writer(44_100, 2);
        // One full publish interval of stereo audio at amplitude 0.5.
        let frames = PUBLISH_INTERVAL_FRAMES;
        let buf = vec![0.5_f32; frames * 2];
        writer.push(&buf);

        let snap = tap.snapshot();
        assert_eq!(snap.sequence, 1, "one snapshot should have published");
        assert_eq!(snap.sample_rate_hz, 44_100);
        assert!(!snap.spectrum.is_empty());
        assert!(!snap.waveform.is_empty());
        // Mono of two 0.5 channels is 0.5; the waveform peak should reflect it.
        assert!((snap.peak_amplitude() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn short_writes_do_not_publish_early() {
        let tap = AudioTap::new();
        let mut writer = tap.writer(44_100, 2);
        // Well under the publish interval.
        writer.push(&vec![0.2_f32; 100]);
        assert_eq!(tap.snapshot().sequence, 0, "no snapshot expected yet");
    }

    #[test]
    fn spectrum_window_is_mono_averaged() {
        let tap = AudioTap::new();
        let mut writer = tap.writer(44_100, 2);
        // Left = 1.0, right = 0.0 -> mono average 0.5.
        let mut buf = Vec::new();
        for _ in 0..PUBLISH_INTERVAL_FRAMES {
            buf.push(1.0);
            buf.push(0.0);
        }
        writer.push(&buf);
        let snap = tap.snapshot();
        assert!(snap.spectrum.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn sequence_advances_each_publish() {
        let tap = AudioTap::new();
        let mut writer = tap.writer(44_100, 2);
        let buf = vec![0.1_f32; PUBLISH_INTERVAL_FRAMES * 2];
        writer.push(&buf);
        writer.push(&buf);
        assert_eq!(tap.snapshot().sequence, 2);
    }
}
