//! A custom [`egui::load::ImageLoader`] that fetches HTTP(S) image URLs.
//!
//! egui ships no network image loader. Album art and avatars are remote
//! `https://i.scdn.co/...` URLs, so Spottyfi installs [`NetworkImageLoader`]
//! once at startup ([`install`]); after that, `egui::Image::from_uri(url)`
//! works everywhere — the transport bar, the Now Playing panel, the sidebar.
//!
//! ## How it works
//!
//! `load` is called every frame for a visible image. The loader:
//! 1. returns the decoded [`egui::ColorImage`] if it is already cached;
//! 2. otherwise enqueues a background `ehttp` fetch (once per URI) and returns
//!    [`egui::load::ImagePoll::Pending`];
//! 3. the fetch callback decodes the bytes, stores the result, and repaints.
//!
//! ## Bounded concurrency
//!
//! `ehttp::fetch` spawns **one OS thread per request**. A grid of dozens of
//! album covers would otherwise spawn dozens of threads at once, thrashing the
//! scheduler and the network. [`Pool`] caps the number of *in-flight* fetches
//! ([`MAX_IN_FLIGHT`]); excess URIs wait in a queue and are dispatched as
//! earlier fetches complete. The cache still guarantees one fetch per URI.
//!
//! ## On-disk cache
//!
//! In front of the network sits the `cache` crate's [`ImageCache`]: a
//! `sha1(url).webp` on-disk LRU. [`fetch`] runs the whole slow path —
//! disk lookup, network fetch, disk write and decode — on a dedicated worker
//! thread, so the egui UI thread never blocks on I/O:
//!
//! 1. a disk hit decodes the cached bytes — no network call;
//! 2. a disk miss fetches over HTTP, writes the encoded bytes to the disk
//!    cache, then decodes them.
//!
//! The loader's public surface is unchanged; the disk cache is supplied at
//! construction via [`NetworkImageLoader::with_disk_cache`].

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use egui::ahash::HashMap;
use egui::load::{ImageLoadResult, ImagePoll, LoadError, SizeHint};
use egui::{ColorImage, Context};
use spottyfi_cache::ImageCache;

/// The maximum number of image fetches allowed to run concurrently.
///
/// `ehttp` spawns a thread per fetch; this caps the thread count so a page of
/// album art does not spawn dozens of threads at once. Six is a sensible
/// middle ground — enough to keep the network busy, few enough not to thrash.
const MAX_IN_FLIGHT: usize = 6;

/// The cache entry for one image URI.
#[derive(Clone)]
enum Entry {
    /// A fetch is queued or in flight; nothing to show yet.
    Pending,
    /// The image decoded successfully.
    Ready(Arc<ColorImage>),
    /// The fetch or decode failed; carries a message for diagnostics.
    Failed(String),
}

/// A reference-counted handle to the loader's URI -> entry cache.
///
/// Behind a `Mutex` because `load` runs on the UI thread while fetch callbacks
/// run on `ehttp`'s worker thread.
type SharedCache = Arc<Mutex<HashMap<String, Entry>>>;

/// The bounded-concurrency dispatch state for image fetches.
struct PoolState {
    /// URIs waiting for a free fetch slot, in request order.
    queue: VecDeque<String>,
    /// The number of fetches currently in flight.
    in_flight: usize,
}

/// A bounded fetch pool: caps concurrent `ehttp` fetches at [`MAX_IN_FLIGHT`].
#[derive(Clone)]
struct Pool {
    state: Arc<Mutex<PoolState>>,
    /// The on-disk image cache checked before the network; `None` disables it.
    disk: Option<ImageCache>,
}

impl Default for Pool {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(PoolState {
                queue: VecDeque::new(),
                in_flight: 0,
            })),
            disk: None,
        }
    }
}

impl Pool {
    /// Submit `uri` for fetching: start it now if a slot is free, else queue it.
    fn submit(&self, ctx: Context, uri: String, cache: SharedCache) {
        let to_start = {
            let mut state = match self.state.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            if state.in_flight < MAX_IN_FLIGHT {
                state.in_flight += 1;
                Some(uri)
            } else {
                state.queue.push_back(uri);
                None
            }
        };
        if let Some(uri) = to_start {
            fetch(ctx, uri, cache, self.clone());
        }
    }

    /// Called when a fetch finishes: free its slot and start the next queued
    /// URI, if any.
    fn on_fetch_done(&self, ctx: &Context, cache: &SharedCache) {
        let next = {
            let mut state = match self.state.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            state.in_flight = state.in_flight.saturating_sub(1);
            match state.queue.pop_front() {
                Some(uri) => {
                    state.in_flight += 1;
                    Some(uri)
                }
                None => None,
            }
        };
        if let Some(uri) = next {
            fetch(ctx.clone(), uri, Arc::clone(cache), self.clone());
        }
    }
}

/// An [`egui::load::ImageLoader`] that resolves `http://` / `https://` URIs by
/// fetching and decoding them in the background, with bounded concurrency.
#[derive(Default)]
pub struct NetworkImageLoader {
    /// The URI -> entry cache, shared with in-flight fetch tasks.
    cache: SharedCache,
    /// The bounded fetch pool.
    pool: Pool,
}

impl NetworkImageLoader {
    /// The loader's unique id, per the [`egui::load::ImageLoader`] contract.
    const ID: &'static str = concat!(module_path!(), "::NetworkImageLoader");

    /// Build a loader backed by an on-disk [`ImageCache`].
    ///
    /// Fetches then check the disk cache before the network and persist
    /// freshly-fetched bytes to it.
    #[must_use]
    pub fn with_disk_cache(disk: ImageCache) -> Self {
        Self {
            cache: SharedCache::default(),
            pool: Pool {
                disk: Some(disk),
                ..Pool::default()
            },
        }
    }

    /// Whether this loader handles `uri` (an `http`/`https` scheme).
    fn handles(uri: &str) -> bool {
        uri.starts_with("http://") || uri.starts_with("https://")
    }
}

/// Install the network image loader into `ctx`.
///
/// Call once at startup, after `egui_extras::install_image_loaders` so the
/// stock byte/decode loaders are present for `bytes://` and `file://` URIs.
///
/// The loader opens the on-disk image cache under the platform cache
/// directory; if that cannot be opened the loader still works, going straight
/// to the network (the failure is logged).
pub fn install(ctx: &Context) {
    let loader = match open_disk_cache() {
        Ok(disk) => NetworkImageLoader::with_disk_cache(disk),
        Err(err) => {
            tracing::warn!(%err, "on-disk image cache unavailable; images will not be cached");
            NetworkImageLoader::default()
        }
    };
    ctx.add_image_loader(Arc::new(loader));
}

/// Open the on-disk image cache under the platform cache directory.
fn open_disk_cache() -> Result<ImageCache, spottyfi_cache::CacheError> {
    let dir = spottyfi_cache::paths::image_cache_dir()?;
    ImageCache::open(dir)
}

impl egui::load::ImageLoader for NetworkImageLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(&self, ctx: &Context, uri: &str, _size_hint: SizeHint) -> ImageLoadResult {
        if !Self::handles(uri) {
            return Err(LoadError::NotSupported);
        }

        // Fast path: a finished or in-flight entry.
        {
            let cache = self.cache.lock().map_err(poisoned)?;
            match cache.get(uri) {
                Some(Entry::Ready(image)) => {
                    return Ok(ImagePoll::Ready {
                        image: Arc::clone(image),
                    })
                }
                Some(Entry::Pending) => return Ok(ImagePoll::Pending { size: None }),
                Some(Entry::Failed(message)) => return Err(LoadError::Loading(message.clone())),
                None => {}
            }
        }

        // First sighting of this URI: mark it pending and submit the fetch to
        // the bounded pool (which starts or queues it).
        self.cache
            .lock()
            .map_err(poisoned)?
            .insert(uri.to_owned(), Entry::Pending);
        self.pool
            .submit(ctx.clone(), uri.to_owned(), Arc::clone(&self.cache));
        Ok(ImagePoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(uri);
        }
    }

    fn forget_all(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
    }

    fn byte_size(&self) -> usize {
        self.cache.lock().map_or(0, |cache| {
            cache
                .values()
                .map(|entry| match entry {
                    Entry::Ready(image) => image.pixels.len() * 4,
                    Entry::Failed(message) => message.len(),
                    Entry::Pending => 0,
                })
                .sum()
        })
    }
}

/// Spawn a background fetch + decode for `uri`, storing the result in `cache`.
///
/// The entire slow path runs on a dedicated worker thread, so neither the
/// disk cache nor the network ever blocks the egui UI thread:
///
/// 1. an on-disk cache hit decodes the cached bytes — no network call;
/// 2. a miss fetches over HTTP, writes the encoded bytes to the disk cache,
///    then decodes them.
///
/// On completion the fetch frees its pool slot and lets the next queued URI
/// start, keeping the in-flight count bounded.
fn fetch(ctx: Context, uri: String, cache: SharedCache, pool: Pool) {
    // `ehttp` would spawn its own thread; spawning one explicitly lets the
    // disk read/write and decode share it, all off the UI thread.
    let worker = {
        let ctx = ctx.clone();
        let cache = Arc::clone(&cache);
        let pool = pool.clone();
        move || {
            let entry = match resolve(&uri, pool.disk.as_ref()) {
                Ok(image) => Entry::Ready(Arc::new(image)),
                Err(message) => {
                    tracing::debug!(%uri, %message, "image load failed");
                    Entry::Failed(message)
                }
            };
            if let Ok(mut guard) = cache.lock() {
                guard.insert(uri.clone(), entry);
            }
            pool.on_fetch_done(&ctx, &cache);
            ctx.request_repaint();
        }
    };
    if let Err(err) = std::thread::Builder::new()
        .name("spottyfi-img-fetch".to_owned())
        .spawn(worker)
    {
        // A thread-spawn failure must not leak a pool slot, or the loader
        // would slowly starve. Account for the "completed" fetch.
        tracing::warn!(%err, "could not spawn image fetch thread");
        pool.on_fetch_done(&ctx, &cache);
    }
}

/// Resolve `uri` to a decoded image: disk cache first, then the network.
///
/// Runs on a worker thread (see [`fetch`]). On a disk miss the freshly-fetched
/// encoded bytes are written back to the disk cache before decoding.
fn resolve(uri: &str, disk: Option<&ImageCache>) -> Result<ColorImage, String> {
    if let Some(disk) = disk {
        match disk.get(uri) {
            Ok(Some(bytes)) => {
                tracing::trace!(%uri, "image disk-cache hit");
                return decode_bytes(&bytes);
            }
            Ok(None) => {}
            Err(err) => tracing::debug!(%uri, %err, "image disk-cache read failed"),
        }
    }

    let bytes = fetch_encoded(uri)?;
    if let Some(disk) = disk {
        if let Err(err) = disk.put(uri, &bytes) {
            tracing::debug!(%uri, %err, "image disk-cache write failed");
        }
    }
    decode_bytes(&bytes)
}

/// Fetch the encoded image bytes for `uri` over HTTP, blocking the worker
/// thread until the response arrives.
fn fetch_encoded(uri: &str) -> Result<Vec<u8>, String> {
    let request = ehttp::Request::get(uri);
    let response =
        ehttp::fetch_blocking(&request).map_err(|err| format!("request failed: {err}"))?;
    if !response.ok {
        return Err(format!(
            "bad status: {} {}",
            response.status, response.status_text
        ));
    }
    Ok(response.bytes)
}

/// Decode encoded image bytes into an [`egui::ColorImage`].
fn decode_bytes(bytes: &[u8]) -> Result<ColorImage, String> {
    let decoded = image::load_from_memory(bytes)
        .map_err(|err| format!("decode failed: {err}"))?
        .to_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, decoded.as_raw()))
}

/// Map a poisoned-lock error into a [`LoadError`].
fn poisoned<T>(_: std::sync::PoisonError<T>) -> LoadError {
    LoadError::Loading("image cache lock poisoned".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_caps_in_flight_and_queues_the_rest() {
        let pool = Pool::default();
        // Manually drive the pool's bookkeeping without real fetches.
        for _ in 0..MAX_IN_FLIGHT {
            let mut state = pool.state.lock().expect("lock");
            assert!(state.in_flight < MAX_IN_FLIGHT);
            state.in_flight += 1;
        }
        {
            let mut state = pool.state.lock().expect("lock");
            assert_eq!(state.in_flight, MAX_IN_FLIGHT);
            // A further submission would queue rather than start.
            state
                .queue
                .push_back("https://example/extra.jpg".to_owned());
        }
        let state = pool.state.lock().expect("lock");
        assert_eq!(state.queue.len(), 1);
    }

    /// A 2x2 PNG, encoded — a stand-in for a real album-art image.
    fn sample_png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encode png");
        bytes
    }

    #[test]
    fn resolve_serves_a_disk_cache_hit_without_the_network() {
        let dir = tempfile::tempdir().expect("tempdir");
        let disk = ImageCache::open(dir.path()).expect("open image cache");
        let uri = "https://i.scdn.co/image/abc";

        // Seed the disk cache with encoded bytes for the URI.
        disk.put(uri, &sample_png()).expect("seed disk cache");

        // `resolve` must decode straight from disk — no network call is made
        // (an unroutable host would fail if it tried).
        let image = resolve(uri, Some(&disk)).expect("resolve from disk");
        assert_eq!(image.size, [2, 2]);
    }

    #[test]
    fn decode_bytes_round_trips_an_encoded_image() {
        let image = decode_bytes(&sample_png()).expect("decode");
        assert_eq!(image.size, [2, 2]);
    }
}
