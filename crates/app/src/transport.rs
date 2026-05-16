//! The bottom transport bar and the debug "play a URI" control.
//!
//! Both are wired to real playback state and controls. They are deliberately
//! plain — Phase 4 builds the polished dock shell — but functional: the
//! scrubber emits a seek on drag-release, the volume slider drives the mixer,
//! and the debug field demonstrates playback before the browsing UI exists.

use std::time::Duration;

use spottyfi_audio::PlaybackState;

use crate::playback_controller::EngineStatus;

/// Height of the transport bar, per `PLAN.md`'s UI shell spec.
pub const TRANSPORT_HEIGHT: f32 = 76.0;

/// Elevated panel grey (`#1f1f1f`).
const ELEVATED: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x1f, 0x1f);
/// Card grey, used for the art placeholder (`#181818`).
const CARD: egui::Color32 = egui::Color32::from_rgb(0x28, 0x28, 0x28);
/// Accent green (`#1ed760`).
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x1e, 0xd7, 0x60);
/// Muted secondary text.
const MUTED: egui::Color32 = egui::Color32::from_rgb(0xb3, 0xb3, 0xb3);
/// Error red.
const ERROR: egui::Color32 = egui::Color32::from_rgb(0xf1, 0x5e, 0x6c);

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
/// Held by the app so the scrubber's in-drag value and the debug text field
/// survive between frames.
#[derive(Default)]
pub struct TransportUiState {
    /// The track URI typed into the debug control.
    pub debug_uri: String,
    /// While the user drags the scrubber, the in-progress position fraction;
    /// `None` when not dragging, so the bar follows live playback.
    scrub: Option<f32>,
}

/// Render the bottom transport bar. Returns any [`TransportIntent`] issued.
pub fn transport_bar(
    ui: &mut egui::Ui,
    ui_state: &mut TransportUiState,
    playback: &PlaybackState,
) -> Option<TransportIntent> {
    let mut intent = None;

    egui::Panel::bottom("transport")
        .exact_size(TRANSPORT_HEIGHT)
        .frame(egui::Frame::new().fill(ELEVATED).inner_margin(10.0))
        .show_inside(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // Left: now-playing art + title/artist.
                now_playing(ui, playback);

                ui.add_space(16.0);

                // Centre: play/pause + scrubber, taking the remaining width
                // minus a fixed slice for the volume control on the right.
                let volume_width = 160.0;
                let centre_width = (ui.available_width() - volume_width).max(180.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(centre_width, TRANSPORT_HEIGHT - 20.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        if let Some(i) = centre_controls(ui, ui_state, playback) {
                            intent = Some(i);
                        }
                    },
                );

                // Right: volume slider.
                if let Some(i) = volume_control(ui, playback) {
                    intent = Some(i);
                }
            });
        });

    intent
}

/// The now-playing block: an art placeholder plus title and artist.
fn now_playing(ui: &mut egui::Ui, playback: &PlaybackState) {
    // Art placeholder. Network art loading arrives with the image loaders in
    // Phase 4; `playback.track.art_url` is already populated for then.
    let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 4.0, CARD);

    ui.add_space(10.0);
    ui.vertical(|ui| {
        ui.add_space(6.0);
        match &playback.track {
            Some(track) => {
                ui.label(
                    egui::RichText::new(&track.title)
                        .color(egui::Color32::WHITE)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(track.artist_line())
                        .color(MUTED)
                        .size(12.0),
                );
            }
            None => {
                ui.label(egui::RichText::new("Nothing playing").color(MUTED));
            }
        }
    });
}

/// The centre block: a play/pause button above a seek scrubber.
fn centre_controls(
    ui: &mut egui::Ui,
    ui_state: &mut TransportUiState,
    playback: &PlaybackState,
) -> Option<TransportIntent> {
    let mut intent = None;

    // egui's default font lacks the media glyphs; use plain text labels.
    let label = if playback.buffering {
        "Loading…"
    } else if playback.playing {
        "Pause"
    } else {
        "Play"
    };

    let button = egui::Button::new(
        egui::RichText::new(label)
            .color(egui::Color32::BLACK)
            .strong(),
    )
    .fill(ACCENT)
    .corner_radius(16.0)
    .min_size(egui::vec2(84.0, 28.0));
    if ui
        .add_enabled(playback.track.is_some(), button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        intent = Some(TransportIntent::TogglePlayPause);
    }

    ui.add_space(4.0);

    // Seek scrubber. While dragging we hold a local fraction so the thumb
    // tracks the pointer; on release we emit a single Seek.
    let duration = playback
        .track
        .as_ref()
        .map_or(Duration::ZERO, |t| t.duration);
    let live_fraction = playback.progress_fraction();
    let mut shown = ui_state.scrub.unwrap_or(live_fraction);

    ui.spacing_mut().slider_width = ui.available_width().max(120.0);
    let slider = ui.add_enabled(
        !duration.is_zero(),
        egui::Slider::new(&mut shown, 0.0..=1.0)
            .show_value(false)
            .handle_shape(egui::style::HandleShape::Circle),
    );

    if slider.drag_started() || slider.dragged() {
        ui_state.scrub = Some(shown);
    }
    if slider.drag_stopped() {
        let target = Duration::from_secs_f32(shown * duration.as_secs_f32());
        intent = Some(TransportIntent::Seek(target));
        ui_state.scrub = None;
    } else if slider.clicked() {
        // A plain click on the track also seeks.
        let target = Duration::from_secs_f32(shown * duration.as_secs_f32());
        intent = Some(TransportIntent::Seek(target));
        ui_state.scrub = None;
    }

    // Elapsed / total readout.
    let position = if ui_state.scrub.is_some() {
        Duration::from_secs_f32(shown * duration.as_secs_f32())
    } else {
        playback.position
    };
    ui.label(
        egui::RichText::new(format!(
            "{} / {}",
            fmt_duration(position),
            fmt_duration(duration)
        ))
        .color(MUTED)
        .size(11.0),
    );

    intent
}

/// The right-hand volume slider.
fn volume_control(ui: &mut egui::Ui, playback: &PlaybackState) -> Option<TransportIntent> {
    let mut intent = None;
    let mut volume = playback.volume;
    ui.vertical(|ui| {
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Vol").color(MUTED).size(11.0));
            ui.spacing_mut().slider_width = 110.0;
            let response = ui.add(
                egui::Slider::new(&mut volume, 0.0..=1.0)
                    .show_value(false)
                    .handle_shape(egui::style::HandleShape::Circle),
            );
            if response.changed() {
                intent = Some(TransportIntent::SetVolume(volume));
            }
        });
    });
    intent
}

/// The debug control: a URI field plus a Play button, shown in the logged-in
/// view so playback can be exercised before the browsing UI exists (Phase 5).
pub fn debug_play_control(
    ui: &mut egui::Ui,
    ui_state: &mut TransportUiState,
    engine: &EngineStatus,
) -> Option<TransportIntent> {
    let mut intent = None;

    ui.group(|ui| {
        ui.set_max_width(440.0);
        ui.label(
            egui::RichText::new("Debug — play a track")
                .color(egui::Color32::WHITE)
                .strong(),
        );
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("Paste a spotify:track: URI or an open.spotify.com link.")
                .color(MUTED)
                .size(11.0),
        );
        ui.add_space(6.0);

        match engine {
            EngineStatus::Idle => {
                ui.label(
                    egui::RichText::new("Audio engine not started.")
                        .color(MUTED)
                        .size(11.0),
                );
            }
            EngineStatus::Starting => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
                    ui.label(
                        egui::RichText::new("Connecting the audio engine…")
                            .color(MUTED)
                            .size(11.0),
                    );
                });
            }
            EngineStatus::Failed(message) => {
                ui.label(
                    egui::RichText::new("Audio engine failed")
                        .color(ERROR)
                        .strong(),
                );
                ui.label(egui::RichText::new(message).color(MUTED).size(11.0));
            }
            EngineStatus::Ready => {}
        }

        let ready = matches!(engine, EngineStatus::Ready);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let field = ui.add_enabled(
                ready,
                egui::TextEdit::singleline(&mut ui_state.debug_uri)
                    .hint_text("spotify:track:…")
                    .desired_width(280.0),
            );
            let submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

            let play = ui
                .add_enabled(
                    ready && !ui_state.debug_uri.trim().is_empty(),
                    egui::Button::new(
                        egui::RichText::new("Play")
                            .color(egui::Color32::BLACK)
                            .strong(),
                    )
                    .fill(ACCENT)
                    .corner_radius(14.0),
                )
                .clicked();

            if (play || submit) && !ui_state.debug_uri.trim().is_empty() {
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
