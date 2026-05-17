//! The New Releases page: Spotify's newly-released albums.
//!
//! Uses Spotify's `GET /browse/new-releases`. `rspotify` marks that endpoint
//! deprecated and its status for newly-registered apps is uncertain — when it
//! comes back [`ApiError::EndpointUnavailable`](spottyfi_api::ApiError) the
//! page shows a clean explanatory note instead of crashing.

use std::sync::Arc;

use spottyfi_api::ApiError;
use spottyfi_models::SimplifiedAlbum;
use spottyfi_ui::components;

use super::cards;
use super::{loading_spinner, Loadable, Page, PageAction, PageContext, PageServices};

/// How many new releases to fetch.
const RELEASES_LEN: u32 = 30;

/// The data the New Releases page loads.
type Loaded = Result<Vec<SimplifiedAlbum>, ApiError>;

/// The New Releases tab.
pub struct NewReleasesPage {
    /// The async load of the new-releases list.
    data: Loadable<Loaded>,
}

impl NewReleasesPage {
    /// Build the page and kick off the load.
    #[must_use]
    pub fn new(services: &PageServices) -> Self {
        let api = Arc::clone(&services.api);
        let data = Loadable::spawn_tracked(
            &services.runtime,
            &services.ctx,
            &services.activity,
            "Loading new releases…",
            async move { api.new_releases(RELEASES_LEN).await },
        );
        Self { data }
    }
}

impl Page for NewReleasesPage {
    fn title(&self) -> String {
        "New Releases".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;
        let Some(loaded) = self.data.value() else {
            loading_spinner(ui, &palette, "Loading new releases…");
            return None;
        };

        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("New Releases")
                        .family(spottyfi_ui::fonts::semibold())
                        .size(28.0)
                        .color(palette.text),
                );
                ui.add_space(16.0);

                match loaded {
                    // The endpoint is dead for this app — a clean note, no crash.
                    Err(ApiError::EndpointUnavailable { .. }) => {
                        cards::calm_note(
                            ui,
                            &palette,
                            spottyfi_ui::Icon::NewReleases,
                            "Spotify's New Releases feed isn't available to this app. \
                             Try Browse or Charts for fresh music.",
                        );
                    }
                    Err(err) => {
                        cards::calm_note(
                            ui,
                            &palette,
                            spottyfi_ui::Icon::NewReleases,
                            &format!("Couldn't load new releases: {err}"),
                        );
                    }
                    Ok(albums) if albums.is_empty() => {
                        ui.label(components::muted(
                            &palette,
                            "No new releases right now.",
                            13.0,
                        ));
                    }
                    Ok(albums) => {
                        if let Some(a) = cards::album_grid(ui, &palette, albums) {
                            action = Some(a);
                        }
                    }
                }
            });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::Page as PageTrait;
    use spottyfi_api::MockSpotifyApi;

    fn services(mock: MockSpotifyApi) -> (PageServices, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build runtime");
        let services = PageServices {
            api: Arc::new(mock),
            lastfm: None,
            lyrics: Default::default(),
            runtime: runtime.handle().clone(),
            ctx: egui::Context::default(),
            activity: spottyfi_state::ActivityRegistry::new(),
        };
        (services, runtime)
    }

    #[test]
    fn title_is_static() {
        let mut mock = MockSpotifyApi::new();
        mock.expect_new_releases().returning(|_| Ok(Vec::new()));
        let (services, _rt) = services(mock);
        assert_eq!(
            PageTrait::title(&NewReleasesPage::new(&services)),
            "New Releases"
        );
    }

    #[test]
    fn deprecated_endpoint_resolves_without_panicking() {
        let mut mock = MockSpotifyApi::new();
        mock.expect_new_releases().returning(|_| {
            Err(ApiError::EndpointUnavailable {
                endpoint: "new_releases",
            })
        });
        let (services, _rt) = services(mock);
        let page = NewReleasesPage::new(&services);
        for _ in 0..200 {
            if page.data.value().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let loaded = page.data.value().expect("load resolved");
        assert!(matches!(loaded, Err(ApiError::EndpointUnavailable { .. })));
    }
}
