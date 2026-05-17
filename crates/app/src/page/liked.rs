//! The Liked Songs page: the user's saved-tracks table.
//!
//! The saved tracks stream in **incrementally** so the first page renders the
//! instant it arrives rather than waiting for every page to be collected.

use spottyfi_api::ApiError;
use spottyfi_models::{SavedTrack, SpotifyId as _};
use spottyfi_ui::components;
use spottyfi_ui::track_table::{self, TrackColumns, TrackRow, TrackTableState};

use super::incremental::IncrementalLoad;
use super::track_view::{self, Entry, PlayContext};
use super::{loading_spinner, Page, PageAction, PageContext, PageServices};

/// The Liked Songs page: a header over a sortable saved-tracks table.
pub struct LikedSongsPage {
    /// The incremental stream of the user's saved tracks.
    tracks: IncrementalLoad<Entry>,
    /// The track table's sort state.
    sort: TrackTableState,
    /// The currently displayed (sorted) rows.
    sorted: Vec<Entry>,
    /// The sort + track-count the `sorted` cache was built for.
    sorted_for: Option<(TrackTableState, usize)>,
}

impl LikedSongsPage {
    /// Build the page and kick off the saved-tracks load.
    #[must_use]
    pub fn new(services: &PageServices) -> Self {
        Self {
            tracks: spawn_tracks(services),
            sort: TrackTableState::default(),
            sorted: Vec::new(),
            sorted_for: None,
        }
    }
}

/// Spawn the incremental saved-tracks stream. Each [`SavedTrack`] carries
/// Spotify's `added_at`, so the date-added column and its sort are populated.
fn spawn_tracks(services: &PageServices) -> IncrementalLoad<Entry> {
    use futures::StreamExt as _;
    let stream = services
        .api
        .saved_tracks_stream()
        .map(|item: Result<SavedTrack, ApiError>| {
            item.map(|saved| Entry {
                track: saved.track,
                added_at: saved.added_at,
            })
        });
    IncrementalLoad::spawn(
        &services.runtime,
        &services.ctx,
        &services.activity,
        "Loading liked songs…",
        stream,
    )
}

impl Page for LikedSongsPage {
    fn title(&self) -> String {
        "Liked Songs".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;

        let still_loading = !self.tracks.is_done();
        if self.tracks.len() == 0 && still_loading {
            loading_spinner(ui, &palette, "Loading your liked songs…");
            return None;
        }

        let sort = self.sort;
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
        let stream_error = self.tracks.with(|s| s.error.map(ToString::to_string));

        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    components::album_art(ui, &palette, None, 160.0, 0.0);
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.label(components::muted(&palette, "Playlist", 11.0));
                        ui.label(
                            egui::RichText::new("Liked Songs")
                                .family(spottyfi_ui::fonts::semibold())
                                .size(30.0)
                                .color(palette.text),
                        );
                        ui.add_space(6.0);
                        ui.label(components::muted(
                            &palette,
                            format!("{} songs", self.sorted.len()),
                            12.0,
                        ));
                    });
                });
                ui.add_space(14.0);

                let playing_uri = ctx.playback.track.as_ref().map(|t| t.uri.as_str());
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
                        let context = PlayContext {
                            uri: "spotify:collection:tracks".to_owned(),
                            name: "Liked Songs".to_owned(),
                        };
                        action = track_view::resolve_action(table_action, &self.sorted, &context);
                    }
                }

                if still_loading {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(13.0).color(palette.accent));
                        ui.label(components::muted(&palette, "Loading more songs…", 11.0));
                    });
                } else if let Some(err) = &stream_error {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!("Some songs failed to load: {err}"))
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
