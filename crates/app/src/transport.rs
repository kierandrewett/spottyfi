//! The bottom transport bar and the debug "play a URI" control.
//!
//! Phase 4 promotes the transport to the real, polished bar: now-playing album
//! art (loaded from the live URL via the `ui` crate's network image loader),
//! title/artist, centred controls (shuffle, prev, play/pause, next, repeat), a
//! progress scrubber with elapsed/total readouts, and a right cluster of
//! lyrics/queue/devices toggle placeholders plus a volume slider.
//!
//! Shuffle, repeat and the right-cluster toggles are visual placeholders this
//! phase — they have no engine wiring until later phases — but they are themed
//! and laid out so the bar reads as complete.

use std::time::Duration;

use spottyfi_audio::PlaybackState;
use spottyfi_ui::components;
use spottyfi_ui::icons::{self, Icon};
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
}

/// Per-frame, mutable UI state for the transport widgets.
///
/// Held by the app so the scrubber's in-drag value, the debug field, and the
/// state of the (not-yet-wired) shuffle / repeat toggles survive between
/// frames.
#[derive(Default)]
pub struct TransportUiState {
    /// The track URI typed into the debug control.
    pub debug_uri: String,
    /// While the user drags the scrubber, the in-progress position fraction;
    /// `None` when not dragging, so the bar follows live playback.
    scrub: Option<f32>,
    /// Visual-only shuffle toggle (no engine wiring yet).
    shuffle: bool,
    /// Visual-only repeat toggle (no engine wiring yet).
    repeat: bool,
}

/// Render the bottom transport bar. Returns any [`TransportIntent`] issued.
///
/// `palette` themes the bar; `playback` is the live snapshot the controls
/// project.
pub fn transport_bar(
    ui: &mut egui::Ui,
    palette: &Palette,
    ui_state: &mut TransportUiState,
    playback: &PlaybackState,
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
            ui.horizontal_centered(|ui| {
                // Left third: now-playing art + title/artist.
                let side = (ui.available_width() * 0.28).clamp(180.0, 360.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(side, TRANSPORT_HEIGHT - 16.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| now_playing(ui, palette, playback),
                );

                // Right cluster reserves a fixed slice; the centre takes the rest.
                let right_width = 210.0;
                let centre_width = (ui.available_width() - right_width).max(220.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(centre_width, TRANSPORT_HEIGHT - 16.0),
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        if let Some(i) = centre_controls(ui, palette, ui_state, playback) {
                            intent = Some(i);
                        }
                    },
                );

                // Right cluster: toggle placeholders + volume.
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width().max(120.0), TRANSPORT_HEIGHT - 16.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if let Some(i) = right_cluster(ui, palette, playback) {
                            intent = Some(i);
                        }
                    },
                );
            });
        });

    intent
}

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
                ui.label(components::muted(palette, "Ogg Vorbis 320 kbps", 9.5));
            }
            None => {
                ui.label(components::muted(palette, "Nothing playing", 12.5));
            }
        }
    });
}

/// The centre block: a control row above a seek scrubber, both centred
/// horizontally and the pair centred vertically within the transport band.
fn centre_controls(
    ui: &mut egui::Ui,
    palette: &Palette,
    ui_state: &mut TransportUiState,
    playback: &PlaybackState,
) -> Option<TransportIntent> {
    let mut intent = None;

    // The two stacked rows have a fixed combined height; lay them out in a
    // top-down block that the enclosing centred-and-justified layout centres
    // vertically. The block spans the full centre width so the scrubber and
    // the (horizontally-centred) control row share the same axis.
    const CONTROL_ROW_H: f32 = 30.0;
    const SCRUBBER_ROW_H: f32 = 16.0;
    const ROW_GAP: f32 = 4.0;
    let block_height = CONTROL_ROW_H + ROW_GAP + SCRUBBER_ROW_H;
    let width = ui.available_width();

    ui.allocate_ui_with_layout(
        egui::vec2(width, block_height),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            // The transport control row, centred horizontally.
            if let Some(i) = control_row(ui, palette, ui_state, playback, CONTROL_ROW_H) {
                intent = Some(i);
            }
            // The seek scrubber row, spanning the full block width.
            if let Some(i) = scrubber_row(ui, palette, ui_state, playback, width) {
                intent = Some(i);
            }
        },
    );

    intent
}

/// The shuffle / prev / play-pause / next / repeat control row, sized to a
/// fixed height and centred horizontally by the enclosing top-down layout.
fn control_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    ui_state: &mut TransportUiState,
    playback: &PlaybackState,
    height: f32,
) -> Option<TransportIntent> {
    let mut intent = None;
    let has_track = playback.track.is_some();

    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
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
                        ui_state.shuffle,
                        "Shuffle",
                    )
                    .clicked()
                    {
                        ui_state.shuffle = !ui_state.shuffle;
                    }
                    icons::icon_button(ui, palette, Icon::SkipBack, 16.0, false, "Previous");

                    // The play/pause control: the one accent-green element.
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(30.0, 30.0),
                        if has_track {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        },
                    );
                    if ui.is_rect_visible(rect) {
                        let fill = if has_track {
                            palette.accent
                        } else {
                            palette.outline
                        };
                        ui.painter().rect_filled(rect, 0, fill);
                        let glyph = if playback.playing {
                            Icon::Pause
                        } else {
                            Icon::Play
                        };
                        let g = 14.0;
                        glyph.image(g, egui::Color32::BLACK).paint_at(
                            ui,
                            egui::Rect::from_center_size(rect.center(), egui::vec2(g, g)),
                        );
                    }
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        intent = Some(TransportIntent::TogglePlayPause);
                    }

                    icons::icon_button(ui, palette, Icon::SkipForward, 16.0, false, "Next");
                    if icons::icon_button(
                        ui,
                        palette,
                        Icon::Repeat,
                        15.0,
                        ui_state.repeat,
                        "Repeat",
                    )
                    .clicked()
                    {
                        ui_state.repeat = !ui_state.repeat;
                    }
                },
            );
        },
    );

    intent
}

/// The seek scrubber row: elapsed / total readouts flanking a progress slider.
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
    let mut shown = ui_state.scrub.unwrap_or(live_fraction);

    let position = if ui_state.scrub.is_some() {
        Duration::from_secs_f32(shown * duration.as_secs_f32())
    } else {
        playback.position
    };

    ui.allocate_ui_with_layout(
        egui::vec2(width, SCRUBBER_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(components::muted(palette, fmt_duration(position), 10.5));

            let track_width = (ui.available_width() - 44.0).max(80.0);
            ui.spacing_mut().slider_width = track_width;
            let slider = ui.add_enabled(
                !duration.is_zero(),
                egui::Slider::new(&mut shown, 0.0..=1.0)
                    .show_value(false)
                    .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.5 }),
            );

            if slider.drag_started() || slider.dragged() {
                ui_state.scrub = Some(shown);
            }
            if slider.drag_stopped() || slider.clicked() {
                let target = Duration::from_secs_f32(shown * duration.as_secs_f32());
                intent = Some(TransportIntent::Seek(target));
                ui_state.scrub = None;
            }

            ui.label(components::muted(palette, fmt_duration(duration), 10.5));
        },
    );

    intent
}

/// The fixed height of the seek-scrubber row.
const SCRUBBER_HEIGHT: f32 = 16.0;

/// The right cluster: lyrics/queue/devices toggle placeholders + volume.
fn right_cluster(
    ui: &mut egui::Ui,
    palette: &Palette,
    playback: &PlaybackState,
) -> Option<TransportIntent> {
    let mut intent = None;

    // Volume slider (right-to-left layout, so this lands rightmost).
    let mut volume = playback.volume;
    ui.spacing_mut().slider_width = 80.0;
    let response = ui.add(
        egui::Slider::new(&mut volume, 0.0..=1.0)
            .show_value(false)
            .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.5 }),
    );
    if response.changed() {
        intent = Some(TransportIntent::SetVolume(volume));
    }
    let vol_icon = if playback.volume <= 0.001 {
        Icon::VolumeMuted
    } else {
        Icon::Volume
    };
    icons::icon_button(ui, palette, vol_icon, 15.0, false, "Volume");

    ui.add_space(2.0);
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
