//! Bridges the Spotify Connect device list and remote playback to the UI.
//!
//! A background task polls `GET /me/player/devices` and `GET /me/player`; the
//! UI reads [`ArcSwap`] snapshots every frame. Transfers and — while playback
//! is on another device — play/pause/next/previous/seek are dispatched here as
//! detached tasks against the Web API.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use spottyfi_api::{ApiResult, SpotifyApi};
use spottyfi_models::{Device, RemotePlayback};
use tokio::runtime::Handle;

/// How often the device list and remote playback are refreshed.
const POLL_INTERVAL: Duration = Duration::from_secs(8);

/// How long to wait after a command before re-fetching, so Spotify has
/// settled and the refreshed state reflects the command.
const COMMAND_SETTLE: Duration = Duration::from_millis(500);

/// A fetched remote-playback snapshot plus when it was fetched, so progress
/// can be extrapolated between the (infrequent) polls.
struct RemoteState {
    /// The playback snapshot as fetched.
    playback: RemotePlayback,
    /// When [`Self::playback`] was fetched.
    fetched: Instant,
}

/// Owns the Connect device list + remote-playback snapshots and dispatches
/// device and remote-playback commands.
pub struct DevicesController {
    /// The Spotify Web API client.
    api: Arc<dyn SpotifyApi>,
    /// Runtime handle used to spawn the poller and command tasks.
    runtime: Handle,
    /// egui context, woken whenever a snapshot changes.
    egui_ctx: egui::Context,
    /// The device-list snapshot the UI reads each frame.
    devices: Arc<ArcSwap<Vec<Device>>>,
    /// The remote-playback snapshot, `None` when nothing plays anywhere.
    remote: Arc<ArcSwap<Option<RemoteState>>>,
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
            remote: Arc::new(ArcSwap::from_pointee(None)),
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

    /// The remote-playback snapshot with its progress extrapolated to *now*,
    /// so a banner reads as live between the infrequent polls.
    #[must_use]
    pub fn remote_playback(&self) -> Option<RemotePlayback> {
        let state = self.remote.load_full();
        let state = state.as_ref().as_ref()?;
        let mut playback = state.playback.clone();
        if playback.is_playing && playback.duration_ms > 0 {
            let elapsed = u32::try_from(state.fetched.elapsed().as_millis()).unwrap_or(u32::MAX);
            playback.progress_ms = playback
                .progress_ms
                .saturating_add(elapsed)
                .min(playback.duration_ms);
        }
        Some(playback)
    }

    /// Fetch the device list and remote playback once, immediately.
    pub fn refresh(&self) {
        self.spawn_fetch(Duration::ZERO);
    }

    /// Transfer playback to `device_id`, then refresh.
    pub fn transfer(&self, device_id: String) {
        self.spawn_command(move |api| async move { api.transfer_playback(&device_id, true).await });
    }

    /// Toggle play/pause on the remote device.
    pub fn remote_play_pause(&self) {
        let playing = self
            .remote
            .load_full()
            .as_ref()
            .as_ref()
            .is_some_and(|s| s.playback.is_playing);
        self.spawn_command(move |api| async move {
            if playing {
                api.remote_pause().await
            } else {
                api.remote_resume().await
            }
        });
    }

    /// Skip to the next track on the remote device.
    pub fn remote_next(&self) {
        self.spawn_command(|api| async move { api.remote_next().await });
    }

    /// Skip to the previous track on the remote device.
    pub fn remote_previous(&self) {
        self.spawn_command(|api| async move { api.remote_previous().await });
    }

    /// Seek the remote device to `position_ms`.
    pub fn remote_seek(&self, position_ms: u32) {
        self.spawn_command(move |api| async move { api.remote_seek(position_ms).await });
    }

    /// Spawn a remote-playback command, then settle and re-fetch.
    fn spawn_command<F, Fut>(&self, command: F)
    where
        F: FnOnce(Arc<dyn SpotifyApi>) -> Fut + Send + 'static,
        Fut: Future<Output = ApiResult<()>> + Send,
    {
        let api = Arc::clone(&self.api);
        let devices = Arc::clone(&self.devices);
        let remote = Arc::clone(&self.remote);
        let ctx = self.egui_ctx.clone();
        self.runtime.spawn(async move {
            if let Err(err) = command(Arc::clone(&api)).await {
                tracing::warn!(%err, "remote playback command failed");
            }
            tokio::time::sleep(COMMAND_SETTLE).await;
            Self::fetch_into(&api, &devices, &remote, &ctx).await;
        });
    }

    /// Spawn a one-off fetch after `delay`.
    fn spawn_fetch(&self, delay: Duration) {
        let api = Arc::clone(&self.api);
        let devices = Arc::clone(&self.devices);
        let remote = Arc::clone(&self.remote);
        let ctx = self.egui_ctx.clone();
        self.runtime.spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            Self::fetch_into(&api, &devices, &remote, &ctx).await;
        });
    }

    /// Spawn the periodic background refresh task.
    fn spawn_poller(&self) {
        let api = Arc::clone(&self.api);
        let devices = Arc::clone(&self.devices);
        let remote = Arc::clone(&self.remote);
        let ctx = self.egui_ctx.clone();
        self.runtime.spawn(async move {
            let mut tick = tokio::time::interval(POLL_INTERVAL);
            // `interval` fires immediately; the constructor already did the
            // first fetch, so drop that tick.
            tick.tick().await;
            loop {
                tick.tick().await;
                Self::fetch_into(&api, &devices, &remote, &ctx).await;
            }
        });
    }

    /// Fetch the device list and remote playback, store both, wake the UI.
    async fn fetch_into(
        api: &Arc<dyn SpotifyApi>,
        devices: &ArcSwap<Vec<Device>>,
        remote: &ArcSwap<Option<RemoteState>>,
        ctx: &egui::Context,
    ) {
        match api.devices().await {
            Ok(list) => devices.store(Arc::new(list)),
            Err(err) => tracing::debug!(%err, "device list refresh failed"),
        }
        match api.current_playback().await {
            Ok(Some(playback)) => remote.store(Arc::new(Some(RemoteState {
                playback,
                fetched: Instant::now(),
            }))),
            Ok(None) => remote.store(Arc::new(None)),
            Err(err) => tracing::debug!(%err, "current playback refresh failed"),
        }
        ctx.request_repaint();
    }
}
