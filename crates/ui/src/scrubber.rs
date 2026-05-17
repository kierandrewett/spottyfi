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
        }
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
        // The widget's allocated rect: full width, tall enough for the knob.
        let height = (self.knob_radius * 2.0).max(self.track_thickness) + 4.0;
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
            }
            if response.drag_stopped() || (state.dragging && ui.input(|i| i.pointer.any_released()))
            {
                state.dragging = false;
                committed = Some(shown);
            } else if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    shown = pointer_fraction(pos.x);
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
}
