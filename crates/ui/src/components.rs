//! Reusable egui widgets shared across Spottyfi's shell and pages.
//!
//! Deliberately small for Phase 4 — section headers, an album-art widget, icon
//! buttons and a few text helpers. Track rows, cards and tables arrive with the
//! page system in Phase 5.

use crate::theme::Palette;

/// Row density — the vertical rhythm of list-style content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Density {
    /// Roomier rows; the default.
    #[default]
    Comfortable,
    /// Tighter rows for power users / information density.
    Compact,
}

impl Density {
    /// A human-readable label for a settings toggle.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Density::Comfortable => "Comfortable",
            Density::Compact => "Compact",
        }
    }

    /// The natural row height in points for this density.
    #[must_use]
    pub fn row_height(self) -> f32 {
        match self {
            Density::Comfortable => 48.0,
            Density::Compact => 34.0,
        }
    }

    /// Toggle to the other density.
    #[must_use]
    pub fn toggled(self) -> Density {
        match self {
            Density::Comfortable => Density::Compact,
            Density::Compact => Density::Comfortable,
        }
    }
}

/// A section header: a bold title with an optional muted trailing caption.
pub fn section_header(ui: &mut egui::Ui, palette: &Palette, title: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(title)
            .family(crate::fonts::semibold())
            .size(15.0)
            .color(palette.text),
    );
    ui.add_space(4.0);
}

/// Muted secondary text at the given point size.
pub fn muted(palette: &Palette, text: impl Into<String>, size: f32) -> egui::RichText {
    egui::RichText::new(text.into())
        .color(palette.text_muted)
        .size(size)
}

/// An album-art widget that renders a remote image URL, falling back to a
/// rounded placeholder while it loads or when no URL is available.
///
/// Remote URLs resolve through the [`crate::image_loader`] installed at
/// startup, so callers just pass the `https://i.scdn.co/...` URL.
pub fn album_art(
    ui: &mut egui::Ui,
    palette: &Palette,
    url: Option<&str>,
    size: f32,
    corner_radius: f32,
) -> egui::Response {
    let desired = egui::vec2(size, size);
    match url {
        Some(url) if !url.is_empty() => ui.add(
            egui::Image::from_uri(url.to_owned())
                .fit_to_exact_size(desired)
                .corner_radius(corner_radius)
                .show_loading_spinner(true),
        ),
        _ => {
            let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, corner_radius, palette.elevated);
            // A simple note-ish glyph centred in the placeholder.
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "\u{266B}",
                egui::FontId::proportional(size * 0.4),
                palette.text_muted,
            );
            response
        }
    }
}

/// A compact, frameless icon button rendered from a glyph string.
///
/// `active` tints the glyph with the accent colour (e.g. for a toggled
/// shuffle/repeat control). Returns the click [`egui::Response`].
pub fn icon_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    glyph: &str,
    size: f32,
    active: bool,
    tooltip: &str,
) -> egui::Response {
    let color = if active {
        palette.accent
    } else {
        palette.text_muted
    };
    let button = egui::Button::new(egui::RichText::new(glyph).size(size).color(color))
        .frame(false)
        .min_size(egui::vec2(size + 12.0, size + 8.0));
    let response = ui
        .add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// A round, filled accent button — Spotify's primary call-to-action shape.
pub fn primary_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    min_size: egui::Vec2,
) -> egui::Response {
    let button = egui::Button::new(
        egui::RichText::new(label)
            .family(crate::fonts::semibold())
            .size(14.0)
            .color(egui::Color32::BLACK),
    )
    .fill(palette.accent)
    .corner_radius(min_size.y / 2.0)
    .min_size(min_size);
    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small pill-shaped filter chip. `selected` draws it filled.
pub fn filter_chip(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    selected: bool,
) -> egui::Response {
    let (bg, fg) = if selected {
        (palette.text, egui::Color32::BLACK)
    } else {
        (palette.card, palette.text)
    };
    let button = egui::Button::new(egui::RichText::new(label).size(12.5).color(fg))
        .fill(bg)
        .corner_radius(14.0)
        .min_size(egui::vec2(0.0, 26.0));
    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_toggles_and_round_trips() {
        assert_eq!(Density::Comfortable.toggled(), Density::Compact);
        assert_eq!(Density::Compact.toggled(), Density::Comfortable);
        assert!(Density::Compact.row_height() < Density::Comfortable.row_height());
    }
}
