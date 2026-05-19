//! HTTP audio playback — the OpenSubsonic (and any plain-HTTP) backend.
//!
//! Spotify plays through librespot; an OpenSubsonic server just serves an
//! audio file over HTTP. [`HttpAudioPlayer`] fetches that URL, decodes it
//! with [`symphonia`] (FLAC / MP3 / Ogg-Vorbis) and feeds the PCM into a
//! [`CpalOutput`] — the *same* output stage librespot uses, so volume, pause
//! and the gain ramp behave identically whatever the source.
//!
//! `cpal::Stream` is `!Send`, so the output and the decode loop live on one
//! dedicated thread; [`HttpAudioPlayer`] is a `Send` handle that drives it
//! through an [`mpsc`](std::sync::mpsc) channel and a few shared atomics.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;
use tokio::runtime::Handle;

use crate::cpal_sink::{CpalOutput, Resampler, SinkControls};
use crate::error::{AudioError, AudioResult};

/// The `seek_ms` sentinel meaning "no seek requested".
const NO_SEEK: u64 = u64::MAX;

/// Mutable state shared between the [`HttpAudioPlayer`] handle and its
/// decode thread.
struct Shared {
    /// Bumped on every `load`/`stop`; the decode loop aborts when the value
    /// it started with no longer matches, so a stale track stops at once.
    generation: AtomicU64,
    /// The decoded play position, in milliseconds.
    position_ms: AtomicU64,
    /// `true` while a track is actively decoding.
    playing: AtomicBool,
    /// `true` once the current track has decoded through to its end.
    finished: AtomicBool,
    /// A pending seek target in milliseconds, or [`NO_SEEK`].
    seek_ms: AtomicU64,
}

impl Shared {
    /// Fresh, idle shared state.
    fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            position_ms: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            seek_ms: AtomicU64::new(NO_SEEK),
        }
    }
}

/// A command sent to the decode thread.
enum Command {
    /// Decode and play these already-fetched bytes, if `generation` is still
    /// current.
    Play {
        /// The complete audio file.
        bytes: Vec<u8>,
        /// The generation this load belongs to.
        generation: u64,
    },
    /// Tear the thread down.
    Shutdown,
}

/// A handle that plays audio fetched over HTTP.
///
/// Cheap to construct failures aside; clone-free — hold one per engine. All
/// methods are non-blocking: the fetch runs on the tokio runtime and the
/// decode on the player's own thread.
pub struct HttpAudioPlayer {
    /// The tokio handle used to fetch track bytes.
    runtime: Handle,
    /// The command channel to the decode thread.
    commands: Sender<Command>,
    /// The real-time control surface (gain / pause / flush) — shared with the
    /// [`CpalOutput`] on the decode thread.
    controls: SinkControls,
    /// State shared with the decode thread.
    shared: Arc<Shared>,
    /// The HTTP client for fetching streams.
    http: reqwest::Client,
}

impl HttpAudioPlayer {
    /// Open the audio device and start the decode thread.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NoBackend`] if the output device cannot be
    /// opened or the decode thread cannot be spawned.
    pub fn new(runtime: Handle) -> AudioResult<Self> {
        let controls = SinkControls::new(1.0);
        let shared = Arc::new(Shared::new());
        let (commands, command_rx) = std::sync::mpsc::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();

        let thread_controls = controls.clone();
        let thread_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("spottyfi-http-audio".to_owned())
            .spawn(move || decode_thread(&thread_controls, &command_rx, &thread_shared, &ready_tx))
            .map_err(|err| {
                tracing::error!(%err, "spawning the http-audio thread failed");
                AudioError::NoBackend
            })?;

        // The thread opens the cpal device first and reports the result.
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(_) => return Err(AudioError::NoBackend),
        }

        let http = reqwest::Client::builder()
            .user_agent(concat!("Spottyfi/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| {
                tracing::error!(%err, "building the http-audio client failed");
                AudioError::NoBackend
            })?;

        Ok(Self {
            runtime,
            commands,
            controls,
            shared,
            http,
        })
    }

    /// Fetch `url` and start playing it, replacing whatever was playing.
    pub fn load(&self, url: String) {
        let generation = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared.position_ms.store(0, Ordering::SeqCst);
        self.shared.finished.store(false, Ordering::SeqCst);
        self.shared.seek_ms.store(NO_SEEK, Ordering::SeqCst);
        // Drop the previous track's buffered audio and un-pause.
        self.controls.flush.store(true, Ordering::SeqCst);
        self.controls.paused.store(false, Ordering::SeqCst);

        let http = self.http.clone();
        let commands = self.commands.clone();
        let shared = Arc::clone(&self.shared);
        self.runtime.spawn(async move {
            let bytes = match fetch(&http, &url).await {
                Ok(bytes) => bytes,
                Err(err) => {
                    tracing::warn!(%err, %url, "fetching the audio stream failed");
                    return;
                }
            };
            // A newer load may have superseded this one while fetching.
            if shared.generation.load(Ordering::SeqCst) == generation {
                let _ = commands.send(Command::Play { bytes, generation });
            }
        });
    }

    /// Pause playback — instantly, the audio callback emits silence.
    pub fn pause(&self) {
        self.controls.paused.store(true, Ordering::SeqCst);
    }

    /// Resume playback.
    pub fn resume(&self) {
        self.controls.paused.store(false, Ordering::SeqCst);
    }

    /// Stop playback and discard the current track.
    pub fn stop(&self) {
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        self.controls.paused.store(true, Ordering::SeqCst);
        // Drop the buffered audio so a later resume cannot replay a stopped
        // track's tail.
        self.controls.flush.store(true, Ordering::SeqCst);
    }

    /// Set the output volume from a `0.0..=1.0` fraction.
    ///
    /// A perceptual (cubic) curve is applied so the fader feels even, then
    /// the gain lands instantly via the callback's smoothing ramp.
    pub fn set_volume(&self, volume: f32) {
        let gain = volume.clamp(0.0, 1.0).powi(3);
        self.controls.gain.store(gain.to_bits(), Ordering::SeqCst);
    }

    /// Seek within the current track.
    pub fn seek(&self, position: Duration) {
        let ms = u64::try_from(position.as_millis()).unwrap_or(0);
        self.shared.seek_ms.store(ms, Ordering::SeqCst);
    }

    /// The current decoded play position.
    #[must_use]
    pub fn position(&self) -> Duration {
        Duration::from_millis(self.shared.position_ms.load(Ordering::SeqCst))
    }

    /// Whether a track is actively playing.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::SeqCst)
    }

    /// Whether the current track has decoded through to its end.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.shared.finished.load(Ordering::SeqCst)
    }
}

impl Drop for HttpAudioPlayer {
    fn drop(&mut self) {
        // Bump the generation first so a decode loop in progress aborts
        // promptly, then ask the thread to exit. Without the bump the thread
        // would not see `Shutdown` until the current track finished decoding.
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        let _ = self.commands.send(Command::Shutdown);
    }
}

/// Fetch a URL's full body.
async fn fetch(http: &reqwest::Client, url: &str) -> AudioResult<Vec<u8>> {
    let response = http
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|err| AudioError::Connect(err.to_string()))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|err| AudioError::Connect(err.to_string()))?;
    Ok(bytes.to_vec())
}

/// The decode thread: owns the `!Send` [`CpalOutput`], then serves commands.
fn decode_thread(
    controls: &SinkControls,
    commands: &Receiver<Command>,
    shared: &Shared,
    ready: &Sender<AudioResult<()>>,
) {
    let flush = Arc::clone(&controls.flush);
    let (output, device_rate) = match CpalOutput::open(controls.clone()) {
        Ok(opened) => {
            let _ = ready.send(Ok(()));
            opened
        }
        Err(err) => {
            let _ = ready.send(Err(err));
            return;
        }
    };

    while let Ok(command) = commands.recv() {
        match command {
            Command::Shutdown => break,
            Command::Play { bytes, generation } => {
                if shared.generation.load(Ordering::SeqCst) != generation {
                    continue;
                }
                decode_and_play(&output, device_rate, &flush, bytes, generation, shared);
            }
        }
    }
}

/// Decode `bytes` and stream the PCM into `output` until the track ends or a
/// newer generation supersedes it.
///
/// `finished` is set on *every* terminal exit — a clean end *and* a probe or
/// decoder failure — so the queue advances past an unplayable track rather
/// than stalling forever waiting for a "finished" that never comes.
fn decode_and_play(
    output: &CpalOutput,
    device_rate: u32,
    flush: &AtomicBool,
    bytes: Vec<u8>,
    generation: u64,
    shared: &Shared,
) {
    let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let probed = match symphonia::default::get_probe().format(
        &Hint::new(),
        stream,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) {
        Ok(probed) => probed,
        Err(err) => {
            tracing::warn!(%err, "could not probe the audio stream");
            shared.finished.store(true, Ordering::SeqCst);
            return;
        }
    };
    let mut format = probed.format;
    let Some(track) = format.default_track() else {
        tracing::warn!("the audio stream has no playable track");
        shared.finished.store(true, Ordering::SeqCst);
        return;
    };
    let track_id = track.id;
    let time_base = track.codec_params.time_base;
    let file_rate = track.codec_params.sample_rate.unwrap_or(device_rate);
    let mut decoder = match symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
    {
        Ok(decoder) => decoder,
        Err(err) => {
            tracing::warn!(%err, "no decoder for the audio stream");
            shared.finished.store(true, Ordering::SeqCst);
            return;
        }
    };
    let mut resampler = (file_rate != device_rate).then(|| Resampler::new(file_rate, device_rate));

    shared.playing.store(true, Ordering::SeqCst);
    shared.finished.store(false, Ordering::SeqCst);

    let mut stereo: Vec<f32> = Vec::new();
    let mut resampled: Vec<f32> = Vec::new();

    loop {
        // A newer load (or a stop) supersedes this track.
        if shared.generation.load(Ordering::SeqCst) != generation {
            break;
        }
        // Apply a pending seek.
        let seek = shared.seek_ms.swap(NO_SEEK, Ordering::SeqCst);
        if seek != NO_SEEK {
            let target = Time::from(Duration::from_millis(seek).as_secs_f64());
            if let Err(err) = format.seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: target,
                    track_id: Some(track_id),
                },
            ) {
                tracing::debug!(%err, "seek failed");
            }
            decoder.reset();
            flush.store(true, Ordering::SeqCst);
        }

        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(_) => {
                // No more packets — the track is done.
                shared.finished.store(true, Ordering::SeqCst);
                break;
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        if let Some(time_base) = time_base {
            let time = time_base.calc_time(packet.ts());
            let ms = time.seconds * 1000 + (time.frac * 1000.0) as u64;
            shared.position_ms.store(ms, Ordering::SeqCst);
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(symphonia::core::errors::Error::DecodeError(err)) => {
                tracing::debug!(%err, "skipping an undecodable packet");
                continue;
            }
            Err(err) => {
                tracing::warn!(%err, "decode error; stopping the track");
                break;
            }
        };

        let spec = *decoded.spec();
        let mut sample_buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        to_stereo(sample_buf.samples(), spec.channels.count(), &mut stereo);

        // Push abortably: if the ring is full while paused it is frozen, so a
        // plain blocking push could wedge the thread past a stop / new load.
        let superseded = || shared.generation.load(Ordering::SeqCst) != generation;
        let pushed = match resampler.as_mut() {
            Some(resampler) => {
                resampled.clear();
                resampler.process(&stereo, &mut resampled);
                output.push_samples_abortable(&resampled, superseded)
            }
            None => output.push_samples_abortable(&stereo, superseded),
        };
        if !pushed {
            break;
        }
    }

    shared.playing.store(false, Ordering::SeqCst);
}

/// Fold an interleaved buffer of `channels` channels down/up to stereo.
fn to_stereo(interleaved: &[f32], channels: usize, out: &mut Vec<f32>) {
    out.clear();
    match channels {
        0 => {}
        1 => {
            for &sample in interleaved {
                out.push(sample);
                out.push(sample);
            }
        }
        2 => out.extend_from_slice(interleaved),
        n => {
            for frame in interleaved.chunks(n) {
                let left = frame.first().copied().unwrap_or(0.0);
                let right = frame.get(1).copied().unwrap_or(left);
                out.push(left);
                out.push(right);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_is_duplicated_to_stereo() {
        let mut out = Vec::new();
        to_stereo(&[0.5, -0.5], 1, &mut out);
        assert_eq!(out, [0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn stereo_passes_through() {
        let mut out = Vec::new();
        to_stereo(&[0.1, 0.2, 0.3, 0.4], 2, &mut out);
        assert_eq!(out, [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn surround_keeps_the_front_pair() {
        let mut out = Vec::new();
        // One 5.1 frame: L R C LFE Ls Rs.
        to_stereo(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 6, &mut out);
        assert_eq!(out, [1.0, 2.0]);
    }
}
