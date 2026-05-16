//! The dock tab model.
//!
//! Every surface in the centre dock is a [`Tab`]. Tabs split into **page tabs**
//! (navigable content: Home, Library, Liked Songs, Playlist, Album, Artist)
//! and **panel tabs** (auxiliary surfaces: Now Playing Art, Queue, Debug).
//!
//! A `Tab` is only a lightweight, serialisable *key*: the dock
//! ([`egui_dock::DockState`]) stores `Tab`s so the whole layout round-trips
//! through RON. The live, stateful [`Page`](crate::page::Page) objects — which
//! carry the in-flight loads and per-page UI state — live in a
//! [`PageRegistry`](crate::page::PageRegistry) keyed by `Tab`.

use serde::{Deserialize, Serialize};
use spottyfi_audio::PlaybackState;
use spottyfi_ui::theme::Palette;

use crate::page::{PageAction, PageContext, PageRegistry};
use crate::playback_controller::EngineStatus;
use crate::transport::{self, TransportIntent, TransportUiState};

/// A single dock tab — the serialisable key for one centre-dock surface.
///
/// Id-carrying variants ([`Tab::Playlist`], [`Tab::Album`], [`Tab::Artist`])
/// identify which object the page renders; the bare base-62 Spotify id is
/// stored, not the full URI.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    /// The Home landing page.
    Home,
    /// The Your Library page (playlists + saved albums).
    Library,
    /// The Liked Songs (saved tracks) page.
    LikedSongs,
    /// A playlist page, keyed by playlist id.
    Playlist(String),
    /// An album page, keyed by album id.
    Album(String),
    /// An artist page, keyed by artist id.
    Artist(String),
    /// The Search page (real search lands in Phase 6).
    Search,
    /// The Browse page: the genre/category grid plus Last.fm charts.
    Browse,
    /// A browse-category page, keyed by the Spotify category id.
    Category(String),
    /// The Charts page: Last.fm global top tracks and artists.
    Charts,
    /// The New Releases page (Spotify `new-releases`).
    NewReleases,
    /// The Made For You page: recommendations seeded from the user's top
    /// artists and tracks via Last.fm.
    MadeForYou,
    /// A not-yet-built page. Carries its display name; the body is a "coming
    /// soon" placeholder until the real page is implemented.
    Placeholder(String),
    /// The Now Playing album-art panel.
    NowPlayingArt,
    /// The play queue panel.
    Queue,
    /// The debug panel: the "paste a URI and play" control.
    Debug,
}

impl Tab {
    /// The tab's static fallback title.
    ///
    /// Page tabs may show a richer, data-derived title once their page has
    /// loaded (the playlist's name, say); the registry supplies that. This is
    /// the label shown before the load resolves.
    #[must_use]
    pub fn title(&self) -> &'static str {
        match self {
            Tab::Home => "Home",
            Tab::Library => "Your Library",
            Tab::LikedSongs => "Liked Songs",
            Tab::Playlist(_) => "Playlist",
            Tab::Album(_) => "Album",
            Tab::Artist(_) => "Artist",
            Tab::Search => "Search",
            Tab::Browse => "Browse",
            Tab::Category(_) => "Category",
            Tab::Charts => "Charts",
            Tab::NewReleases => "New Releases",
            Tab::MadeForYou => "Made For You",
            Tab::Placeholder(_) => "Coming soon",
            Tab::NowPlayingArt => "Now Playing",
            Tab::Queue => "Queue",
            Tab::Debug => "Debug",
        }
    }

    /// The plain, human-readable tab title shown on the dock tab bar.
    ///
    /// For an id-carrying page (playlist / album / artist) this is the
    /// object's own name, supplied by the page registry once its data has
    /// loaded; for everything else it is the static label. Never a
    /// breadcrumb path — see `docs/ui-reference.md`.
    #[must_use]
    pub fn display_title(&self, page_title: &str) -> String {
        match self {
            Tab::Playlist(_) | Tab::Album(_) | Tab::Artist(_) | Tab::Category(_) => {
                page_title.to_owned()
            }
            Tab::Placeholder(name) => name.clone(),
            _ => self.title().to_owned(),
        }
    }

    /// Whether this tab is a panel (as opposed to a navigable page).
    ///
    /// Panels are closeable; the Home page is kept open so the dock is never
    /// empty. `Placeholder` is a self-rendered surface — neither a
    /// registry-backed page nor an auxiliary panel — and is classified as a
    /// panel so the page registry never tries to build it. `Search` is a
    /// real, registry-backed page (Phase 6).
    #[must_use]
    pub fn is_panel(&self) -> bool {
        matches!(
            self,
            Tab::NowPlayingArt | Tab::Queue | Tab::Debug | Tab::Placeholder(_)
        )
    }

    /// Whether this tab is a navigable page (rendered via the page registry).
    #[must_use]
    pub fn is_page(&self) -> bool {
        !self.is_panel()
    }
}

/// Something a dock tab raised this frame that the shell must act on.
#[derive(Debug, Clone, PartialEq)]
pub enum DockIntent {
    /// A transport command (e.g. from the debug panel, or a page's play).
    Transport(TransportIntent),
    /// Open (navigate to) a page tab, replacing the focused leaf.
    Open(Tab),
    /// Copy a string to the system clipboard (a Spotify URI).
    CopyToClipboard(String),
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
    /// The live page objects, keyed by tab.
    pub pages: &'a mut PageRegistry,
    /// Any [`DockIntent`]s raised this frame, in order.
    pub intents: Vec<DockIntent>,
}

/// The `egui_dock` tab viewer: renders tab titles and bodies.
pub struct ShellTabViewer<'a> {
    /// The per-frame context shared with every tab body.
    pub ctx: TabContext<'a>,
}

impl egui_dock::TabViewer for ShellTabViewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        let page_title = if tab.is_page() {
            self.ctx.pages.title(tab)
        } else {
            tab.title().to_owned()
        };
        tab.display_title(&page_title).into()
    }

    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::new(("spottyfi-tab", tab.clone()))
    }

    fn closeable(&mut self, tab: &mut Self::Tab) -> bool {
        // Home stays open so the dock is never empty; everything else closes.
        !matches!(tab, Tab::Home)
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        let palette = self.ctx.palette;
        egui::Frame::new()
            .fill(palette.base)
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                match tab {
                    Tab::NowPlayingArt => now_playing_art_tab(ui, &self.ctx),
                    Tab::Queue => queue_tab(ui, &self.ctx),
                    Tab::Placeholder(name) => placeholder_tab(ui, &self.ctx, name),
                    Tab::Debug => {
                        if let Some(intent) = debug_tab(ui, &mut self.ctx) {
                            self.ctx.intents.push(DockIntent::Transport(intent));
                        }
                    }
                    page_tab => {
                        let page_ctx = PageContext {
                            palette,
                            playback: self.ctx.playback,
                        };
                        if let Some(action) = self.ctx.pages.ui(page_tab, ui, &page_ctx) {
                            self.ctx.intents.push(page_action_to_intent(action));
                        }
                    }
                }
            });
    }
}

/// Translate a [`PageAction`] into the shell-level [`DockIntent`].
fn page_action_to_intent(action: PageAction) -> DockIntent {
    match action {
        PageAction::Play(uri) => DockIntent::Transport(TransportIntent::PlayUri(uri)),
        PageAction::Open(tab) => DockIntent::Open(tab),
        PageAction::CopyToClipboard(text) => DockIntent::CopyToClipboard(text),
    }
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

/// A not-yet-built page (Browse, Charts, New Releases, …).
fn placeholder_tab(ui: &mut egui::Ui, ctx: &TabContext<'_>, name: &str) {
    coming_soon(
        ui,
        &ctx.palette,
        name,
        "This page is part of a later phase and isn't built yet.",
    );
}

/// A centred "coming soon" placeholder used by pages that aren't built yet.
fn coming_soon(ui: &mut egui::Ui, palette: &Palette, title: &str, detail: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.34);
        spottyfi_ui::icons::icon(ui, spottyfi_ui::Icon::Discover, 40.0, palette.text_muted);
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(title)
                .family(spottyfi_ui::fonts::semibold())
                .size(18.0)
                .color(palette.text),
        );
        ui.add_space(4.0);
        ui.label(spottyfi_ui::components::muted(palette, detail, 12.0));
    });
}

/// The Debug panel: the "paste a URI and play" control kept reachable for
/// quick playback testing.
fn debug_tab(ui: &mut egui::Ui, ctx: &mut TabContext<'_>) -> Option<TransportIntent> {
    let palette = ctx.palette;
    spottyfi_ui::components::section_header(ui, &palette, "Debug");
    ui.add_space(4.0);
    transport::debug_play_control(ui, &palette, ctx.transport_ui, ctx.engine)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_and_panels_are_classified() {
        assert!(Tab::Home.is_page());
        assert!(Tab::Playlist("x".into()).is_page());
        assert!(Tab::Album("x".into()).is_page());
        assert!(Tab::Queue.is_panel());
        assert!(Tab::Debug.is_panel());
        assert!(!Tab::Debug.is_page());
    }

    #[test]
    fn id_carrying_tabs_compare_by_id() {
        assert_eq!(Tab::Album("a".into()), Tab::Album("a".into()));
        assert_ne!(Tab::Album("a".into()), Tab::Album("b".into()));
    }

    #[test]
    fn page_actions_map_to_dock_intents() {
        assert_eq!(
            page_action_to_intent(PageAction::Play("spotify:track:x".into())),
            DockIntent::Transport(TransportIntent::PlayUri("spotify:track:x".into()))
        );
        assert_eq!(
            page_action_to_intent(PageAction::Open(Tab::LikedSongs)),
            DockIntent::Open(Tab::LikedSongs)
        );
        assert_eq!(
            page_action_to_intent(PageAction::CopyToClipboard("uri".into())),
            DockIntent::CopyToClipboard("uri".into())
        );
    }
}
