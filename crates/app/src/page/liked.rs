//! The Liked Songs page: the user's saved-tracks table.

use std::sync::Arc;

use spottyfi_api::ApiError;
use spottyfi_models::SpotifyId as _;
use spottyfi_ui::components;
use spottyfi_ui::track_table::{self, TrackColumns, TrackRow, TrackTableState};

use super::track_view::{self, Entry};
use super::{load_error, loading_spinner, Loadable, Page, PageAction, PageContext, PageServices};

/// The data the page loads: every saved track in load order.
type Loaded = Result<Vec<Entry>, ApiError>;

/// The Liked Songs page: a header over a sortable saved-tracks table.
pub struct LikedSongsPage {
    /// The async load of the saved tracks.
    data: Loadable<Loaded>,
    /// The track table's sort state.
    sort: TrackTableState,
    /// The currently displayed (sorted) rows.
    sorted: Vec<Entry>,
    /// The sort the `sorted` cache was built for.
    sorted_for: Option<TrackTableState>,
}

impl LikedSongsPage {
    /// Build the page and kick off the saved-tracks load.
    #[must_use]
    pub fn new(services: &PageServices) -> Self {
        let data = spawn_load(services);
        Self {
            data,
            sort: TrackTableState::default(),
            sorted: Vec::new(),
            sorted_for: None,
        }
    }
}

/// Spawn the saved-tracks load. Spotify's saved-tracks endpoint does not carry
/// the per-track add date in the mapped [`Track`](spottyfi_models::Track), so
/// the date-added column stays empty here for now.
fn spawn_load(services: &PageServices) -> Loadable<Loaded> {
    let api = Arc::clone(&services.api);
    Loadable::spawn(&services.runtime, &services.ctx, async move {
        let mut original = Vec::new();
        let mut offset = 0u32;
        loop {
            let page = api.saved_tracks(offset, 50).await?;
            let count = page.items.len() as u32;
            for track in page.items {
                original.push(Entry {
                    track,
                    added_at: None,
                });
            }
            if !page.has_next || count == 0 {
                break;
            }
            offset += count;
        }
        Ok(original)
    })
}

impl Page for LikedSongsPage {
    fn title(&self) -> String {
        "Liked Songs".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;
        let Some(loaded) = self.data.value() else {
            loading_spinner(ui, &palette, "Loading your liked songs…");
            return None;
        };
        let original = match loaded {
            Ok(tracks) => tracks,
            Err(err) => {
                load_error(ui, &palette, &err.to_string());
                return None;
            }
        };

        if self.sorted_for != Some(self.sort) {
            self.sorted = original.clone();
            track_view::sort_entries(
                &mut self.sorted,
                original,
                self.sort.column,
                self.sort.descending,
            );
            self.sorted_for = Some(self.sort);
        }

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
                            format!("{} songs", original.len()),
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
                        action = track_view::resolve_action(table_action, &self.sorted);
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
