//! UI building blocks: egui widgets, panels, the theme and reusable components.
//!
//! This crate is a pure projection of `state` — it renders snapshots and emits
//! intent, never mutating state directly.
//!
//! ## What lives here (Phase 4)
//!
//! - [`theme`] — the two selectable dark colour palettes and their application
//!   to an [`egui::Style`].
//! - [`fonts`] — the bundled Inter / JetBrains Mono faces and their
//!   registration into egui.
//! - [`icons`] — the bundled Lucide line-icon set as tinted SVG widgets.
//! - [`image_loader`] — a custom network [`egui::load::ImageLoader`] so
//!   `egui::Image::from_uri(http_url)` resolves remote album art and avatars.
//! - [`components`] — reusable widgets (section headers, album art, icon and
//!   primary buttons, filter chips) plus the row-[`components::Density`] notion.
//! - [`scrubber`] — the Spotify-style hover-scrub progress/seek widget, shared
//!   by the transport's seek bar and its volume control.
//! - [`track_table`] — the sortable track-table widget (Phase 5) shared by the
//!   playlist, album and liked-songs pages.
//! - [`visualiser`] — the audio-visualiser render widgets (WS7): a spectrum
//!   analyser and an oscilloscope, fed already-analysed data.
//!
//! The dock shell itself lives in the `app` binary, which is the only crate
//! that may depend on both `audio` and `ui`.
#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

pub mod components;
pub mod fonts;
pub mod icons;
pub mod image_loader;
pub mod scrubber;
pub mod theme;
pub mod track_table;
pub mod visualiser;

pub use components::Density;
pub use icons::Icon;
pub use scrubber::{Scrubber, ScrubberResponse, ScrubberState};
pub use theme::{Palette, Theme};
pub use track_table::{
    track_table, SortColumn, TrackAction, TrackColumns, TrackRow, TrackTableState,
};
pub use visualiser::VisualiserMode;

/// Install Spottyfi's fonts and image loaders into an egui context.
///
/// Call once from the eframe creation context. This registers the bundled
/// fonts, the stock `egui_extras` image loaders (via the caller — see below)
/// and Spottyfi's network image loader. The theme is applied separately by the
/// caller because the chosen theme is persisted app state.
///
/// Note: `egui_extras::install_image_loaders` must be called by the `app`
/// crate (which depends on `egui_extras`); [`image_loader::install`] is then
/// layered on top for `http(s)` URLs.
pub fn install_fonts_and_network_loader(ctx: &egui::Context) {
    fonts::install(ctx);
    image_loader::install(ctx);
    tune_rendering(ctx);
}

/// Tune egui's tessellation for crisp, smooth rendering.
///
/// Feathering is egui's only edge anti-aliasing; with it off, shape and glyph
/// edges look jagged ("crispy"). It is on by default, but pinning it here makes
/// the intent explicit and survives any future style reset. The feathering
/// width is left at egui's default (1 physical pixel), which is correct at the
/// native display scale — eframe adopts the OS `pixels_per_point` automatically
/// and Spottyfi never overrides it.
fn tune_rendering(ctx: &egui::Context) {
    ctx.options_mut(|opts| {
        opts.tessellation_options.feathering = true;
        opts.tessellation_options.feathering_size_in_pixels = 1.0;
    });
}
