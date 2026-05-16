//! A reusable track-table widget shared by the playlist, album and liked-songs
//! pages.
//!
//! The widget is a thin, stateless renderer over [`egui_extras::TableBuilder`].
//! It owns no data: the caller passes the rows to draw plus a [`TrackTableState`]
//! that records the active sort column. Every user interaction (a double-click,
//! a context-menu choice, a header sort) is returned to the caller as a
//! [`TrackAction`]; the widget never mutates application state.
//!
//! The caller is responsible for *applying* the sort — the widget only reports
//! which column the user clicked. This keeps the comparison logic (which
//! depends on the page's row type) out of `ui`.

use egui_extras::{Column, TableBuilder};
use spottyfi_models::{SpotifyId as _, Track};

use crate::components;
use crate::theme::Palette;

/// Which column a track table is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SortColumn {
    /// The original, unsorted index (the "#" column).
    #[default]
    Index,
    /// The track title.
    Title,
    /// The album name.
    Album,
    /// The date the track was added (playlist / liked pages only).
    DateAdded,
    /// The track duration.
    Duration,
}

/// The sort state of a track table, persisted per page across frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct TrackTableState {
    /// The column the table is currently sorted by.
    pub column: SortColumn,
    /// Whether the sort is descending (`false` is ascending).
    pub descending: bool,
}

impl TrackTableState {
    /// Register a click on `column`'s header.
    ///
    /// Clicking the active column flips the direction; clicking a new column
    /// selects it in ascending order.
    pub fn toggle(&mut self, column: SortColumn) {
        if self.column == column {
            self.descending = !self.descending;
        } else {
            self.column = column;
            self.descending = false;
        }
    }
}

/// Which columns a track table should show.
///
/// Album pages drop the album column (every row shares the album) and the
/// date-added column (an album has no per-track add date).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackColumns {
    /// Show the album column.
    pub album: bool,
    /// Show the date-added column.
    pub date_added: bool,
}

impl TrackColumns {
    /// The full column set used by playlist and liked-songs pages.
    #[must_use]
    pub fn full() -> Self {
        Self {
            album: true,
            date_added: true,
        }
    }

    /// The reduced column set used by album pages.
    #[must_use]
    pub fn album_page() -> Self {
        Self {
            album: false,
            date_added: false,
        }
    }
}

/// One row to render in a track table.
pub struct TrackRow<'a> {
    /// The track itself.
    pub track: &'a Track,
    /// The 1-based position shown in the "#" column.
    pub position: usize,
    /// The "Date added" cell text, if the page has one.
    pub date_added: Option<&'a str>,
    /// Whether this row is the track currently playing.
    pub is_playing: bool,
}

/// Something the user did to a track table this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackAction {
    /// A row was double-clicked — play this track (carries its `position`).
    Play(usize),
    /// "Play next" was chosen for the row at `position`.
    PlayNext(usize),
    /// "Add to queue" was chosen for the row at `position`.
    AddToQueue(usize),
    /// "Copy URI" was chosen for the row at `position`.
    CopyUri(usize),
    /// "Go to album" was chosen — navigate to this album id.
    GoToAlbum(String),
    /// "Go to artist" was chosen — navigate to this artist id.
    GoToArtist(String),
    /// A header was clicked — re-sort by this column.
    Sort(SortColumn),
}

/// Render a sortable track table. Returns the user's action this frame, if any.
///
/// `state` records the active sort column so headers can show a direction
/// arrow; the widget does not sort the rows itself (see the module docs).
pub fn track_table(
    ui: &mut egui::Ui,
    palette: &Palette,
    state: TrackTableState,
    columns: TrackColumns,
    rows: &[TrackRow<'_>],
    row_height: f32,
) -> Option<TrackAction> {
    let mut action: Option<TrackAction> = None;

    let mut builder = TableBuilder::new(ui)
        .striped(false)
        .resizable(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .sense(egui::Sense::click())
        .column(Column::exact(34.0)) // #
        .column(Column::remainder().at_least(220.0)); // Title
    if columns.album {
        builder = builder.column(Column::remainder().at_least(140.0));
    }
    if columns.date_added {
        builder = builder.column(Column::exact(120.0));
    }
    builder = builder.column(Column::exact(64.0)); // Duration

    builder
        .header(26.0, |mut header| {
            header.col(|ui| {
                sort_header(ui, palette, state, SortColumn::Index, "#", &mut action);
            });
            header.col(|ui| {
                sort_header(ui, palette, state, SortColumn::Title, "Title", &mut action);
            });
            if columns.album {
                header.col(|ui| {
                    sort_header(ui, palette, state, SortColumn::Album, "Album", &mut action);
                });
            }
            if columns.date_added {
                header.col(|ui| {
                    sort_header(
                        ui,
                        palette,
                        state,
                        SortColumn::DateAdded,
                        "Date added",
                        &mut action,
                    );
                });
            }
            header.col(|ui| {
                sort_header(
                    ui,
                    palette,
                    state,
                    SortColumn::Duration,
                    "Time",
                    &mut action,
                );
            });
        })
        .body(|body| {
            body.rows(row_height, rows.len(), |mut row| {
                let index = row.index();
                let Some(track_row) = rows.get(index) else {
                    return;
                };
                if let Some(a) = render_row(&mut row, palette, columns, track_row) {
                    action = Some(a);
                }
            });
        });

    action
}

/// Draw one clickable, sortable column header.
fn sort_header(
    ui: &mut egui::Ui,
    palette: &Palette,
    state: TrackTableState,
    column: SortColumn,
    label: &str,
    action: &mut Option<TrackAction>,
) {
    let active = state.column == column;
    let text = if active {
        let arrow = if state.descending {
            " \u{25be}"
        } else {
            " \u{25b4}"
        };
        format!("{label}{arrow}")
    } else {
        label.to_owned()
    };
    let color = if active {
        palette.text
    } else {
        palette.text_muted
    };
    let button = egui::Button::new(
        egui::RichText::new(text)
            .family(crate::fonts::medium())
            .size(11.5)
            .color(color),
    )
    .frame(false);
    if ui
        .add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        *action = Some(TrackAction::Sort(column));
    }
}

/// Render one track row across every visible column, wiring up the
/// double-click-to-play and right-click context-menu interactions.
fn render_row(
    row: &mut egui_extras::TableRow<'_, '_>,
    palette: &Palette,
    columns: TrackColumns,
    track_row: &TrackRow<'_>,
) -> Option<TrackAction> {
    let mut action: Option<TrackAction> = None;
    let track = track_row.track;

    // "#" — the play indicator replaces the number for the active track.
    row.col(|ui| {
        let text = if track_row.is_playing {
            egui::RichText::new("\u{25b6}")
                .size(11.0)
                .color(palette.accent)
        } else {
            egui::RichText::new(track_row.position.to_string())
                .size(12.0)
                .color(palette.text_muted)
        };
        ui.label(text);
    });

    // Title — album-art thumbnail + track name over the artist line.
    row.col(|ui| {
        components::album_art(ui, palette, primary_image(track), 36.0, 4.0);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            let name_color = if track_row.is_playing {
                palette.accent
            } else {
                palette.text
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&track.name)
                        .family(crate::fonts::medium())
                        .size(13.0)
                        .color(name_color),
                )
                .truncate(),
            );
            ui.add(
                egui::Label::new(components::muted(palette, artist_line(track), 11.5)).truncate(),
            );
        });
    });

    // Album.
    if columns.album {
        row.col(|ui| {
            ui.add(
                egui::Label::new(components::muted(palette, track.album.name.clone(), 12.0))
                    .truncate(),
            );
        });
    }

    // Date added.
    if columns.date_added {
        row.col(|ui| {
            let text = track_row.date_added.map(format_date).unwrap_or_default();
            ui.label(components::muted(palette, text, 11.5));
        });
    }

    // Duration.
    row.col(|ui| {
        ui.label(components::muted(
            palette,
            format_duration(track.duration_ms),
            11.5,
        ));
    });

    // Whole-row interactions: double-click plays, right-click opens the menu.
    let response = row.response();
    if response.double_clicked() {
        action = Some(TrackAction::Play(track_row.position));
    }
    response.context_menu(|ui| {
        if let Some(a) = context_menu(ui, track_row) {
            action = Some(a);
        }
    });

    action
}

/// The right-click context menu for a track row.
fn context_menu(ui: &mut egui::Ui, track_row: &TrackRow<'_>) -> Option<TrackAction> {
    let mut action = None;
    let track = track_row.track;
    ui.set_min_width(170.0);

    if ui.button("Play").clicked() {
        action = Some(TrackAction::Play(track_row.position));
        ui.close();
    }
    // "Play next" / "Add to queue" need the Phase 8 queue; offered but inert.
    if ui.button("Play next").clicked() {
        action = Some(TrackAction::PlayNext(track_row.position));
        ui.close();
    }
    if ui.button("Add to queue").clicked() {
        action = Some(TrackAction::AddToQueue(track_row.position));
        ui.close();
    }
    ui.separator();
    if ui
        .add_enabled(track.album.id.is_some(), egui::Button::new("Go to album"))
        .clicked()
    {
        if let Some(id) = &track.album.id {
            action = Some(TrackAction::GoToAlbum(id.id().to_owned()));
        }
        ui.close();
    }
    let first_artist = track.artists.iter().find_map(|a| a.id.as_ref());
    if ui
        .add_enabled(first_artist.is_some(), egui::Button::new("Go to artist"))
        .clicked()
    {
        if let Some(id) = first_artist {
            action = Some(TrackAction::GoToArtist(id.id().to_owned()));
        }
        ui.close();
    }
    ui.separator();
    if ui.button("Copy URI").clicked() {
        action = Some(TrackAction::CopyUri(track_row.position));
        ui.close();
    }

    action
}

/// The largest-available album-art URL for a track, if any.
fn primary_image(track: &Track) -> Option<&str> {
    track.album.images.first().map(|image| image.url.as_str())
}

/// The track's artists joined into a display string.
fn artist_line(track: &Track) -> String {
    track
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Format a track duration (milliseconds) as `m:ss`.
#[must_use]
pub fn format_duration(duration_ms: u32) -> String {
    let total = duration_ms / 1000;
    format!("{}:{:02}", total / 60, total % 60)
}

/// Format an RFC 3339 added-at timestamp down to just its `YYYY-MM-DD` date.
fn format_date(added_at: &str) -> String {
    added_at.split('T').next().unwrap_or(added_at).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_the_same_column_flips_direction() {
        let mut state = TrackTableState::default();
        state.toggle(SortColumn::Title);
        assert_eq!(state.column, SortColumn::Title);
        assert!(!state.descending);
        state.toggle(SortColumn::Title);
        assert!(state.descending);
    }

    #[test]
    fn toggling_a_new_column_resets_to_ascending() {
        let mut state = TrackTableState {
            column: SortColumn::Title,
            descending: true,
        };
        state.toggle(SortColumn::Album);
        assert_eq!(state.column, SortColumn::Album);
        assert!(!state.descending);
    }

    #[test]
    fn album_columns_drop_album_and_date() {
        let cols = TrackColumns::album_page();
        assert!(!cols.album);
        assert!(!cols.date_added);
        let full = TrackColumns::full();
        assert!(full.album && full.date_added);
    }

    #[test]
    fn duration_formats_as_minutes_seconds() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(65_000), "1:05");
        assert_eq!(format_duration(605_000), "10:05");
    }

    #[test]
    fn date_trims_to_the_calendar_day() {
        assert_eq!(format_date("2024-03-15T09:30:00Z"), "2024-03-15");
        assert_eq!(format_date("2024-03-15"), "2024-03-15");
    }
}
