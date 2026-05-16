//! Spottyfi — a native Rust Spotify client.
//!
//! This binary is the only crate that knows about both `audio` and `ui`; it
//! wires the dock layout, the tokio runtime and the egui render loop together.
//!
//! Phase 1: OAuth login screen that transitions to a logged-in view. See
//! `PLAN.md` for the roadmap.

mod app;
mod auth_controller;
mod avatar;
mod cli;
mod login;
mod page;
mod playback_controller;
mod shell;
mod transport;

use anyhow::Context as _;
use clap::Parser as _;

use crate::app::SpottyfiApp;
use crate::cli::Cli;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(&cli);

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting Spottyfi");
    if cli.no_audio {
        tracing::warn!("--no-audio: audio engine will not be started");
    }
    if cli.offline {
        tracing::warn!("--offline: network requests will be suppressed");
    }
    if cli.clear_cache {
        tracing::warn!("--clear-cache: wiping the metadata and image caches");
        match spottyfi_cache::clear_on_disk() {
            Ok(()) => tracing::info!("caches cleared"),
            Err(err) => tracing::error!(%err, "failed to clear caches"),
        }
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Spottyfi")
            .with_app_id("dev.drewett.spottyfi")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Spottyfi",
        native_options,
        Box::new(move |cc| {
            SpottyfiApp::new(cc, cli.no_audio)
                .map(|app| Box::new(app) as Box<dyn eframe::App>)
                .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> { err.into() })
        }),
    )
    .map_err(|err| anyhow::anyhow!("eframe failed: {err}"))
    .context("running the Spottyfi window")?;

    tracing::info!("Spottyfi exited cleanly");
    Ok(())
}

/// Install the `tracing` subscriber.
///
/// `RUST_LOG` wins when set; otherwise the `--log-level` flag drives a
/// `spottyfi=<level>` filter, which (by prefix match) covers every workspace
/// crate — `RUST_LOG=spottyfi=debug` works out of the box.
fn init_tracing(cli: &Cli) {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("spottyfi={}", cli.log_level)));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}
