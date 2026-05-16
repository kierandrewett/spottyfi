//! The Home page.
//!
//! Phase 5 replaces the Phase 4 placeholder with a simple real-data home: a
//! "Jump back in" shelf of the user's first playlists and a shelf of their
//! recently-saved albums. The rich, recommendation-driven Home is Phase 7+.

use std::sync::Arc;

use spottyfi_api::ApiError;
use spottyfi_models::{Album, SimplifiedPlaylist, SpotifyId as _};
use spottyfi_ui::components;

use super::{load_error, loading_spinner, Loadable, Page, PageAction, PageContext, PageServices};
use crate::shell::Tab;

/// How many items each Home shelf shows.
const SHELF_LEN: usize = 6;

/// The data the Home page loads.
type Loaded = Result<HomeData, ApiError>;

/// The first slices of the user's playlists and saved albums.
struct HomeData {
    /// The first few playlists.
    playlists: Vec<SimplifiedPlaylist>,
    /// The first few saved albums.
    albums: Vec<Album>,
}

/// The Home tab.
pub struct HomePage {
    /// The async load of the home shelves.
    data: Loadable<Loaded>,
}

impl HomePage {
    /// Build the page and kick off the home load.
    #[must_use]
    pub fn new(services: &PageServices) -> Self {
        Self {
            data: spawn_load(services),
        }
    }
}

/// Spawn the home load: one page each of playlists and saved albums.
fn spawn_load(services: &PageServices) -> Loadable<Loaded> {
    let api = Arc::clone(&services.api);
    Loadable::spawn(&services.runtime, &services.ctx, async move {
        let playlists = api.user_playlists(0, SHELF_LEN as u32).await?.items;
        let albums = api.saved_albums(0, SHELF_LEN as u32).await?.items;
        Ok(HomeData { playlists, albums })
    })
}

impl Page for HomePage {
    fn title(&self) -> String {
        "Home".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;
        let Some(loaded) = self.data.value() else {
            loading_spinner(ui, &palette, "Loading your home…");
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
                    egui::RichText::new("Good day")
                        .family(spottyfi_ui::fonts::semibold())
                        .size(28.0)
                        .color(palette.text),
                );
                ui.add_space(16.0);

                if !data.playlists.is_empty() {
                    components::section_header(ui, &palette, "Jump back in");
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for playlist in &data.playlists {
                            let art = playlist.images.first().map(|i| i.url.as_str());
                            if card(ui, &palette, &playlist.name, art) {
                                action = Some(PageAction::Open(Tab::Playlist(
                                    playlist.id.id().to_owned(),
                                )));
                            }
                        }
                    });
                    ui.add_space(16.0);
                }

                if !data.albums.is_empty() {
                    components::section_header(ui, &palette, "Recently saved");
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for album in &data.albums {
                            let art = album.images.first().map(|i| i.url.as_str());
                            if card(ui, &palette, &album.name, art) {
                                action = Some(PageAction::Open(Tab::Album(
                                    album.id.id().to_owned(),
                                )));
                            }
                        }
                    });
                }

                if data.playlists.is_empty() && data.albums.is_empty() {
                    ui.label(components::muted(
                        &palette,
                        "Save some playlists and albums to see them here.",
                        13.0,
                    ));
                }
            });
        action
    }
}

/// A clickable Home shelf card. Returns `true` when it was clicked.
fn card(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    title: &str,
    art: Option<&str>,
) -> bool {
    let size = egui::vec2(150.0, 190.0);
    let frame = egui::Frame::new()
        .fill(palette.card)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(10));
    let response = frame
        .show(ui, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.vertical(|ui| {
                components::album_art(ui, palette, art, 128.0, 6.0);
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
            });
        })
        .response
        .interact(egui::Sense::click());
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}
