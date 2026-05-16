//! The left sidebar: "Your Library" with Liked Songs pinned at the top and the
//! user's real playlists below.
//!
//! The playlist list is loaded asynchronously when the API is attached
//! (see [`ShellState::attach_api`]); the sidebar draws a spinner until it
//! resolves. Clicking an entry pushes a navigation [`Tab`] onto the frame's
//! `nav` list, which the shell applies to the dock.

use spottyfi_models::{SimplifiedPlaylist, SpotifyId as _};
use spottyfi_ui::theme::Palette;

use super::{ShellState, Tab};

/// Render the left sidebar. Navigation clicks are pushed onto `nav`.
pub(super) fn sidebar(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    palette: &Palette,
    nav: &mut Vec<Tab>,
) {
    let collapsed = state.persisted.sidebar_collapsed;
    let width = if collapsed {
        64.0
    } else {
        state.persisted.sidebar_width
    };

    let panel = egui::Panel::left("sidebar")
        .frame(
            egui::Frame::new()
                .fill(palette.card)
                .inner_margin(egui::Margin::same(10)),
        )
        .resizable(!collapsed)
        .min_size(if collapsed { 64.0 } else { 240.0 })
        .max_size(320.0)
        .default_size(width);

    let response = panel.show_inside(ui, |ui| {
        // Header: title + collapse toggle.
        ui.horizontal(|ui| {
            if !collapsed {
                ui.label(
                    egui::RichText::new("Your Library")
                        .family(spottyfi_ui::fonts::semibold())
                        .size(14.0)
                        .color(palette.text),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let glyph = if collapsed { "\u{00bb}" } else { "\u{00ab}" };
                if spottyfi_ui::components::icon_button(
                    ui,
                    palette,
                    glyph,
                    14.0,
                    false,
                    "Collapse sidebar",
                )
                .clicked()
                {
                    state.persisted.sidebar_collapsed = !collapsed;
                }
            });
        });
        ui.add_space(8.0);

        // Pinned entries: Liked Songs and the full Library page.
        if entry(ui, palette, "\u{2665}", "Liked Songs", collapsed) {
            nav.push(Tab::LikedSongs);
        }
        if entry(ui, palette, "\u{1f4da}", "Your Library", collapsed) {
            nav.push(Tab::Library);
        }
        ui.add_space(6.0);
        if !collapsed {
            ui.separator();
            ui.add_space(2.0);
        }

        // The user's real playlists.
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                playlists(ui, state, palette, collapsed, nav);
            });
    });

    // Persist a user-dragged width.
    if !collapsed {
        let w = response.response.rect.width();
        if (w - state.persisted.sidebar_width).abs() > 0.5 {
            state.persisted.sidebar_width = w;
        }
    }
}

/// Render the playlist list — a spinner while loading, an error line on
/// failure, the playlist rows once loaded.
fn playlists(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    palette: &Palette,
    collapsed: bool,
    nav: &mut Vec<Tab>,
) {
    let Some(session) = state.session.as_mut() else {
        ui.add(egui::Spinner::new().size(16.0).color(palette.accent));
        return;
    };
    match session.sidebar_playlists.value() {
        None => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0).color(palette.accent));
                if !collapsed {
                    ui.label(spottyfi_ui::components::muted(palette, "Loading…", 11.5));
                }
            });
        }
        Some(Err(err)) => {
            if !collapsed {
                ui.label(
                    egui::RichText::new("Couldn't load playlists")
                        .size(11.5)
                        .color(palette.error),
                );
                ui.label(spottyfi_ui::components::muted(palette, err.clone(), 10.5));
            }
        }
        Some(Ok(list)) => {
            if list.is_empty() && !collapsed {
                ui.label(spottyfi_ui::components::muted(
                    palette,
                    "No playlists yet.",
                    11.5,
                ));
            }
            for playlist in list {
                if playlist_entry(ui, palette, playlist, collapsed) {
                    nav.push(Tab::Playlist(playlist.id.id().to_owned()));
                }
            }
        }
    }
}

/// One playlist row: cover thumbnail + name (icon-only when collapsed).
fn playlist_entry(
    ui: &mut egui::Ui,
    palette: &Palette,
    playlist: &SimplifiedPlaylist,
    collapsed: bool,
) -> bool {
    let art = playlist.images.first().map(|i| i.url.as_str());
    let response = ui
        .scope(|ui| {
            ui.horizontal(|ui| {
                spottyfi_ui::components::album_art(ui, palette, art, 32.0, 4.0);
                if !collapsed {
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&playlist.name)
                                .size(12.5)
                                .color(palette.text),
                        )
                        .truncate()
                        .selectable(false),
                    );
                }
            });
        })
        .response
        .interact(egui::Sense::click());
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(&playlist.name)
        .clicked()
}

/// One pinned sidebar entry — icon + label. Returns `true` when clicked.
fn entry(ui: &mut egui::Ui, palette: &Palette, glyph: &str, label: &str, collapsed: bool) -> bool {
    let text = if collapsed {
        egui::RichText::new(glyph).size(18.0)
    } else {
        egui::RichText::new(format!("{glyph}   {label}")).size(13.0)
    };
    let button = egui::Button::new(text.color(palette.text))
        .frame(false)
        .min_size(egui::vec2(ui.available_width(), 34.0));
    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
        .clicked()
}
