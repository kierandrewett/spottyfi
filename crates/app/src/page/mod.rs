//! The page system: navigable content tabs and their async data loading.
//!
//! Phase 4 shipped a single hard-coded `Home` tab. Phase 5 turns the centre
//! dock's *page tabs* into real, data-backed pages:
//!
//! - A [`Page`] is one navigable surface (a playlist, an album, an artist, the
//!   liked songs, the library, the home screen). Each knows its tab title,
//!   how to **load itself asynchronously**, and how to render — drawing a
//!   spinner while the load is pending and the data once it is ready.
//! - A [`Tab`](crate::shell::Tab) is the lightweight, serialisable *key* for a
//!   page. The dock ([`egui_dock::DockState`]) stores `Tab`s; the live,
//!   stateful [`Page`] objects live in a [`PageRegistry`] keyed by `Tab`.
//! - Loading uses [`Loadable`], the one-shot promise wrapper over the app's
//!   tokio runtime (see `docs/threading.md`).
//!
//! Pages emit [`PageAction`]s — "play this URI", "open this album tab",
//! "copy this to the clipboard" — which the shell hands back to `app`. Pages
//! never mutate engine or auth state directly.

mod album;
mod artist;
mod browse;
mod cards;
mod category;
mod charts;
mod home;
mod incremental;
mod library;
mod liked;
mod made_for_you;
mod new_releases;
mod playlist;
mod promise;
mod search;
mod search_load;
mod settings;
mod track_view;

use std::collections::HashMap;
use std::sync::Arc;

use spottyfi_api::lastfm::LastfmClient;
use spottyfi_api::SpotifyApi;
use spottyfi_audio::PlaybackState;
use spottyfi_state::ActivityRegistry;
use spottyfi_ui::theme::Palette;
use tokio::runtime::Handle;

pub use incremental::IncrementalLoad;
pub use promise::Loadable;

use crate::shell::Tab;

pub use album::AlbumPage;
pub use artist::ArtistPage;
pub use browse::BrowsePage;
pub use category::CategoryPage;
pub use charts::ChartsPage;
pub use home::HomePage;
pub use library::LibraryPage;
pub use liked::LikedSongsPage;
pub use made_for_you::MadeForYouPage;
pub use new_releases::NewReleasesPage;
pub use playlist::PlaylistPage;
pub use search::SearchPage;
pub use settings::{settings_page, SettingsAction, SettingsContext};

/// Shared services a page needs to load its data.
///
/// Cloned cheaply (every field is an `Arc` / handle) and handed to a page when
/// it is first created so it can spawn its async load.
#[derive(Clone)]
pub struct PageServices {
    /// The Spotify Web API client (the real client, or a mock in tests).
    pub api: Arc<dyn SpotifyApi>,
    /// The Last.fm client used by Browse for charts and recommendations.
    ///
    /// `None` when `SPOTTYFI_LASTFM_API_KEY` is unset — Browse degrades
    /// gracefully, showing the Spotify category grid and a calm "set the key"
    /// note in place of the Last.fm-backed sections.
    pub lastfm: Option<LastfmClient>,
    /// The tokio runtime the async loads are spawned onto.
    pub runtime: Handle,
    /// The egui context, woken when a load resolves.
    pub ctx: egui::Context,
    /// The shared background-activity registry; page loads register here so
    /// the menu-bar indicator reflects them.
    pub activity: Arc<ActivityRegistry>,
}

/// Everything a page needs to render one frame, borrowed for the call.
pub struct PageContext<'a> {
    /// The active theme palette.
    pub palette: Palette,
    /// The live playback snapshot — used to highlight the playing track.
    pub playback: &'a PlaybackState,
}

/// Something a page asked the app to do this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageAction {
    /// Play a context — a playlist/album's full resolved track list — starting
    /// at `offset`, so Next/Prev walk the list.
    PlayContext {
        /// The context's own Spotify URI.
        uri: String,
        /// The context's display name (shown in the queue panel).
        name: String,
        /// The context's tracks, in play order.
        tracks: Vec<spottyfi_audio::QueueTrack>,
        /// The index in `tracks` to start playback at.
        offset: usize,
    },
    /// Add a track to the front of the manual queue (play it next).
    PlayNext(spottyfi_audio::QueueTrack),
    /// Add a track to the end of the manual queue.
    Enqueue(spottyfi_audio::QueueTrack),
    /// Open (navigate to) another page tab.
    Open(Tab),
    /// Copy a string (a Spotify URI) to the system clipboard.
    CopyToClipboard(String),
}

/// One navigable page: a typed, stateful surface in the centre dock.
///
/// Implementors own their [`Loadable`] data handle and any per-page UI state
/// (the track table's sort column, for example). The [`PageRegistry`] keeps
/// one boxed `Page` alive per open [`Tab`].
pub trait Page {
    /// The tab's display title (shown on its dock tab).
    fn title(&self) -> String;

    /// Render the page body, returning any [`PageAction`] the user raised.
    ///
    /// Implementations draw a spinner while their [`Loadable`] is pending and
    /// the real content once it resolves.
    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction>;
}

/// The live, stateful [`Page`] objects, keyed by their [`Tab`].
///
/// The dock only stores `Tab`s (so the layout serialises); the registry holds
/// the matching `Page` objects, which carry the in-flight loads and UI state.
/// A page is created lazily the first time its tab is rendered and dropped
/// when the tab closes.
pub struct PageRegistry {
    /// Shared services handed to each page on creation.
    services: PageServices,
    /// The boxed pages, one per open page tab.
    pages: HashMap<Tab, Box<dyn Page>>,
}

impl PageRegistry {
    /// Build an empty registry over the given services.
    #[must_use]
    pub fn new(services: PageServices) -> Self {
        Self {
            services,
            pages: HashMap::new(),
        }
    }

    /// The display title for `tab` — used by the dock's tab bar.
    ///
    /// Falls back to the tab's static label until the page has loaded a richer
    /// title (e.g. the playlist's name).
    #[must_use]
    pub fn title(&self, tab: &Tab) -> String {
        self.pages
            .get(tab)
            .map_or_else(|| tab.title().to_owned(), |page| page.title())
    }

    /// Render `tab`'s page, creating it on first use. Returns its action.
    pub fn ui(
        &mut self,
        tab: &Tab,
        ui: &mut egui::Ui,
        ctx: &PageContext<'_>,
    ) -> Option<PageAction> {
        let services = &self.services;
        let page = self
            .pages
            .entry(tab.clone())
            .or_insert_with(|| build_page(tab, services));
        page.ui(ui, ctx)
    }

    /// Drop the pages whose tabs are no longer open.
    ///
    /// Called once per frame with the set of tabs the dock still holds, so a
    /// closed tab's in-flight load and UI state are released.
    pub fn retain_open<'a>(&mut self, open: impl Iterator<Item = &'a Tab>) {
        let open: std::collections::HashSet<&Tab> = open.collect();
        self.pages.retain(|tab, _| open.contains(tab));
    }
}

/// Draw a centred loading spinner with a caption — shown while a page's
/// [`Loadable`] is still pending.
pub(crate) fn loading_spinner(ui: &mut egui::Ui, palette: &Palette, caption: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.4);
        ui.add(egui::Spinner::new().size(28.0).color(palette.accent));
        ui.add_space(8.0);
        ui.label(spottyfi_ui::components::muted(palette, caption, 12.5));
    });
}

/// Draw a centred error message — shown when a page's load failed.
pub(crate) fn load_error(ui: &mut egui::Ui, palette: &Palette, message: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.35);
        ui.label(
            egui::RichText::new("Could not load this page")
                .family(spottyfi_ui::fonts::semibold())
                .size(16.0)
                .color(palette.error),
        );
        ui.add_space(4.0);
        ui.label(spottyfi_ui::components::muted(palette, message, 12.0));
    });
}

/// Construct the [`Page`] for a given [`Tab`], kicking off its async load.
fn build_page(tab: &Tab, services: &PageServices) -> Box<dyn Page> {
    match tab {
        Tab::Home => Box::new(HomePage::new(services)),
        Tab::Library => Box::new(LibraryPage::new(services)),
        Tab::LikedSongs => Box::new(LikedSongsPage::new(services)),
        Tab::Playlist(id) => Box::new(PlaylistPage::new(services, id.clone())),
        Tab::Album(id) => Box::new(AlbumPage::new(services, id.clone())),
        Tab::Artist(id) => Box::new(ArtistPage::new(services, id.clone())),
        Tab::Search => Box::new(SearchPage::new(services)),
        Tab::Browse => Box::new(BrowsePage::new(services)),
        Tab::Category(id) => Box::new(CategoryPage::new(services, id.clone())),
        Tab::Charts => Box::new(ChartsPage::new(services)),
        Tab::NewReleases => Box::new(NewReleasesPage::new(services)),
        Tab::MadeForYou => Box::new(MadeForYouPage::new(services)),
        // Panels are not pages; the registry is only consulted for page tabs.
        // `Settings` is self-rendered by the shell, not registry-backed.
        Tab::NowPlayingArt
        | Tab::Queue
        | Tab::Visualiser
        | Tab::Debug
        | Tab::Placeholder(_)
        | Tab::Settings => Box::new(HomePage::new(services)),
    }
}
