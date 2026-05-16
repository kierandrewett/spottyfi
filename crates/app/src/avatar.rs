//! Optional, non-blocking avatar image loading.
//!
//! egui has no built-in HTTP image loader. This module fetches the avatar
//! bytes with `reqwest` on the tokio runtime, decodes them with the `image`
//! crate into an [`egui::ColorImage`], and hands the result back to the UI
//! thread through an [`ArcSwap`]. The UI uploads it as a texture once.
//!
//! Avatar loading is best-effort: any failure is logged and otherwise ignored
//! — it must never block or fail login.

use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::runtime::Handle;

/// The decoded avatar image, ready to be uploaded as an egui texture.
pub type SharedAvatar = Arc<ArcSwap<Option<egui::ColorImage>>>;

/// Spawn a background task that fetches and decodes the avatar at `url`.
///
/// The decoded image is stored into `slot`; the UI picks it up on its next
/// frame (the egui context is repainted to wake it). Returns immediately.
pub fn spawn_fetch(runtime: &Handle, egui_ctx: egui::Context, url: String, slot: SharedAvatar) {
    runtime.spawn(async move {
        match fetch_and_decode(&url).await {
            Ok(image) => {
                slot.store(Arc::new(Some(image)));
                egui_ctx.request_repaint();
                tracing::debug!("avatar loaded");
            }
            Err(err) => {
                // Best-effort: a missing avatar is not an error worth surfacing.
                tracing::debug!(%err, "avatar load failed; continuing without it");
            }
        }
    });
}

/// Fetch the avatar bytes and decode them into an [`egui::ColorImage`].
async fn fetch_and_decode(url: &str) -> Result<egui::ColorImage, String> {
    let bytes = reqwest::get(url)
        .await
        .map_err(|err| format!("request failed: {err}"))?
        .error_for_status()
        .map_err(|err| format!("bad status: {err}"))?
        .bytes()
        .await
        .map_err(|err| format!("reading body failed: {err}"))?;

    // Decoding is CPU work; keep it off the async worker threads.
    tokio::task::spawn_blocking(move || decode(&bytes))
        .await
        .map_err(|err| format!("decode task panicked: {err}"))?
}

/// Decode image bytes into an RGBA [`egui::ColorImage`].
fn decode(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|err| format!("decode failed: {err}"))?
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        image.as_raw(),
    ))
}
