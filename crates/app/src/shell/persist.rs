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

use super::tabs::Tab;

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
                Ok(shell) => {
                    tracing::debug!(path = %path.display(), "restored shell layout");
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
/// Art panel above the Queue panel.
#[must_use]
pub fn default_dock() -> egui_dock::DockState<Tab> {
    use egui_dock::{NodeIndex, SurfaceIndex};

    let mut dock = egui_dock::DockState::new(vec![Tab::Home]);
    let surface = SurfaceIndex::main();

    // Right column: take 26% of the width for the panel stack.
    let [_, right] = dock[surface].split_right(NodeIndex::root(), 0.74, vec![Tab::NowPlayingArt]);
    // Stack the Queue below the Now Playing Art panel.
    dock[surface].split_below(right, 0.55, vec![Tab::Queue]);

    dock
}
