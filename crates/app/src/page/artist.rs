//! The artist page: an artist header, the artist's top tracks and a grid of
//! their albums.

use std::sync::Arc;

use spottyfi_api::ApiError;
use spottyfi_models::{Artist, SimplifiedAlbum, SpotifyId as _, Track};
use spottyfi_ui::components;
use spottyfi_ui::track_table::{self, TrackColumns, TrackRow, TrackTableState};

use super::track_view::{self, Entry};
use super::{load_error, loading_spinner, Loadable, Page, PageAction, PageContext, PageServices};
use crate::shell::Tab;

/// The data the artist page loads.
type Loaded = Result<ArtistData, ApiError>;

/// A fully-loaded artist with their top tracks and albums.
struct ArtistData {
    /// The artist metadata.
    artist: Artist,
    /// The artist's top tracks. Empty if Spotify's deprecated top-tracks
    /// endpoint is unavailable to this app — see `docs/questions.md`.
    top_tracks: Vec<Entry>,
    /// The artist's albums.
    albums: Vec<SimplifiedAlbum>,
}

/// An artist tab.
pub struct ArtistPage {
    /// The async load of the artist, top tracks and albums.
    data: Loadable<Loaded>,
    /// The top-tracks table sort state.
    sort: TrackTableState,
    /// The currently displayed (sorted) top-track rows.
    sorted: Vec<Entry>,
    /// The sort the `sorted` cache was built for.
    sorted_for: Option<TrackTableState>,
}

impl ArtistPage {
    /// Build the page and kick off the artist load.
    #[must_use]
    pub fn new(services: &PageServices, id: String) -> Self {
        let data = spawn_load(services, id);
        Self {
            data,
            sort: TrackTableState::default(),
            sorted: Vec::new(),
            sorted_for: None,
        }
    }
}

/// Spawn the artist load: the artist, their top tracks and their albums.
fn spawn_load(services: &PageServices, id: String) -> Loadable<Loaded> {
    let api = Arc::clone(&services.api);
    Loadable::spawn_tracked(
        &services.runtime,
        &services.ctx,
        &services.activity,
        "Loading artist…",
        async move {
            let artist = api.artist(&id).await?;
            // Top tracks rely on a Spotify endpoint that may be unavailable to
            // apps registered after 2024-11-27; treat that as "no top tracks"
            // rather than failing the whole page.
            let top_tracks = match api.artist_top_tracks(&id).await {
                Ok(tracks) => tracks
                    .into_iter()
                    .map(|track| Entry {
                        track,
                        added_at: None,
                    })
                    .collect(),
                Err(ApiError::EndpointUnavailable { endpoint }) => {
                    tracing::warn!(endpoint, "artist top tracks unavailable to this app");
                    Vec::new()
                }
                Err(err) => return Err(err),
            };
            let albums = api.artist_albums(&id, 0, 50).await?.items;
            Ok(ArtistData {
                artist,
                top_tracks,
                albums,
            })
        },
    )
}

impl Page for ArtistPage {
    fn title(&self) -> String {
        match self.data.value() {
            Some(Ok(data)) => data.artist.name.clone(),
            _ => "Artist".to_owned(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;
        let Some(loaded) = self.data.value() else {
            loading_spinner(ui, &palette, "Loading artist…");
            return None;
        };
        let data = match loaded {
            Ok(data) => data,
            Err(err) => {
                load_error(ui, &palette, &err.to_string());
                return None;
            }
        };

        if self.sorted_for != Some(self.sort) {
            self.sorted = data.top_tracks.clone();
            track_view::sort_entries(
                &mut self.sorted,
                &data.top_tracks,
                self.sort.column,
                self.sort.descending,
            );
            self.sorted_for = Some(self.sort);
        }

        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                header(ui, &palette, &data.artist);
                ui.add_space(16.0);

                // Top tracks.
                components::section_header(ui, &palette, "Popular");
                if self.sorted.is_empty() {
                    ui.label(components::muted(
                        &palette,
                        "Top tracks are unavailable for this artist.",
                        12.0,
                    ));
                } else {
                    let playing_uri = ctx.playback.track.as_ref().map(|t| t.uri.as_str());
                    let rows: Vec<TrackRow<'_>> = self
                        .sorted
                        .iter()
                        .enumerate()
                        .map(|(i, entry)| TrackRow {
                            track: &entry.track,
                            position: i + 1,
                            date_added: None,
                            is_playing: is_playing(&entry.track, playing_uri),
                        })
                        .collect();
                    if let Some(table_action) = track_table::track_table(
                        ui,
                        &palette,
                        self.sort,
                        TrackColumns::album_page(),
                        &rows,
                        38.0,
                    ) {
                        if let track_table::TrackAction::Sort(column) = &table_action {
                            self.sort.toggle(*column);
                        } else if let Some(a) =
                            track_view::resolve_action(table_action, &self.sorted)
                        {
                            action = Some(a);
                        }
                    }
                }

                ui.add_space(16.0);
                // Albums grid.
                components::section_header(ui, &palette, "Albums");
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for album in &data.albums {
                        if let Some(a) = album_card(ui, &palette, album) {
                            action = Some(a);
                        }
                    }
                });
            });
        action
    }
}

/// Whether `track` is the one currently playing.
fn is_playing(track: &Track, playing_uri: Option<&str>) -> bool {
    match (track.id.as_ref(), playing_uri) {
        (Some(id), Some(uri)) => id.uri() == uri,
        _ => false,
    }
}

/// The artist header: avatar, name and a follower/genre line.
fn header(ui: &mut egui::Ui, palette: &spottyfi_ui::theme::Palette, artist: &Artist) {
    ui.horizontal(|ui| {
        let art = artist.images.first().map(|i| i.url.as_str());
        components::album_art(ui, palette, art, 160.0, 0.0);
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.label(components::muted(palette, "Artist", 11.0));
            ui.label(
                egui::RichText::new(&artist.name)
                    .family(spottyfi_ui::fonts::semibold())
                    .size(32.0)
                    .color(palette.text),
            );
            if !artist.genres.is_empty() {
                ui.add_space(4.0);
                ui.label(components::muted(palette, artist.genres.join(" · "), 12.0));
            }
        });
    });
}

/// A clickable album card in the artist's albums grid.
fn album_card(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    album: &SimplifiedAlbum,
) -> Option<PageAction> {
    let mut action = None;
    let size = egui::vec2(150.0, 196.0);
    let frame = egui::Frame::new()
        .fill(palette.card)
        .corner_radius(0)
        .inner_margin(egui::Margin::same(10));
    let response = frame
        .show(ui, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.vertical(|ui| {
                let art = album.images.first().map(|i| i.url.as_str());
                components::album_art(ui, palette, art, 128.0, 0.0);
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&album.name)
                            .family(spottyfi_ui::fonts::medium())
                            .size(13.0)
                            .color(palette.text),
                    )
                    .truncate(),
                );
                if let Some(date) = &album.release_date {
                    ui.label(components::muted(palette, date.clone(), 11.0));
                }
            });
        })
        .response
        .interact(egui::Sense::click());
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        if let Some(id) = &album.id {
            action = Some(PageAction::Open(Tab::Album(id.id().to_owned())));
        }
    }
    action
}
