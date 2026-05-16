//! The album page: an album header over its track list.

use std::sync::Arc;

use spottyfi_api::ApiError;
use spottyfi_models::{Album, SimplifiedAlbum, SimplifiedTrack, SpotifyId as _, Track};
use spottyfi_ui::components;
use spottyfi_ui::track_table::{self, TrackColumns, TrackRow, TrackTableState};

use super::track_view::{self, Entry};
use super::{load_error, loading_spinner, Loadable, Page, PageAction, PageContext, PageServices};

/// The data the album page loads.
type Loaded = Result<AlbumData, ApiError>;

/// A fully-loaded album and its tracks (already projected to [`Entry`]s).
struct AlbumData {
    /// The album metadata.
    album: Album,
    /// Every track in track-number order.
    original: Vec<Entry>,
}

/// An album tab: header card plus a track table.
pub struct AlbumPage {
    /// The async load of the album and its tracks.
    data: Loadable<Loaded>,
    /// The track table's sort state.
    sort: TrackTableState,
    /// The currently displayed (sorted) rows.
    sorted: Vec<Entry>,
    /// The sort the `sorted` cache was built for.
    sorted_for: Option<TrackTableState>,
}

impl AlbumPage {
    /// Build the page and kick off the album load.
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

/// Spawn the album load and project its simplified tracks to full [`Track`]s.
fn spawn_load(services: &PageServices, id: String) -> Loadable<Loaded> {
    let api = Arc::clone(&services.api);
    Loadable::spawn(&services.runtime, &services.ctx, async move {
        let album = api.album(&id).await?;
        let simplified = SimplifiedAlbum {
            id: Some(album.id.clone()),
            name: album.name.clone(),
            images: album.images.clone(),
            artists: album.artists.clone(),
            release_date: Some(album.release_date.clone()),
        };
        let original = album
            .tracks
            .items
            .iter()
            .map(|track| Entry {
                track: to_track(track, &simplified),
                added_at: None,
            })
            .collect();
        Ok(AlbumData { album, original })
    })
}

/// Promote an album's [`SimplifiedTrack`] to a full [`Track`] by attaching the
/// parent album — the track table needs the album for its art thumbnail.
fn to_track(track: &SimplifiedTrack, album: &SimplifiedAlbum) -> Track {
    Track {
        id: track.id.clone(),
        name: track.name.clone(),
        artists: track.artists.clone(),
        album: album.clone(),
        duration_ms: track.duration_ms,
        explicit: track.explicit,
        popularity: 0,
        track_number: track.track_number,
        is_local: false,
    }
}

impl Page for AlbumPage {
    fn title(&self) -> String {
        match self.data.value() {
            Some(Ok(data)) => data.album.name.clone(),
            _ => "Album".to_owned(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;
        let Some(loaded) = self.data.value() else {
            loading_spinner(ui, &palette, "Loading album…");
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
                if let Some(a) = header(ui, &palette, &data.album) {
                    action = Some(a);
                }
                ui.add_space(14.0);

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
                    } else if let Some(a) = track_view::resolve_action(table_action, &self.sorted) {
                        action = Some(a);
                    }
                }
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

/// The album header: cover art, title, artists and release year. A click on an
/// artist name navigates to that artist's page.
fn header(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    album: &Album,
) -> Option<PageAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let art = album.images.first().map(|i| i.url.as_str());
        components::album_art(ui, palette, art, 160.0, 0.0);
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.label(components::muted(palette, "Album", 11.0));
            ui.label(
                egui::RichText::new(&album.name)
                    .family(spottyfi_ui::fonts::semibold())
                    .size(30.0)
                    .color(palette.text),
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for (i, artist) in album.artists.iter().enumerate() {
                    if i > 0 {
                        ui.label(components::muted(palette, ",", 12.0));
                    }
                    if let Some(id) = &artist.id {
                        let link = ui.add(
                            egui::Label::new(
                                egui::RichText::new(&artist.name)
                                    .size(12.5)
                                    .color(palette.text),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if link
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            action = Some(PageAction::Open(crate::shell::Tab::Artist(
                                id.id().to_owned(),
                            )));
                        }
                    } else {
                        ui.label(components::muted(palette, &artist.name, 12.5));
                    }
                }
                ui.label(components::muted(
                    palette,
                    format!(
                        "  ·  {}  ·  {} tracks",
                        album.release_date, album.total_tracks
                    ),
                    12.0,
                ));
            });
        });
    });
    action
}
