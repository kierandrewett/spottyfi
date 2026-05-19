//! The Browse page: the discovery surface.
//!
//! Two parts, top to bottom:
//!
//! - a **genre/category grid** from Spotify's `GET /browse/categories` — that
//!   endpoint still works for new apps (only "a Category's playlists" is dead).
//!   Each tile renders the category's art **rotated** for a dense, dynamic
//!   look, per `PLAN.md` Phase 7. Clicking a tile opens its [`CategoryPage`].
//! - **Charts shelves** from Last.fm — the global top tracks and top artists.
//!
//! Degradation: with no `SPOTTYFI_LASTFM_API_KEY` the category grid still
//! renders and the Charts section shows a calm "set the key" note. A failure
//! of the Spotify category grid likewise shows a note, never a crash.
//!
//! [`CategoryPage`]: super::CategoryPage

use std::sync::Arc;

use spottyfi_api::lastfm::{LastfmClient, LastfmError, LastfmResolver};
use spottyfi_api::ApiError;
use spottyfi_models::{Artist, Category, Track};
use spottyfi_ui::components;
use spottyfi_ui::theme::Palette;

use super::cards;
use super::{LoadState, Loadable, Page, PageAction, PageContext, PageServices};
use crate::shell::Tab;

/// How many categories to request from Spotify.
const CATEGORY_LEN: u32 = 40;
/// How many chart entries to pull for the Browse shelves.
const SHELF_LEN: u32 = 12;
/// The category tile's footprint.
const TILE_SIZE: egui::Vec2 = egui::vec2(168.0, 100.0);

/// The Spotify-sourced part of Browse: the category grid.
type CategoriesLoaded = Result<Vec<Category>, ApiError>;

/// The Last.fm-sourced part of Browse: the charts shelves.
type ChartsLoaded = Result<ChartShelves, LastfmError>;

/// The resolved Browse charts shelves.
struct ChartShelves {
    /// Top tracks, resolved to Spotify tracks.
    tracks: Vec<Track>,
    /// Top artists, resolved to Spotify artists.
    artists: Vec<Artist>,
}

/// The Browse tab.
pub struct BrowsePage {
    /// The Spotify category grid load.
    categories: Loadable<CategoriesLoaded>,
    /// The Last.fm charts load — `None` when Last.fm is not configured.
    charts: Option<Loadable<ChartsLoaded>>,
}

impl BrowsePage {
    /// Build the page and kick off both loads.
    #[must_use]
    pub fn new(services: &PageServices) -> Self {
        let api = Arc::clone(&services.api);
        let categories = Loadable::spawn_tracked(
            &services.runtime,
            &services.ctx,
            &services.activity,
            "Loading categories…",
            async move { api.browse_categories(CATEGORY_LEN).await },
        );
        let charts = services
            .lastfm
            .as_ref()
            .map(|lastfm| spawn_charts(services, lastfm.clone()));
        Self { categories, charts }
    }
}

/// Spawn the Last.fm charts load for the Browse shelves.
fn spawn_charts(services: &PageServices, lastfm: LastfmClient) -> Loadable<ChartsLoaded> {
    let resolver = LastfmResolver::new(Arc::clone(&services.api));
    Loadable::spawn_tracked(
        &services.runtime,
        &services.ctx,
        &services.activity,
        "Loading charts…",
        async move {
            let lf_tracks = lastfm.chart_top_tracks(SHELF_LEN).await?;
            let lf_artists = lastfm.chart_top_artists(SHELF_LEN).await?;
            let tracks = resolver
                .resolve_tracks(&lf_tracks)
                .await
                .map_err(|e| LastfmError::Network(e.to_string()))?;
            let artists = resolver
                .resolve_artists(&lf_artists)
                .await
                .map_err(|e| LastfmError::Network(e.to_string()))?;
            Ok(ChartShelves { tracks, artists })
        },
    )
}

impl Page for BrowsePage {
    fn title(&self) -> String {
        "Browse".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;
        let mut action = None;

        // The page header stays fixed; only the content below scrolls.
        ui.label(
            egui::RichText::new("Browse")
                .family(spottyfi_ui::fonts::semibold())
                .size(28.0)
                .color(palette.text),
        );
        ui.add_space(12.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // The Spotify category grid.
                components::section_header(ui, &palette, "Genres & Moods");
                ui.add_space(6.0);
                match self.categories.state() {
                    LoadState::Pending => {
                        ui.add(egui::Spinner::new().size(16.0).color(palette.accent));
                    }
                    LoadState::Cancelled => {
                        cards::calm_note(
                            ui,
                            &palette,
                            spottyfi_ui::Icon::Browse,
                            "Loading categories was cancelled.",
                        );
                    }
                    LoadState::Ready(Err(ApiError::EndpointUnavailable { .. })) => {
                        cards::calm_note(
                            ui,
                            &palette,
                            spottyfi_ui::Icon::Browse,
                            "Spotify's category list isn't available to this app.",
                        );
                    }
                    LoadState::Ready(Err(err)) => {
                        cards::calm_note(
                            ui,
                            &palette,
                            spottyfi_ui::Icon::Browse,
                            &format!("Couldn't load categories: {err}"),
                        );
                    }
                    LoadState::Ready(Ok(categories)) if categories.is_empty() => {
                        ui.label(components::muted(&palette, "No categories to show.", 13.0));
                    }
                    LoadState::Ready(Ok(categories)) => {
                        if let Some(a) = category_grid(ui, &palette, categories) {
                            action = Some(a);
                        }
                    }
                }
                ui.add_space(24.0);

                // The Last.fm charts shelves.
                if let Some(a) = charts_section(ui, &palette, self.charts.as_ref(), ctx) {
                    action = Some(a);
                }
            });
        action
    }
}

/// The Charts section: shelves of top tracks and artists, or a calm note when
/// Last.fm is unconfigured / failed.
fn charts_section(
    ui: &mut egui::Ui,
    palette: &Palette,
    charts: Option<&Loadable<ChartsLoaded>>,
    ctx: &PageContext<'_>,
) -> Option<PageAction> {
    components::section_header(ui, palette, "Charts");
    ui.add_space(6.0);

    let Some(charts) = charts else {
        cards::calm_note(
            ui,
            palette,
            spottyfi_ui::Icon::Charts,
            "Set SPOTTYFI_LASTFM_API_KEY to enable charts & recommendations.",
        );
        return None;
    };
    let loaded = match charts.state() {
        LoadState::Ready(loaded) => loaded,
        LoadState::Pending => {
            ui.add(egui::Spinner::new().size(16.0).color(palette.accent));
            return None;
        }
        LoadState::Cancelled => {
            cards::calm_note(
                ui,
                palette,
                spottyfi_ui::Icon::Charts,
                "Loading charts was cancelled.",
            );
            return None;
        }
    };

    match loaded {
        Err(err) => {
            cards::calm_note(
                ui,
                palette,
                spottyfi_ui::Icon::Charts,
                &format!("Couldn't load the charts: {err}"),
            );
            None
        }
        Ok(shelves) => {
            let mut action = None;
            let playing = ctx.playback.track.as_ref().map(|t| t.uri.as_str());
            if !shelves.tracks.is_empty() {
                ui.label(components::muted(palette, "Top Tracks", 11.5));
                ui.add_space(2.0);
                let context = super::track_view::PlayContext {
                    uri: "spottyfi:browse:top-tracks".to_owned(),
                    name: "Top Tracks".to_owned(),
                };
                if let Some(a) = cards::track_list(ui, palette, &shelves.tracks, playing, &context)
                {
                    action = Some(a);
                }
                ui.add_space(16.0);
            }
            if !shelves.artists.is_empty() {
                ui.label(components::muted(palette, "Top Artists", 11.5));
                ui.add_space(2.0);
                if let Some(a) = cards::artist_grid(ui, palette, &shelves.artists) {
                    action = Some(a);
                }
            }
            action
        }
    }
}

/// A wrapping grid of category tiles.
fn category_grid(
    ui: &mut egui::Ui,
    palette: &Palette,
    categories: &[Category],
) -> Option<PageAction> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        for (i, category) in categories.iter().enumerate() {
            if category_tile(ui, palette, category, i) {
                action = Some(PageAction::Open(Tab::Category(category.id.clone())));
            }
        }
    });
    action
}

/// One category tile: the category name over its art, the art **rotated** for
/// a dynamic, dense Browse look (per `PLAN.md` Phase 7). Returns `true` when
/// clicked.
fn category_tile(ui: &mut egui::Ui, palette: &Palette, category: &Category, index: usize) -> bool {
    let frame = egui::Frame::new()
        .fill(tile_color(palette, index))
        .corner_radius(0)
        .inner_margin(egui::Margin::same(10));
    let response = frame
        .show(ui, |ui| {
            ui.set_min_size(TILE_SIZE);
            ui.set_max_size(TILE_SIZE);
            let rect = ui.max_rect();

            // The category name, top-left.
            ui.painter().text(
                rect.left_top() + egui::vec2(2.0, 2.0),
                egui::Align2::LEFT_TOP,
                &category.name,
                egui::FontId::new(15.0, spottyfi_ui::fonts::semibold()),
                palette.text,
            );

            // The category art, bottom-right, rotated like the official client.
            if let Some(icon) = category.icons.first() {
                let art = 64.0;
                let art_rect = egui::Rect::from_min_size(
                    rect.right_bottom() - egui::vec2(art - 14.0, art - 14.0),
                    egui::vec2(art, art),
                );
                // A small index-derived tilt keeps the grid lively but stable
                // (the same category always tilts the same way).
                let angle = tilt(index);
                egui::Image::from_uri(icon.url.clone())
                    .fit_to_exact_size(egui::vec2(art, art))
                    .texture_options(egui::TextureOptions::LINEAR)
                    .rotate(angle, egui::Vec2::splat(0.5))
                    .paint_at(ui, art_rect);
            }
        })
        .response
        .interact(egui::Sense::click());
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// A small, deterministic tilt angle (radians) for a tile at `index`.
fn tilt(index: usize) -> f32 {
    // Cycle through a few fixed angles so neighbouring tiles differ.
    const ANGLES: [f32; 4] = [0.42, -0.30, 0.20, -0.46];
    ANGLES[index % ANGLES.len()]
}

/// A tile background colour, cycled so the grid is not monotone.
fn tile_color(palette: &Palette, index: usize) -> egui::Color32 {
    // Lerp the card colour towards the accent by a small, index-varying amount.
    let t = [0.06_f32, 0.14, 0.10, 0.18][index % 4];
    let lerp =
        |a: u8, b: u8| -> u8 { (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8 };
    let (c, a) = (palette.card, palette.accent);
    egui::Color32::from_rgb(lerp(c.r(), a.r()), lerp(c.g(), a.g()), lerp(c.b(), a.b()))
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
        mock.expect_browse_categories()
            .returning(|_| Ok(Vec::new()));
        let (services, _rt) = services(mock);
        assert_eq!(PageTrait::title(&BrowsePage::new(&services)), "Browse");
    }

    #[test]
    fn no_lastfm_client_means_no_charts_load() {
        let mut mock = MockSpotifyApi::new();
        mock.expect_browse_categories()
            .returning(|_| Ok(Vec::new()));
        let (services, _rt) = services(mock);
        let page = BrowsePage::new(&services);
        assert!(page.charts.is_none());
    }

    #[test]
    fn tilt_is_deterministic_and_varies() {
        assert_eq!(tilt(0), tilt(4));
        assert_ne!(tilt(0), tilt(1));
    }

    #[test]
    fn categories_load_resolves_from_the_api() {
        let mut mock = MockSpotifyApi::new();
        mock.expect_browse_categories().returning(|_| {
            Ok(vec![Category {
                id: "rock".to_owned(),
                name: "Rock".to_owned(),
                icons: Vec::new(),
            }])
        });
        let (services, _rt) = services(mock);
        let page = BrowsePage::new(&services);
        for _ in 0..200 {
            if page.categories.value().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let loaded = page.categories.value().expect("categories resolved");
        let cats = loaded.as_ref().expect("categories ok");
        assert_eq!(cats[0].id, "rock");
    }
}
