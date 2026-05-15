//! Command-line interface.

use clap::Parser;

/// Spottyfi — a native Rust Spotify client.
#[derive(Debug, Parser)]
#[command(name = "spottyfi", version, about)]
pub struct Cli {
    /// Start without the audio engine (UI-only development).
    #[arg(long)]
    pub no_audio: bool,

    /// Suppress all network requests; render from cache only.
    #[arg(long)]
    pub offline: bool,

    /// Wipe the metadata and image caches on startup.
    #[arg(long)]
    pub clear_cache: bool,

    /// Default log level when `RUST_LOG` is not set.
    ///
    /// One of `error`, `warn`, `info`, `debug`, `trace`.
    #[arg(long, default_value = "info", value_name = "LEVEL")]
    pub log_level: String,
}
