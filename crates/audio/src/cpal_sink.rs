//! A direct [`cpal`] audio-output [`Sink`] — Spottyfi's own playback backend.
//!
//! librespot's stock `rodio` backend buffers roughly half a second of audio
//! and its `stop()` blocks the decode thread until that buffer fully drains
//! (`rodio::Sink::sleep_until_end`). Every pause and every volume change then
//! sits behind that half-second queue. [`CpalSink`] replaces it:
//!
//! * a small lock-free SPSC ring buffer sits between librespot's decode thread
//!   (the [`Sink::write`] producer) and the `cpal` audio callback (consumer);
//! * `start`/`stop` flip an atomic — `stop` is *instant*, it never drains;
//! * a paused callback emits silence and drops the buffered audio, so resume
//!   starts clean and pause is heard immediately.
//!
//! The ring is sized for a low, fixed output latency ([`RING_LATENCY_MS`]), so
//! the player feels snappy rather than sludgy. The callback is the single
//! consumer and the only writer of the read cursor; the decode thread is the
//! single producer and the only writer of the write cursor — a textbook
//! single-producer/single-consumer queue, lock-free on the hot path.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use librespot::playback::audio_backend::{Sink, SinkError, SinkResult};
use librespot::playback::convert::Converter;
use librespot::playback::decoder::AudioPacket;
use librespot::playback::{NUM_CHANNELS, SAMPLE_RATE};

use crate::error::{AudioError, AudioResult};

/// Target output latency: the ring buffer holds about this much audio.
///
/// Small enough that pause/volume feel immediate, large enough to absorb
/// normal scheduler jitter on the decode thread without an underrun.
const RING_LATENCY_MS: usize = 150;

/// How long the producer parks when the ring is full before re-checking.
///
/// A short, bounded wait: the consumer notifies on every drain, but the
/// timeout guarantees progress even if a wakeup is missed.
const PRODUCER_PARK: Duration = Duration::from_millis(5);

/// Time constant of the per-sample output-gain ramp.
///
/// Every volume change, pause and crossfade step is delivered as a new target
/// gain; the callback glides to it over roughly this long. Short enough to
/// feel instant, long enough that no gain change ever clicks ("zipper noise").
const GAIN_RAMP_SECONDS: f32 = 0.008;

/// The real-time control surface the engine shares with the running sink.
///
/// Every field is an atomic the engine writes and the audio callback reads, so
/// volume, pause and seek take effect within one device buffer (a few ms)
/// rather than behind the ring buffer.
#[derive(Clone)]
pub struct SinkControls {
    /// Target output gain as `f32` bits — the callback ramps toward it.
    pub gain: Arc<AtomicU32>,
    /// While `true` the callback emits silence and freezes the ring, so a
    /// resume replays the buffered audio instantly. Driven by `start`/`stop`.
    pub paused: Arc<AtomicBool>,
    /// A one-shot flag: when set, the callback drops all buffered audio. The
    /// engine raises it after a seek so stale pre-seek audio is never heard.
    pub flush: Arc<AtomicBool>,
}

impl SinkControls {
    /// Build a control surface with the given initial gain.
    #[must_use]
    pub fn new(initial_gain: f32) -> Self {
        Self {
            gain: Arc::new(AtomicU32::new(initial_gain.to_bits())),
            paused: Arc::new(AtomicBool::new(false)),
            flush: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// The lock-free single-producer/single-consumer sample ring.
///
/// Stores interleaved stereo `f32`. `write_pos` is only ever advanced by the
/// producer (the decode thread); `read_pos` only by the consumer (the audio
/// callback). Both are free-running sample counts; the backing index is
/// `pos & mask` with a power-of-two `capacity`.
struct Ring {
    /// The sample storage. `UnsafeCell` because the producer and consumer
    /// write disjoint regions concurrently; the cursors enforce disjointness.
    buf: Box<[UnsafeCell<f32>]>,
    /// Capacity in samples — always a power of two.
    capacity: usize,
    /// `capacity - 1`, for cheap index masking.
    mask: usize,
    /// Total samples written by the producer.
    write_pos: AtomicUsize,
    /// Total samples read by the consumer.
    read_pos: AtomicUsize,
    /// Signalled by the consumer after a drain so a full producer can wake.
    space: Condvar,
    /// Guards [`Ring::space`] waits; never held on the consumer's hot path.
    space_lock: Mutex<()>,
}

// SAFETY: the producer only writes `[write_pos, write_pos + free)` and the
// consumer only reads `[read_pos, read_pos + avail)`; the cursors keep those
// ranges disjoint, so the `UnsafeCell` accesses never overlap.
unsafe impl Send for Ring {}
unsafe impl Sync for Ring {}

impl Ring {
    /// Build an empty ring with at least `min_samples` capacity.
    fn new(min_samples: usize) -> Self {
        let capacity = min_samples.next_power_of_two().max(2);
        let buf = (0..capacity).map(|_| UnsafeCell::new(0.0)).collect();
        Self {
            buf,
            capacity,
            mask: capacity - 1,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            space: Condvar::new(),
            space_lock: Mutex::new(()),
        }
    }
}

/// The producer half of the ring — held by [`CpalSink`] on the decode thread.
struct RingProducer {
    ring: Arc<Ring>,
}

impl RingProducer {
    /// Push every sample in `samples`, blocking while the ring is full.
    ///
    /// Blocking *is* the back-pressure: librespot decodes faster than realtime,
    /// so a full ring simply means the consumer has not caught up yet.
    fn push_blocking(&self, samples: &[f32]) {
        let mut written = 0;
        while written < samples.len() {
            let write_pos = self.ring.write_pos.load(Ordering::Relaxed);
            let read_pos = self.ring.read_pos.load(Ordering::Acquire);
            let free = self.ring.capacity - (write_pos - read_pos);
            if free == 0 {
                let guard = self
                    .ring
                    .space_lock
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let _ = self.ring.space.wait_timeout(guard, PRODUCER_PARK);
                continue;
            }
            let n = free.min(samples.len() - written);
            for i in 0..n {
                let idx = (write_pos + i) & self.ring.mask;
                // SAFETY: `idx` is in `[write_pos, write_pos + free)`, a range
                // the consumer never touches until `write_pos` is published.
                unsafe { *self.ring.buf[idx].get() = samples[written + i] }
            }
            self.ring.write_pos.store(write_pos + n, Ordering::Release);
            written += n;
        }
    }
}

/// The consumer half of the ring — moved into the `cpal` audio callback.
struct RingConsumer {
    ring: Arc<Ring>,
}

// SAFETY: `RingConsumer` is the sole consumer; `Ring` is `Send`/`Sync`.
unsafe impl Send for RingConsumer {}

impl RingConsumer {
    /// Pop up to `out.len()` samples into `out`; returns how many were copied.
    fn pop(&self, out: &mut [f32]) -> usize {
        let read_pos = self.ring.read_pos.load(Ordering::Relaxed);
        let write_pos = self.ring.write_pos.load(Ordering::Acquire);
        let n = (write_pos - read_pos).min(out.len());
        for (i, slot) in out.iter_mut().enumerate().take(n) {
            let idx = (read_pos + i) & self.ring.mask;
            // SAFETY: `idx` is in `[read_pos, write_pos)`, already published
            // by the producer and not reused until `read_pos` advances.
            *slot = unsafe { *self.ring.buf[idx].get() };
        }
        self.ring.read_pos.store(read_pos + n, Ordering::Release);
        if n > 0 {
            self.ring.space.notify_one();
        }
        n
    }

    /// Drop all buffered audio — used after a seek so stale audio is skipped.
    fn drain(&self) {
        let write_pos = self.ring.write_pos.load(Ordering::Acquire);
        self.ring.read_pos.store(write_pos, Ordering::Release);
        self.ring.space.notify_one();
    }
}

/// Create a producer/consumer pair over a fresh ring.
fn ring_pair(min_samples: usize) -> (RingProducer, RingConsumer) {
    let ring = Arc::new(Ring::new(min_samples));
    (
        RingProducer {
            ring: Arc::clone(&ring),
        },
        RingConsumer { ring },
    )
}

/// Linear-interpolating stereo resampler, used only when the output device
/// cannot run at librespot's native 44.1 kHz.
struct Resampler {
    /// Output-to-input sample-rate ratio (`44100 / device_rate`).
    step: f64,
    /// Fractional read position within the input stream.
    pos: f64,
    /// The most recent input frame, for interpolation across `write` calls.
    last: [f32; 2],
}

impl Resampler {
    /// Resample interleaved-stereo `input` from 44.1 kHz to the device rate,
    /// appending the result to `out`.
    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let frames = input.len() / 2;
        while self.pos < frames as f64 {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            let prev = if i == 0 {
                self.last
            } else {
                [input[(i - 1) * 2], input[(i - 1) * 2 + 1]]
            };
            let cur = [input[i * 2], input[i * 2 + 1]];
            out.push(prev[0] + (cur[0] - prev[0]) * frac);
            out.push(prev[1] + (cur[1] - prev[1]) * frac);
            self.pos += self.step;
        }
        self.pos -= frames as f64;
        if frames > 0 {
            self.last = [input[(frames - 1) * 2], input[(frames - 1) * 2 + 1]];
        }
    }
}

/// A direct `cpal` output sink with instant pause and a low, fixed latency.
pub struct CpalSink {
    /// The live `cpal` stream. Kept alive for the sink's lifetime; never moved
    /// off the thread it was built on (`cpal::Stream` is `!Send`).
    _stream: cpal::Stream,
    /// The ring producer the decode thread writes decoded PCM into.
    producer: RingProducer,
    /// `true` while paused — the callback then emits silence and freezes the
    /// ring. Shared with the engine; flipped by `start`/`stop`.
    paused: Arc<AtomicBool>,
    /// Optional resampler, present only when the device is not 44.1 kHz.
    resampler: Option<Resampler>,
    /// Scratch buffer for the `f64 -> f32` (and resample) conversion.
    scratch: Vec<f32>,
}

impl CpalSink {
    /// Open the default output device and start its stream.
    ///
    /// `controls` is the real-time surface the engine drives — output gain,
    /// pause and seek-flush all take effect inside the audio callback.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NoBackend`] if no output device is available or
    /// the stream cannot be built.
    pub fn open(controls: SinkControls) -> AudioResult<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(AudioError::NoBackend)?;
        let supported = device.default_output_config().map_err(|err| {
            tracing::error!(%err, "no default output config");
            AudioError::NoBackend
        })?;

        // Prefer a native 44.1 kHz stereo config so no resampling is needed;
        // fall back to the device default otherwise.
        let chosen = device
            .supported_output_configs()
            .ok()
            .and_then(|mut configs| {
                configs.find_map(|range| {
                    (range.channels() == u16::from(NUM_CHANNELS))
                        .then(|| range.try_with_sample_rate(cpal::SampleRate(SAMPLE_RATE)))
                        .flatten()
                })
            })
            .unwrap_or(supported);

        let sample_format = chosen.sample_format();
        let config: cpal::StreamConfig = chosen.into();
        let device_rate = config.sample_rate.0;
        let channels = config.channels as usize;

        let latency_samples = SAMPLE_RATE as usize / 1000 * RING_LATENCY_MS * 2;
        let (producer, consumer) = ring_pair(latency_samples);
        let paused = Arc::clone(&controls.paused);

        let stream = build_stream(
            &device,
            &config,
            sample_format,
            consumer,
            controls,
            channels,
            device_rate,
        )?;
        stream.play().map_err(|err| {
            tracing::error!(%err, "starting the cpal stream failed");
            AudioError::NoBackend
        })?;

        let resampler = (device_rate != SAMPLE_RATE).then(|| {
            tracing::info!(device_rate, "output device is not 44.1 kHz; resampling");
            Resampler {
                step: f64::from(SAMPLE_RATE) / f64::from(device_rate),
                pos: 0.0,
                last: [0.0; 2],
            }
        });

        tracing::info!(device_rate, channels, ?sample_format, "cpal sink open");
        Ok(Self {
            _stream: stream,
            producer,
            paused,
            resampler,
            scratch: Vec::new(),
        })
    }
}

impl Sink for CpalSink {
    fn start(&mut self) -> SinkResult<()> {
        self.paused.store(false, Ordering::Release);
        Ok(())
    }

    fn stop(&mut self) -> SinkResult<()> {
        // Instant: just flag the callback. It emits silence and freezes the
        // ring — no draining, no blocking. `stop` and `write` are serialised
        // on librespot's player thread, so no `write` is ever in flight here.
        self.paused.store(true, Ordering::Release);
        Ok(())
    }

    fn write(&mut self, packet: AudioPacket, _converter: &mut Converter) -> SinkResult<()> {
        let AudioPacket::Samples(samples) = packet else {
            // Raw (already-encoded) packets are only produced for passthrough
            // formats Spottyfi never requests; nothing to play.
            return Ok(());
        };
        if samples.is_empty() {
            return Ok(());
        }

        self.scratch.clear();
        match self.resampler.as_mut() {
            None => self.scratch.extend(samples.iter().map(|&s| s as f32)),
            Some(resampler) => {
                // Resample needs a contiguous f32 input; reuse a second pass.
                let input: Vec<f32> = samples.iter().map(|&s| s as f32).collect();
                resampler.process(&input, &mut self.scratch);
            }
        }
        self.producer.push_blocking(&self.scratch);
        Ok(())
    }
}

/// Build the `cpal` output stream for the device's sample format.
fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: SampleFormat,
    consumer: RingConsumer,
    controls: SinkControls,
    channels: usize,
    rate: u32,
) -> AudioResult<cpal::Stream> {
    match format {
        SampleFormat::F32 => stream_for::<f32>(device, config, consumer, controls, channels, rate),
        SampleFormat::I16 => stream_for::<i16>(device, config, consumer, controls, channels, rate),
        SampleFormat::I32 => stream_for::<i32>(device, config, consumer, controls, channels, rate),
        SampleFormat::U16 => stream_for::<u16>(device, config, consumer, controls, channels, rate),
        SampleFormat::F64 => stream_for::<f64>(device, config, consumer, controls, channels, rate),
        other => {
            tracing::error!(?other, "unsupported output sample format");
            Err(AudioError::NoBackend)
        }
    }
}

/// Build the stream for a concrete output sample type `T`.
fn stream_for<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    consumer: RingConsumer,
    controls: SinkControls,
    channels: usize,
    rate: u32,
) -> AudioResult<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    // Per-sample one-pole coefficient for the gain ramp at the device rate.
    let ramp = 1.0 - (-1.0 / (GAIN_RAMP_SECONDS * rate as f32)).exp();
    // The gain the callback is currently *at* — glides toward `controls.gain`.
    let mut gain = f32::from_bits(controls.gain.load(Ordering::Relaxed));
    // Interleaved-stereo scratch the callback pops into before spreading the
    // samples across however many channels the device actually wants.
    let mut stereo: Vec<f32> = Vec::new();

    let callback = move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        // A seek just happened: drop the now-stale buffered audio.
        if controls.flush.swap(false, Ordering::AcqRel) {
            consumer.drain();
        }
        // Paused: emit silence and *freeze* the ring — leaving it full means
        // a resume replays instantly, with no refill gap.
        if controls.paused.load(Ordering::Acquire) {
            data.fill(T::EQUILIBRIUM);
            return;
        }

        let frames = data.len() / channels.max(1);
        stereo.resize(frames * 2, 0.0);
        let got = consumer.pop(&mut stereo[..frames * 2]);
        // Anything the ring could not supply is an underrun — emit silence.
        stereo[got..frames * 2].fill(0.0);

        let target = f32::from_bits(controls.gain.load(Ordering::Relaxed));
        for (frame_index, frame) in data.chunks_mut(channels).enumerate() {
            // Glide the gain one sample-frame closer to its target: an abrupt
            // jump would click, this ramp is inaudible.
            gain += (target - gain) * ramp;
            let left = stereo[frame_index * 2] * gain;
            let right = stereo[frame_index * 2 + 1] * gain;
            match channels {
                1 => frame[0] = T::from_sample((left + right) * 0.5),
                _ => {
                    frame[0] = T::from_sample(left);
                    frame[1] = T::from_sample(right);
                    for extra in &mut frame[2..] {
                        *extra = T::EQUILIBRIUM;
                    }
                }
            }
        }
    };

    device
        .build_output_stream(config, callback, stream_error, None)
        .map_err(|err| {
            tracing::error!(%err, "building the cpal output stream failed");
            AudioError::NoBackend
        })
}

/// Log a `cpal` stream error. The stream may recover; if not, output stops.
fn stream_error(err: cpal::StreamError) {
    tracing::error!(%err, "cpal output stream error");
}

/// A silent fallback sink, used if the real device cannot be opened so the
/// engine still starts (the user sees no playback rather than a crash).
pub struct NullSink;

impl Sink for NullSink {
    fn write(&mut self, _: AudioPacket, _: &mut Converter) -> SinkResult<()> {
        Err(SinkError::NotConnected("no audio output device".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_round_trips_in_order() {
        let (producer, consumer) = ring_pair(8);
        producer.push_blocking(&[1.0, 2.0, 3.0, 4.0]);
        let mut out = [0.0; 4];
        assert_eq!(consumer.pop(&mut out), 4);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn ring_pop_reports_partial_fill() {
        let (producer, consumer) = ring_pair(8);
        producer.push_blocking(&[5.0, 6.0]);
        let mut out = [0.0; 4];
        assert_eq!(consumer.pop(&mut out), 2, "only two samples were available");
    }

    #[test]
    fn ring_wraps_around_capacity() {
        // Capacity rounds up to 8; push/pop more than that to force a wrap.
        let (producer, consumer) = ring_pair(5);
        let mut next = 0.0_f32;
        let mut expect = 0.0_f32;
        for _ in 0..10 {
            producer.push_blocking(&[next, next + 1.0]);
            next += 2.0;
            let mut out = [0.0; 2];
            assert_eq!(consumer.pop(&mut out), 2);
            assert_eq!(out, [expect, expect + 1.0]);
            expect += 2.0;
        }
    }

    #[test]
    fn drain_drops_buffered_audio() {
        let (producer, consumer) = ring_pair(8);
        producer.push_blocking(&[1.0, 2.0, 3.0, 4.0]);
        consumer.drain();
        let mut out = [0.0; 4];
        assert_eq!(consumer.pop(&mut out), 0, "drained ring yields nothing");
    }

    #[test]
    fn resampler_upsamples_to_a_higher_rate() {
        // 44.1 kHz -> 88.2 kHz: roughly twice as many output frames.
        let mut resampler = Resampler {
            step: 44_100.0 / 88_200.0,
            pos: 0.0,
            last: [0.0; 2],
        };
        let input: Vec<f32> = (0..200).map(|n| n as f32).collect();
        let mut out = Vec::new();
        resampler.process(&input, &mut out);
        let in_frames = input.len() / 2;
        let out_frames = out.len() / 2;
        assert!(
            out_frames >= in_frames * 2 - 2 && out_frames <= in_frames * 2 + 2,
            "expected ~{} frames, got {out_frames}",
            in_frames * 2,
        );
    }
}
