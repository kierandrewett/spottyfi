//! The Spottyfi colour palette and theme application.
//!
//! Two selectable dark themes are provided: [`Theme::SpotifyDark`], a faithful
//! near-black Spotify-like palette, and [`Theme::TealLilac`], an alternate dark
//! theme built on a teal-green accent with lilac highlights. Both are applied
//! by mutating an [`egui::Style`] / [`egui::Visuals`] via [`Theme::apply`].

use serde::{Deserialize, Serialize};

/// A named colour palette plus the metrics needed to dress an [`egui::Style`].
///
/// A `Palette` is a plain value: it carries no egui state. [`Theme::palette`]
/// returns the palette for a theme, and [`Theme::apply`] installs it.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// The window base background — the darkest surface.
    pub base: egui::Color32,
    /// Card / panel background, one step lighter than [`Self::base`].
    pub card: egui::Color32,
    /// Elevated surface (menus, popups, the transport bar).
    pub elevated: egui::Color32,
    /// A hovered surface tint, between [`Self::card`] and [`Self::elevated`].
    pub hover: egui::Color32,
    /// The brand accent colour (play buttons, active states).
    pub accent: egui::Color32,
    /// A darker accent for pressed / hovered accent controls.
    pub accent_dark: egui::Color32,
    /// Primary text colour.
    pub text: egui::Color32,
    /// Muted secondary text colour.
    pub text_muted: egui::Color32,
    /// Error / destructive colour.
    pub error: egui::Color32,
    /// Hairline separators and inactive control outlines.
    pub outline: egui::Color32,
}

/// The selectable dark themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Theme {
    /// The faithful Spotify-like near-black palette with the green accent.
    #[default]
    SpotifyDark,
    /// An alternate dark theme: teal-green accent with lilac highlights.
    TealLilac,
    /// A pure-black (AMOLED-friendly) palette with the Spotify-green accent.
    Amoled,
    /// The Nord palette: a cool blue-grey base with a frost-blue accent.
    Nord,
}

impl Theme {
    /// A human-readable label, e.g. for a settings dropdown.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Theme::SpotifyDark => "Spotify Dark",
            Theme::TealLilac => "Teal & Lilac",
            Theme::Amoled => "AMOLED Black",
            Theme::Nord => "Nord",
        }
    }

    /// Every theme variant, in display order.
    #[must_use]
    pub fn all() -> [Theme; 4] {
        [
            Theme::SpotifyDark,
            Theme::TealLilac,
            Theme::Amoled,
            Theme::Nord,
        ]
    }

    /// The colour palette for this theme.
    #[must_use]
    pub fn palette(self) -> Palette {
        match self {
            Theme::SpotifyDark => Palette {
                base: egui::Color32::from_rgb(0x12, 0x12, 0x12),
                card: egui::Color32::from_rgb(0x18, 0x18, 0x18),
                elevated: egui::Color32::from_rgb(0x1f, 0x1f, 0x1f),
                hover: egui::Color32::from_rgb(0x2a, 0x2a, 0x2a),
                accent: egui::Color32::from_rgb(0x1e, 0xd7, 0x60),
                accent_dark: egui::Color32::from_rgb(0x1a, 0xbf, 0x54),
                text: egui::Color32::from_rgb(0xff, 0xff, 0xff),
                text_muted: egui::Color32::from_rgb(0xb3, 0xb3, 0xb3),
                error: egui::Color32::from_rgb(0xf1, 0x5e, 0x6c),
                outline: egui::Color32::from_rgb(0x3a, 0x3a, 0x3a),
            },
            Theme::TealLilac => Palette {
                base: egui::Color32::from_rgb(0x10, 0x16, 0x1a),
                card: egui::Color32::from_rgb(0x17, 0x20, 0x25),
                elevated: egui::Color32::from_rgb(0x1f, 0x2b, 0x31),
                hover: egui::Color32::from_rgb(0x2a, 0x39, 0x40),
                accent: egui::Color32::from_rgb(0x2d, 0xd4, 0xbf),
                accent_dark: egui::Color32::from_rgb(0x24, 0xb3, 0xa1),
                text: egui::Color32::from_rgb(0xf2, 0xee, 0xff),
                text_muted: egui::Color32::from_rgb(0xa6, 0x9f, 0xc4),
                error: egui::Color32::from_rgb(0xf1, 0x6c, 0x8b),
                outline: egui::Color32::from_rgb(0x39, 0x44, 0x55),
            },
            Theme::Amoled => Palette {
                base: egui::Color32::from_rgb(0x00, 0x00, 0x00),
                card: egui::Color32::from_rgb(0x0b, 0x0b, 0x0b),
                elevated: egui::Color32::from_rgb(0x15, 0x15, 0x15),
                hover: egui::Color32::from_rgb(0x24, 0x24, 0x24),
                accent: egui::Color32::from_rgb(0x1e, 0xd7, 0x60),
                accent_dark: egui::Color32::from_rgb(0x1a, 0xbf, 0x54),
                text: egui::Color32::from_rgb(0xff, 0xff, 0xff),
                text_muted: egui::Color32::from_rgb(0x9a, 0x9a, 0x9a),
                error: egui::Color32::from_rgb(0xf1, 0x5e, 0x6c),
                outline: egui::Color32::from_rgb(0x2b, 0x2b, 0x2b),
            },
            Theme::Nord => Palette {
                base: egui::Color32::from_rgb(0x2e, 0x34, 0x40),
                card: egui::Color32::from_rgb(0x34, 0x3b, 0x49),
                elevated: egui::Color32::from_rgb(0x3b, 0x42, 0x52),
                hover: egui::Color32::from_rgb(0x43, 0x4c, 0x5e),
                accent: egui::Color32::from_rgb(0x88, 0xc0, 0xd0),
                accent_dark: egui::Color32::from_rgb(0x81, 0xa1, 0xc1),
                text: egui::Color32::from_rgb(0xec, 0xef, 0xf4),
                text_muted: egui::Color32::from_rgb(0x9a, 0xa5, 0xb9),
                error: egui::Color32::from_rgb(0xbf, 0x61, 0x6a),
                outline: egui::Color32::from_rgb(0x4c, 0x56, 0x6a),
            },
        }
    }

    /// Apply this theme to an egui context, replacing its [`egui::Style`].
    ///
    /// Sets the visuals (colours, rounding, selection), spacing, and the
    /// scrollbar / widget styling that gives Spottyfi its dense dark look.
    pub fn apply(self, ctx: &egui::Context) {
        let palette = self.palette();
        let mut style = (*ctx.global_style()).clone();
        apply_palette(&mut style, &palette);
        ctx.set_global_style(style);
    }
}

/// Dress an [`egui::Style`] in-place with `palette`.
///
/// The Spottyfi look is a flat Dear-ImGui-style application: corner radius is
/// `0` everywhere, there are no widget drop shadows, and spacing is tight.
fn apply_palette(style: &mut egui::Style, palette: &Palette) {
    let mut visuals = egui::Visuals::dark();

    visuals.dark_mode = true;
    visuals.override_text_color = Some(palette.text);
    visuals.panel_fill = palette.base;
    visuals.window_fill = palette.elevated;
    visuals.extreme_bg_color = palette.base;
    visuals.faint_bg_color = palette.card;
    visuals.code_bg_color = palette.card;
    visuals.hyperlink_color = palette.accent;
    visuals.warn_fg_color = palette.error;
    visuals.error_fg_color = palette.error;

    visuals.window_stroke = egui::Stroke::new(1.0, palette.outline);

    // Sharp corners everywhere — this is a flat ImGui-style application.
    let sharp = egui::CornerRadius::ZERO;
    visuals.window_corner_radius = sharp;
    visuals.menu_corner_radius = sharp;

    // Flat: a faint popup shadow only (a 1px outline carries depth instead).
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: egui::Color32::from_black_alpha(120),
    };
    visuals.window_shadow = egui::epaint::Shadow::NONE;

    // The selection highlight is a flat, slightly-lighter fill — no glow.
    visuals.selection.bg_fill = palette.hover;
    visuals.selection.stroke = egui::Stroke::NONE;

    // Inactive (resting) widgets.
    visuals.widgets.noninteractive.bg_fill = palette.base;
    visuals.widgets.noninteractive.weak_bg_fill = palette.base;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, palette.outline);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, palette.text_muted);
    visuals.widgets.noninteractive.corner_radius = sharp;

    visuals.widgets.inactive.bg_fill = palette.card;
    visuals.widgets.inactive.weak_bg_fill = palette.card;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, palette.text);
    visuals.widgets.inactive.corner_radius = sharp;

    visuals.widgets.hovered.bg_fill = palette.hover;
    visuals.widgets.hovered.weak_bg_fill = palette.hover;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, palette.outline);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, palette.text);
    visuals.widgets.hovered.corner_radius = sharp;

    visuals.widgets.active.bg_fill = palette.elevated;
    visuals.widgets.active.weak_bg_fill = palette.elevated;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, palette.outline);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, palette.text);
    visuals.widgets.active.corner_radius = sharp;

    visuals.widgets.open.bg_fill = palette.elevated;
    visuals.widgets.open.weak_bg_fill = palette.elevated;
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, palette.outline);
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, palette.text);
    visuals.widgets.open.corner_radius = sharp;

    style.visuals = visuals;

    // Spacing — dense, but with enough room that controls never feel cramped.
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    // Roomier buttons; this also sets the height of menu entries, so menus
    // read comfortably instead of feeling tight.
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    // No inner margin on menus — entries run edge to edge inside the menu's
    // outline; each entry's own button padding gives the breathing room.
    style.spacing.menu_margin = egui::Margin::ZERO;
    // A comfortable minimum interactive height so menu entries, combo boxes
    // and small buttons are easy to hit and evenly sized.
    style.spacing.interact_size.y = 22.0;
    // A touch more gap between a checkbox/radio and its label.
    style.spacing.icon_spacing = 6.0;
    style.spacing.scroll = egui::style::ScrollStyle::thin();
    // egui 0.34 paints a dark fade gradient at a scroll area's scrolled edge;
    // it reads as an unwanted shadow creeping over content while scrolling.
    // Disable it — the app is flat and the scrollbar already signals overflow.
    style.spacing.scroll.fade.strength = 0.0;

    // Immediate, non-animated scrolling. This kills the lerp used by
    // `scroll_to_*` (jump-to-tab, focus-follows-selection) so navigation lands
    // instantly. egui 0.34 has no public knob for mouse-wheel input smoothing
    // itself — that remains hardcoded in `egui::input_state` — so a faint
    // wheel-momentum tail can still be observed; everything programmatic is
    // now instant.
    style.scroll_animation = egui::style::ScrollAnimation::none();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_has_a_label() {
        for theme in Theme::all() {
            assert!(!theme.label().is_empty());
        }
    }

    #[test]
    fn theme_round_trips_through_serde() {
        for theme in Theme::all() {
            let json = serde_json::to_string(&theme).expect("serialise");
            let back: Theme = serde_json::from_str(&json).expect("deserialise");
            assert_eq!(theme, back);
        }
    }

    #[test]
    fn default_theme_is_spotify_dark() {
        assert_eq!(Theme::default(), Theme::SpotifyDark);
    }
}
