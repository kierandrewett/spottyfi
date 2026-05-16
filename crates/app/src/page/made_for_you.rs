//! The Made For You page: recommendations built from the user's listening.
//!
//! Spotify's algorithmic mixes and `/recommendations` are dead for new apps
//! (see `docs/questions.md` #7). Made For You is rebuilt from parts that *do*
//! work: the user's Spotify top artists and top tracks (`GET /me/top/*`, not
//! deprecated) seed Last.fm's `artist.getSimilar` / `track.getSimilar`, and the
//! resulting names are resolved back to Spotify objects.
//!
//! With no `SPOTTYFI_LASTFM_API_KEY` configured the page shows a calm note —
//! it never errors or crashes.

use std::sync::Arc;

use spottyfi_api::lastfm::{LastfmClient, LastfmError, LastfmResolver};
use spottyfi_models::{Artist, Track};
use spottyfi_ui::components;

use super::cards;
use super::{loading_spinner, Loadable, Page, PageAction, PageContext, PageServices};

/// How many of the user's top items to seed recommendations from.
const SEED_COUNT: u32 = 5;
/// How many similar items to request per seed.
const PER_SEED: u32 = 8;

/// The data the Made For You page loads.
type Loaded = Result<MixData, LastfmError>;

/// The resolved recommendation mixes.
struct MixData {
    /// A track mix: tracks similar to the user's top tracks.
    track_mix: Vec<Track>,
    /// An artist mix: artists similar to the user's top artists.
    artist_mix: Vec<Artist>,
}

/// The Made For You tab.
pub struct MadeForYouPage {
    /// The async load — `None` until the page has a Last.fm client.
    data: Option<Loadable<Loaded>>,
}

impl MadeForYouPage {
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

/// Spawn the recommendation load.
///
/// The user's Spotify top tracks/artists seed Last.fm similarity lookups; the
/// similar names are resolved back to Spotify objects and de-duplicated.
fn spawn_load(services: &PageServices, lastfm: LastfmClient) -> Loadable<Loaded> {
    let api = Arc::clone(&services.api);
    let resolver = LastfmResolver::new(Arc::clone(&services.api));
    Loadable::spawn_tracked(
        &services.runtime,
        &services.ctx,
        &services.activity,
        "Building your mixes…",
        async move {
            // Seed from the user's top items (not deprecated endpoints).
            let top_tracks = api
                .current_user_top_tracks(SEED_COUNT)
                .await
                .map_err(|e| LastfmError::Network(e.to_string()))?;
            let top_artists = api
                .current_user_top_artists(SEED_COUNT)
                .await
                .map_err(|e| LastfmError::Network(e.to_string()))?;

            // Track mix: tracks similar to each top track.
            let mut lf_tracks = Vec::new();
            for track in &top_tracks {
                let artist = track
                    .artists
                    .first()
                    .map(|a| a.name.as_str())
                    .unwrap_or_default();
                if let Ok(similar) = lastfm.similar_tracks(artist, &track.name, PER_SEED).await {
                    lf_tracks.extend(similar);
                }
            }

            // Artist mix: artists similar to each top artist.
            let mut lf_artists = Vec::new();
            for artist in &top_artists {
                if let Ok(similar) = lastfm.similar_artists(&artist.name, PER_SEED).await {
                    lf_artists.extend(similar);
                }
            }

            let mut track_mix = resolver
                .resolve_tracks(&lf_tracks)
                .await
                .map_err(|e| LastfmError::Network(e.to_string()))?;
            dedup_tracks(&mut track_mix);

            let mut artist_mix = resolver
                .resolve_artists(&lf_artists)
                .await
                .map_err(|e| LastfmError::Network(e.to_string()))?;
            dedup_artists(&mut artist_mix);

            Ok(MixData {
                track_mix,
                artist_mix,
            })
        },
    )
}

/// Drop duplicate tracks (by Spotify id), preserving first-seen order.
fn dedup_tracks(tracks: &mut Vec<Track>) {
    let mut seen = std::collections::HashSet::new();
    tracks.retain(|t| match &t.id {
        Some(id) => seen.insert(id.0.clone()),
        None => true,
    });
}

/// Drop duplicate artists (by Spotify id), preserving first-seen order.
fn dedup_artists(artists: &mut Vec<Artist>) {
    let mut seen = std::collections::HashSet::new();
    artists.retain(|a| seen.insert(a.id.0.clone()));
}

impl Page for MadeForYouPage {
    fn title(&self) -> String {
        "Made For You".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        let palette = ctx.palette;

        let Some(data) = self.data.as_ref() else {
            cards::calm_note(
                ui,
                &palette,
                spottyfi_ui::Icon::MadeForYou,
                "Set SPOTTYFI_LASTFM_API_KEY to enable charts & recommendations.",
            );
            return None;
        };
        let Some(loaded) = data.value() else {
            loading_spinner(ui, &palette, "Building your mixes…");
            return None;
        };

        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Made For You")
                        .family(spottyfi_ui::fonts::semibold())
                        .size(28.0)
                        .color(palette.text),
                );
                ui.label(components::muted(
                    &palette,
                    "Recommendations from your top artists and tracks, via Last.fm.",
                    12.0,
                ));
                ui.add_space(16.0);

                match loaded {
                    Err(err) => {
                        cards::calm_note(
                            ui,
                            &palette,
                            spottyfi_ui::Icon::MadeForYou,
                            &format!("Couldn't build your mixes: {err}"),
                        );
                    }
                    Ok(data) if data.track_mix.is_empty() && data.artist_mix.is_empty() => {
                        ui.label(components::muted(
                            &palette,
                            "Listen to more music and your mixes will appear here.",
                            13.0,
                        ));
                    }
                    Ok(data) => {
                        let playing = ctx.playback.track.as_ref().map(|t| t.uri.as_str());
                        if !data.track_mix.is_empty() {
                            components::section_header(ui, &palette, "Your Track Mix");
                            ui.add_space(4.0);
                            if let Some(a) =
                                cards::track_list(ui, &palette, &data.track_mix, playing)
                            {
                                action = Some(a);
                            }
                            ui.add_space(20.0);
                        }
                        if !data.artist_mix.is_empty() {
                            components::section_header(ui, &palette, "Artists You Might Like");
                            ui.add_space(4.0);
                            if let Some(a) = cards::artist_grid(ui, &palette, &data.artist_mix) {
                                action = Some(a);
                            }
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
    use spottyfi_models::{ArtistId, TrackId};

    fn services() -> (PageServices, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build runtime");
        let services = PageServices {
            api: Arc::new(MockSpotifyApi::new()),
            lastfm: None,
            runtime: runtime.handle().clone(),
            ctx: egui::Context::default(),
            activity: spottyfi_state::ActivityRegistry::new(),
        };
        (services, runtime)
    }

    fn artist(id: &str) -> Artist {
        Artist {
            id: ArtistId::new(id),
            name: id.to_owned(),
            images: Vec::new(),
            genres: Vec::new(),
            popularity: 0,
        }
    }

    #[test]
    fn title_is_static() {
        let (services, _rt) = services();
        assert_eq!(
            PageTrait::title(&MadeForYouPage::new(&services)),
            "Made For You"
        );
    }

    #[test]
    fn no_lastfm_client_means_no_load() {
        let (services, _rt) = services();
        assert!(MadeForYouPage::new(&services).data.is_none());
    }

    #[test]
    fn dedup_artists_keeps_first_occurrence() {
        let mut artists = vec![artist("a"), artist("b"), artist("a")];
        dedup_artists(&mut artists);
        assert_eq!(artists.len(), 2);
        assert_eq!(artists[0].id.0, "a");
        assert_eq!(artists[1].id.0, "b");
    }

    #[test]
    fn dedup_tracks_drops_repeated_ids() {
        let make = |id: &str| Track {
            id: Some(TrackId::new(id)),
            name: id.to_owned(),
            artists: Vec::new(),
            album: spottyfi_models::SimplifiedAlbum {
                id: None,
                name: "x".to_owned(),
                images: Vec::new(),
                artists: Vec::new(),
                release_date: None,
            },
            duration_ms: 0,
            explicit: false,
            popularity: 0,
            track_number: 1,
            is_local: false,
        };
        let mut tracks = vec![make("t1"), make("t2"), make("t1")];
        dedup_tracks(&mut tracks);
        assert_eq!(tracks.len(), 2);
    }
}
