//! The bottom transport bar and the debug "play a URI" control.
//!
//! The transport is a true three-region layout: now-playing album art +
//! title/artist on the left, the control cluster and seek scrubber genuinely
//! centred in the window, and a volume control + toggle placeholders on the
//! right. The central play/pause button is a larger accent-green circle — the
//! one deliberately rounded element in an otherwise sharp UI.
//!
//! Both the seek bar and the volume control are the shared
//! [`spottyfi_ui::scrubber::Scrubber`] widget: hover-to-preview, click/drag to
//! seek, the seek committed on release so the engine is not spammed mid-drag.
//!
//! Shuffle and repeat are wired to the engine: they project the live
//! [`spottyfi_audio::QueueState`] and emit intents on click. The right-cluster
//! toggles (settings / devices / queue) remain visual placeholders, themed and
//! laid out so the bar reads as complete.

use std::time::Duration;

use spottyfi_audio::{PlaybackState, QueueState, RepeatMode};
use spottyfi_ui::components;
use spottyfi_ui::icons::{self, Icon};
use spottyfi_ui::scrubber::{Scrubber, ScrubberState};
use spottyfi_ui::theme::Palette;

use crate::playback_controller::EngineStatus;

/// Height of the transport bar — a tight, dense strip.
pub const TRANSPORT_HEIGHT: f32 = 68.0;

/// A transport command the user issued this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum TransportIntent {
    /// Toggle play/pause.
    TogglePlayPause,
    /// Seek to the given position (emitted on scrubber drag-release).
    Seek(Duration),
    /// Set the output volume to a `0.0..=1.0` fraction.
    SetVolume(f32),
    /// Play the given Spotify URI / URL (from the debug field).
    PlayUri(String),
    /// Skip to the next track (manual queue first, then the context).
    Next,
    /// Skip to the previous context track.
    Previous,
    /// Play a context — a full resolved track list — starting at `offset`.
    PlayContext {
        /// The context's own Spotify URI.
        uri: String,
        /// The context's display name (shown in the queue panel).
        name: String,
        /// The context's tracks, in play order.
        tracks: Vec<spottyfi_audio::QueueTrack>,
        /// The index in `tracks` to start playback at.
        offset: usize,
    },
    /// Add a track to the front of the manual queue (play it next).
    PlayNext(spottyfi_audio::QueueTrack),
    /// Add a track to the end of the manual queue.
    Enqueue(spottyfi_audio::QueueTrack),
    /// Jump to manual-queue entry `index` (a click in the queue panel).
    SkipToManual(usize),
    /// Jump to upcoming-context entry `index` (a click in the queue panel).
    SkipToContext(usize),
    /// Move manual-queue entry `from` to `to` (drag-to-reorder).
    ReorderManual {
        /// The source index in the manual queue.
        from: usize,
        /// The destination index in the manual queue.
        to: usize,
    },
    /// Remove manual-queue entry `index`.
    RemoveManual(usize),
    /// Set shuffle on or off.
    SetShuffle(bool),
    /// Set the repeat mode (off / repeat-all / repeat-one).
    SetRepeat(RepeatMode),
}

/// Per-frame, mutable UI state for the transport widgets.
///
/// Held by the app so the seek/volume scrubbers' drag state and the debug
/// field survive between frames. Shuffle and repeat are *not* held here —
/// they are projected from the live [`QueueState`] so the buttons always
/// reflect the engine's real state.
#[derive(Default)]
pub struct TransportUiState {
    /// The track URI typed into the debug control.
    pub debug_uri: String,
    /// The seek scrubber's per-instance drag state.
    seek: ScrubberState,
    /// The volume scrubber's per-instance drag state.
    volume_scrub: ScrubberState,
    /// While the user drags the volume scrubber, the in-progress fraction so
    /// the icon and fill follow the drag before the engine catches up.
    volume_preview: Option<f32>,
}

/// Render the bottom transport bar. Returns any [`TransportIntent`] issued.
///
/// `palette` themes the bar; `playback` is the live snapshot the controls
/// project.
///
/// ## Layout
///
/// The bar is a true three-region layout. The centre region — the control
/// cluster and seek scrubber — is placed in a rect *centred on the panel's own
/// width*, so it stays put regardless of how wide the left (now-playing) and
/// right (volume) regions are. The left and right regions are then drawn in
/// the gaps either side. This is the "measure, place a centred rect" approach
/// the brief asks for — no width-fudging hacks.
pub fn transport_bar(
    ui: &mut egui::Ui,
    palette: &Palette,
    ui_state: &mut TransportUiState,
    playback: &PlaybackState,
    queue: &QueueState,
) -> Option<TransportIntent> {
    let mut intent = None;

    egui::Panel::bottom("transport")
        .exact_size(TRANSPORT_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(palette.elevated)
                .inner_margin(egui::Margin::symmetric(12, 6)),
        )
        .show_inside(ui, |ui| {
            let full = ui.available_rect_before_wrap();

            // The centre region is a fixed-width band centred on the panel.
            // Clamp so it always fits between the side regions on a narrow
            // window.
            const SIDE_MIN: f32 = 150.0;
            let centre_width = CENTRE_WIDTH.min((full.width() - SIDE_MIN * 2.0).max(220.0));
            let centre_rect = egui::Rect::from_center_size(
                full.center(),
                egui::vec2(centre_width, full.height()),
            );

            // The left region fills from the panel's left edge to the centre
            // band; the right region from the centre band to the right edge.
            let left_rect =
                egui::Rect::from_min_max(full.min, egui::pos2(centre_rect.left(), full.bottom()));
            let right_rect =
                egui::Rect::from_min_max(egui::pos2(centre_rect.right(), full.top()), full.max);

            // Left: now-playing art + title/artist.
            let mut left = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(left_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            now_playing(&mut left, palette, playback);

            // Centre: the control cluster over the seek scrubber, genuinely
            // centred in the window.
            let mut centre = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(centre_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            if let Some(i) = centre_controls(&mut centre, palette, ui_state, playback, queue) {
                intent = Some(i);
            }

            // Right: volume control + toggle placeholders, anchored right.
            let mut right = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(right_rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            if let Some(i) = right_cluster(&mut right, palette, ui_state, playback) {
                intent = Some(i);
            }
        });

    intent
}

/// The fixed width of the centred transport region (controls + scrubber).
const CENTRE_WIDTH: f32 = 520.0;

/// The now-playing block: album art (live URL), title + artist, and a dimmed
/// bitrate line.
fn now_playing(ui: &mut egui::Ui, palette: &Palette, playback: &PlaybackState) {
    let art_url = playback.track.as_ref().and_then(|t| t.art_url.as_deref());
    components::album_art(ui, palette, art_url, 48.0, 0.0);

    ui.add_space(10.0);
    ui.vertical(|ui| {
        ui.add_space(4.0);
        match &playback.track {
            Some(track) => {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&track.title)
                            .family(spottyfi_ui::fonts::medium())
                            .size(12.5)
                            .color(palette.text),
                    )
                    .truncate(),
                );
                ui.add(
                    egui::Label::new(components::muted(palette, track.artist_line(), 11.0))
                        .truncate(),
                );
                // The real configured codec/bitrate, reported by the engine.
                if let Some(codec_line) = playback.codec_line() {
                    ui.label(components::muted(palette, codec_line, 9.5));
                }
            }
            None => {
                ui.label(components::muted(palette, "Nothing playing", 12.5));
            }
        }
    });
}

/// The diameter of the central play/pause button — deliberately larger than
/// the surrounding controls, and the one rounded element in an otherwise sharp
/// UI (see `docs/ui-reference.md`).
const PLAY_BUTTON_DIAMETER: f32 = 34.0;

/// The centre block: a control row above a seek scrubber, both centred
/// horizontally and the pair centred vertically within the transport band.
fn centre_controls(
    ui: &mut egui::Ui,
    palette: &Palette,
    ui_state: &mut TransportUiState,
    playback: &PlaybackState,
    queue: &QueueState,
) -> Option<TransportIntent> {
    let mut intent = None;

    // The two stacked rows have a fixed combined height; centre the block
    // vertically within the band, then lay the rows top-down inside it.
    const CONTROL_ROW_H: f32 = PLAY_BUTTON_DIAMETER;
    const SCRUBBER_ROW_H: f32 = 14.0;
    const ROW_GAP: f32 = 4.0;
    let block_height = CONTROL_ROW_H + ROW_GAP + SCRUBBER_ROW_H;
    let band = ui.available_rect_before_wrap();
    let width = band.width();

    // A block of the exact combined height, vertically centred in the band.
    let block_rect = egui::Rect::from_center_size(band.center(), egui::vec2(width, block_height));
    let mut block = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(block_rect)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    block.spacing_mut().item_spacing.y = ROW_GAP;
    // The transport control row, centred horizontally.
    if let Some(i) = control_row(&mut block, palette, playback, queue, CONTROL_ROW_H) {
        intent = Some(i);
    }
    // The seek scrubber row, spanning the full block width.
    if let Some(i) = scrubber_row(&mut block, palette, ui_state, playback, width) {
        intent = Some(i);
    }

    intent
}

/// The shuffle / prev / play-pause / next / repeat control row, sized to a
/// fixed height and centred horizontally by the enclosing top-down layout.
///
/// Shuffle and repeat are projected from the live [`QueueState`]: the buttons
/// reflect the engine's real state and emit an intent on click rather than
/// holding any UI-local toggle.
fn control_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    playback: &PlaybackState,
    queue: &QueueState,
    height: f32,
) -> Option<TransportIntent> {
    let mut intent = None;

    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // A horizontally-centred cluster: an inner group whose intrinsic
            // width egui centres within the row.
            ui.with_layout(
                egui::Layout::left_to_right(egui::Align::Center)
                    .with_main_align(egui::Align::Center),
                |ui| {
                    if icons::icon_button(
                        ui,
                        palette,
                        Icon::Shuffle,
                        15.0,
                        queue.shuffle,
                        "Shuffle",
                    )
                    .clicked()
                    {
                        intent = Some(TransportIntent::SetShuffle(!queue.shuffle));
                    }
                    if icons::icon_button(ui, palette, Icon::SkipBack, 16.0, false, "Previous")
                        .clicked()
                    {
                        intent = Some(TransportIntent::Previous);
                    }

                    if play_button(ui, palette, playback).clicked() {
                        intent = Some(TransportIntent::TogglePlayPause);
                    }

                    if icons::icon_button(ui, palette, Icon::SkipForward, 16.0, false, "Next")
                        .clicked()
                    {
                        intent = Some(TransportIntent::Next);
                    }
                    if let Some(i) = repeat_button(ui, palette, queue.repeat) {
                        intent = Some(i);
                    }
                },
            );
        },
    );

    intent
}

/// The repeat control: cycles `off → repeat-all → repeat-one → off`.
///
/// There is no dedicated repeat-one glyph, so all three states reuse the
/// [`Icon::Repeat`] glyph: off is muted, repeat-all is accent-tinted, and
/// repeat-one is accent-tinted with a small accent dot below to mark the
/// "single track" variant — matching the Spotify client's affordance.
fn repeat_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    repeat: RepeatMode,
) -> Option<TransportIntent> {
    let active = repeat != RepeatMode::Off;
    let tooltip = match repeat {
        RepeatMode::Off => "Repeat: off",
        RepeatMode::Context => "Repeat: all",
        RepeatMode::Track => "Repeat: one",
    };
    let response = icons::icon_button(ui, palette, Icon::Repeat, 15.0, active, tooltip);

    if repeat == RepeatMode::Track && ui.is_rect_visible(response.rect) {
        // A small accent dot under the glyph marks the repeat-one variant.
        let dot = egui::pos2(response.rect.center().x, response.rect.center().y + 9.0);
        ui.painter().circle_filled(dot, 1.6, palette.accent);
    }

    response
        .clicked()
        .then(|| TransportIntent::SetRepeat(repeat.cycled()))
}

/// The central play/pause control: a filled accent-green **circle**, bigger
/// than the surrounding icon buttons.
///
/// This is the one deliberately rounded element in Spottyfi's otherwise sharp,
/// zero-radius UI — the reference screenshot shows exactly this. It brightens
/// slightly on hover and dims to the outline colour when no track is loaded.
fn play_button(ui: &mut egui::Ui, palette: &Palette, playback: &PlaybackState) -> egui::Response {
    let has_track = playback.track.is_some();
    let diameter = PLAY_BUTTON_DIAMETER;
    let sense = if has_track {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(diameter, diameter), sense);

    if ui.is_rect_visible(rect) {
        let fill = if !has_track {
            palette.outline
        } else if response.hovered() {
            // A touch brighter on hover, like the Spotify client.
            palette.accent
        } else {
            palette.accent_dark
        };
        ui.painter()
            .circle_filled(rect.center(), diameter * 0.5, fill);
        let glyph = if playback.playing {
            Icon::Pause
        } else {
            Icon::Play
        };
        let g = diameter * 0.42;
        glyph.image(g, egui::Color32::BLACK).paint_at(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(g, g)),
        );
    }

    if has_track {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The fixed height of the seek-scrubber row.
const SCRUBBER_HEIGHT: f32 = 14.0;

/// The seek scrubber row: elapsed / total readouts flanking the custom
/// [`Scrubber`] widget.
///
/// The displayed elapsed time follows the dragged position while a drag is in
/// progress; the actual [`TransportIntent::Seek`] is emitted only on release
/// (or a plain click), so the engine is not spammed mid-drag.
fn scrubber_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    ui_state: &mut TransportUiState,
    playback: &PlaybackState,
    width: f32,
) -> Option<TransportIntent> {
    let mut intent = None;

    let duration = playback
        .track
        .as_ref()
        .map_or(Duration::ZERO, |t| t.duration);
    let live_fraction = playback.progress_fraction();

    ui.allocate_ui_with_layout(
        egui::vec2(width, SCRUBBER_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            // Reserve a fixed slot for each readout so the scrubber's left and
            // right edges hold still as the time text changes width.
            const READOUT_W: f32 = 34.0;
            let elapsed_slot = ui.cursor().min;

            // Placeholder for the elapsed readout — drawn after the scrubber,
            // once the dragged position is known.
            ui.add_space(READOUT_W);

            let track_width = (ui.available_width() - READOUT_W).max(60.0);
            let scrub = Scrubber::new(palette, "transport-seek")
                .width(track_width)
                .track_thickness(4.0)
                .knob_radius(6.0)
                .enabled(!duration.is_zero())
                .show(ui, &mut ui_state.seek, live_fraction);

            if let Some(fraction) = scrub.committed {
                let target = Duration::from_secs_f32(fraction * duration.as_secs_f32());
                intent = Some(TransportIntent::Seek(target));
            }

            ui.label(components::muted(palette, fmt_duration(duration), 10.5));

            // Draw the elapsed readout into its reserved slot: the dragged
            // position while dragging, otherwise live playback position.
            let position = if scrub.dragging {
                Duration::from_secs_f32(scrub.fraction * duration.as_secs_f32())
            } else {
                playback.position
            };
            let mut elapsed = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(
                        elapsed_slot,
                        egui::vec2(READOUT_W, SCRUBBER_HEIGHT),
                    ))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            elapsed.label(components::muted(palette, fmt_duration(position), 10.5));
        },
    );

    intent
}

/// The right cluster: a volume scrubber + toggle placeholders.
fn right_cluster(
    ui: &mut egui::Ui,
    palette: &Palette,
    ui_state: &mut TransportUiState,
    playback: &PlaybackState,
) -> Option<TransportIntent> {
    let mut intent = None;

    // The right region is laid out right-to-left, so widgets are added
    // rightmost-first: volume scrubber, volume icon, then the toggles.
    ui.spacing_mut().item_spacing.x = 4.0;

    // The volume scrubber reuses the same component, just shorter and with no
    // hover-preview cue (knob still appears on hover/drag).
    let volume = ui_state.volume_preview.unwrap_or(playback.volume);
    let scrub = Scrubber::new(palette, "transport-volume")
        .width(88.0)
        .track_thickness(4.0)
        .knob_radius(5.0)
        .show(ui, &mut ui_state.volume_scrub, volume);
    if scrub.dragging {
        ui_state.volume_preview = Some(scrub.fraction);
    }
    // Volume changes apply live (drag and click), so emit on any change.
    if let Some(fraction) = scrub.committed {
        intent = Some(TransportIntent::SetVolume(fraction));
        ui_state.volume_preview = None;
    } else if scrub.dragging {
        intent = Some(TransportIntent::SetVolume(scrub.fraction));
    }

    let shown_volume = ui_state.volume_preview.unwrap_or(playback.volume);
    let vol_icon = if shown_volume <= 0.001 {
        Icon::VolumeMuted
    } else {
        Icon::Volume
    };
    icons::icon_button(ui, palette, vol_icon, 15.0, false, "Volume");

    ui.add_space(4.0);
    // Toggle placeholders — wired in later phases.
    icons::icon_button(ui, palette, Icon::Settings, 15.0, false, "Settings (later)");
    icons::icon_button(ui, palette, Icon::Devices, 15.0, false, "Devices (later)");
    icons::icon_button(ui, palette, Icon::Queue, 15.0, false, "Queue (later)");

    intent
}

/// The debug control: a URI field plus a Play button, shown in the Debug panel
/// so playback can be exercised before the browsing UI exists (Phase 5).
pub fn debug_play_control(
    ui: &mut egui::Ui,
    palette: &Palette,
    ui_state: &mut TransportUiState,
    engine: &EngineStatus,
) -> Option<TransportIntent> {
    let mut intent = None;

    egui::Frame::new()
        .fill(palette.card)
        .corner_radius(0)
        .inner_margin(egui::Margin::same(12))
        .stroke(egui::Stroke::new(1.0, palette.outline))
        .show(ui, |ui| {
            ui.set_max_width(460.0);
            ui.label(
                egui::RichText::new("Play a track")
                    .family(spottyfi_ui::fonts::medium())
                    .color(palette.text),
            );
            ui.add_space(2.0);
            ui.label(components::muted(
                palette,
                "Paste a spotify:track: URI or an open.spotify.com link.",
                11.0,
            ));
            ui.add_space(8.0);

            match engine {
                EngineStatus::Idle => {
                    ui.label(components::muted(
                        palette,
                        "Audio engine not started.",
                        11.0,
                    ));
                }
                EngineStatus::Starting => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0).color(palette.accent));
                        ui.label(components::muted(
                            palette,
                            "Connecting the audio engine…",
                            11.0,
                        ));
                    });
                }
                EngineStatus::Failed(message) => {
                    ui.label(
                        egui::RichText::new("Audio engine failed")
                            .color(palette.error)
                            .strong(),
                    );
                    ui.label(components::muted(palette, message.clone(), 11.0));
                }
                EngineStatus::Ready => {}
            }

            let ready = matches!(engine, EngineStatus::Ready);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let field = ui.add_enabled(
                    ready,
                    egui::TextEdit::singleline(&mut ui_state.debug_uri)
                        .hint_text("spotify:track:…")
                        .desired_width(290.0),
                );
                let submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                let can_play = ready && !ui_state.debug_uri.trim().is_empty();
                let play = ui
                    .add_enabled_ui(can_play, |ui| {
                        components::primary_button(ui, palette, "Play", egui::vec2(72.0, 30.0))
                    })
                    .inner
                    .clicked();

                if (play || submit) && can_play {
                    intent = Some(TransportIntent::PlayUri(
                        ui_state.debug_uri.trim().to_owned(),
                    ));
                }
            });
        });

    intent
}

/// Format a duration as `m:ss`.
fn fmt_duration(d: Duration) -> String {
    let total = d.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}
