//! Persisting the dock layout and user settings across restarts.
//!
//! The dock [`egui_dock::DockState`] and the chosen theme / density are
//! serialised to `<config_dir>/layout.ron` on shutdown and restored on launch.
//! `egui_dock` 0.19's `serde` feature derives `Serialize`/`Deserialize` on
//! `DockState`, so the whole layout round-trips through RON.
//!
//! Persistence is best-effort: a missing or corrupt file falls back to the
//! default layout — it never blocks startup.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use spottyfi_ui::components::Density;
use spottyfi_ui::theme::Theme;
use spottyfi_ui::visualiser::VisualiserMode;

use super::dock_model::DockExtras;
use super::tabs::Tab;
use crate::settings::AppSettings;

/// A predefined dock layout, selectable from the View menu.
///
/// Switching a layout rebuilds the dock tree (and, for [`Layout::PowerUser`],
/// nudges the density). [`Layout::custom`] is the implicit state after the
/// user drags a panel — no named layout is "selected" any more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Layout {
    /// The first-launch layout: a centre Home tab with a right-hand column of
    /// Now Playing Art over Queue.
    #[default]
    Default,
    /// Compact, table-dense: the Queue (and Lyrics, once Phase 11 ships it)
    /// docked in a right column, density nudged to compact.
    PowerUser,
    /// A single centre tab, no right-hand panel column.
    Minimal,
}

impl Layout {
    /// Every selectable layout, in menu order.
    #[must_use]
    pub fn all() -> [Layout; 3] {
        [Layout::Default, Layout::PowerUser, Layout::Minimal]
    }

    /// The human-readable menu label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Layout::Default => "Default",
            Layout::PowerUser => "Power user",
            Layout::Minimal => "Minimal",
        }
    }

    /// Build this layout's dock tree.
    #[must_use]
    pub fn build_dock(self) -> egui_dock::DockState<Tab> {
        match self {
            Layout::Default => default_dock(),
            Layout::PowerUser => power_user_dock(),
            Layout::Minimal => minimal_dock(),
        }
    }
}

/// The full persisted shell state: the dock layout plus user settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedShell {
    /// The dock layout (tab tree, splits, sizes).
    pub dock: egui_dock::DockState<Tab>,
    /// The selected colour theme.
    pub theme: Theme,
    /// The selected row density.
    pub density: Density,
    /// Whether the left sidebar is collapsed to its icon rail.
    pub sidebar_collapsed: bool,
    /// The left sidebar width in points.
    pub sidebar_width: f32,
    /// The collapsed sidebar tree sections, by key (`main`, `library`,
    /// `playlists`). A key present here is collapsed; absent means expanded.
    #[serde(default)]
    pub collapsed_sections: Vec<String>,
    /// The app-layer dock state — tab pinning, per-tab history, the
    /// closed-tab stack. `#[serde(default)]` so a pre-Phase-10 file loads.
    #[serde(default)]
    pub dock_extras: DockExtras,
    /// The currently-applied predefined layout (Phase 10). `#[serde(default)]`
    /// so a pre-Phase-10 file loads as [`Layout::Default`].
    #[serde(default)]
    pub layout: Layout,
    /// The power-user settings shown on the Settings page (audio, equalizer,
    /// local files). `#[serde(default)]` so a pre-WS5 file loads with the
    /// settings defaults.
    #[serde(default)]
    pub settings: AppSettings,
    /// The selected audio-visualiser mode (spectrum / oscilloscope).
    /// `#[serde(default)]` so a pre-WS7 file loads with the spectrum default.
    #[serde(default)]
    pub visualiser_mode: VisualiserMode,
}

impl Default for PersistedShell {
    fn default() -> Self {
        Self {
            dock: default_dock(),
            theme: Theme::default(),
            density: Density::default(),
            sidebar_collapsed: false,
            sidebar_width: 240.0,
            collapsed_sections: Vec::new(),
            dock_extras: DockExtras::default(),
            layout: Layout::default(),
            settings: AppSettings::default(),
            visualiser_mode: VisualiserMode::default(),
        }
    }
}

impl PersistedShell {
    /// Load the persisted shell from disk, or fall back to the default.
    ///
    /// Any error (no file, unreadable, corrupt RON) is logged at debug level
    /// and the default layout is returned — this must never fail startup.
    #[must_use]
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            tracing::debug!("no config dir; using the default layout");
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match ron::from_str::<PersistedShell>(&text) {
                Ok(mut shell) => {
                    tracing::debug!(path = %path.display(), "restored shell layout");
                    // Clamp any out-of-range persisted settings so a
                    // hand-edited config can't reach the engine or EQ.
                    shell.settings.sanitise();
                    shell
                }
                Err(err) => {
                    tracing::debug!(%err, "layout file unreadable; using default");
                    Self::default()
                }
            },
            Err(err) => {
                tracing::debug!(%err, "no layout file; using default");
                Self::default()
            }
        }
    }

    /// Write the shell state to `<config_dir>/layout.ron`.
    ///
    /// Errors are logged and swallowed — failing to persist a layout is not
    /// worth interrupting shutdown.
    pub fn save(&self) {
        let Some(path) = config_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                tracing::warn!(%err, "could not create config dir");
                return;
            }
        }
        match ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()) {
            Ok(text) => {
                if let Err(err) = std::fs::write(&path, text) {
                    tracing::warn!(%err, "could not write layout file");
                } else {
                    tracing::debug!(path = %path.display(), "saved shell layout");
                }
            }
            Err(err) => tracing::warn!(%err, "could not serialise layout"),
        }
    }
}

/// The platform config-directory path for the layout file.
fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "drewett", "spottyfi")
        .map(|dirs| dirs.config_dir().join("layout.ron"))
}

/// Build the first-launch dock layout from `docs/docking.md`:
/// a centre group with one Home tab, and a right column with the Now Playing
/// Art panel above a Queue / Visualiser tab group.
#[must_use]
pub fn default_dock() -> egui_dock::DockState<Tab> {
    use egui_dock::{NodeIndex, SurfaceIndex};

    let mut dock = egui_dock::DockState::new(vec![Tab::Home]);
    let surface = SurfaceIndex::main();

    // Right column: take 26% of the width for the panel stack.
    let [_, right] = dock[surface].split_right(NodeIndex::root(), 0.74, vec![Tab::NowPlayingArt]);
    // Stack the Queue and the audio Visualiser as a tab group below the Now
    // Playing Art panel — the Visualiser is one tab over from the Queue.
    dock[surface].split_below(right, 0.55, vec![Tab::Queue, Tab::Visualiser]);

    dock
}

/// Build the **Power user** layout: a centre Home tab with the Queue docked in
/// a slim right column.
///
/// The Lyrics panel lands in Phase 11; this layout is forward-compatible —
/// once a `Tab::Lyrics` variant exists it can be stacked above the Queue here.
/// Until then the layout degrades gracefully to Queue-only, exactly as
/// specified. Switching to this layout also nudges the density to compact (see
/// [`Layout::PowerUser`] handling in the View menu).
#[must_use]
pub fn power_user_dock() -> egui_dock::DockState<Tab> {
    use egui_dock::{NodeIndex, SurfaceIndex};

    let mut dock = egui_dock::DockState::new(vec![Tab::Home]);
    let surface = SurfaceIndex::main();

    // A slim right column for the Queue panel — narrower than Default's, so
    // the dense centre tables get more width.
    dock[surface].split_right(NodeIndex::root(), 0.8, vec![Tab::Queue]);

    dock
}

/// Build the **Minimal** layout: a single centre Home tab, no right-hand panel
/// column at all.
#[must_use]
pub fn minimal_dock() -> egui_dock::DockState<Tab> {
    egui_dock::DockState::new(vec![Tab::Home])
}
