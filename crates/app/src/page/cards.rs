//! Shared card and list widgets for the Browse-family pages.
//!
//! `BrowsePage`, `CategoryPage`, `ChartsPage`, `NewReleasesPage` and
//! `MadeForYouPage` all render the same handful of building blocks — a
//! clickable artist / album / track card, a track list, a calm note. They live
//! here so the pages stay thin and consistent.

use spottyfi_models::{Artist, SimplifiedAlbum, SimplifiedArtist, SpotifyId as _, Track};
use spottyfi_ui::components;
use spottyfi_ui::theme::Palette;
use spottyfi_ui::track_table::{self, TrackColumns, TrackRow, TrackTableState};

use super::track_view::{self, Entry, PlayContext};
use super::PageAction;
use crate::shell::Tab;

/// The fixed footprint of a Browse card.
const CARD_SIZE: egui::Vec2 = egui::vec2(150.0, 210.0);
/// The cover-art edge inside a card.
const CARD_ART: f32 = 128.0;

/// A wrapping grid of artist cards; clicking one opens its artist page.
pub fn artist_grid(ui: &mut egui::Ui, palette: &Palette, artists: &[Artist]) -> Option<PageAction> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        for artist in artists {
            let art = artist.images.first().map(|i| i.url.as_str());
            if card(ui, palette, &artist.name, "Artist", art) {
                action = Some(PageAction::Open(Tab::Artist(artist.id.id().to_owned())));
            }
        }
    });
    action
}

/// A wrapping grid of album cards; clicking one opens its album page.
pub fn album_grid(
    ui: &mut egui::Ui,
    palette: &Palette,
    albums: &[SimplifiedAlbum],
) -> Option<PageAction> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        for album in albums {
            let art = album.images.first().map(|i| i.url.as_str());
            let subtitle = artist_names(&album.artists);
            if card(ui, palette, &album.name, &subtitle, art) {
                if let Some(id) = &album.id {
                    action = Some(PageAction::Open(Tab::Album(id.id().to_owned())));
                }
            }
        }
    });
    action
}

/// A track list rendered with the shared track-table widget.
///
/// Double-click plays the whole list (so Next/Prev walk it) from that track;
/// the row context menu queues / navigates / copies. The list has no inherent
/// user-sortable order, so header sorts are ignored. `context` names the
/// playback context the list belongs to (a chart, a category, …).
pub fn track_list(
    ui: &mut egui::Ui,
    palette: &Palette,
    tracks: &[Track],
    playing_uri: Option<&str>,
    context: &PlayContext,
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
    if matches!(table_action, track_table::TrackAction::Sort(_)) {
        return None;
    }
    track_view::resolve_action(table_action, &entries, context)
}

/// Whether `track` is the one currently playing.
fn is_playing(track: &Track, playing_uri: Option<&str>) -> bool {
    match (track.id.as_ref(), playing_uri) {
        (Some(id), Some(uri)) => id.uri() == uri,
        _ => false,
    }
}

/// A generic clickable card: cover art, a title and a muted subtitle.
/// Returns `true` when clicked.
pub fn card(
    ui: &mut egui::Ui,
    palette: &Palette,
    title: &str,
    subtitle: &str,
    art: Option<&str>,
) -> bool {
    let frame = egui::Frame::new()
        .fill(palette.card)
        .corner_radius(0)
        .inner_margin(egui::Margin::same(10));
    let response = frame
        .show(ui, |ui| {
            ui.set_min_size(CARD_SIZE);
            ui.set_max_size(CARD_SIZE);
            ui.vertical(|ui| {
                components::album_art(ui, palette, art, CARD_ART, 0.0);
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

/// A calm, centred note — used when a Last.fm-backed section is unavailable
/// because no API key is configured, or when an endpoint is deprecated.
pub fn calm_note(ui: &mut egui::Ui, palette: &Palette, icon: spottyfi_ui::Icon, message: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(18.0);
        spottyfi_ui::icons::icon(ui, icon, 32.0, palette.text_muted);
        ui.add_space(8.0);
        ui.add(egui::Label::new(components::muted(palette, message, 12.5)).wrap());
        ui.add_space(18.0);
    });
}

/// Join a list of simplified artists into a display string.
pub fn artist_names(artists: &[SimplifiedArtist]) -> String {
    artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
