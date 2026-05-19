//! The Recently Played page: the user's recent listening history.
//!
//! A single `GET /me/player/recently-played` call, served with
//! stale-while-revalidate caching by the `api` crate, rendered as a track
//! list. The same track can appear more than once — it is a play history,
//! not a deduplicated library.

use std::sync::Arc;

use spottyfi_api::ApiError;
use spottyfi_models::Track;
use spottyfi_ui::components;

use super::cards;
use super::track_view::PlayContext;
use super::{
    load_cancelled, loading_spinner, LoadState, Loadable, Page, PageAction, PageContext,
    PageServices,
};

/// How many recent tracks to request.
const LIMIT: u32 = 50;

/// The Recently Played tab.
pub struct RecentlyPlayedPage {
    /// The async load of the recent-tracks list.
    data: Loadable<Result<Vec<Track>, ApiError>>,
}

impl RecentlyPlayedPage {
    /// Build the page, kicking off the load.
    #[must_use]
    pub fn new(services: &PageServices) -> Self {
        let api = Arc::clone(&services.api);
        let data = Loadable::spawn_tracked(
            &services.runtime,
            &services.ctx,
            &services.activity,
            "Loading recently played…",
            async move { api.recently_played(LIMIT).await },
        );
        Self { data }
    }
}

impl Page for RecentlyPlayedPage {
    fn title(&self) -> String {
        "Recently Played".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;

        let loaded = match self.data.state() {
            LoadState::Ready(loaded) => loaded,
            LoadState::Pending => {
                loading_spinner(ui, &palette, "Loading recently played…");
                return None;
            }
            LoadState::Cancelled => {
                load_cancelled(ui, &palette, "Loading recently played was cancelled.");
                return None;
            }
        };

        let mut action = None;
        // The page header is fixed; only the content below scrolls.
        ui.label(
            egui::RichText::new("Recently Played")
                .family(spottyfi_ui::fonts::semibold())
                .size(28.0)
                .color(palette.text),
        );
        ui.label(components::muted(
            &palette,
            "The tracks you have played most recently.",
            12.0,
        ));
        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match loaded {
                Err(err) => cards::calm_note(
                    ui,
                    &palette,
                    spottyfi_ui::Icon::RecentlyPlayed,
                    &format!("Couldn't load recently played: {err}"),
                ),
                Ok(tracks) if tracks.is_empty() => {
                    ui.label(components::muted(
                        &palette,
                        "Nothing played recently — start listening and it'll show up here.",
                        13.0,
                    ));
                }
                Ok(tracks) => {
                    let playing = ctx.playback.track.as_ref().map(|t| t.uri.as_str());
                    let context = PlayContext {
                        uri: "spottyfi:recently-played".to_owned(),
                        name: "Recently Played".to_owned(),
                    };
                    if let Some(a) = cards::track_list(ui, &palette, tracks, playing, &context) {
                        action = Some(a);
                    }
                }
            });
        action
    }
}
