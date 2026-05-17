//! Background full-song waveform analysis.
//!
//! The transport's seek bar draws the waveform of the *whole* track, not a
//! live rolling window. There is no Spotify API for a track waveform, so when
//! a track starts [`WaveformAnalyzer::analyze`] decodes the entire audio file
//! in the background — fetching and decrypting it through librespot, then
//! decoding the Ogg Vorbis with `lewton` — and builds a fixed-resolution peak
//! envelope the UI draws once it is ready.
//!
//! The decode is wholly independent of playback: it opens its own
//! [`AudioFile`], runs the heavy work on a blocking worker, and on *any*
//! failure simply publishes nothing, so the seek bar falls back to a plain
//! capsule. Playback is never touched.

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwap;
use librespot::audio::{AudioDecrypt, AudioFile};
use librespot::core::{FileId, Session, SpotifyId};
use librespot::metadata::audio::AudioFileFormat;
use tokio::runtime::Handle;

/// Number of envelope points spanning the whole track.
pub const WAVEFORM_RESOLUTION: usize = 1600;

/// Spotify prepends a custom header before the real Ogg Vorbis stream; the
/// Vorbis data begins at this byte offset.
const SPOTIFY_OGG_HEADER_END: u64 = 0xa7;

/// Mono frames summarised by one intermediate peak point during decode. The
/// intermediate envelope is downsampled again to [`WAVEFORM_RESOLUTION`].
const DECODE_SLICE_FRAMES: usize = 2048;

/// A decoded full-song waveform, published by [`WaveformAnalyzer`].
#[derive(Debug, Clone, Default)]
pub struct TrackWaveform {
    /// The canonical Spotify URI this envelope belongs to. Empty before any
    /// analysis has run; the UI matches it against the playing track so a
    /// stale envelope is never drawn under the wrong song.
    pub uri: String,
    /// Peak-amplitude envelope, oldest-first, normalised to `0.0..=1.0`.
    /// [`WAVEFORM_RESOLUTION`] points once analysis succeeds; empty while
    /// analysis is in flight or after a failure (the seek bar then falls back
    /// to a plain capsule).
    pub envelope: Vec<f32>,
}

/// Shared handle to the background waveform analyser.
///
/// Cloning is cheap (an `Arc` bump). The engine holds one and drives
/// [`Self::analyze`] on each track change; the UI holds one and reads
/// [`Self::current`] every frame.
#[derive(Clone)]
pub struct WaveformAnalyzer {
    inner: Arc<Inner>,
}

/// The analyser's shared interior.
struct Inner {
    /// The latest published waveform.
    published: ArcSwap<TrackWaveform>,
    /// Bumped on every [`WaveformAnalyzer::analyze`] call; a decode task
    /// publishes only while its captured generation is still current, so a
    /// track skipped mid-decode cannot overwrite the newer track's waveform.
    generation: AtomicU64,
}

impl WaveformAnalyzer {
    /// Create an analyser with no waveform yet published.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                published: ArcSwap::from_pointee(TrackWaveform::default()),
                generation: AtomicU64::new(0),
            }),
        }
    }

    /// The most recently published waveform. A single atomic load — safe to
    /// call from the UI thread every frame.
    #[must_use]
    pub fn current(&self) -> Arc<TrackWaveform> {
        self.inner.published.load_full()
    }

    /// Begin analysing `uri`'s audio file in the background.
    ///
    /// `track_id` keys the decryption-key request; `files` is the track's
    /// available audio files (from the librespot `AudioItem`). Any earlier
    /// in-flight analysis is superseded — its result will be discarded. The
    /// async fetch and the blocking decode both run on `runtime`.
    pub fn analyze(
        &self,
        runtime: &Handle,
        session: Session,
        track_id: SpotifyId,
        uri: String,
        files: HashMap<AudioFileFormat, FileId>,
    ) {
        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // Publish an empty placeholder for the new URI so the UI drops the
        // previous track's waveform immediately rather than showing it stale.
        self.inner.published.store(Arc::new(TrackWaveform {
            uri: uri.clone(),
            envelope: Vec::new(),
        }));

        let Some((format, file_id)) = pick_vorbis_file(&files) else {
            tracing::debug!(%uri, "waveform: no Ogg Vorbis file available; skipping");
            return;
        };
        let inner = Arc::clone(&self.inner);
        let runtime_for_blocking = runtime.clone();

        runtime.spawn(async move {
            match decode_envelope(&runtime_for_blocking, &session, track_id, file_id, format).await
            {
                Ok(envelope) => {
                    // Only publish if no newer analysis has started since.
                    if inner.generation.load(Ordering::SeqCst) == generation {
                        inner
                            .published
                            .store(Arc::new(TrackWaveform { uri, envelope }));
                        tracing::debug!("waveform: full-song analysis complete");
                    }
                }
                Err(err) => tracing::debug!(%err, %uri, "waveform: analysis failed"),
            }
        });
    }
}

impl Default for WaveformAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// Pick the best available Ogg Vorbis file, highest bitrate first.
///
/// Only Ogg Vorbis is handled — MP3 fallbacks (rare for Premium accounts)
/// yield `None` and the seek bar simply keeps its plain capsule.
fn pick_vorbis_file(files: &HashMap<AudioFileFormat, FileId>) -> Option<(AudioFileFormat, FileId)> {
    [
        AudioFileFormat::OGG_VORBIS_320,
        AudioFileFormat::OGG_VORBIS_160,
        AudioFileFormat::OGG_VORBIS_96,
    ]
    .into_iter()
    .find_map(|format| files.get(&format).map(|id| (format, *id)))
}

/// The streaming data rate librespot uses to size the file's read-ahead.
fn bytes_per_second(format: AudioFileFormat) -> usize {
    let kbps = match format {
        AudioFileFormat::OGG_VORBIS_96 => 12.0,
        AudioFileFormat::OGG_VORBIS_160 => 20.0,
        _ => 40.0,
    };
    (kbps * 1024.0_f32).ceil() as usize
}

/// Fetch, decrypt and decode the whole track, returning its peak envelope.
async fn decode_envelope(
    runtime: &Handle,
    session: &Session,
    track_id: SpotifyId,
    file_id: FileId,
    format: AudioFileFormat,
) -> Result<Vec<f32>, String> {
    let file = AudioFile::open(session, file_id, bytes_per_second(format))
        .await
        .map_err(|err| format!("open audio file: {err}"))?;
    // Some files are unencrypted; a missing key just means no decryption.
    let key = session.audio_key().request(track_id, file_id).await.ok();
    let decrypted = AudioDecrypt::new(key, file);

    // Decoding reads the file synchronously and blocks on the network, so the
    // whole decode runs on a blocking worker rather than a runtime thread.
    runtime
        .spawn_blocking(move || decode_blocking(decrypted))
        .await
        .map_err(|err| format!("decode task join: {err}"))?
}

/// Decode every Vorbis packet and build the intermediate peak envelope.
///
/// Runs on a blocking worker — the [`AudioFile`] read path blocks on network
/// fetches.
fn decode_blocking(decrypted: AudioDecrypt<AudioFile>) -> Result<Vec<f32>, String> {
    // The real Ogg stream starts after Spotify's custom header.
    let reader = OffsetReader::new(decrypted, SPOTIFY_OGG_HEADER_END)
        .map_err(|err| format!("seek past Spotify Ogg header: {err}"))?;
    let mut ogg = lewton::inside_ogg::OggStreamReader::new(reader)
        .map_err(|err| format!("open Vorbis stream: {err}"))?;
    let channels = usize::from(ogg.ident_hdr.audio_channels).max(1);

    let mut slices: Vec<f32> = Vec::new();
    let mut slice_peak = 0.0_f32;
    let mut slice_frames = 0_usize;

    loop {
        match ogg.read_dec_packet_itl() {
            Ok(Some(packet)) => {
                for frame in packet.chunks(channels) {
                    let mono = frame
                        .iter()
                        .map(|&s| f32::from(s) / f32::from(i16::MAX))
                        .sum::<f32>()
                        / channels as f32;
                    slice_peak = slice_peak.max(mono.abs());
                    slice_frames += 1;
                    if slice_frames >= DECODE_SLICE_FRAMES {
                        slices.push(slice_peak);
                        slice_peak = 0.0;
                        slice_frames = 0;
                    }
                }
            }
            Ok(None) => break,
            Err(err) => return Err(format!("decode Vorbis packet: {err}")),
        }
    }
    if slice_frames > 0 {
        slices.push(slice_peak);
    }
    if slices.is_empty() {
        return Err("no audio decoded".to_owned());
    }
    Ok(resample_envelope(&slices, WAVEFORM_RESOLUTION))
}

/// Downsample `slices` to exactly `target` points by bucketed peak, then
/// normalise the result so the loudest point reads as `1.0`.
fn resample_envelope(slices: &[f32], target: usize) -> Vec<f32> {
    if slices.is_empty() || target == 0 {
        return Vec::new();
    }
    let len = slices.len();
    let mut out = Vec::with_capacity(target);
    for i in 0..target {
        let start = i * len / target;
        let end = (((i + 1) * len / target).max(start + 1)).min(len);
        let peak = slices[start..end].iter().copied().fold(0.0_f32, f32::max);
        out.push(peak);
    }
    let max = out.iter().copied().fold(0.0_f32, f32::max);
    if max > 1e-6 {
        for v in &mut out {
            *v = (*v / max).clamp(0.0, 1.0);
        }
    }
    out
}

/// A `Read + Seek` adapter that hides the first `offset` bytes of `inner`,
/// presenting the Ogg Vorbis stream that begins after Spotify's custom header.
struct OffsetReader<T> {
    /// The underlying decrypted audio file.
    inner: T,
    /// Bytes hidden at the start of `inner`.
    offset: u64,
}

impl<T: Seek> OffsetReader<T> {
    /// Wrap `inner`, positioning it at `offset` so reading starts at the Ogg
    /// stream proper.
    fn new(mut inner: T, offset: u64) -> io::Result<Self> {
        inner.seek(SeekFrom::Start(offset))?;
        Ok(Self { inner, offset })
    }
}

impl<T: Read> Read for OffsetReader<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<T: Seek> Seek for OffsetReader<T> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let physical = match pos {
            SeekFrom::Start(o) => self.inner.seek(SeekFrom::Start(o + self.offset))?,
            SeekFrom::Current(o) => self.inner.seek(SeekFrom::Current(o))?,
            SeekFrom::End(o) => self.inner.seek(SeekFrom::End(o))?,
        };
        Ok(physical.saturating_sub(self.offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_to_fewer_points_keeps_the_peaks() {
        let slices = [0.1, 0.9, 0.2, 0.3, 0.8, 0.1];
        let out = resample_envelope(&slices, 3);
        assert_eq!(out.len(), 3);
        // Normalised: the loudest input (0.9) becomes 1.0.
        assert!((out.iter().copied().fold(0.0_f32, f32::max) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn resample_to_more_points_does_not_panic() {
        let out = resample_envelope(&[0.5, 1.0], 1600);
        assert_eq!(out.len(), 1600);
    }

    #[test]
    fn resample_handles_the_empty_and_silent_cases() {
        assert!(resample_envelope(&[], 100).is_empty());
        // All-silent input must not divide by zero.
        let out = resample_envelope(&[0.0, 0.0, 0.0], 2);
        assert_eq!(out, vec![0.0, 0.0]);
    }

    #[test]
    fn offset_reader_translates_positions() {
        let data: Vec<u8> = (0..50).collect();
        let mut reader = OffsetReader::new(io::Cursor::new(data), 10).expect("new");
        // Logical position 0 is physical byte 10.
        let mut one = [0_u8; 1];
        reader.read_exact(&mut one).expect("read");
        assert_eq!(one[0], 10);
        // Seek is reported in logical coordinates.
        assert_eq!(reader.seek(SeekFrom::Start(5)).expect("seek"), 5);
        reader.read_exact(&mut one).expect("read");
        assert_eq!(one[0], 15);
    }
}
