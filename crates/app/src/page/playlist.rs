//! The playlist page: a playlist header over a sortable track table.
//!
//! The track list streams in **incrementally** — the playlist header and its
//! first page of tracks render the instant they arrive, and the rest of the
//! tracks stream in underneath without ever blocking the UI thread.

use std::sync::Arc;

use futures::StreamExt as _;
use spottyfi_api::ApiError;
use spottyfi_models::{Playlist, PlaylistTrack, SpotifyId as _};
use spottyfi_ui::components;
use spottyfi_ui::track_table::{self, TrackColumns, TrackRow, TrackTableState};

use super::incremental::IncrementalLoad;
use super::track_view::{self, Entry, PlayContext};
use super::{load_error, loading_spinner, Loadable, Page, PageAction, PageContext, PageServices};

/// The playlist metadata load: name, description, art, owner.
type LoadedMeta = Result<Playlist, ApiError>;

/// A playlist tab: header card plus a sortable track table.
pub struct PlaylistPage {
    /// The async load of the playlist metadata (one fetch).
    meta: Loadable<LoadedMeta>,
    /// The incremental stream of the playlist's tracks.
    tracks: IncrementalLoad<Entry>,
    /// The track table's sort state (column + direction).
    sort: TrackTableState,
    /// The currently displayed (sorted) rows; rebuilt when the sort changes
    /// or more tracks stream in.
    sorted: Vec<Entry>,
    /// The sort + track-count the `sorted` cache was built for, so it is
    /// rebuilt only when the sort changes or new tracks arrive.
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

/// Spawn the incremental stream of the playlist's tracks. Each track is mapped
/// to an [`Entry`] as it arrives so the table renders it immediately.
fn spawn_tracks(services: &PageServices, id: String) -> IncrementalLoad<Entry> {
    let stream = services.api.playlist_tracks_stream(&id).filter_map(
        |item: Result<PlaylistTrack, ApiError>| async move {
            match item {
                Ok(track) => track.track.map(|t| {
                    Ok(Entry {
                        track: t,
                        added_at: track.added_at,
                    })
                }),
                Err(err) => Some(Err(err)),
            }
        },
    );
    IncrementalLoad::spawn(
        &services.runtime,
        &services.ctx,
        &services.activity,
        "Loading playlist tracks…",
        stream,
    )
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

        // Rebuild the sorted view when the sort changed or more tracks
        // streamed in; keep it cheap by checking the streamed item count.
        self.tracks.with(|snapshot| {
            let key = (sort, snapshot.items.len());
            if self.sorted_for != Some(key) {
                self.sorted = snapshot.items.to_vec();
                track_view::sort_entries(
                    &mut self.sorted,
                    snapshot.items,
                    sort.column,
                    sort.descending,
                );
                self.sorted_for = Some(key);
            }
        });

        let still_loading = !self.tracks.is_done();
        let stream_error = self.tracks.with(|s| s.error.map(ToString::to_string));

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
                        action = track_view::resolve_action(table_action, &self.sorted, &context);
                    }
                }

                // A thin "still streaming" / error footer beneath the table.
                if still_loading {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(13.0).color(palette.accent));
                        ui.label(components::muted(
                            &palette,
                            format!("Loading more tracks… ({} so far)", self.sorted.len()),
                            11.0,
                        ));
                    });
                } else if let Some(err) = &stream_error {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!("Some tracks failed to load: {err}"))
                            .size(11.0)
                            .color(palette.error),
                    );
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
