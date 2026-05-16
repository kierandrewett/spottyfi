//! The left sidebar: a collapsible tree of navigation sections.
//!
//! Three sections — `MAIN`, `YOUR LIBRARY` and `PLAYLISTS` — each with an
//! uppercase dimmed header, a caret, and a thin separator rule. `MAIN` and
//! `YOUR LIBRARY` are static navigation entries; `PLAYLISTS` lists the user's
//! real playlists, loaded asynchronously when the API is attached (see
//! [`ShellState::attach_api`]).
//!
//! Clicking an entry pushes a navigation [`Tab`] onto the frame's `nav` list,
//! which the shell applies to the dock. Entries whose page isn't built yet
//! open a [`Tab::Placeholder`] "coming soon" surface.

use spottyfi_models::{SimplifiedPlaylist, SpotifyId as _};
use spottyfi_ui::icons::Icon;
use spottyfi_ui::theme::Palette;

use super::{ShellState, Tab};

/// One static navigation entry: an icon, a label and the tab it opens.
struct Entry {
    /// The leading line icon.
    icon: Icon,
    /// The row label.
    label: &'static str,
    /// The tab the row opens when clicked.
    tab: Tab,
}

/// The `MAIN` section entries.
fn main_entries() -> Vec<Entry> {
    vec![
        Entry {
            icon: Icon::Home,
            label: "Home",
            tab: Tab::Home,
        },
        Entry {
            icon: Icon::Search,
            label: "Search",
            tab: Tab::Search,
        },
        Entry {
            icon: Icon::Browse,
            label: "Browse",
            tab: Tab::Placeholder("Browse".to_owned()),
        },
        Entry {
            icon: Icon::Charts,
            label: "Charts",
            tab: Tab::Placeholder("Charts".to_owned()),
        },
        Entry {
            icon: Icon::NewReleases,
            label: "New Releases",
            tab: Tab::Placeholder("New Releases".to_owned()),
        },
        Entry {
            icon: Icon::Discover,
            label: "Discover",
            tab: Tab::Placeholder("Discover".to_owned()),
        },
        Entry {
            icon: Icon::Podcast,
            label: "Podcasts",
            tab: Tab::Placeholder("Podcasts".to_owned()),
        },
    ]
}

/// The `YOUR LIBRARY` section entries.
fn library_entries() -> Vec<Entry> {
    vec![
        Entry {
            icon: Icon::MadeForYou,
            label: "Made For You",
            tab: Tab::Placeholder("Made For You".to_owned()),
        },
        Entry {
            icon: Icon::RecentlyPlayed,
            label: "Recently Played",
            tab: Tab::Placeholder("Recently Played".to_owned()),
        },
        Entry {
            icon: Icon::Heart,
            label: "Liked Songs",
            tab: Tab::LikedSongs,
        },
        Entry {
            icon: Icon::Podcast,
            label: "Your Podcasts",
            tab: Tab::Placeholder("Your Podcasts".to_owned()),
        },
        Entry {
            icon: Icon::Disc,
            label: "Your Albums",
            tab: Tab::Library,
        },
        Entry {
            icon: Icon::User,
            label: "Your Artists",
            tab: Tab::Placeholder("Your Artists".to_owned()),
        },
        Entry {
            icon: Icon::List,
            label: "Local Files",
            tab: Tab::Placeholder("Local Files".to_owned()),
        },
    ]
}

/// Render the left sidebar. Navigation clicks are pushed onto `nav`.
pub(super) fn sidebar(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    palette: &Palette,
    nav: &mut Vec<Tab>,
) {
    let collapsed = state.persisted.sidebar_collapsed;
    let width = if collapsed {
        56.0
    } else {
        state.persisted.sidebar_width
    };

    let panel = egui::Panel::left("sidebar")
        .frame(
            egui::Frame::new()
                .fill(palette.card)
                .inner_margin(egui::Margin::symmetric(if collapsed { 6 } else { 8 }, 8)),
        )
        .resizable(!collapsed)
        .min_size(if collapsed { 56.0 } else { 200.0 })
        .max_size(if collapsed { 56.0 } else { 340.0 })
        .default_size(width);

    let response = panel.show_inside(ui, |ui| {
        // A tight, full-width selection highlight reads better than the
        // default inset; widen item spacing only vertically.
        ui.spacing_mut().item_spacing.y = 1.0;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // MAIN.
                if section_header(ui, state, palette, "MAIN", "main", collapsed, false) {
                    for entry in main_entries() {
                        if row(ui, palette, entry.icon, entry.label, collapsed) {
                            nav.push(entry.tab);
                        }
                    }
                }
                ui.add_space(8.0);

                // YOUR LIBRARY.
                if section_header(
                    ui,
                    state,
                    palette,
                    "YOUR LIBRARY",
                    "library",
                    collapsed,
                    false,
                ) {
                    for entry in library_entries() {
                        if row(ui, palette, entry.icon, entry.label, collapsed) {
                            nav.push(entry.tab);
                        }
                    }
                }
                ui.add_space(8.0);

                // PLAYLISTS — header carries a `+` action.
                if section_header(
                    ui,
                    state,
                    palette,
                    "PLAYLISTS",
                    "playlists",
                    collapsed,
                    true,
                ) {
                    playlists(ui, state, palette, collapsed, nav);
                }
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

/// Draw a section header — uppercase dimmed label, a caret toggle and a thin
/// separator rule. Returns whether the section is currently expanded.
///
/// `with_add` draws a trailing `+` action (used by `PLAYLISTS`); a click on it
/// opens a placeholder for the not-yet-built create-playlist flow.
fn section_header(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    palette: &Palette,
    label: &str,
    key: &str,
    collapsed: bool,
    with_add: bool,
) -> bool {
    // When the rail is collapsed, sections are always shown expanded (there is
    // no room for a header) — draw a faint divider instead.
    if collapsed {
        ui.add_space(4.0);
        let rect = ui.available_rect_before_wrap();
        ui.painter().hline(
            rect.x_range(),
            rect.top(),
            egui::Stroke::new(1.0, palette.outline),
        );
        ui.add_space(4.0);
        return true;
    }

    let expanded = state.section_expanded(key);
    ui.add_space(4.0);

    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let caret = if expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let caret_rect = egui::Rect::from_min_size(
            rect.left_center() - egui::vec2(0.0, 6.0),
            egui::vec2(12.0, 12.0),
        );
        caret
            .image(12.0, palette.text_muted)
            .paint_at(ui, caret_rect);
        ui.painter().text(
            rect.left_center() + egui::vec2(16.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::new(10.5, spottyfi_ui::fonts::semibold()),
            palette.text_muted,
        );
    }
    if response.clicked() {
        state.toggle_section(key);
    }
    let now_expanded = state.section_expanded(key);

    // The trailing `+` action.
    if with_add {
        let plus_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            egui::vec2(14.0, 14.0),
        );
        let plus = ui.interact(
            plus_rect,
            ui.id().with(("sidebar-add", key)),
            egui::Sense::click(),
        );
        let plus_color = if plus.hovered() {
            palette.text
        } else {
            palette.text_muted
        };
        Icon::Plus
            .image(13.0, plus_color)
            .paint_at(ui, plus_rect.shrink(1.0));
        let _ = plus.on_hover_text("New playlist (later)");
    }

    // The separator rule beneath the header.
    let line_y = rect.bottom() + 3.0;
    ui.painter().hline(
        rect.x_range(),
        line_y,
        egui::Stroke::new(1.0, palette.outline),
    );
    ui.add_space(6.0);

    now_expanded
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
        ui.add(egui::Spinner::new().size(14.0).color(palette.accent));
        return;
    };
    match session.sidebar_playlists.value() {
        None => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(13.0).color(palette.accent));
                if !collapsed {
                    ui.label(spottyfi_ui::components::muted(palette, "Loading…", 11.0));
                }
            });
        }
        Some(Err(err)) => {
            if !collapsed {
                ui.label(
                    egui::RichText::new("Couldn't load playlists")
                        .size(11.0)
                        .color(palette.error),
                );
                ui.label(spottyfi_ui::components::muted(palette, err.clone(), 10.0));
            }
        }
        Some(Ok(list)) => {
            if list.is_empty() && !collapsed {
                ui.label(spottyfi_ui::components::muted(
                    palette,
                    "No playlists yet.",
                    11.0,
                ));
            }
            for playlist in list {
                if playlist_row(ui, palette, playlist, collapsed) {
                    nav.push(Tab::Playlist(playlist.id.id().to_owned()));
                }
            }
        }
    }
}

/// One playlist row: a music-note icon and the playlist name.
fn playlist_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    playlist: &SimplifiedPlaylist,
    collapsed: bool,
) -> bool {
    row(ui, palette, Icon::Queue, &playlist.name, collapsed)
}

/// Draw one tight sidebar row — a leading line icon and a label — with a flat
/// full-width hover highlight. Returns `true` when clicked.
fn row(ui: &mut egui::Ui, palette: &Palette, icon: Icon, label: &str, collapsed: bool) -> bool {
    let height = 26.0;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(rect, 0, palette.hover);
        }
        let color = if response.hovered() {
            palette.text
        } else {
            palette.text_muted
        };
        let icon_size = 15.0;
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 6.0 + icon_size / 2.0, rect.center().y),
            egui::vec2(icon_size, icon_size),
        );
        icon.image(icon_size, color).paint_at(ui, icon_rect);
        if !collapsed {
            ui.painter().text(
                egui::pos2(icon_rect.right() + 9.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(12.5),
                color,
            );
        }
    }

    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if collapsed {
        response.on_hover_text(label).clicked()
    } else {
        response.clicked()
    }
}
