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
//! 2. otherwise spawns a background `ehttp` fetch (once per URI) and returns
//!    [`egui::load::ImagePoll::Pending`];
//! 3. the fetch callback decodes the bytes, stores the result, and repaints.
//!
//! ## Phase 9 seam
//!
//! The in-memory cache is the seam for the Phase 9 on-disk cache: [`fetch`] is
//! the single place that performs network I/O. Phase 9 slots a
//! `sha1(url).webp` disk lookup in front of the `ehttp` call and a disk write
//! into its callback, with no change to the loader's public surface.

use std::sync::{Arc, Mutex};

use egui::ahash::HashMap;
use egui::load::{ImageLoadResult, ImagePoll, LoadError, SizeHint};
use egui::{ColorImage, Context};

/// The cache entry for one image URI.
#[derive(Clone)]
enum Entry {
    /// A fetch is in flight; nothing to show yet.
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

/// An [`egui::load::ImageLoader`] that resolves `http://` / `https://` URIs by
/// fetching and decoding them in the background.
#[derive(Default)]
pub struct NetworkImageLoader {
    /// The URI -> entry cache, shared with in-flight fetch tasks.
    cache: SharedCache,
}

impl NetworkImageLoader {
    /// The loader's unique id, per the [`egui::load::ImageLoader`] contract.
    const ID: &'static str = concat!(module_path!(), "::NetworkImageLoader");

    /// Whether this loader handles `uri` (an `http`/`https` scheme).
    fn handles(uri: &str) -> bool {
        uri.starts_with("http://") || uri.starts_with("https://")
    }
}

/// Install the network image loader into `ctx`.
///
/// Call once at startup, after `egui_extras::install_image_loaders` so the
/// stock byte/decode loaders are present for `bytes://` and `file://` URIs.
pub fn install(ctx: &Context) {
    ctx.add_image_loader(Arc::new(NetworkImageLoader::default()));
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

        // First sighting of this URI: mark it pending and kick off the fetch.
        self.cache
            .lock()
            .map_err(poisoned)?
            .insert(uri.to_owned(), Entry::Pending);
        fetch(ctx.clone(), uri.to_owned(), Arc::clone(&self.cache));
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
fn fetch(ctx: Context, uri: String, cache: SharedCache) {
    let request = ehttp::Request::get(&uri);
    ehttp::fetch(request, move |result| {
        let entry = match decode_response(result) {
            Ok(image) => Entry::Ready(Arc::new(image)),
            Err(message) => {
                tracing::debug!(%uri, %message, "remote image load failed");
                Entry::Failed(message)
            }
        };
        if let Ok(mut cache) = cache.lock() {
            cache.insert(uri.clone(), entry);
        }
        ctx.request_repaint();
    });
}

/// Decode an `ehttp` response body into an [`egui::ColorImage`].
fn decode_response(result: Result<ehttp::Response, String>) -> Result<ColorImage, String> {
    let response = result.map_err(|err| format!("request failed: {err}"))?;
    if !response.ok {
        return Err(format!(
            "bad status: {} {}",
            response.status, response.status_text
        ));
    }
    let decoded = image::load_from_memory(&response.bytes)
        .map_err(|err| format!("decode failed: {err}"))?
        .to_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, decoded.as_raw()))
}

/// Map a poisoned-lock error into a [`LoadError`].
fn poisoned<T>(_: std::sync::PoisonError<T>) -> LoadError {
    LoadError::Loading("image cache lock poisoned".to_owned())
}
