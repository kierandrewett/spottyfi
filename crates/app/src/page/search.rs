//! The Search page: a debounced, cancellable catalogue search.
//!
//! Layout, top to bottom:
//!
//! - a **search input** (there is no top-bar search box — search is its own
//!   page);
//! - a row of **category tabs** — All, Songs, Artists, Albums, Playlists,
//!   Podcasts;
//! - the results for the active category. **All** shows a large *Top result*
//!   card beside a short Songs list, then horizontal shelves of the other
//!   kinds; each other category shows the full list or grid for its kind.
//!
//! Typing re-runs the query, debounced ~250ms, with the previous in-flight
//! request cancelled — see [`search_load`](super::search_load).
//!
//! Podcasts/audiobooks: the `api` crate's [`SearchType`](spottyfi_api::SearchType)
//! does not yet carry a show/episode variant, so the Podcasts tab renders an
//! explanatory note rather than results. See the Phase 6 report.

use std::time::Instant;

use spottyfi_models::{
    Artist, SearchResults, SimplifiedAlbum, SimplifiedArtist, SimplifiedPlaylist, SpotifyId as _,
    Track,
};
use spottyfi_ui::components;
use spottyfi_ui::track_table::{self, TrackColumns, TrackRow, TrackTableState};

use super::search_load::{Debounce, SearchLoad};
use super::track_view::{self, Entry};
use super::{load_error, Page, PageAction, PageContext, PageServices};
use crate::shell::Tab;

/// How many songs the **All** tab's inline Songs list shows.
const ALL_SONGS_LEN: usize = 5;

/// A result category — one tab within the search page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    /// Everything: a Top result, a short Songs list, shelves of the rest.
    All,
    /// The full track list.
    Songs,
    /// The full artist grid.
    Artists,
    /// The full album grid.
    Albums,
    /// The full playlist grid.
    Playlists,
    /// Podcasts — not yet wired (see the module docs).
    Podcasts,
}

impl Category {
    /// Every category, in tab-bar order.
    const ALL: [Category; 6] = [
        Category::All,
        Category::Songs,
        Category::Artists,
        Category::Albums,
        Category::Playlists,
        Category::Podcasts,
    ];

    /// The category's tab label.
    fn label(self) -> &'static str {
        match self {
            Category::All => "All",
            Category::Songs => "Songs",
            Category::Artists => "Artists",
            Category::Albums => "Albums",
            Category::Playlists => "Playlists",
            Category::Podcasts => "Podcasts",
        }
    }
}

/// The Search tab.
pub struct SearchPage {
    /// Shared services (API, runtime, egui context, activity registry).
    services: PageServices,
    /// The current text in the search input.
    query: String,
    /// The query string the [`SearchLoad`] was last dispatched with — so an
    /// edit is detected and the debounce armed.
    dispatched: String,
    /// The keystroke debouncer.
    debounce: Debounce,
    /// The debounced, cancellable search itself.
    load: SearchLoad,
    /// The active result category tab.
    category: Category,
    /// Set when the input should grab focus next frame (`Ctrl/Cmd+K`, or the
    /// first frame the page opens).
    focus_input: bool,
}

impl SearchPage {
    /// Build the Search page. Nothing is queried until the user types.
    #[must_use]
    pub fn new(services: &PageServices) -> Self {
        Self {
            services: services.clone(),
            query: String::new(),
            dispatched: String::new(),
            debounce: Debounce::default(),
            load: SearchLoad::new(),
            category: Category::All,
            // Grab focus the first frame so the user can type immediately.
            focus_input: true,
        }
    }
}

impl Page for SearchPage {
    fn title(&self) -> String {
        "Search".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;
        let mut action = None;

        // Ctrl/Cmd+K focuses the input whenever this page is on screen.
        if ui.input(|i| i.key_pressed(egui::Key::K) && (i.modifiers.command || i.modifiers.ctrl)) {
            self.focus_input = true;
        }

        // The search input.
        self.search_input(ui, &palette);

        // Debounce: arm on each edit, dispatch once the user pauses.
        if self.query != self.dispatched {
            self.debounce.edited(Instant::now());
            self.dispatched = self.query.clone();
        }
        if self.debounce.due(Instant::now()) {
            self.load.dispatch(
                &self.query,
                &self.services.api,
                &self.services.runtime,
                &self.services.ctx,
                &self.services.activity,
            );
        }
        // Keep repainting while a debounce is counting down so the dispatch
        // fires without needing an unrelated event to wake the UI.
        if self.debounce.is_pending() {
            ui.ctx().request_repaint_after(super::search_load::DEBOUNCE);
        }

        ui.add_space(10.0);

        // Nothing typed yet — a calm prompt.
        if self.load.query().trim().is_empty() {
            empty_prompt(ui, &palette);
            return action;
        }

        // The category tabs.
        self.category_bar(ui, &palette);
        ui.add_space(8.0);

        // The results body.
        if self.load.is_loading() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0).color(palette.accent));
                ui.label(components::muted(&palette, "Searching…", 12.0));
            });
            return action;
        }

        let category = self.category;
        let playing_uri = ctx.playback.track.as_ref().map(|t| t.uri.as_str());
        self.load.with_result(|result| {
            let Some(result) = result else {
                return;
            };
            match result {
                Err(err) => load_error(ui, &palette, &err.to_string()),
                Ok(results) => {
                    if let Some(a) = results_body(ui, &palette, category, results, playing_uri) {
                        action = Some(a);
                    }
                }
            }
        });

        action
    }
}

impl SearchPage {
    /// Draw the search input row, applying any pending focus request.
    fn search_input(&mut self, ui: &mut egui::Ui, palette: &spottyfi_ui::theme::Palette) {
        ui.horizontal(|ui| {
            spottyfi_ui::icons::icon(ui, spottyfi_ui::Icon::Search, 16.0, palette.text_muted);
            ui.add_space(6.0);
            let edit = egui::TextEdit::singleline(&mut self.query)
                .hint_text("Search for songs, artists, albums, playlists")
                .desired_width(ui.available_width().min(520.0))
                .font(egui::FontId::proportional(15.0));
            let response = ui.add(edit);
            if std::mem::take(&mut self.focus_input) {
                response.request_focus();
            }
        });
    }

    /// Draw the category tab bar, flat and sharp.
    fn category_bar(&mut self, ui: &mut egui::Ui, palette: &spottyfi_ui::theme::Palette) {
        ui.horizontal(|ui| {
            for category in Category::ALL {
                let selected = self.category == category;
                if components::filter_chip(ui, palette, category.label(), selected).clicked() {
                    self.category = category;
                }
                ui.add_space(4.0);
            }
        });
    }
}

/// A calm prompt shown before the user has typed anything.
fn empty_prompt(ui: &mut egui::Ui, palette: &spottyfi_ui::theme::Palette) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.3);
        spottyfi_ui::icons::icon(ui, spottyfi_ui::Icon::Search, 36.0, palette.text_muted);
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Search Spotify")
                .family(spottyfi_ui::fonts::semibold())
                .size(16.0)
                .color(palette.text),
        );
        ui.add_space(4.0);
        ui.label(components::muted(
            palette,
            "Find songs, artists, albums and playlists.",
            12.0,
        ));
    });
}

/// Render the results body for the active `category`.
fn results_body(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    category: Category,
    results: &SearchResults,
    playing_uri: Option<&str>,
) -> Option<PageAction> {
    let mut action = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            match category {
                Category::All => {
                    if let Some(a) = all_tab(ui, palette, results, playing_uri) {
                        action = Some(a);
                    }
                }
                Category::Songs => {
                    if let Some(a) = song_list(ui, palette, &results.tracks.items, playing_uri) {
                        action = Some(a);
                    }
                }
                Category::Artists => {
                    if let Some(a) = artist_grid(ui, palette, &results.artists.items) {
                        action = Some(a);
                    }
                }
                Category::Albums => {
                    if let Some(a) = album_grid(ui, palette, &results.albums.items) {
                        action = Some(a);
                    }
                }
                Category::Playlists => {
                    if let Some(a) = playlist_grid(ui, palette, &results.playlists.items) {
                        action = Some(a);
                    }
                }
                Category::Podcasts => podcasts_note(ui, palette),
            }
            if category != Category::Podcasts && is_empty(results) {
                ui.label(components::muted(palette, "No results found.", 13.0));
            }
        });
    action
}

/// Whether a search returned nothing at all.
fn is_empty(results: &SearchResults) -> bool {
    results.tracks.items.is_empty()
        && results.artists.items.is_empty()
        && results.albums.items.is_empty()
        && results.playlists.items.is_empty()
}

/// The **All** tab: a Top result card, a short Songs list, then shelves.
fn all_tab(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    results: &SearchResults,
    playing_uri: Option<&str>,
) -> Option<PageAction> {
    let mut action = None;

    // Top result + inline Songs list, side by side.
    let has_top = top_result(results).is_some();
    if has_top || !results.tracks.items.is_empty() {
        ui.columns(2, |cols| {
            if let Some(top) = top_result(results) {
                components::section_header(&mut cols[0], palette, "Top result");
                if let Some(a) = top_result_card(&mut cols[0], palette, &top) {
                    action = Some(a);
                }
            }
            if !results.tracks.items.is_empty() {
                components::section_header(&mut cols[1], palette, "Songs");
                let songs = &results.tracks.items[..results.tracks.items.len().min(ALL_SONGS_LEN)];
                if let Some(a) = song_list(&mut cols[1], palette, songs, playing_uri) {
                    action = Some(a);
                }
            }
        });
        ui.add_space(16.0);
    }

    // Horizontal shelves for the remaining kinds.
    if !results.artists.items.is_empty() {
        components::section_header(ui, palette, "Artists");
        ui.add_space(4.0);
        if let Some(a) = artist_shelf(ui, palette, &results.artists.items) {
            action = Some(a);
        }
        ui.add_space(16.0);
    }
    if !results.albums.items.is_empty() {
        components::section_header(ui, palette, "Albums");
        ui.add_space(4.0);
        if let Some(a) = album_shelf(ui, palette, &results.albums.items) {
            action = Some(a);
        }
        ui.add_space(16.0);
    }
    if !results.playlists.items.is_empty() {
        components::section_header(ui, palette, "Playlists");
        ui.add_space(4.0);
        if let Some(a) = playlist_shelf(ui, palette, &results.playlists.items) {
            action = Some(a);
        }
    }

    action
}

/// The single best match across kinds — the **Top result**.
///
/// Spotify orders each result list by relevance, so the top track or the top
/// artist is the natural candidate; an artist match is preferred (it is the
/// most common "I meant this" intent), else the first track.
fn top_result(results: &SearchResults) -> Option<TopResult> {
    if let Some(artist) = results.artists.items.first() {
        return Some(TopResult::Artist(artist.clone()));
    }
    if let Some(album) = results.albums.items.first() {
        return Some(TopResult::Album(album.clone()));
    }
    if let Some(track) = results.tracks.items.first() {
        return Some(TopResult::Track(track.clone()));
    }
    results
        .playlists
        .items
        .first()
        .cloned()
        .map(TopResult::Playlist)
}

/// The kind of object shown in the Top result card.
enum TopResult {
    /// The top artist match.
    Artist(Artist),
    /// The top album match.
    Album(SimplifiedAlbum),
    /// The top track match.
    Track(Track),
    /// The top playlist match.
    Playlist(SimplifiedPlaylist),
}

/// The large Top result card. Clicking it navigates to the object's page.
fn top_result_card(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    top: &TopResult,
) -> Option<PageAction> {
    let (art, title, kicker, tab) = match top {
        TopResult::Artist(a) => (
            a.images.first().map(|i| i.url.as_str()),
            a.name.clone(),
            "Artist".to_owned(),
            Tab::Artist(a.id.id().to_owned()),
        ),
        TopResult::Album(a) => (
            a.images.first().map(|i| i.url.as_str()),
            a.name.clone(),
            format!("Album · {}", artist_names(&a.artists)),
            a.id.as_ref()
                .map_or(Tab::Search, |id| Tab::Album(id.id().to_owned())),
        ),
        TopResult::Track(t) => (
            t.album.images.first().map(|i| i.url.as_str()),
            t.name.clone(),
            format!("Song · {}", artist_names(&t.artists)),
            // A track has no page of its own; the card opens its album.
            t.album
                .id
                .as_ref()
                .map_or(Tab::Search, |id| Tab::Album(id.id().to_owned())),
        ),
        TopResult::Playlist(p) => (
            p.images.first().map(|i| i.url.as_str()),
            p.name.clone(),
            "Playlist".to_owned(),
            Tab::Playlist(p.id.id().to_owned()),
        ),
    };

    let frame = egui::Frame::new()
        .fill(palette.card)
        .corner_radius(0)
        .inner_margin(egui::Margin::same(16));
    let response = frame
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.vertical(|ui| {
                components::album_art(ui, palette, art, 92.0, 0.0);
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&title)
                            .family(spottyfi_ui::fonts::semibold())
                            .size(22.0)
                            .color(palette.text),
                    )
                    .truncate(),
                );
                ui.add_space(2.0);
                ui.label(components::muted(palette, kicker, 11.5));
            });
        })
        .response
        .interact(egui::Sense::click());

    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        && tab != Tab::Search
    {
        return Some(PageAction::Open(tab));
    }
    None
}

/// A track list rendered with the shared track-table widget.
///
/// Double-click plays; the row context menu navigates / copies. Search results
/// have no inherent order, so the table's index column is its only sort.
fn song_list(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    tracks: &[Track],
    playing_uri: Option<&str>,
) -> Option<PageAction> {
    if tracks.is_empty() {
        return None;
    }
    let entries: Vec<Entry> = tracks
        .iter()
        .map(|track| Entry {
            track: track.clone(),
            added_at: None,
        })
        .collect();
    let rows: Vec<TrackRow<'_>> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| TrackRow {
            track: &entry.track,
            position: i + 1,
            date_added: None,
            is_playing: is_playing(&entry.track, playing_uri),
        })
        .collect();
    let table_action = track_table::track_table(
        ui,
        palette,
        TrackTableState::default(),
        TrackColumns::album_page(),
        &rows,
        38.0,
    )?;
    // Search results are not user-sortable; ignore header clicks.
    if matches!(table_action, track_table::TrackAction::Sort(_)) {
        return None;
    }
    track_view::resolve_action(table_action, &entries)
}

/// Whether `track` is the one currently playing.
fn is_playing(track: &Track, playing_uri: Option<&str>) -> bool {
    match (track.id.as_ref(), playing_uri) {
        (Some(id), Some(uri)) => id.uri() == uri,
        _ => false,
    }
}

/// A wrapping grid of artist cards.
fn artist_grid(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    artists: &[Artist],
) -> Option<PageAction> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        for artist in artists {
            if let Some(a) = artist_card(ui, palette, artist) {
                action = Some(a);
            }
        }
    });
    action
}

/// A single-row horizontal shelf of artist cards.
fn artist_shelf(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    artists: &[Artist],
) -> Option<PageAction> {
    let mut action = None;
    egui::ScrollArea::horizontal()
        .id_salt("search-artist-shelf")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for artist in artists {
                    if let Some(a) = artist_card(ui, palette, artist) {
                        action = Some(a);
                    }
                }
            });
        });
    action
}

/// A clickable artist card; clicking opens the artist page.
fn artist_card(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    artist: &Artist,
) -> Option<PageAction> {
    let art = artist.images.first().map(|i| i.url.as_str());
    card(ui, palette, &artist.name, "Artist", art)
        .then(|| PageAction::Open(Tab::Artist(artist.id.id().to_owned())))
}

/// A wrapping grid of album cards.
fn album_grid(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    albums: &[SimplifiedAlbum],
) -> Option<PageAction> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        for album in albums {
            if let Some(a) = album_card(ui, palette, album) {
                action = Some(a);
            }
        }
    });
    action
}

/// A single-row horizontal shelf of album cards.
fn album_shelf(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    albums: &[SimplifiedAlbum],
) -> Option<PageAction> {
    let mut action = None;
    egui::ScrollArea::horizontal()
        .id_salt("search-album-shelf")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for album in albums {
                    if let Some(a) = album_card(ui, palette, album) {
                        action = Some(a);
                    }
                }
            });
        });
    action
}

/// A clickable album card; clicking opens the album page.
fn album_card(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    album: &SimplifiedAlbum,
) -> Option<PageAction> {
    let art = album.images.first().map(|i| i.url.as_str());
    let clicked = card(ui, palette, &album.name, &artist_names(&album.artists), art);
    if clicked {
        album
            .id
            .as_ref()
            .map(|id| PageAction::Open(Tab::Album(id.id().to_owned())))
    } else {
        None
    }
}

/// A wrapping grid of playlist cards.
fn playlist_grid(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    playlists: &[SimplifiedPlaylist],
) -> Option<PageAction> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        for playlist in playlists {
            if let Some(a) = playlist_card(ui, palette, playlist) {
                action = Some(a);
            }
        }
    });
    action
}

/// A single-row horizontal shelf of playlist cards.
fn playlist_shelf(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    playlists: &[SimplifiedPlaylist],
) -> Option<PageAction> {
    let mut action = None;
    egui::ScrollArea::horizontal()
        .id_salt("search-playlist-shelf")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for playlist in playlists {
                    if let Some(a) = playlist_card(ui, palette, playlist) {
                        action = Some(a);
                    }
                }
            });
        });
    action
}

/// A clickable playlist card; clicking opens the playlist page.
fn playlist_card(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    playlist: &SimplifiedPlaylist,
) -> Option<PageAction> {
    let art = playlist.images.first().map(|i| i.url.as_str());
    let owner = playlist
        .owner
        .display_name
        .clone()
        .unwrap_or_else(|| playlist.owner.id.to_string());
    card(ui, palette, &playlist.name, &owner, art)
        .then(|| PageAction::Open(Tab::Playlist(playlist.id.id().to_owned())))
}

/// A generic clickable result card: cover art, a title and a muted subtitle.
/// Returns `true` when clicked.
fn card(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    title: &str,
    subtitle: &str,
    art: Option<&str>,
) -> bool {
    let size = egui::vec2(150.0, 210.0);
    let frame = egui::Frame::new()
        .fill(palette.card)
        .corner_radius(0)
        .inner_margin(egui::Margin::same(10));
    let response = frame
        .show(ui, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.vertical(|ui| {
                components::album_art(ui, palette, art, 128.0, 0.0);
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(title)
                            .family(spottyfi_ui::fonts::medium())
                            .size(13.0)
                            .color(palette.text),
                    )
                    .truncate(),
                );
                if !subtitle.is_empty() {
                    ui.add(egui::Label::new(components::muted(palette, subtitle, 11.0)).truncate());
                }
            });
        })
        .response
        .interact(egui::Sense::click());
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// A note explaining the Podcasts tab is not yet wired.
fn podcasts_note(ui: &mut egui::Ui, palette: &spottyfi_ui::theme::Palette) {
    ui.label(components::muted(
        palette,
        "Podcast search isn't wired up yet — the API client's search types \
         don't cover shows. Tracked for a follow-up.",
        12.5,
    ));
}

/// Join a list of simplified artists into a display string.
fn artist_names(artists: &[SimplifiedArtist]) -> String {
    artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::page::Page as PageTrait;
    use spottyfi_api::MockSpotifyApi;
    use spottyfi_models::{AlbumId, ArtistId, Page, PlaylistId, TrackId, User, UserId};

    /// Build `PageServices` over a mock API and a fresh tokio runtime.
    fn services(mock: MockSpotifyApi) -> (PageServices, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build runtime");
        let services = PageServices {
            api: Arc::new(mock),
            runtime: runtime.handle().clone(),
            ctx: egui::Context::default(),
            activity: spottyfi_state::ActivityRegistry::new(),
        };
        (services, runtime)
    }

    fn artist(name: &str) -> Artist {
        Artist {
            id: ArtistId::new(format!("artist-{name}")),
            name: name.to_owned(),
            images: Vec::new(),
            genres: Vec::new(),
            popularity: 0,
        }
    }

    fn track(name: &str) -> Track {
        Track {
            id: Some(TrackId::new(format!("track-{name}"))),
            name: name.to_owned(),
            artists: Vec::new(),
            album: SimplifiedAlbum {
                id: Some(AlbumId::new("album-x")),
                name: "Album".to_owned(),
                images: Vec::new(),
                artists: Vec::new(),
                release_date: None,
            },
            duration_ms: 1000,
            explicit: false,
            popularity: 0,
            track_number: 1,
            is_local: false,
        }
    }

    fn playlist(name: &str) -> SimplifiedPlaylist {
        SimplifiedPlaylist {
            id: PlaylistId::new(format!("pl-{name}")),
            name: name.to_owned(),
            description: None,
            images: Vec::new(),
            owner: User {
                id: UserId::new("owner"),
                display_name: Some("Owner".to_owned()),
                images: Vec::new(),
            },
            collaborative: false,
            total_tracks: 0,
        }
    }

    #[test]
    fn title_is_static() {
        let (services, _rt) = services(MockSpotifyApi::new());
        assert_eq!(PageTrait::title(&SearchPage::new(&services)), "Search");
    }

    #[test]
    fn top_result_prefers_an_artist_then_album_then_track() {
        let mut results = SearchResults {
            tracks: Page {
                items: vec![track("t")],
                ..Page::default()
            },
            ..SearchResults::default()
        };
        // With only a track, the track is the top result.
        assert!(matches!(top_result(&results), Some(TopResult::Track(_))));

        results.albums = Page {
            items: vec![SimplifiedAlbum {
                id: Some(AlbumId::new("a")),
                name: "Album".to_owned(),
                images: Vec::new(),
                artists: Vec::new(),
                release_date: None,
            }],
            ..Page::default()
        };
        // An album outranks a track.
        assert!(matches!(top_result(&results), Some(TopResult::Album(_))));

        results.artists = Page {
            items: vec![artist("a")],
            ..Page::default()
        };
        // An artist outranks everything.
        assert!(matches!(top_result(&results), Some(TopResult::Artist(_))));
    }

    #[test]
    fn top_result_is_none_for_an_empty_search() {
        assert!(top_result(&SearchResults::default()).is_none());
    }

    #[test]
    fn is_empty_detects_a_search_with_no_results() {
        assert!(is_empty(&SearchResults::default()));
        let results = SearchResults {
            playlists: Page {
                items: vec![playlist("p")],
                ..Page::default()
            },
            ..SearchResults::default()
        };
        assert!(!is_empty(&results));
    }

    #[test]
    fn artist_names_joins_with_commas() {
        let artists = vec![
            SimplifiedArtist {
                id: None,
                name: "A".to_owned(),
            },
            SimplifiedArtist {
                id: None,
                name: "B".to_owned(),
            },
        ];
        assert_eq!(artist_names(&artists), "A, B");
    }
}
