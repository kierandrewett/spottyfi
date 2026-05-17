//! A reusable Spotify-style scrubber: a thin track with an accent-filled
//! portion and a hover-revealed knob.
//!
//! The same widget backs both the transport's seek bar and its volume control
//! — the volume control is simply a shorter [`Scrubber`] with no hover-preview.
//!
//! The widget is *value-agnostic*: it works in a normalised `0.0..=1.0`
//! fraction. Callers map that fraction to a [`std::time::Duration`] (seek) or a
//! gain (volume) themselves. It reports whether a drag is in progress so the
//! caller can show the dragged position live and commit (seek) only on release.

use crate::theme::Palette;

/// What a [`Scrubber`] reported after one frame.
///
/// The widget never mutates anything itself; the caller inspects this and
/// decides what to do. `fraction` is the value to *display* this frame (it
/// follows the pointer while dragging); `committed` is `Some` only on the frame
/// the interaction finishes (drag-release or a plain click).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrubberResponse {
    /// The fraction to display this frame, `0.0..=1.0`. While the user drags,
    /// this is the dragged position; otherwise it is the input `fraction`.
    pub fraction: f32,
    /// `true` while the user is actively dragging or pressing the track.
    pub dragging: bool,
    /// The fraction to commit, set only on the frame the interaction ends
    /// (drag-release or click). `None` on every other frame.
    pub committed: Option<f32>,
    /// The fraction the pointer is hovering over, `0.0..=1.0`, when the pointer
    /// is over the widget and not dragging — for a hover-preview cue. `None`
    /// when the pointer is elsewhere.
    pub hover: Option<f32>,
}

/// Per-widget persistent state: whether a press/drag is currently in progress.
///
/// One of these is held by the caller per scrubber instance so the drag
/// survives between frames. It is tiny and `Copy`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScrubberState {
    /// Whether the user is mid-drag on this scrubber.
    dragging: bool,
}

impl ScrubberState {
    /// Whether this scrubber is currently being dragged.
    #[must_use]
    pub fn is_dragging(self) -> bool {
        self.dragging
    }
}

/// A thin, flat progress/seek scrubber styled like the Spotify client.
///
/// Build one, set the geometry knobs, then [`Scrubber::show`] it. The track is
/// a thin rounded capsule; the played portion is filled in the theme accent;
/// the knob is a small circle that appears on hover or while dragging.
pub struct Scrubber<'a> {
    palette: &'a Palette,
    /// A stable id so the drag is tracked per instance.
    id_salt: egui::Id,
    /// Total width the widget occupies.
    width: f32,
    /// Thickness of the track capsule.
    track_thickness: f32,
    /// Radius of the hover/drag knob.
    knob_radius: f32,
    /// Whether the widget is interactive.
    enabled: bool,
    /// An optional explicit widget height. When `None` the height is derived
    /// from the knob radius / track thickness; waveform mode wants a taller
    /// band so the envelope has room to breathe.
    height: Option<f32>,
    /// An optional rolling-waveform envelope: peak amplitudes in `0.0..=1.0`,
    /// oldest first. When set, the track is drawn as a mirrored waveform
    /// instead of a flat capsule — the live Spotify-style seek bar. The
    /// played portion is accent-coloured, the rest dimmed.
    waveform: Option<&'a [f32]>,
    /// How far one mouse-wheel notch moves the value, as a `0.0..=1.0`
    /// fraction. `0.0` disables scroll-to-adjust.
    scroll_step: f32,
}

impl<'a> Scrubber<'a> {
    /// Start a scrubber. `id_salt` must be unique per instance on a frame.
    pub fn new(palette: &'a Palette, id_salt: impl std::hash::Hash) -> Self {
        Self {
            palette,
            id_salt: egui::Id::new(id_salt),
            width: 200.0,
            track_thickness: 4.0,
            knob_radius: 6.0,
            enabled: true,
            height: None,
            waveform: None,
            scroll_step: 0.05,
        }
    }

    /// Set how far one mouse-wheel notch moves the value (`0.0..=1.0`).
    ///
    /// Defaults to `0.05`; pass `0.0` to disable scroll-to-adjust entirely.
    #[must_use]
    pub fn scroll_step(mut self, step: f32) -> Self {
        self.scroll_step = step.clamp(0.0, 1.0);
        self
    }

    /// Override the widget's total height. Useful for waveform mode, where a
    /// taller band gives the envelope room; the default derives the height
    /// from the knob radius and track thickness.
    #[must_use]
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// Draw the track as a live rolling waveform rather than a flat capsule.
    ///
    /// `envelope` is a window of recent peak amplitudes in `0.0..=1.0`, oldest
    /// first — typically a [`spottyfi_audio`-style](crate) audio-tap waveform.
    /// The played portion (left of `fraction`) is painted in the accent
    /// colour, the unplayed portion dimmed. Passing an empty slice falls back
    /// to the plain capsule, so a paused / pre-playback scrubber still reads.
    ///
    /// [`spottyfi_audio`-style]: crate
    #[must_use]
    pub fn waveform(mut self, envelope: &'a [f32]) -> Self {
        self.waveform = if envelope.is_empty() {
            None
        } else {
            Some(envelope)
        };
        self
    }

    /// Set the total width the scrubber occupies.
    #[must_use]
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Set the track capsule thickness.
    #[must_use]
    pub fn track_thickness(mut self, thickness: f32) -> Self {
        self.track_thickness = thickness;
        self
    }

    /// Set the hover/drag knob radius. A `0.0` radius hides the knob entirely.
    #[must_use]
    pub fn knob_radius(mut self, radius: f32) -> Self {
        self.knob_radius = radius;
        self
    }

    /// Enable or disable interaction. A disabled scrubber still draws.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Render the scrubber for `fraction` (`0.0..=1.0`) and report what the
    /// user did. `state` is the caller-held per-instance drag state.
    pub fn show(
        self,
        ui: &mut egui::Ui,
        state: &mut ScrubberState,
        fraction: f32,
    ) -> ScrubberResponse {
        let fraction = fraction.clamp(0.0, 1.0);
        // The widget's allocated rect: full width, tall enough for the knob
        // (or the caller-requested height — waveform mode wants a tall band).
        let height = self
            .height
            .unwrap_or_else(|| (self.knob_radius * 2.0).max(self.track_thickness) + 4.0);
        let sense = if self.enabled {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::hover()
        };
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(self.width, height), egui::Sense::hover());
        // Interact under the caller-supplied stable id so the per-instance
        // drag survives layout reshuffles between frames.
        let response = ui.interact(rect, ui.id().with(self.id_salt), sense);

        // The interactive track spans the rect minus a knob-radius inset at
        // each end, so the knob centre can reach 0.0 and 1.0 without clipping.
        let inset = self.knob_radius.max(self.track_thickness * 0.5);
        let track_left = rect.left() + inset;
        let track_right = rect.right() - inset;
        let track_span = (track_right - track_left).max(1.0);

        // Map a pointer x to a 0..1 fraction along the track.
        let pointer_fraction = |x: f32| ((x - track_left) / track_span).clamp(0.0, 1.0);

        let mut shown = fraction;
        let mut committed = None;
        let mut hover = None;

        if self.enabled {
            if response.drag_started() || (response.is_pointer_button_down_on() && !state.dragging)
            {
                state.dragging = true;
            }
            if state.dragging {
                if let Some(pos) = response
                    .interact_pointer_pos()
                    .or_else(|| ui.ctx().pointer_interact_pos())
                {
                    shown = pointer_fraction(pos.x);
                }
                // End the drag the moment the button is no longer held — this
                // catches a release off-widget or outside the window, which a
                // plain `drag_stopped`/`any_released` check misses and which
                // otherwise leaves the scrubber stuck tracking the pointer.
                if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
                    state.dragging = false;
                    committed = Some(shown);
                }
            } else if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    shown = pointer_fraction(pos.x);
                    committed = Some(shown);
                }
            }
            // Mouse-wheel over the track nudges the value one step per notch.
            // Raw `MouseWheel` events are read (not the smoothed scroll delta)
            // so one physical notch is exactly one discrete step.
            if !state.dragging && response.hovered() && self.scroll_step > 0.0 {
                let scroll: f32 = ui.input(|i| {
                    i.events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                            _ => None,
                        })
                        .sum()
                });
                if scroll != 0.0 {
                    shown = (shown + scroll.signum() * self.scroll_step).clamp(0.0, 1.0);
                    committed = Some(shown);
                }
            }
            if !state.dragging {
                if let Some(pos) = response.hover_pos() {
                    hover = Some(pointer_fraction(pos.x));
                }
            }
        }

        if ui.is_rect_visible(rect) {
            self.paint(
                ui,
                rect,
                track_left,
                track_span,
                shown,
                hover,
                state.dragging,
            );
        }

        if self.enabled {
            response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }

        ScrubberResponse {
            fraction: shown,
            dragging: state.dragging,
            committed,
            hover,
        }
    }

    /// Paint the track, the filled portion and (on hover/drag) the knob.
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        track_left: f32,
        track_span: f32,
        shown: f32,
        hover: Option<f32>,
        dragging: bool,
    ) {
        if let Some(envelope) = self.waveform {
            self.paint_waveform(ui, rect, track_left, track_span, shown, hover, envelope);
            return;
        }

        let painter = ui.painter();
        let cy = rect.center().y;
        let half = self.track_thickness * 0.5;
        let radius = half;

        // The unfilled track — a thin capsule in a muted surface colour.
        let track_rect = egui::Rect::from_min_max(
            egui::pos2(track_left, cy - half),
            egui::pos2(track_left + track_span, cy + half),
        );
        painter.rect_filled(track_rect, radius, self.palette.outline);

        let active = dragging || hover.is_some();

        // A faint hover-preview fill up to the pointer, drawn under the
        // accent fill so the played portion still reads clearly.
        if let Some(h) = hover {
            if !dragging {
                let preview_rect = egui::Rect::from_min_max(
                    egui::pos2(track_left, cy - half),
                    egui::pos2(track_left + track_span * h, cy + half),
                );
                painter.rect_filled(preview_rect, radius, self.palette.text_muted);
            }
        }

        // The played/selected portion in the theme accent.
        let fill_w = track_span * shown;
        if fill_w > 0.0 {
            let fill_rect = egui::Rect::from_min_max(
                egui::pos2(track_left, cy - half),
                egui::pos2(track_left + fill_w, cy + half),
            );
            painter.rect_filled(fill_rect, radius, self.palette.accent);
        }

        // The knob — only while hovered or dragging, like the real client.
        if self.knob_radius > 0.0 && active {
            let knob_x = track_left + fill_w;
            painter.circle_filled(egui::pos2(knob_x, cy), self.knob_radius, self.palette.text);
        }
    }

    /// Paint the track as a live rolling waveform: mirrored amplitude bars,
    /// accent-filled up to `shown` and dimmed beyond it.
    ///
    /// One thin vertical bar is drawn per pixel column of the track; each
    /// column's height is the envelope sample under it. A hover preview
    /// brightens the dimmed portion up to the pointer; the knob is drawn at
    /// the play head.
    #[allow(clippy::too_many_arguments)]
    fn paint_waveform(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        track_left: f32,
        track_span: f32,
        shown: f32,
        hover: Option<f32>,
        envelope: &[f32],
    ) {
        let painter = ui.painter();
        let cy = rect.center().y;
        // The bars fill the widget's full height, leaving a hair of margin so
        // the knob is not clipped. A floor keeps even silence visible as a
        // thin centre line rather than vanishing entirely.
        let max_half = (rect.height() * 0.5 - 1.0).max(1.0);
        let min_half = 0.75_f32;

        // One bar per integer column across the track.
        let columns = track_span.max(1.0) as usize;
        let play_x = track_left + track_span * shown;
        let hover_x = hover.map(|h| track_left + track_span * h);

        for col in 0..columns {
            let x = track_left + col as f32 + 0.5;
            let frac = (col as f32 + 0.5) / track_span;
            let amp = sample_envelope(envelope, frac);
            let half = (min_half + amp * (max_half - min_half)).min(max_half);

            // Colour: accent for the played portion, a faint hover-preview
            // tint up to the pointer, otherwise the dimmed unplayed colour.
            let colour = if x <= play_x {
                self.palette.accent
            } else if hover_x.is_some_and(|hx| x <= hx) {
                self.palette.text_muted
            } else {
                self.palette.outline
            };

            painter.line_segment(
                [egui::pos2(x, cy - half), egui::pos2(x, cy + half)],
                egui::Stroke::new(1.0, colour),
            );
        }

        // The play-head knob, always shown on the waveform so the seek
        // position is unambiguous against the busy bar field.
        if self.knob_radius > 0.0 {
            painter.circle_filled(egui::pos2(play_x, cy), self.knob_radius, self.palette.text);
        }
    }
}

/// Sample a waveform `envelope` at a normalised position `frac` (`0.0..=1.0`).
///
/// `frac` is mapped onto the envelope's index range with nearest-sample
/// lookup. An empty envelope yields `0.0`. Pulled out as a free function so the
/// envelope→bar mapping is unit-tested without an egui context.
fn sample_envelope(envelope: &[f32], frac: f32) -> f32 {
    if envelope.is_empty() {
        return 0.0;
    }
    let last = envelope.len() - 1;
    let idx = (frac.clamp(0.0, 1.0) * last as f32).round() as usize;
    envelope[idx.min(last)].clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    /// The value->position->value round trip the widget relies on. The widget
    /// maps a pointer x within `[track_left, track_left + span]` to a fraction
    /// and back; this mirrors that arithmetic so the math is unit-tested
    /// without an egui context.
    fn fraction_at(x: f32, track_left: f32, span: f32) -> f32 {
        ((x - track_left) / span).clamp(0.0, 1.0)
    }

    fn x_at(fraction: f32, track_left: f32, span: f32) -> f32 {
        track_left + span * fraction.clamp(0.0, 1.0)
    }

    #[test]
    fn fraction_is_clamped_to_unit_range() {
        let (left, span) = (10.0, 100.0);
        assert_eq!(fraction_at(-50.0, left, span), 0.0);
        assert_eq!(fraction_at(10.0, left, span), 0.0);
        assert_eq!(fraction_at(110.0, left, span), 1.0);
        assert_eq!(fraction_at(500.0, left, span), 1.0);
    }

    #[test]
    fn fraction_maps_linearly_across_the_track() {
        let (left, span) = (10.0, 100.0);
        assert!((fraction_at(60.0, left, span) - 0.5).abs() < f32::EPSILON);
        assert!((fraction_at(35.0, left, span) - 0.25).abs() < f32::EPSILON);
        assert!((fraction_at(85.0, left, span) - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn position_round_trips_through_fraction() {
        let (left, span) = (24.0, 256.0);
        for &f in &[0.0_f32, 0.1, 0.333, 0.5, 0.875, 1.0] {
            let back = fraction_at(x_at(f, left, span), left, span);
            assert!((back - f).abs() < 1e-5, "round trip failed for {f}");
        }
    }

    #[test]
    fn position_is_clamped_to_the_track_extent() {
        let (left, span) = (24.0, 256.0);
        assert_eq!(x_at(-1.0, left, span), left);
        assert_eq!(x_at(2.0, left, span), left + span);
    }

    #[test]
    fn sample_envelope_handles_the_empty_case() {
        assert_eq!(super::sample_envelope(&[], 0.5), 0.0);
    }

    #[test]
    fn sample_envelope_maps_the_ends_to_first_and_last() {
        let env = [0.1, 0.4, 0.9, 0.2];
        assert!((super::sample_envelope(&env, 0.0) - 0.1).abs() < f32::EPSILON);
        assert!((super::sample_envelope(&env, 1.0) - 0.2).abs() < f32::EPSILON);
    }

    #[test]
    fn sample_envelope_picks_the_nearest_sample() {
        let env = [0.0, 1.0];
        // Just past the midpoint rounds up to the second sample.
        assert!((super::sample_envelope(&env, 0.6) - 1.0).abs() < f32::EPSILON);
        assert!((super::sample_envelope(&env, 0.4) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn sample_envelope_clamps_out_of_range_input() {
        let env = [0.3, 0.7];
        assert!((super::sample_envelope(&env, -5.0) - 0.3).abs() < f32::EPSILON);
        assert!((super::sample_envelope(&env, 9.0) - 0.7).abs() < f32::EPSILON);
        // An out-of-unit-range amplitude is clamped to 0..1.
        assert_eq!(super::sample_envelope(&[2.0], 0.0), 1.0);
        assert_eq!(super::sample_envelope(&[-1.0], 0.0), 0.0);
    }
}
