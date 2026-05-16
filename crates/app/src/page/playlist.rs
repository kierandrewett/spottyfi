//! The playlist page: a playlist header over a sortable track table.

use std::sync::Arc;

use spottyfi_api::ApiError;
use spottyfi_models::{Playlist, SpotifyId as _};
use spottyfi_ui::components;
use spottyfi_ui::track_table::{self, TrackColumns, TrackRow, TrackTableState};

use super::track_view::{self, Entry};
use super::{load_error, loading_spinner, Loadable, Page, PageAction, PageContext, PageServices};

/// The data a playlist page loads: the playlist plus every track in it.
type Loaded = Result<PlaylistData, ApiError>;

/// A fully-loaded playlist and its tracks.
struct PlaylistData {
    /// The playlist metadata (name, description, art, owner).
    playlist: Playlist,
    /// Every track in load order — the canonical, unsorted list.
    original: Vec<Entry>,
}

/// A playlist tab: header card plus a sortable track table.
pub struct PlaylistPage {
    /// The async load of the playlist and its tracks.
    data: Loadable<Loaded>,
    /// The track table's sort state (column + direction).
    sort: TrackTableState,
    /// The currently displayed (sorted) rows; rebuilt when the sort changes.
    sorted: Vec<Entry>,
    /// The sort the `sorted` cache was built for, so it is rebuilt only on
    /// change.
    sorted_for: Option<TrackTableState>,
}

impl PlaylistPage {
    /// Build the page and kick off the async playlist load.
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

/// Spawn the playlist + full-track-list load onto the runtime.
fn spawn_load(services: &PageServices, id: String) -> Loadable<Loaded> {
    let api = Arc::clone(&services.api);
    Loadable::spawn(&services.runtime, &services.ctx, async move {
        let playlist = api.playlist(&id).await?;
        let mut original = Vec::new();
        // The playlist object carries its first page; fetch the rest in pages.
        let mut offset = 0u32;
        loop {
            let page = api.playlist_tracks(&id, offset, 100).await?;
            let count = page.items.len() as u32;
            for item in page.items {
                if let Some(track) = item.track {
                    original.push(Entry {
                        track,
                        added_at: item.added_at,
                    });
                }
            }
            if !page.has_next || count == 0 {
                break;
            }
            offset += count;
        }
        Ok(PlaylistData { playlist, original })
    })
}

impl Page for PlaylistPage {
    fn title(&self) -> String {
        match self.data.value() {
            Some(Ok(data)) => data.playlist.name.clone(),
            _ => "Playlist".to_owned(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;
        let Some(loaded) = self.data.value() else {
            loading_spinner(ui, &palette, "Loading playlist…");
            return None;
        };
        let data = match loaded {
            Ok(data) => data,
            Err(err) => {
                load_error(ui, &palette, &err.to_string());
                return None;
            }
        };

        // Rebuild the sorted view only when the sort state changed.
        if self.sorted_for != Some(self.sort) {
            self.sorted = data.original.clone();
            track_view::sort_entries(
                &mut self.sorted,
                &data.original,
                self.sort.column,
                self.sort.descending,
            );
            self.sorted_for = Some(self.sort);
        }

        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                header(ui, &palette, &data.playlist);
                ui.add_space(14.0);

                let playing_uri = ctx
                    .playback
                    .track
                    .as_ref()
                    .map(|t| t.uri.as_str());
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
                    56.0,
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

/// The playlist header: cover art, name, owner and track count.
fn header(ui: &mut egui::Ui, palette: &spottyfi_ui::theme::Palette, playlist: &Playlist) {
    ui.horizontal(|ui| {
        let art = playlist.images.first().map(|i| i.url.as_str());
        components::album_art(ui, palette, art, 160.0, 8.0);
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.label(components::muted(palette, "Playlist", 11.0));
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
        });
    });
}
