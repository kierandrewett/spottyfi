//! The Library page: a grid of the user's playlists and saved albums.

use std::sync::Arc;

use spottyfi_api::{ApiError, SpotifyApi};
use spottyfi_models::{Album, SimplifiedPlaylist, SpotifyId as _};
use spottyfi_ui::components;

use super::{load_error, loading_spinner, Loadable, Page, PageAction, PageContext, PageServices};
use crate::shell::Tab;

/// The data the Library page loads.
type Loaded = Result<LibraryData, ApiError>;

/// The user's playlists and saved albums.
struct LibraryData {
    /// Every playlist the user owns or follows.
    playlists: Vec<SimplifiedPlaylist>,
    /// Every album the user has saved.
    albums: Vec<Album>,
}

/// The Library tab.
pub struct LibraryPage {
    /// The async load of the playlists and albums.
    data: Loadable<Loaded>,
}

impl LibraryPage {
    /// Build the page and kick off the library load.
    #[must_use]
    pub fn new(services: &PageServices) -> Self {
        Self {
            data: spawn_load(services),
        }
    }
}

/// Spawn the library load: the user's playlists and saved albums.
fn spawn_load(services: &PageServices) -> Loadable<Loaded> {
    let api = Arc::clone(&services.api);
    Loadable::spawn(&services.runtime, &services.ctx, async move {
        let mut playlists = Vec::new();
        let mut offset = 0u32;
        loop {
            let page = api.user_playlists(offset, 50).await?;
            let count = page.items.len() as u32;
            playlists.extend(page.items);
            if !page.has_next || count == 0 {
                break;
            }
            offset += count;
        }
        let albums = api.saved_albums(0, 50).await?.items;
        Ok(LibraryData { playlists, albums })
    })
}

impl Page for LibraryPage {
    fn title(&self) -> String {
        "Your Library".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;
        let Some(loaded) = self.data.value() else {
            loading_spinner(ui, &palette, "Loading your library…");
            return None;
        };
        let data = match loaded {
            Ok(data) => data,
            Err(err) => {
                load_error(ui, &palette, &err.to_string());
                return None;
            }
        };

        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Your Library")
                        .family(spottyfi_ui::fonts::semibold())
                        .size(26.0)
                        .color(palette.text),
                );
                ui.add_space(12.0);

                components::section_header(ui, &palette, "Playlists");
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for playlist in &data.playlists {
                        if let Some(a) = playlist_card(ui, &palette, playlist) {
                            action = Some(a);
                        }
                    }
                });

                ui.add_space(16.0);
                components::section_header(ui, &palette, "Saved albums");
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

/// A clickable playlist card.
fn playlist_card(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    playlist: &SimplifiedPlaylist,
) -> Option<PageAction> {
    let art = playlist.images.first().map(|i| i.url.as_str());
    let subtitle = format!("{} tracks", playlist.total_tracks);
    card(ui, palette, &playlist.name, &subtitle, art)
        .then(|| PageAction::Open(Tab::Playlist(playlist.id.id().to_owned())))
}

/// A clickable saved-album card.
fn album_card(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    album: &Album,
) -> Option<PageAction> {
    let art = album.images.first().map(|i| i.url.as_str());
    let subtitle = album
        .artists
        .first()
        .map(|a| a.name.clone())
        .unwrap_or_default();
    card(ui, palette, &album.name, &subtitle, art)
        .then(|| PageAction::Open(Tab::Album(album.id.id().to_owned())))
}

/// Draw a generic content card. Returns `true` when it was clicked.
fn card(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    title: &str,
    subtitle: &str,
    art: Option<&str>,
) -> bool {
    let size = egui::vec2(160.0, 212.0);
    let frame = egui::Frame::new()
        .fill(palette.card)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(10));
    let response = frame
        .show(ui, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.vertical(|ui| {
                components::album_art(ui, palette, art, 138.0, 6.0);
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
                ui.add(egui::Label::new(components::muted(palette, subtitle, 11.0)).truncate());
            });
        })
        .response
        .interact(egui::Sense::click());
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}
