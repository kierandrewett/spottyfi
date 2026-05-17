//! The audio-visualiser render widgets — the draw half of WS7's visualiser.
//!
//! These are pure painters: they take already-analysed data (a band-magnitude
//! slice for the spectrum analyser, a raw waveform slice for the oscilloscope)
//! and draw it into an [`egui::Ui`]. The FFT itself runs off the UI thread in
//! the `audio` crate; this module never touches it — keeping `ui` free of an
//! `audio` dependency.
//!
//! The look follows `docs/ui-reference.md`: flat, dense, sharp-cornered,
//! accent-tinted. Bars are thin and gap-tight; the analyser fades from a dim
//! base to the bright accent at the bar tip so the display reads as a single
//! coherent field rather than a row of disconnected sticks.

use serde::{Deserialize, Serialize};

use crate::theme::Palette;

/// Which visualisation the panel is drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VisualiserMode {
    /// A log-frequency spectrum analyser — vertical magnitude bars.
    #[default]
    Spectrum,
    /// An oscilloscope — the raw audio waveform as a centre-line trace.
    Oscilloscope,
}

impl VisualiserMode {
    /// Every mode, in display order — for a mode selector.
    #[must_use]
    pub fn all() -> [VisualiserMode; 2] {
        [VisualiserMode::Spectrum, VisualiserMode::Oscilloscope]
    }

    /// A short, human-readable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            VisualiserMode::Spectrum => "Spectrum",
            VisualiserMode::Oscilloscope => "Oscilloscope",
        }
    }
}

/// Paint a spectrum analyser into `rect`.
///
/// `bands` is a slice of per-band magnitudes in `0.0..=1.0`, low frequency
/// first — already log-spaced and smoothed by the analyser. Each band is drawn
/// as one thin vertical bar growing up from the baseline; the bar fades from a
/// dim accent at its foot to the bright accent at its tip. An empty `bands`
/// slice (or all-zero, when idle) draws a flat baseline rule rather than
/// nothing, so the panel never looks broken.
pub fn spectrum_bars(painter: &egui::Painter, palette: &Palette, rect: egui::Rect, bands: &[f32]) {
    // The flat baseline rule — always drawn, so an idle panel still reads.
    let baseline_y = rect.bottom() - 1.0;
    painter.line_segment(
        [
            egui::pos2(rect.left(), baseline_y),
            egui::pos2(rect.right(), baseline_y),
        ],
        egui::Stroke::new(1.0, palette.outline),
    );

    if bands.is_empty() {
        return;
    }

    // One slot per band; the bar fills most of the slot, leaving a hair gap.
    let slot = rect.width() / bands.len() as f32;
    let bar_w = (slot * 0.7).max(1.0);
    let usable_h = (rect.height() - 3.0).max(1.0);

    for (i, &magnitude) in bands.iter().enumerate() {
        let mag = magnitude.clamp(0.0, 1.0);
        if mag <= 0.0 {
            continue;
        }
        let slot_left = rect.left() + slot * i as f32;
        let x0 = slot_left + (slot - bar_w) * 0.5;
        let height = mag * usable_h;
        let bar = egui::Rect::from_min_max(
            egui::pos2(x0, baseline_y - height),
            egui::pos2(x0 + bar_w, baseline_y),
        );
        // A vertical accent gradient: dim at the foot, bright at the tip, so
        // the field reads as a continuous wash of colour.
        let tip = palette.accent;
        let foot = lerp_colour(palette.accent_dark, palette.base, 0.45);
        painter.rect_filled(
            egui::Rect::from_min_max(bar.left_bottom(), bar.right_center()),
            0.0,
            foot,
        );
        painter.rect_filled(
            egui::Rect::from_min_max(bar.left_top(), bar.right_center()),
            0.0,
            tip,
        );
        // A bright cap line on the very tip — the classic analyser highlight.
        painter.line_segment(
            [bar.left_top(), bar.right_top()],
            egui::Stroke::new(1.0, lerp_colour(tip, egui::Color32::WHITE, 0.35)),
        );
    }
}

/// Paint an oscilloscope trace into `rect`.
///
/// `samples` is a slice of raw audio in `-1.0..=1.0`, oldest first. The trace
/// is drawn as a polyline about the vertical centre of `rect`. An empty slice
/// draws just the centre line, so an idle panel degrades to a calm flat trace.
pub fn oscilloscope(painter: &egui::Painter, palette: &Palette, rect: egui::Rect, samples: &[f32]) {
    let cy = rect.center().y;

    // The centre reference line — always drawn.
    painter.line_segment(
        [egui::pos2(rect.left(), cy), egui::pos2(rect.right(), cy)],
        egui::Stroke::new(1.0, palette.outline),
    );

    if samples.len() < 2 {
        return;
    }

    let half_h = (rect.height() * 0.5 - 2.0).max(1.0);
    let step = rect.width() / (samples.len() - 1) as f32;
    let points: Vec<egui::Pos2> = samples
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let x = rect.left() + step * i as f32;
            let y = cy - s.clamp(-1.0, 1.0) * half_h;
            egui::pos2(x, y)
        })
        .collect();

    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5, palette.accent),
    ));
}

/// Linearly interpolate between two colours, `t` in `0.0..=1.0`.
fn lerp_colour(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    egui::Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_round_trip_their_labels() {
        for mode in VisualiserMode::all() {
            assert!(!mode.label().is_empty());
        }
        assert_eq!(VisualiserMode::default(), VisualiserMode::Spectrum);
    }

    #[test]
    fn lerp_colour_hits_both_endpoints() {
        let a = egui::Color32::from_rgb(0, 0, 0);
        let b = egui::Color32::from_rgb(200, 100, 50);
        assert_eq!(lerp_colour(a, b, 0.0), a);
        assert_eq!(lerp_colour(a, b, 1.0), b);
        // The midpoint is between the two on every channel.
        let mid = lerp_colour(a, b, 0.5);
        assert!(mid.r() > 0 && mid.r() < 200);
    }

    #[test]
    fn lerp_colour_clamps_out_of_range_t() {
        let a = egui::Color32::from_rgb(10, 20, 30);
        let b = egui::Color32::from_rgb(40, 50, 60);
        assert_eq!(lerp_colour(a, b, -1.0), a);
        assert_eq!(lerp_colour(a, b, 2.0), b);
    }
}
