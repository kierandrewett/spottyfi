//! The Lyrics panel.
//!
//! A dock panel that shows lyrics for the **currently playing** track. It is
//! registry-backed (it needs [`PageServices`] to fetch lyrics) but is docked
//! and opened like the other auxiliary panels — Now Playing Art, Queue,
//! Visualiser.
//!
//! ## Behaviour
//!
//! - The panel watches the live [`PlaybackState`](spottyfi_audio::PlaybackState)
//!   and re-fetches whenever the current track changes.
//! - **Synced** lyrics highlight the line current at the live playback
//!   position and auto-scroll to keep it in view; **clicking a line seeks**
//!   the transport to that line's timestamp.
//! - **Plain** (unsynced) lyrics render as a static scrollable column.
//! - Empty / loading / unavailable states are calm and flat, matching the
//!   dense `docs/ui-reference.md` aesthetic.
//!
//! With no lyrics source configured the fetch resolves to
//! [`LyricsError::NoSourceConfigured`](spottyfi_api::lyrics::LyricsError) and
//! the panel shows a quiet "no lyrics source configured" note rather than an
//! error — see the lyrics layer docs.

use std::time::Duration;

use spottyfi_api::lyrics::{Lyrics, LyricsError, LyricsService, TrackRef};
use spottyfi_ui::components;

use super::{LoadState, Loadable, Page, PageAction, PageContext, PageServices};

/// The result of one lyrics fetch.
type Loaded = Result<Lyrics, LyricsError>;

/// The Lyrics panel — lyrics for the current track.
pub struct LyricsPanel {
    /// Services kept so the panel can re-fetch when the track changes.
    services: PageServices,
    /// The lyrics service the fetch goes through.
    lyrics: LyricsService,
    /// The track URI the current load is for; `None` before the first track.
    loaded_uri: Option<String>,
    /// The in-flight (or resolved) lyrics fetch for [`Self::loaded_uri`].
    data: Option<Loadable<Loaded>>,
}

impl LyricsPanel {
    /// Build the panel. The first fetch is deferred to the first frame, once a
    /// playback snapshot with a track is available.
    #[must_use]
    pub fn new(services: &PageServices) -> Self {
        Self {
            services: services.clone(),
            lyrics: services.lyrics.clone(),
            loaded_uri: None,
            data: None,
        }
    }

    /// Spawn a lyrics fetch for `track`, replacing any in-flight load.
    fn fetch(&mut self, track: TrackRef) {
        let lyrics = self.lyrics.clone();
        self.loaded_uri = Some(track.uri.clone());
        self.data = Some(Loadable::spawn_tracked(
            &self.services.runtime,
            &self.services.ctx,
            &self.services.activity,
            "Loading lyrics…",
            async move { lyrics.lyrics(&track).await },
        ));
    }

    /// Re-fetch if the playing track changed since the last load.
    fn sync_track(&mut self, ctx: &PageContext<'_>) {
        let current = ctx.playback.track.as_ref();
        let current_uri = current.map(|t| t.uri.as_str());
        if current_uri == self.loaded_uri.as_deref() {
            return;
        }
        match current {
            Some(track) => self.fetch(TrackRef {
                uri: track.uri.clone(),
                title: track.title.clone(),
                artist: track.artists.first().cloned().unwrap_or_default(),
            }),
            None => {
                // Nothing playing — drop any stale load.
                self.loaded_uri = None;
                self.data = None;
            }
        }
    }
}

impl Page for LyricsPanel {
    fn title(&self) -> String {
        "Lyrics".to_owned()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &PageContext<'_>) -> Option<PageAction> {
        self.sync_track(ctx);
        let palette = ctx.palette;

        // Nothing playing — a calm empty state.
        let Some(track) = ctx.playback.track.as_ref() else {
            empty_state(
                ui,
                &palette,
                "Nothing playing",
                "Play a track to see its lyrics.",
            );
            return None;
        };

        // A track header, then the lyrics body below it.
        ui.label(
            egui::RichText::new(&track.title)
                .family(spottyfi_ui::fonts::semibold())
                .size(15.0)
                .color(palette.text),
        );
        ui.label(components::muted(&palette, track.artist_line(), 12.0));
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);

        let Some(data) = self.data.as_ref() else {
            super::loading_spinner(ui, &palette, "Loading lyrics…");
            return None;
        };
        let loaded = match data.state() {
            LoadState::Ready(loaded) => loaded,
            LoadState::Pending => {
                super::loading_spinner(ui, &palette, "Loading lyrics…");
                return None;
            }
            LoadState::Cancelled => {
                super::load_cancelled(ui, &palette, "Loading lyrics was cancelled.");
                return None;
            }
        };

        match loaded {
            Ok(lyrics) if lyrics.is_empty() => {
                empty_state(
                    ui,
                    &palette,
                    "No lyrics",
                    "This track has no lyrics available.",
                );
                None
            }
            Ok(Lyrics::Synced(lines)) => synced_lyrics(ui, &palette, lines, ctx.playback.position),
            Ok(Lyrics::Plain(lines)) => {
                plain_lyrics(ui, &palette, lines);
                None
            }
            Err(LyricsError::NoSourceConfigured) => {
                empty_state(
                    ui,
                    &palette,
                    "No lyrics source configured",
                    "Build with the `musixmatch` feature and set SPOTTYFI_MUSIXMATCH_KEY \
                     to enable lyrics.",
                );
                None
            }
            Err(LyricsError::NotFound) => {
                empty_state(
                    ui,
                    &palette,
                    "No lyrics",
                    "No lyrics were found for this track.",
                );
                None
            }
            Err(err) => {
                empty_state(ui, &palette, "Lyrics unavailable", &err.to_string());
                None
            }
        }
    }
}

/// The fixed height of one lyric line row.
const LINE_HEIGHT: f32 = 26.0;

/// Render time-synced lyrics: highlight the current line, auto-scroll to keep
/// it visible, and seek when a line is clicked.
fn synced_lyrics(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    lines: &[spottyfi_api::lyrics::SyncedLine],
    position: Duration,
) -> Option<PageAction> {
    let current = spottyfi_api::lyrics::current_synced_line(lines, position);
    let mut action = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, line) in lines.iter().enumerate() {
                let is_current = current == Some(index);
                let response = lyric_row(ui, palette, &line.text, is_current);
                if response.clicked() {
                    action = Some(PageAction::Seek(line.at));
                }
                // Auto-scroll: keep the current line in view. Only scroll when
                // the line is freshly current, so manual scrolling is not
                // fought every frame — `scroll_to_me` with no animation when
                // already visible is a no-op, but egui still respects an
                // explicit request, so it is gated on `is_current`.
                if is_current {
                    response.scroll_to_me(Some(egui::Align::Center));
                }
            }
        });

    // The highlight tracks the live position; keep repainting smoothly.
    ui.ctx().request_repaint_after(Duration::from_millis(120));

    action
}

/// Render plain, unsynced lyrics as a static scrollable column.
fn plain_lyrics(ui: &mut egui::Ui, palette: &spottyfi_ui::theme::Palette, lines: &[String]) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for line in lines {
                if line.trim().is_empty() {
                    ui.add_space(6.0);
                } else {
                    ui.label(
                        egui::RichText::new(line)
                            .family(spottyfi_ui::fonts::medium())
                            .size(13.5)
                            .color(palette.text),
                    );
                }
            }
        });
}

/// One clickable synced-lyric row. The current line is rendered in the accent
/// green and bold; the rest are dimmed. A flat, full-bleed hover highlight.
fn lyric_row(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    text: &str,
    is_current: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), LINE_HEIGHT),
        egui::Sense::click(),
    );
    if !ui.is_rect_visible(rect) {
        return response;
    }

    if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, palette.hover);
    }

    let (color, family) = if is_current {
        (palette.accent, spottyfi_ui::fonts::semibold())
    } else {
        (palette.text_muted, spottyfi_ui::fonts::medium())
    };
    // An empty line (an instrumental gap) shows a muted marker so the row is
    // not an invisible dead click target.
    let shown = if text.trim().is_empty() { "♪" } else { text };

    ui.painter().text(
        egui::pos2(rect.left() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        shown,
        egui::FontId::new(14.0, family),
        color,
    );

    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A calm, centred empty / unavailable state.
fn empty_state(
    ui: &mut egui::Ui,
    palette: &spottyfi_ui::theme::Palette,
    title: &str,
    detail: &str,
) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.32);
        ui.label(
            egui::RichText::new(title)
                .family(spottyfi_ui::fonts::semibold())
                .size(15.0)
                .color(palette.text),
        );
        ui.add_space(4.0);
        ui.label(components::muted(palette, detail, 12.0));
    });
}
