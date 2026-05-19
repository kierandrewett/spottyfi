//! The Your Artists page: the artists the user follows.
//!
//! A cursor-paginated `GET /me/following?type=artist`, served with
//! stale-while-revalidate caching by the `api` crate, rendered as an artist
//! grid.

use std::sync::Arc;

use spottyfi_api::ApiError;
use spottyfi_models::Artist;
use spottyfi_ui::components;

use super::cards;
use super::{
    load_cancelled, loading_spinner, LoadState, Loadable, Page, PageAction, PageContext,
    PageServices,
};

/// Upper bound on followed artists fetched — generous enough to be "all".
const LIMIT: u32 = 2000;

/// The Your Artists tab.
pub struct FollowedArtistsPage {
    /// The async load of the followed-artists list.
    data: Loadable<Result<Vec<Artist>, ApiError>>,
}

impl FollowedArtistsPage {
    /// Build the page, kicking off the load.
    #[must_use]
    pub fn new(services: &PageServices) -> Self {
        let api = Arc::clone(&services.api);
        let data = Loadable::spawn_tracked(
            &services.runtime,
            &services.ctx,
            &services.activity,
            "Loading your artists…",
            async move { api.followed_artists(LIMIT).await },
        );
        Self { data }
    }
}

impl Page for FollowedArtistsPage {
    fn title(&self) -> String {
        "Your Artists".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;

        let loaded = match self.data.state() {
            LoadState::Ready(loaded) => loaded,
            LoadState::Pending => {
                loading_spinner(ui, &palette, "Loading your artists…");
                return None;
            }
            LoadState::Cancelled => {
                load_cancelled(ui, &palette, "Loading your artists was cancelled.");
                return None;
            }
        };

        let mut action = None;
        // The page header is fixed; only the content below scrolls.
        ui.label(
            egui::RichText::new("Your Artists")
                .family(spottyfi_ui::fonts::semibold())
                .size(28.0)
                .color(palette.text),
        );
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match loaded {
                Err(err) => {
                    ui.add_space(16.0);
                    cards::calm_note(
                        ui,
                        &palette,
                        spottyfi_ui::Icon::User,
                        &format!("Couldn't load your artists: {err}"),
                    );
                }
                Ok(artists) if artists.is_empty() => {
                    ui.label(components::muted(
                        &palette,
                        "You're not following any artists yet.",
                        12.0,
                    ));
                }
                Ok(artists) => {
                    ui.label(components::muted(
                        &palette,
                        format!("{} artists you follow.", artists.len()),
                        12.0,
                    ));
                    ui.add_space(16.0);
                    if let Some(a) = cards::artist_grid(ui, &palette, artists) {
                        action = Some(a);
                    }
                }
            });
        action
    }
}
