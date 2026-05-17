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
use crate::icons::Icon;
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

    // Columns sit flush against each other: the default item spacing left an
    // 8px dark gap between cells, which chopped the hovered / now-playing row
    // highlight into disconnected blocks. With zero gap each row reads as one
    // continuous band.
    ui.spacing_mut().item_spacing.x = 0.0;

    // A faint header-row background spanning the table width.
    let header_height = 24.0;

    // Full-row hover highlight. `egui_extras::TableRow::response()` may only be
    // called after a column has been added, so the hovered row is read from the
    // previous frame (kept in egui memory) and the new one recorded after the
    // row's cells — a one-frame lag that is imperceptible.
    let table_id = ui.id().with("track_table_row_hover");
    let prev_hover: Option<usize> = ui.data_mut(|d| d.get_temp(table_id)).flatten();
    let mut new_hover: Option<usize> = None;

    let mut builder = TableBuilder::new(ui)
        .striped(false)
        .resizable(false)
        // The table never owns a scroll viewport: every caller already nests it
        // inside a page-level `ScrollArea`. A `TableBuilder` defaults to wrapping
        // its body in its own vertical `ScrollArea`, and nesting two vertical
        // scroll areas causes the scroll jitter / size-glitching the maintainer
        // saw. With `vscroll(false)` the table lays out at full content height
        // and the single outer `ScrollArea` scrolls the whole page.
        .vscroll(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .sense(egui::Sense::click())
        .column(Column::exact(40.0)) // #
        .column(Column::remainder().at_least(220.0)); // Title
    if columns.album {
        builder = builder.column(Column::remainder().at_least(140.0));
    }
    if columns.date_added {
        builder = builder.column(Column::exact(120.0));
    }
    builder = builder.column(Column::exact(60.0)); // Duration

    builder
        .header(header_height, |mut header| {
            header.col(|ui| {
                paint_header_bg(ui, palette);
                sort_header(ui, palette, state, SortColumn::Index, "#", &mut action);
            });
            header.col(|ui| {
                paint_header_bg(ui, palette);
                sort_header(ui, palette, state, SortColumn::Title, "TITLE", &mut action);
            });
            if columns.album {
                header.col(|ui| {
                    paint_header_bg(ui, palette);
                    column_rule(ui, palette);
                    sort_header(ui, palette, state, SortColumn::Album, "ALBUM", &mut action);
                });
            }
            if columns.date_added {
                header.col(|ui| {
                    paint_header_bg(ui, palette);
                    column_rule(ui, palette);
                    sort_header(
                        ui,
                        palette,
                        state,
                        SortColumn::DateAdded,
                        "DATE ADDED",
                        &mut action,
                    );
                });
            }
            header.col(|ui| {
                paint_header_bg(ui, palette);
                column_rule(ui, palette);
                sort_header(
                    ui,
                    palette,
                    state,
                    SortColumn::Duration,
                    "TIME",
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
                let hovered = prev_hover == Some(index);
                if let Some(a) = render_row(&mut row, palette, columns, track_row, hovered) {
                    action = Some(a);
                }
                if row.response().hovered() {
                    new_hover = Some(index);
                }
            });
        });

    ui.data_mut(|d| d.insert_temp(table_id, new_hover));

    action
}

/// Paint the faint header-row background behind a header cell.
fn paint_header_bg(ui: &mut egui::Ui, palette: &Palette) {
    let rect = ui.max_rect().expand2(egui::vec2(0.0, 2.0));
    ui.painter().rect_filled(rect, 0, palette.card);
}

/// Paint a row cell's background, if any — a flat, sharp, full-bleed fill.
///
/// Called at the start of every cell so a hovered / now-playing row reads as
/// one continuous band across every column.
fn paint_cell_bg(ui: &mut egui::Ui, fill: Option<egui::Color32>) {
    if let Some(fill) = fill {
        let rect = ui.max_rect().expand2(egui::vec2(0.0, 1.0));
        ui.painter().rect_filled(rect, 0, fill);
    }
}

/// The faint Spotify-green wash painted behind the currently-playing row.
fn now_playing_bg(palette: &Palette) -> egui::Color32 {
    // A low-alpha accent tint — present enough to read clearly as "playing"
    // and unmistakably green, subtle enough not to fight the green title.
    let a = palette.accent;
    egui::Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), 42)
}

/// Draw a thin vertical separator at the left edge of a column.
fn column_rule(ui: &mut egui::Ui, palette: &Palette) {
    let rect = ui.max_rect();
    ui.painter().vline(
        rect.left(),
        rect.y_range().expand(2.0),
        egui::Stroke::new(1.0, palette.outline),
    );
}

/// Draw one clickable, sortable column header — uppercase, dimmed, with a
/// sort caret on the active column.
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
            .size(10.5)
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
    hovered: bool,
) -> Option<TrackAction> {
    let mut action: Option<TrackAction> = None;
    let track = track_row.track;

    // The row background: a faint green wash for the now-playing row, a subtle
    // lighter wash on hover (`hovered` is the previous frame's state — see the
    // note in `track_table`), nothing otherwise.
    let row_bg = if track_row.is_playing {
        Some(now_playing_bg(palette))
    } else if hovered {
        Some(palette.hover)
    } else {
        None
    };

    // "#" — a speaker icon marks the currently-playing row; otherwise the
    // position number with a small music-note glyph.
    row.col(|ui| {
        paint_cell_bg(ui, row_bg);
        if track_row.is_playing {
            ui.add_space(2.0);
            crate::icons::icon(ui, Icon::Volume, 13.0, palette.accent);
        } else {
            ui.add_space(2.0);
            crate::icons::icon(ui, Icon::Music, 11.0, palette.text_muted);
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(track_row.position.to_string())
                    .size(11.5)
                    .color(palette.text_muted),
            );
        }
    });

    // Title — album-art thumbnail + track name over the artist line.
    row.col(|ui| {
        paint_cell_bg(ui, row_bg);
        components::album_art(ui, palette, primary_image(track), 34.0, 0.0);
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
            // The artist line: each artist with an id is a clickable link that
            // navigates to that artist's page.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (i, artist) in track.artists.iter().enumerate() {
                    if i > 0 {
                        ui.label(components::muted(palette, ", ", 11.5));
                    }
                    match &artist.id {
                        Some(id) => {
                            if link_label(ui, palette, &artist.name, 11.5) {
                                action = Some(TrackAction::GoToArtist(id.id().to_owned()));
                            }
                        }
                        None => {
                            ui.add(
                                egui::Label::new(components::muted(palette, &artist.name, 11.5))
                                    .truncate(),
                            );
                        }
                    }
                }
            });
        });
    });

    // Album — a clickable link to the album page when it carries an id.
    if columns.album {
        row.col(|ui| {
            paint_cell_bg(ui, row_bg);
            column_rule(ui, palette);
            match &track.album.id {
                Some(id) if !track.album.name.is_empty() => {
                    if link_label(ui, palette, &track.album.name, 12.0) {
                        action = Some(TrackAction::GoToAlbum(id.id().to_owned()));
                    }
                }
                _ => {
                    ui.add(
                        egui::Label::new(components::muted(
                            palette,
                            track.album.name.clone(),
                            12.0,
                        ))
                        .truncate(),
                    );
                }
            }
        });
    }

    // Date added.
    if columns.date_added {
        row.col(|ui| {
            paint_cell_bg(ui, row_bg);
            column_rule(ui, palette);
            let text = track_row.date_added.map(format_date).unwrap_or_default();
            ui.label(components::muted(palette, text, 11.5));
        });
    }

    // Duration — right-aligned so the m:ss values line up on their last digit.
    row.col(|ui| {
        paint_cell_bg(ui, row_bg);
        column_rule(ui, palette);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            ui.label(components::muted(
                palette,
                format_duration(track.duration_ms),
                11.5,
            ));
        });
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

/// Draw an inline, clickable link inside a track-table cell.
///
/// Renders as muted secondary text; on hover it brightens to the primary text
/// colour and gains an underline, with a pointing-hand cursor — the affordance
/// the maintainer asked for on artist and album names. Returns `true` when the
/// link was clicked this frame.
fn link_label(ui: &mut egui::Ui, palette: &Palette, text: &str, size: f32) -> bool {
    let response = ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .size(size)
                .color(palette.text_muted),
        )
        .sense(egui::Sense::click())
        .truncate(),
    );
    if response.hovered() {
        // Repaint the label brighter, underlined, over the muted original.
        let rect = response.rect;
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(size),
            palette.text,
        );
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 1.0,
            egui::Stroke::new(1.0, palette.text),
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
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
