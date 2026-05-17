//! The playlist page: a playlist header over a sortable track table.
//!
//! The playlist metadata and its track list both load through the `api`
//! crate's stale-while-revalidate cache: a revisited playlist renders instantly
//! from the SQLite metadata cache, then a background refresh updates it. The
//! track list is fetched as one cached `Vec` rather than streamed — the cache
//! makes the common (revisit) path instant, which a stream cannot.

use std::sync::Arc;

use spottyfi_api::ApiError;
use spottyfi_models::{Playlist, PlaylistTrack, SpotifyId as _};
use spottyfi_ui::components;
use spottyfi_ui::track_table::{self, TrackColumns, TrackRow, TrackTableState};

use super::track_view::{self, Entry, PlayContext};
use super::{load_error, loading_spinner, Loadable, Page, PageAction, PageContext, PageServices};

/// The playlist metadata load: name, description, art, owner.
type LoadedMeta = Result<Playlist, ApiError>;

/// The playlist track-list load: every resolved track, served from cache.
type LoadedTracks = Result<Vec<Entry>, ApiError>;

/// A playlist tab: header card plus a sortable track table.
pub struct PlaylistPage {
    /// The async load of the playlist metadata.
    meta: Loadable<LoadedMeta>,
    /// The async load of the playlist's full (cached) track list.
    tracks: Loadable<LoadedTracks>,
    /// The track table's sort state (column + direction).
    sort: TrackTableState,
    /// The currently displayed (sorted) rows; rebuilt when the sort changes.
    sorted: Vec<Entry>,
    /// The sort the `sorted` cache was built for, so it is rebuilt only when
    /// the sort changes or the track list first arrives.
    sorted_for: Option<(TrackTableState, usize)>,
}

impl PlaylistPage {
    /// Build the page and kick off the async playlist load.
    #[must_use]
    pub fn new(services: &PageServices, id: String) -> Self {
        let meta = spawn_meta(services, id.clone());
        let tracks = spawn_tracks(services, id);
        Self {
            meta,
            tracks,
            sort: TrackTableState::default(),
            sorted: Vec::new(),
            sorted_for: None,
        }
    }
}

/// Spawn the one-shot load of the playlist metadata.
fn spawn_meta(services: &PageServices, id: String) -> Loadable<LoadedMeta> {
    let api = Arc::clone(&services.api);
    Loadable::spawn_tracked(
        &services.runtime,
        &services.ctx,
        &services.activity,
        "Loading playlist…",
        async move { api.playlist(&id).await },
    )
}

/// Spawn the cached load of the playlist's complete track list.
///
/// `playlist_tracks_all` resolves from the SQLite metadata cache when the
/// listing has been seen before — so a revisit is instant — and refreshes in
/// the background when the cached copy is stale.
fn spawn_tracks(services: &PageServices, id: String) -> Loadable<LoadedTracks> {
    let api = Arc::clone(&services.api);
    Loadable::spawn_tracked(
        &services.runtime,
        &services.ctx,
        &services.activity,
        "Loading playlist tracks…",
        async move {
            let tracks = api.playlist_tracks_all(&id).await?;
            Ok(tracks.into_iter().filter_map(to_entry).collect())
        },
    )
}

/// Project a [`PlaylistTrack`] into a table [`Entry`], dropping items with no
/// resolved track (an unavailable track, or a non-track playlist item).
fn to_entry(item: PlaylistTrack) -> Option<Entry> {
    item.track.map(|track| Entry {
        track,
        added_at: item.added_at,
    })
}

impl Page for PlaylistPage {
    fn title(&self) -> String {
        match self.meta.value() {
            Some(Ok(playlist)) => playlist.name.clone(),
            _ => "Playlist".to_owned(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;
        let Some(loaded) = self.meta.value() else {
            loading_spinner(ui, &palette, "Loading playlist…");
            return None;
        };
        let playlist = match loaded {
            Ok(playlist) => playlist,
            Err(err) => {
                load_error(ui, &palette, &err.to_string());
                return None;
            }
        };

        let mut action = None;
        let playing_uri = ctx.playback.track.as_ref().map(|t| t.uri.as_str());
        let sort = self.sort;

        // The track list resolves separately; show its state below the header.
        let tracks_loaded = self.tracks.value();
        let original: &[Entry] = match tracks_loaded {
            Some(Ok(tracks)) => tracks,
            _ => &[],
        };

        // Rebuild the sorted view when the sort changed or the list arrived.
        let key = (sort, original.len());
        if self.sorted_for != Some(key) {
            self.sorted = original.to_vec();
            track_view::sort_entries(&mut self.sorted, original, sort.column, sort.descending);
            self.sorted_for = Some(key);
        }

        let context = PlayContext {
            uri: playlist.id.uri(),
            name: playlist.name.clone(),
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if header(ui, &palette, playlist) {
                    // The hero plays the whole playlist from the top.
                    let tracks = track_view::queue_tracks(&self.sorted);
                    if !tracks.is_empty() {
                        action = Some(PageAction::PlayContext {
                            uri: context.uri.clone(),
                            name: context.name.clone(),
                            tracks,
                            offset: 0,
                        });
                    }
                }
                ui.add_space(14.0);

                match tracks_loaded {
                    None => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(13.0).color(palette.accent));
                            ui.label(components::muted(&palette, "Loading tracks…", 11.0));
                        });
                    }
                    Some(Err(err)) => {
                        ui.label(
                            egui::RichText::new(format!("Couldn't load tracks: {err}"))
                                .size(11.0)
                                .color(palette.error),
                        );
                    }
                    Some(Ok(_)) => {
                        let rows: Vec<TrackRow<'_>> = self
                            .sorted
                            .iter()
                            .enumerate()
                            .map(|(i, entry)| TrackRow {
                                track: &entry.track,
                                position: i + 1,
                                date_added: entry.added_at.as_deref(),
                                is_playing: is_playing(&entry.track, playing_uri),
                            })
                            .collect();

                        if let Some(table_action) = track_table::track_table(
                            ui,
                            &palette,
                            self.sort,
                            TrackColumns::full(),
                            &rows,
                            38.0,
                        ) {
                            if let track_table::TrackAction::Sort(column) = &table_action {
                                self.sort.toggle(*column);
                            } else {
                                action = track_view::resolve_action(
                                    table_action,
                                    &self.sorted,
                                    &context,
                                );
                            }
                        }
                    }
                }
            });
        action
    }
}

/// Whether `track` is the one currently playing.
fn is_playing(track: &spottyfi_models::Track, playing_uri: Option<&str>) -> bool {
    match (track.id.as_ref(), playing_uri) {
        (Some(id), Some(uri)) => id.uri() == uri,
        _ => false,
    }
}

/// The playlist hero: cover art, an uppercase kicker, the title, description,
/// owner/track-count line and a green circular play button. Returns `true`
/// when the play button is clicked.
fn header(ui: &mut egui::Ui, palette: &spottyfi_ui::theme::Palette, playlist: &Playlist) -> bool {
    let mut play = false;
    ui.horizontal(|ui| {
        let art = playlist.images.first().map(|i| i.url.as_str());
        components::album_art(ui, palette, art, 160.0, 0.0);
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("PLAYLIST")
                    .family(spottyfi_ui::fonts::semibold())
                    .size(10.5)
                    .color(palette.text_muted),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(&playlist.name)
                    .family(spottyfi_ui::fonts::semibold())
                    .size(30.0)
                    .color(palette.text),
            );
            if let Some(description) = &playlist.description {
                if !description.is_empty() {
                    ui.add_space(2.0);
                    ui.label(components::muted(palette, description.clone(), 12.5));
                }
            }
            ui.add_space(6.0);
            let owner = playlist
                .owner
                .display_name
                .clone()
                .unwrap_or_else(|| playlist.owner.id.to_string());
            ui.label(components::muted(
                palette,
                format!("{owner}  ·  {} tracks", playlist.total_tracks),
                12.0,
            ));
            ui.add_space(10.0);
            play = hero_play_button(ui, palette);
        });
    });
    play
}

/// The hero's green circular play button followed by a heart (save) toggle.
/// Returns `true` when the play button is clicked.
fn hero_play_button(ui: &mut egui::Ui, palette: &spottyfi_ui::theme::Palette) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(46.0, 46.0), egui::Sense::click());
        if ui.is_rect_visible(rect) {
            ui.painter()
                .circle_filled(rect.center(), rect.width() / 2.0, palette.accent);
            let g = 20.0;
            spottyfi_ui::Icon::Play
                .image(g, egui::Color32::BLACK)
                .paint_at(
                    ui,
                    egui::Rect::from_center_size(rect.center(), egui::vec2(g, g)),
                );
        }
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            clicked = true;
        }
        ui.add_space(8.0);
        spottyfi_ui::icons::icon_button(ui, palette, spottyfi_ui::Icon::Heart, 20.0, false, "Save");
    });
    clicked
}
