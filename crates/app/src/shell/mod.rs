//! The Spottyfi application shell: top bar, left sidebar, centre dock and the
//! settings window.
//!
//! The shell is the logged-in surface. It is a pure projection: it reads the
//! playback snapshot and auth profile and returns a [`ShellIntent`] describing
//! what the user asked for; `app` applies it. The bottom transport is rendered
//! separately by [`crate::transport`].

mod persist;
mod tabs;

use spottyfi_auth::UserProfile;
use spottyfi_ui::components::Density;
use spottyfi_ui::theme::{Palette, Theme};

pub use persist::{default_dock, PersistedShell};
pub use tabs::Tab;

use crate::playback_controller::EngineStatus;
use crate::transport::{TransportIntent, TransportUiState};
use spottyfi_audio::PlaybackState;
use tabs::{ShellTabViewer, TabContext};

/// Something the user asked the shell to do this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ShellIntent {
    /// Log out and return to the login screen.
    Logout,
    /// Issue a transport command (e.g. from the debug panel).
    Transport(TransportIntent),
}

/// Persistent, non-serialised UI state owned by the shell for one session.
pub struct ShellState {
    /// The persisted layout + settings (dock, theme, density, sidebar).
    pub persisted: PersistedShell,
    /// Whether the settings window is open.
    settings_open: bool,
    /// The omni-search box contents (no real search until Phase 6).
    search_query: String,
    /// The currently applied theme, tracked so we re-`apply` only on change.
    applied_theme: Option<Theme>,
}

impl ShellState {
    /// Build the shell state, restoring the persisted layout from disk.
    #[must_use]
    pub fn load() -> Self {
        Self {
            persisted: PersistedShell::load(),
            settings_open: false,
            search_query: String::new(),
            applied_theme: None,
        }
    }

    /// The active theme.
    #[must_use]
    pub fn theme(&self) -> Theme {
        self.persisted.theme
    }

    /// Persist the shell layout + settings to disk (call on shutdown).
    pub fn save(&self) {
        self.persisted.save();
    }

    /// Re-apply the theme to `ctx` if it changed since the last frame.
    pub fn sync_theme(&mut self, ctx: &egui::Context) {
        if self.applied_theme != Some(self.persisted.theme) {
            self.persisted.theme.apply(ctx);
            self.applied_theme = Some(self.persisted.theme);
        }
    }
}

/// Render the whole logged-in shell, returning any [`ShellIntent`].
///
/// `ui` is eframe's root UI. The shell adds the top bar, sidebar and dock; the
/// caller adds the bottom transport panel before calling this.
pub fn shell(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    profile: &UserProfile,
    avatar: Option<&egui::TextureHandle>,
    playback: &PlaybackState,
    transport_ui: &mut TransportUiState,
    engine: &EngineStatus,
) -> Option<ShellIntent> {
    let palette = state.persisted.theme.palette();
    let mut intent = None;

    // Top bar — fixed height, drawn first so panels below dock under it.
    if let Some(i) = top_bar(ui, state, &palette, profile, avatar) {
        intent = Some(i);
    }

    // Left sidebar — resizable, collapsible.
    sidebar(ui, state, &palette);

    // Centre — the dock area fills the remaining space.
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(palette.base))
        .show_inside(ui, |ui| {
            if let Some(i) = dock(ui, state, &palette, playback, transport_ui, engine) {
                intent = Some(ShellIntent::Transport(i));
            }
        });

    // The settings window floats above everything when open.
    settings_window(ui.ctx(), state, &palette);

    intent
}

/// The ~28px top bar: navigation, Home, omni-search and the profile menu.
fn top_bar(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    palette: &Palette,
    profile: &UserProfile,
    avatar: Option<&egui::TextureHandle>,
) -> Option<ShellIntent> {
    let mut intent = None;

    egui::Panel::top("top-bar")
        .exact_size(48.0)
        .frame(
            egui::Frame::new()
                .fill(palette.base)
                .inner_margin(egui::Margin::symmetric(12, 8)),
        )
        .show_inside(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // Back / forward — placeholders, wired to per-tab history later.
                spottyfi_ui::components::icon_button(ui, palette, "\u{2039}", 16.0, false, "Back");
                spottyfi_ui::components::icon_button(
                    ui, palette, "\u{203a}", 16.0, false, "Forward",
                );
                ui.add_space(4.0);
                spottyfi_ui::components::icon_button(ui, palette, "\u{1f3e0}", 14.0, false, "Home");

                ui.add_space(6.0);
                // View menu — dock layout actions.
                view_menu(ui, state, palette);

                ui.add_space(10.0);

                // Omni-search — Ctrl/Cmd+K focuses it. No real search yet.
                let search_id = egui::Id::new("omni-search");
                let field = egui::TextEdit::singleline(&mut state.search_query)
                    .id(search_id)
                    .hint_text("Search  (Ctrl+K)")
                    .desired_width(320.0)
                    .margin(egui::Margin::symmetric(10, 5));
                ui.add(field);
                let focus_search = ui.input(|i| {
                    i.key_pressed(egui::Key::K) && (i.modifiers.command || i.modifiers.ctrl)
                });
                if focus_search {
                    ui.ctx().memory_mut(|m| m.request_focus(search_id));
                }

                // Profile menu on the far right.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(i) = profile_menu(ui, state, palette, profile, avatar) {
                        intent = Some(i);
                    }
                });
            });
        });

    intent
}

/// The View menu: dock layout actions and panel visibility.
fn view_menu(ui: &mut egui::Ui, state: &mut ShellState, palette: &Palette) {
    let button = egui::Button::new(egui::RichText::new("View").size(12.5).color(palette.text))
        .fill(palette.card)
        .corner_radius(8.0)
        .min_size(egui::vec2(0.0, 26.0));
    let response = ui.add(button);
    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(190.0);
        if ui.button("Reset layout to default").clicked() {
            state.persisted.dock = default_dock();
            ui.close();
        }
        ui.separator();
        for tab in [Tab::Home, Tab::NowPlayingArt, Tab::Queue, Tab::Debug] {
            let present = state.persisted.dock.find_tab(&tab).is_some();
            if ui
                .add_enabled(!present, egui::Button::new(format!("Open {}", tab.title())))
                .clicked()
            {
                state.persisted.dock.push_to_focused_leaf(tab);
                ui.close();
            }
        }
    });
}

/// The profile menu button: avatar + name, with Settings / Log out actions.
fn profile_menu(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    palette: &Palette,
    profile: &UserProfile,
    avatar: Option<&egui::TextureHandle>,
) -> Option<ShellIntent> {
    let mut intent = None;
    let name = profile.display_name.as_deref().unwrap_or("Spotify user");

    let button = egui::Button::new(
        egui::RichText::new(format!("{name}  \u{25be}"))
            .size(12.5)
            .color(palette.text),
    )
    .fill(palette.card)
    .corner_radius(14.0)
    .min_size(egui::vec2(0.0, 28.0));

    let response = ui.add(button);

    if let Some(texture) = avatar {
        ui.add(egui::Image::new((texture.id(), egui::vec2(26.0, 26.0))).corner_radius(13.0));
    }

    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(160.0);
        ui.label(spottyfi_ui::components::muted(
            palette,
            format!("id: {}", profile.id),
            10.5,
        ));
        ui.separator();
        if ui.button("Settings").clicked() {
            state.settings_open = true;
            ui.close();
        }
        if ui.button("Log out").clicked() {
            intent = Some(ShellIntent::Logout);
            ui.close();
        }
    });

    intent
}

/// The left sidebar: "Your Library", filter chips and hardcoded entries.
/// Collapsible to an icon-only rail at narrow widths.
fn sidebar(ui: &mut egui::Ui, state: &mut ShellState, palette: &Palette) {
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

        if !collapsed {
            // Filter chips — purely visual placeholders for now.
            ui.horizontal_wrapped(|ui| {
                for (i, chip) in ["Playlists", "Artists", "Albums"].iter().enumerate() {
                    spottyfi_ui::components::filter_chip(ui, palette, chip, i == 0);
                }
            });
            ui.add_space(10.0);
        }

        // Hardcoded library entries — real playlists arrive in Phase 5.
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (glyph, label) in [
                    ("\u{2665}", "Liked Songs"),
                    ("\u{1f4c0}", "Discover Weekly"),
                    ("\u{1f4da}", "Your Library"),
                ] {
                    library_entry(ui, palette, glyph, label, collapsed);
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

/// One sidebar library row — icon + label (icon-only when collapsed).
fn library_entry(ui: &mut egui::Ui, palette: &Palette, glyph: &str, label: &str, collapsed: bool) {
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
        .on_hover_text(label);
}

/// The centre dock area.
fn dock(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    palette: &Palette,
    playback: &PlaybackState,
    transport_ui: &mut TransportUiState,
    engine: &EngineStatus,
) -> Option<TransportIntent> {
    let mut viewer = ShellTabViewer {
        ctx: TabContext {
            palette: *palette,
            playback,
            transport_ui,
            engine,
            intent: None,
        },
    };

    let mut dock_style = egui_dock::Style::from_egui(ui.style());
    dock_style.tab_bar.fill_tab_bar = true;
    dock_style.dock_area_padding = None;
    dock_style.separator.color_idle = palette.outline;

    egui_dock::DockArea::new(&mut state.persisted.dock)
        .style(dock_style)
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(false)
        .show_inside(ui, &mut viewer);

    viewer.ctx.intent
}

/// The settings window: theme + density selection (both persisted) and a
/// Reset-layout action.
fn settings_window(ctx: &egui::Context, state: &mut ShellState, palette: &Palette) {
    if !state.settings_open {
        return;
    }
    let mut open = state.settings_open;
    egui::Window::new("Settings")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(280.0)
        .show(ctx, |ui| {
            spottyfi_ui::components::section_header(ui, palette, "Appearance");
            ui.horizontal(|ui| {
                ui.label("Theme");
                egui::ComboBox::from_id_salt("theme-combo")
                    .selected_text(state.persisted.theme.label())
                    .show_ui(ui, |ui| {
                        for theme in Theme::all() {
                            ui.selectable_value(&mut state.persisted.theme, theme, theme.label());
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Density");
                egui::ComboBox::from_id_salt("density-combo")
                    .selected_text(state.persisted.density.label())
                    .show_ui(ui, |ui| {
                        for density in [Density::Comfortable, Density::Compact] {
                            ui.selectable_value(
                                &mut state.persisted.density,
                                density,
                                density.label(),
                            );
                        }
                    });
            });

            ui.add_space(10.0);
            spottyfi_ui::components::section_header(ui, palette, "Layout");
            if ui.button("Reset layout to default").clicked() {
                state.persisted.dock = default_dock();
            }

            ui.add_space(8.0);
            ui.label(spottyfi_ui::components::muted(
                palette,
                "Theme, density and the dock layout persist across restarts.",
                11.0,
            ));
        });
    state.settings_open = open;
}
