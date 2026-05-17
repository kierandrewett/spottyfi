//! The Charts page: Last.fm's global top tracks and artists.
//!
//! Spotify's own charts live behind editorial playlists that are dead for new
//! apps (see `docs/questions.md` #7), so Charts is sourced from Last.fm's
//! `chart.getTopTracks` / `chart.getTopArtists`. The Last.fm names are resolved
//! back to Spotify objects so every row is navigable and playable.
//!
//! With no `SPOTTYFI_LASTFM_API_KEY` configured the page shows a calm note
//! explaining how to enable it — it never errors or crashes.

use std::sync::Arc;

use spottyfi_api::lastfm::{LastfmClient, LastfmError, LastfmResolver};
use spottyfi_models::{Artist, Track};
use spottyfi_ui::components;

use super::cards;
use super::{
    load_cancelled, loading_spinner, LoadState, Loadable, Page, PageAction, PageContext,
    PageServices,
};

/// How many chart entries to fetch from Last.fm.
const CHART_LEN: u32 = 24;

/// The data the Charts page loads.
type Loaded = Result<ChartData, LastfmError>;

/// The resolved global charts.
struct ChartData {
    /// Top tracks, resolved to Spotify tracks.
    tracks: Vec<Track>,
    /// Top artists, resolved to Spotify artists.
    artists: Vec<Artist>,
}

/// The Charts tab.
pub struct ChartsPage {
    /// The async load — `None` until the page has a Last.fm client.
    data: Option<Loadable<Loaded>>,
}

impl ChartsPage {
    /// Build the page, kicking off the load when Last.fm is configured.
    #[must_use]
    pub fn new(services: &PageServices) -> Self {
        Self {
            data: services
                .lastfm
                .as_ref()
                .map(|lastfm| spawn_load(services, lastfm.clone())),
        }
    }
}

/// Spawn the chart load: top tracks + top artists, resolved to Spotify objects.
fn spawn_load(services: &PageServices, lastfm: LastfmClient) -> Loadable<Loaded> {
    let resolver = LastfmResolver::new(Arc::clone(&services.api));
    Loadable::spawn_tracked(
        &services.runtime,
        &services.ctx,
        &services.activity,
        "Loading charts…",
        async move {
            let lf_tracks = lastfm.chart_top_tracks(CHART_LEN).await?;
            let lf_artists = lastfm.chart_top_artists(CHART_LEN).await?;
            // A resolver failure is a Spotify-side error; surface it as a
            // network-flavoured Last.fm error so the page shows one note.
            let tracks = resolver
                .resolve_tracks(&lf_tracks)
                .await
                .map_err(|e| LastfmError::Network(e.to_string()))?;
            let artists = resolver
                .resolve_artists(&lf_artists)
                .await
                .map_err(|e| LastfmError::Network(e.to_string()))?;
            Ok(ChartData { tracks, artists })
        },
    )
}

impl Page for ChartsPage {
    fn title(&self) -> String {
        "Charts".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;

        let Some(data) = self.data.as_ref() else {
            return charts_unconfigured(ui, &palette);
        };
        let loaded = match data.state() {
            LoadState::Ready(loaded) => loaded,
            LoadState::Pending => {
                loading_spinner(ui, &palette, "Loading the charts…");
                return None;
            }
            LoadState::Cancelled => {
                load_cancelled(ui, &palette, "Loading the charts was cancelled.");
                return None;
            }
        };

        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Charts")
                        .family(spottyfi_ui::fonts::semibold())
                        .size(28.0)
                        .color(palette.text),
                );
                ui.label(components::muted(
                    &palette,
                    "Global top tracks and artists, via Last.fm.",
                    12.0,
                ));
                ui.add_space(16.0);

                match loaded {
                    Err(LastfmError::NotConfigured) => {
                        if let Some(a) = charts_unconfigured(ui, &palette) {
                            action = Some(a);
                        }
                    }
                    Err(err) => {
                        cards::calm_note(
                            ui,
                            &palette,
                            spottyfi_ui::Icon::Charts,
                            &format!("Couldn't load the charts: {err}"),
                        );
                    }
                    Ok(data) => {
                        let playing = ctx.playback.track.as_ref().map(|t| t.uri.as_str());
                        components::section_header(ui, &palette, "Top Tracks");
                        ui.add_space(4.0);
                        let context = super::track_view::PlayContext {
                            uri: "spottyfi:charts:top-tracks".to_owned(),
                            name: "Top Tracks".to_owned(),
                        };
                        if let Some(a) =
                            cards::track_list(ui, &palette, &data.tracks, playing, &context)
                        {
                            action = Some(a);
                        }
                        ui.add_space(20.0);
                        components::section_header(ui, &palette, "Top Artists");
                        ui.add_space(4.0);
                        if let Some(a) = cards::artist_grid(ui, &palette, &data.artists) {
                            action = Some(a);
                        }
                    }
                }
            });
        action
    }
}

/// The note shown when Last.fm is not configured.
fn charts_unconfigured(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
) -> Option<PageAction> {
    cards::calm_note(
        ui,
        palette,
        spottyfi_ui::Icon::Charts,
        "Set SPOTTYFI_LASTFM_API_KEY to enable charts & recommendations.",
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::Page as PageTrait;
    use spottyfi_api::MockSpotifyApi;

    fn services(lastfm: Option<LastfmClient>) -> (PageServices, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build runtime");
        let services = PageServices {
            api: Arc::new(MockSpotifyApi::new()),
            lastfm,
            lyrics: Default::default(),
            runtime: runtime.handle().clone(),
            ctx: egui::Context::default(),
            activity: spottyfi_state::ActivityRegistry::new(),
        };
        (services, runtime)
    }

    #[test]
    fn title_is_static() {
        let (services, _rt) = services(None);
        assert_eq!(PageTrait::title(&ChartsPage::new(&services)), "Charts");
    }

    #[test]
    fn no_lastfm_client_means_no_load() {
        let (services, _rt) = services(None);
        let page = ChartsPage::new(&services);
        assert!(page.data.is_none(), "no load without a Last.fm client");
    }
}
