//! The Category page: a browse category's top tracks and artists.
//!
//! Spotify's "Get a Category's Playlists" endpoint is dead for new apps (see
//! `docs/questions.md` #7). Instead, the category is mapped to a Last.fm
//! **tag** and the page shows that tag's `tag.getTopTracks` /
//! `tag.getTopArtists`, resolved to Spotify objects so they are navigable and
//! playable.
//!
//! The page is opened with the Spotify category id (the `Tab::Category` key);
//! [`category_tag`] maps that id to a Last.fm tag, falling back to the id
//! itself lowercased — Spotify's genre-style ids (`rock`, `pop`, `hiphop`,
//! `jazz`, …) are already valid Last.fm tags.

use std::sync::Arc;

use spottyfi_api::lastfm::{LastfmClient, LastfmError, LastfmResolver};
use spottyfi_models::{Artist, Track};
use spottyfi_ui::components;

use super::cards;
use super::{
    load_cancelled, loading_spinner, LoadState, Loadable, Page, PageAction, PageContext,
    PageServices,
};

/// How many entries to pull for the category's tracks and artists.
const CATEGORY_LEN: u32 = 20;

/// Map a Spotify browse-category id to a Last.fm tag.
///
/// Spotify uses a mix of human ids (`rock`, `pop`) and opaque base-62 ids for
/// editorial categories. The handful of common opaque ids are mapped
/// explicitly; everything else falls back to the id lowercased, which already
/// works for Spotify's genre-style ids.
fn category_tag(category_id: &str) -> String {
    let id = category_id.to_lowercase();
    match id.as_str() {
        "0jq5dapbmkfz6fasutgaab" | "toplists" => "pop".to_owned(),
        "0jq5dapbmkfgg2tff7gaqd" => "hip-hop".to_owned(),
        "0jq5dapbmkfdxxwe9bdjny" => "rock".to_owned(),
        "0jq5dapbmkfietb6_5g5gx" => "pop".to_owned(),
        "hiphop" => "hip-hop".to_owned(),
        "rnb" => "rnb".to_owned(),
        _ => id,
    }
}

/// The data the Category page loads.
type Loaded = Result<CategoryData, LastfmError>;

/// A category's resolved tracks and artists.
struct CategoryData {
    /// The Last.fm tag this category resolved to (shown in the header).
    tag: String,
    /// The tag's top tracks, resolved to Spotify tracks.
    tracks: Vec<Track>,
    /// The tag's top artists, resolved to Spotify artists.
    artists: Vec<Artist>,
}

/// A browse-category tab.
pub struct CategoryPage {
    /// The Spotify category id this page was opened for.
    category_id: String,
    /// The async load — `None` until the page has a Last.fm client.
    data: Option<Loadable<Loaded>>,
}

impl CategoryPage {
    /// Build the page for a Spotify category id, kicking off its load.
    #[must_use]
    pub fn new(services: &PageServices, category_id: String) -> Self {
        let data = services
            .lastfm
            .as_ref()
            .map(|lastfm| spawn_load(services, lastfm.clone(), category_id.clone()));
        Self { category_id, data }
    }
}

/// Spawn the category load: the tag's top tracks + artists, resolved.
fn spawn_load(
    services: &PageServices,
    lastfm: LastfmClient,
    category_id: String,
) -> Loadable<Loaded> {
    let resolver = LastfmResolver::new(Arc::clone(&services.api));
    Loadable::spawn_tracked(
        &services.runtime,
        &services.ctx,
        &services.activity,
        "Loading category…",
        async move {
            let tag = category_tag(&category_id);
            let lf_tracks = lastfm.tag_top_tracks(&tag, CATEGORY_LEN).await?;
            let lf_artists = lastfm.tag_top_artists(&tag, CATEGORY_LEN).await?;
            let tracks = resolver
                .resolve_tracks(&lf_tracks)
                .await
                .map_err(|e| LastfmError::Network(e.to_string()))?;
            let artists = resolver
                .resolve_artists(&lf_artists)
                .await
                .map_err(|e| LastfmError::Network(e.to_string()))?;
            Ok(CategoryData {
                tag,
                tracks,
                artists,
            })
        },
    )
}

impl Page for CategoryPage {
    fn title(&self) -> String {
        match self.data.as_ref().and_then(Loadable::value) {
            Some(Ok(data)) => titlecase(&data.tag),
            _ => titlecase(&category_tag(&self.category_id)),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;

        let Some(data) = self.data.as_ref() else {
            cards::calm_note(
                ui,
                &palette,
                spottyfi_ui::Icon::Browse,
                "Set SPOTTYFI_LASTFM_API_KEY to browse this category.",
            );
            return None;
        };
        let loaded = match data.state() {
            LoadState::Ready(loaded) => loaded,
            LoadState::Pending => {
                loading_spinner(ui, &palette, "Loading this category…");
                return None;
            }
            LoadState::Cancelled => {
                load_cancelled(ui, &palette, "Loading this category was cancelled.");
                return None;
            }
        };

        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match loaded {
                Err(err) => {
                    cards::calm_note(
                        ui,
                        &palette,
                        spottyfi_ui::Icon::Browse,
                        &format!("Couldn't load this category: {err}"),
                    );
                }
                Ok(data) => {
                    ui.label(
                        egui::RichText::new(titlecase(&data.tag))
                            .family(spottyfi_ui::fonts::semibold())
                            .size(28.0)
                            .color(palette.text),
                    );
                    ui.label(components::muted(
                        &palette,
                        "Top tracks and artists for this genre, via Last.fm.",
                        12.0,
                    ));
                    ui.add_space(16.0);

                    let playing = ctx.playback.track.as_ref().map(|t| t.uri.as_str());
                    if !data.tracks.is_empty() {
                        components::section_header(ui, &palette, "Top Tracks");
                        ui.add_space(4.0);
                        let context = super::track_view::PlayContext {
                            uri: format!("spottyfi:category:{}", self.category_id),
                            name: format!("{} — Top Tracks", titlecase(&data.tag)),
                        };
                        if let Some(a) =
                            cards::track_list(ui, &palette, &data.tracks, playing, &context)
                        {
                            action = Some(a);
                        }
                        ui.add_space(20.0);
                    }
                    if !data.artists.is_empty() {
                        components::section_header(ui, &palette, "Top Artists");
                        ui.add_space(4.0);
                        if let Some(a) = cards::artist_grid(ui, &palette, &data.artists) {
                            action = Some(a);
                        }
                    }
                    if data.tracks.is_empty() && data.artists.is_empty() {
                        ui.label(components::muted(
                            &palette,
                            "Nothing found for this category.",
                            13.0,
                        ));
                    }
                }
            });
        action
    }
}

/// Title-case a Last.fm tag for display (`hip-hop` -> `Hip-Hop`).
fn titlecase(tag: &str) -> String {
    tag.split(['-', ' '])
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::Page as PageTrait;
    use spottyfi_api::MockSpotifyApi;

    fn services() -> (PageServices, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build runtime");
        let services = PageServices {
            api: Arc::new(MockSpotifyApi::new()),
            lastfm: None,
            lyrics: Default::default(),
            runtime: runtime.handle().clone(),
            ctx: egui::Context::default(),
            activity: spottyfi_state::ActivityRegistry::new(),
        };
        (services, runtime)
    }

    #[test]
    fn category_tag_falls_back_to_the_lowercased_id() {
        assert_eq!(category_tag("Rock"), "rock");
        assert_eq!(category_tag("jazz"), "jazz");
    }

    #[test]
    fn category_tag_maps_known_aliases() {
        assert_eq!(category_tag("hiphop"), "hip-hop");
    }

    #[test]
    fn titlecase_handles_hyphenated_tags() {
        assert_eq!(titlecase("hip-hop"), "Hip-Hop");
        assert_eq!(titlecase("rock"), "Rock");
    }

    #[test]
    fn title_derives_from_the_category_id_before_loading() {
        let (services, _rt) = services();
        let page = CategoryPage::new(&services, "rock".to_owned());
        assert_eq!(PageTrait::title(&page), "Rock");
    }
}
