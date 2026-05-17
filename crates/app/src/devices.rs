//! Bridges the Spotify Connect device list to the egui UI thread.
//!
//! Mirrors the other `app` controllers: a background task polls
//! `GET /me/player/devices`, the UI reads an [`ArcSwap`] snapshot every frame,
//! and a playback transfer is dispatched as a detached task.

use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use spottyfi_api::SpotifyApi;
use spottyfi_models::Device;
use tokio::runtime::Handle;

/// How often the device list is refreshed in the background.
const POLL_INTERVAL: Duration = Duration::from_secs(8);

/// Owns the Connect device-list snapshot and dispatches device commands.
pub struct DevicesController {
    /// The Spotify Web API client.
    api: Arc<dyn SpotifyApi>,
    /// Runtime handle used to spawn the poller and the transfer commands.
    runtime: Handle,
    /// egui context, woken whenever the device list changes.
    egui_ctx: egui::Context,
    /// The device-list snapshot the UI reads each frame.
    devices: Arc<ArcSwap<Vec<Device>>>,
}

impl DevicesController {
    /// Build the controller, kick off an immediate fetch and start the
    /// periodic background refresh.
    pub fn new(api: Arc<dyn SpotifyApi>, runtime: Handle, egui_ctx: egui::Context) -> Self {
        let this = Self {
            api,
            runtime,
            egui_ctx,
            devices: Arc::new(ArcSwap::from_pointee(Vec::new())),
        };
        this.spawn_poller();
        this.refresh();
        this
    }

    /// The current Connect device list, read by the UI each frame.
    #[must_use]
    pub fn snapshot(&self) -> Arc<Vec<Device>> {
        self.devices.load_full()
    }

    /// Fetch the device list once, immediately.
    pub fn refresh(&self) {
        let api = Arc::clone(&self.api);
        let slot = Arc::clone(&self.devices);
        let ctx = self.egui_ctx.clone();
        self.runtime.spawn(async move {
            match api.devices().await {
                Ok(list) => {
                    slot.store(Arc::new(list));
                    ctx.request_repaint();
                }
                Err(err) => tracing::debug!(%err, "device list refresh failed"),
            }
        });
    }

    /// Transfer playback to `device_id`, then refresh the list so the UI
    /// reflects the new active device.
    pub fn transfer(&self, device_id: String) {
        let api = Arc::clone(&self.api);
        let slot = Arc::clone(&self.devices);
        let ctx = self.egui_ctx.clone();
        self.runtime.spawn(async move {
            if let Err(err) = api.transfer_playback(&device_id, true).await {
                tracing::warn!(%err, %device_id, "playback transfer failed");
                return;
            }
            // Spotify needs a moment to settle before the device list reports
            // the new active device.
            tokio::time::sleep(Duration::from_millis(600)).await;
            if let Ok(list) = api.devices().await {
                slot.store(Arc::new(list));
            }
            ctx.request_repaint();
        });
    }

    /// Spawn the periodic background refresh task.
    fn spawn_poller(&self) {
        let api = Arc::clone(&self.api);
        let slot = Arc::clone(&self.devices);
        let ctx = self.egui_ctx.clone();
        self.runtime.spawn(async move {
            let mut tick = tokio::time::interval(POLL_INTERVAL);
            // `interval` fires immediately; the constructor already did the
            // first fetch, so drop that tick.
            tick.tick().await;
            loop {
                tick.tick().await;
                if let Ok(list) = api.devices().await {
                    let changed = **slot.load() != list;
                    slot.store(Arc::new(list));
                    if changed {
                        ctx.request_repaint();
                    }
                }
            }
        });
    }
}
