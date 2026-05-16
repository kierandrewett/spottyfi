//! The dock tab model.
//!
//! Every surface in the centre dock is a [`Tab`]. Phase 4 ships a small set —
//! the Home page and the Now Playing Art / Queue / Debug panels — but the enum
//! is shaped so Phase 5 can add page tabs (`Playlist`, `Album`, `Artist`, …)
//! by extending the variant list and the `match` in [`TabViewer::ui`].

use serde::{Deserialize, Serialize};
use spottyfi_audio::PlaybackState;
use spottyfi_ui::theme::Palette;

use crate::playback_controller::EngineStatus;
use crate::transport::{self, TransportIntent, TransportUiState};

/// A single dock tab.
///
/// Tabs split into **page tabs** (navigable content) and **panel tabs**
/// (auxiliary surfaces). Phase 4 implements one page (`Home`) and three panels;
/// the remaining page kinds arrive with the page system in Phase 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    /// The Home landing page (placeholder content for now).
    Home,
    /// The Now Playing album-art panel.
    NowPlayingArt,
    /// The play queue panel (placeholder for now).
    Queue,
    /// The debug panel: the "paste a URI and play" control.
    Debug,
}

impl Tab {
    /// The tab's display title.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Tab::Home => "Home",
            Tab::NowPlayingArt => "Now Playing",
            Tab::Queue => "Queue",
            Tab::Debug => "Debug",
        }
    }

    /// Whether this tab is a panel (as opposed to a navigable page).
    ///
    /// Panels are closeable; the Home page is kept open so the dock is never
    /// empty. Phase 5 revisits this when real page tabs arrive.
    #[must_use]
    pub fn is_panel(self) -> bool {
        !matches!(self, Tab::Home)
    }
}

/// Everything the dock's [`egui_dock::TabViewer`] needs to render a tab's body,
/// borrowed for the duration of one `DockArea::show` call.
pub struct TabContext<'a> {
    /// The active theme palette.
    pub palette: Palette,
    /// The live playback snapshot.
    pub playback: &'a PlaybackState,
    /// Mutable transport UI state (the debug text field lives here).
    pub transport_ui: &'a mut TransportUiState,
    /// The audio-engine lifecycle status, for the debug panel.
    pub engine: &'a EngineStatus,
    /// Any [`TransportIntent`] a tab raised this frame (e.g. debug-play).
    pub intent: Option<TransportIntent>,
}

/// The `egui_dock` tab viewer: renders tab titles and bodies.
pub struct ShellTabViewer<'a> {
    /// The per-frame context shared with every tab body.
    pub ctx: TabContext<'a>,
}

impl egui_dock::TabViewer for ShellTabViewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.title().into()
    }

    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::new(("spottyfi-tab", *tab))
    }

    fn closeable(&mut self, tab: &mut Self::Tab) -> bool {
        tab.is_panel()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        let palette = self.ctx.palette;
        egui::Frame::new()
            .fill(palette.base)
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                match tab {
                    Tab::Home => home_tab(ui, &self.ctx),
                    Tab::NowPlayingArt => now_playing_art_tab(ui, &self.ctx),
                    Tab::Queue => queue_tab(ui, &self.ctx),
                    Tab::Debug => {
                        if let Some(intent) = debug_tab(ui, &mut self.ctx) {
                            self.ctx.intent = Some(intent);
                        }
                    }
                }
            });
    }
}

/// The Home page body — placeholder content until Phase 5 wires real data.
fn home_tab(ui: &mut egui::Ui, ctx: &TabContext<'_>) {
    let palette = ctx.palette;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("Good evening")
                    .family(spottyfi_ui::fonts::semibold())
                    .size(28.0)
                    .color(palette.text),
            );
            ui.add_space(4.0);
            ui.label(spottyfi_ui::components::muted(
                &palette,
                "Your library, recommendations and recently played will appear here in Phase 5.",
                13.0,
            ));
            ui.add_space(20.0);

            // Placeholder "shelf" of cards so the layout reads like a real page.
            spottyfi_ui::components::section_header(ui, &palette, "Jump back in");
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for label in [
                    "Liked Songs",
                    "Discover Weekly",
                    "Release Radar",
                    "Daily Mix 1",
                    "On Repeat",
                ] {
                    placeholder_card(ui, &palette, label);
                }
            });
        });
}

/// A single placeholder content card used on the Home page.
fn placeholder_card(ui: &mut egui::Ui, palette: &Palette, label: &str) {
    let size = egui::vec2(150.0, 190.0);
    egui::Frame::new()
        .fill(palette.card)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.vertical(|ui| {
                spottyfi_ui::components::album_art(ui, palette, None, 128.0, 6.0);
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(label)
                        .family(spottyfi_ui::fonts::medium())
                        .size(13.0)
                        .color(palette.text),
                );
            });
        });
}

/// The Now Playing Art panel: the current track's album art, large.
fn now_playing_art_tab(ui: &mut egui::Ui, ctx: &TabContext<'_>) {
    let palette = ctx.palette;
    ui.vertical_centered(|ui| {
        let side = ui
            .available_width()
            .min(ui.available_height() - 80.0)
            .max(80.0);
        let art_url = ctx
            .playback
            .track
            .as_ref()
            .and_then(|t| t.art_url.as_deref());
        spottyfi_ui::components::album_art(ui, &palette, art_url, side, 10.0);
        ui.add_space(12.0);
        match &ctx.playback.track {
            Some(track) => {
                ui.label(
                    egui::RichText::new(&track.title)
                        .family(spottyfi_ui::fonts::semibold())
                        .size(18.0)
                        .color(palette.text),
                );
                ui.label(spottyfi_ui::components::muted(
                    &palette,
                    track.artist_line(),
                    13.0,
                ));
            }
            None => {
                ui.label(spottyfi_ui::components::muted(
                    &palette,
                    "Nothing playing",
                    13.0,
                ));
            }
        }
    });
}

/// The Queue panel — placeholder until Phase 8 builds the real queue.
fn queue_tab(ui: &mut egui::Ui, ctx: &TabContext<'_>) {
    let palette = ctx.palette;
    spottyfi_ui::components::section_header(ui, &palette, "Queue");
    match &ctx.playback.track {
        Some(track) => {
            ui.label(spottyfi_ui::components::muted(
                &palette,
                "Now playing",
                11.0,
            ));
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                spottyfi_ui::components::album_art(
                    ui,
                    &palette,
                    track.art_url.as_deref(),
                    40.0,
                    4.0,
                );
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(&track.title)
                            .family(spottyfi_ui::fonts::medium())
                            .color(palette.text),
                    );
                    ui.label(spottyfi_ui::components::muted(
                        &palette,
                        track.artist_line(),
                        11.0,
                    ));
                });
            });
        }
        None => {
            ui.label(spottyfi_ui::components::muted(
                &palette,
                "The queue is empty.",
                13.0,
            ));
        }
    }
    ui.add_space(16.0);
    ui.label(spottyfi_ui::components::muted(
        &palette,
        "Next-up and the manual queue arrive in Phase 8.",
        12.0,
    ));
}

/// The Debug panel: the "paste a URI and play" control kept reachable until
/// the browsing UI exists (Phase 5).
fn debug_tab(ui: &mut egui::Ui, ctx: &mut TabContext<'_>) -> Option<TransportIntent> {
    let palette = ctx.palette;
    spottyfi_ui::components::section_header(ui, &palette, "Debug");
    ui.add_space(4.0);
    transport::debug_play_control(ui, &palette, ctx.transport_ui, ctx.engine)
}
