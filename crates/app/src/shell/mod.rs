//! The Spottyfi application shell: top bar, left sidebar, centre dock and the
//! settings window.
//!
//! The shell is the logged-in surface. It is a pure projection: it reads the
//! playback snapshot and auth profile and returns a [`ShellIntent`] describing
//! what the user asked for; `app` applies it. The bottom transport is rendered
//! separately by [`crate::transport`].

mod persist;
mod sidebar;
mod tabs;

use std::sync::Arc;

use spottyfi_api::SpotifyApi;
use spottyfi_auth::UserProfile;
use spottyfi_state::ActivityRegistry;
use spottyfi_ui::components::Density;
use spottyfi_ui::theme::{Palette, Theme};
use tokio::runtime::Handle;

pub use persist::{default_dock, PersistedShell};
pub use tabs::{DockIntent, Tab};

use crate::page::{IncrementalLoad, PageRegistry, PageServices};
use crate::playback_controller::EngineStatus;
use crate::transport::{TransportIntent, TransportUiState};
use spottyfi_audio::PlaybackState;
use tabs::{ShellTabViewer, TabContext};

/// Something the user asked the shell to do this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ShellIntent {
    /// Log out and return to the login screen.
    Logout,
    /// Issue a transport command (e.g. from the debug panel or a page).
    Transport(TransportIntent),
}

/// The session-scoped services and live page state, attached after login.
struct ActiveSession {
    /// The page registry — the live, stateful pages keyed by tab.
    pages: PageRegistry,
    /// The incremental load of the sidebar's playlist list — playlists appear
    /// as they stream in rather than after every page is collected.
    sidebar_playlists: IncrementalLoad<spottyfi_models::SimplifiedPlaylist>,
}

/// Persistent, non-serialised UI state owned by the shell for one session.
pub struct ShellState {
    /// The persisted layout + settings (dock, theme, density, sidebar).
    pub persisted: PersistedShell,
    /// Whether the settings window is open.
    settings_open: bool,
    /// The currently applied theme, tracked so we re-`apply` only on change.
    applied_theme: Option<Theme>,
    /// The session-scoped page state, present once the API is attached.
    session: Option<ActiveSession>,
    /// The shared background-activity registry, surfaced in the menu bar.
    activity: Arc<ActivityRegistry>,
}

impl ShellState {
    /// Build the shell state, restoring the persisted layout from disk.
    #[must_use]
    pub fn load() -> Self {
        Self {
            persisted: PersistedShell::load(),
            settings_open: false,
            applied_theme: None,
            session: None,
            activity: ActivityRegistry::new(),
        }
    }

    /// Attach the Spotify API after login, building the page registry and
    /// kicking off the sidebar's playlist load.
    ///
    /// Idempotent: a second call (e.g. a re-render after login) is ignored.
    pub fn attach_api(&mut self, api: Arc<dyn SpotifyApi>, runtime: Handle, ctx: egui::Context) {
        if self.session.is_some() {
            return;
        }
        // Last.fm powers Browse's charts and recommendations. With no API key
        // configured this is `None` and Browse degrades gracefully.
        let lastfm = match spottyfi_api::lastfm::LastfmClient::from_env() {
            Ok(client) => Some(client),
            Err(err) => {
                tracing::info!(%err, "Last.fm not configured; Browse charts disabled");
                None
            }
        };
        let services = PageServices {
            api: Arc::clone(&api),
            lastfm,
            runtime: runtime.clone(),
            ctx: ctx.clone(),
            activity: Arc::clone(&self.activity),
        };
        let sidebar_playlists = IncrementalLoad::spawn(
            &runtime,
            &ctx,
            &self.activity,
            "Loading playlists…",
            api.user_playlists_stream(),
        );
        self.session = Some(ActiveSession {
            pages: PageRegistry::new(services),
            sidebar_playlists,
        });
    }

    /// Drop the session-scoped state on logout so a future login starts fresh.
    pub fn detach_api(&mut self) {
        self.session = None;
    }

    /// The active theme.
    #[must_use]
    pub fn theme(&self) -> Theme {
        self.persisted.theme
    }

    /// Whether the sidebar tree section `key` is expanded (the default).
    #[must_use]
    fn section_expanded(&self, key: &str) -> bool {
        !self.persisted.collapsed_sections.iter().any(|k| k == key)
    }

    /// Toggle the expanded/collapsed state of sidebar tree section `key`.
    fn toggle_section(&mut self, key: &str) {
        let collapsed = &mut self.persisted.collapsed_sections;
        if let Some(pos) = collapsed.iter().position(|k| k == key) {
            collapsed.remove(pos);
        } else {
            collapsed.push(key.to_owned());
        }
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

/// Open `tab` in the dock: focus it if already open, else add it to the
/// focused leaf.
fn open_tab(dock: &mut egui_dock::DockState<Tab>, tab: Tab) {
    if let Some(path) = dock.find_tab(&tab) {
        let _ = dock.set_active_tab(path);
    } else {
        dock.push_to_focused_leaf(tab);
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
    // Navigation requests collected this frame, applied to the dock after the
    // panels have been drawn (the dock and sidebar both borrow `state`).
    let mut nav: Vec<Tab> = Vec::new();
    let mut copy_to_clipboard: Option<String> = None;

    // Menu bar — fixed height, drawn first so panels below dock under it.
    if let Some(i) = menu_bar(ui, state, &palette, profile, avatar, playback, &mut nav) {
        intent = Some(i);
    }

    // Ctrl/Cmd+K opens the Search page (the search box moved to the sidebar).
    let open_search =
        ui.input(|i| i.key_pressed(egui::Key::K) && (i.modifiers.command || i.modifiers.ctrl));
    if open_search {
        nav.push(Tab::Search);
    }

    // Left sidebar — resizable, collapsible, real playlists.
    sidebar::sidebar(ui, state, &palette, &mut nav);

    // Centre — the dock area fills the remaining space.
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(palette.base))
        .show_inside(ui, |ui| {
            for dock_intent in dock(ui, state, &palette, playback, transport_ui, engine) {
                match dock_intent {
                    DockIntent::Transport(t) => intent = Some(ShellIntent::Transport(t)),
                    DockIntent::Open(tab) => nav.push(tab),
                    DockIntent::CopyToClipboard(text) => copy_to_clipboard = Some(text),
                }
            }
        });

    // Apply navigation requests gathered from the sidebar, top bar and pages.
    for tab in nav {
        open_tab(&mut state.persisted.dock, tab);
    }
    if let Some(text) = copy_to_clipboard {
        ui.ctx().copy_text(text);
    }

    // The settings window floats above everything when open.
    settings_window(ui.ctx(), state, &palette);

    intent
}

/// The thin application menu bar: `File  View  Playback  Tools  Help`.
///
/// This replaces the Phase 4 top bar. Search moved to the sidebar (and
/// `Ctrl/Cmd+K`); the profile actions live under the `File` menu.
fn menu_bar(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    palette: &Palette,
    profile: &UserProfile,
    avatar: Option<&egui::TextureHandle>,
    playback: &PlaybackState,
    nav: &mut Vec<Tab>,
) -> Option<ShellIntent> {
    let mut intent = None;
    // A cheap `Arc` clone so the right-side activity indicator can read the
    // registry without re-borrowing `state` inside the menu closure.
    let activity = Arc::clone(&state.activity);

    egui::Panel::top("menu-bar")
        .exact_size(28.0)
        .frame(
            egui::Frame::new()
                .fill(palette.elevated)
                .inner_margin(egui::Margin::symmetric(4, 0)),
        )
        .show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    ui.set_min_width(180.0);
                    let name = profile.display_name.as_deref().unwrap_or("Spotify user");
                    ui.horizontal(|ui| {
                        if let Some(texture) = avatar {
                            ui.add(egui::Image::new((texture.id(), egui::vec2(22.0, 22.0))));
                        }
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(name)
                                    .family(spottyfi_ui::fonts::medium())
                                    .size(11.5)
                                    .color(palette.text),
                            );
                            ui.label(spottyfi_ui::components::muted(
                                palette,
                                profile.id.to_string(),
                                9.5,
                            ));
                        });
                    });
                    ui.separator();
                    if ui.button("Settings…").clicked() {
                        state.settings_open = true;
                        ui.close();
                    }
                    if ui.button("Log out").clicked() {
                        intent = Some(ShellIntent::Logout);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close();
                    }
                });

                ui.menu_button("View", |ui| {
                    ui.set_min_width(200.0);
                    if ui.button("Reset layout to default").clicked() {
                        state.persisted.dock = default_dock();
                        ui.close();
                    }
                    if ui
                        .checkbox(&mut state.persisted.sidebar_collapsed, "Collapse sidebar")
                        .clicked()
                    {
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("Theme", |ui| {
                        for theme in Theme::all() {
                            if ui
                                .radio(state.persisted.theme == theme, theme.label())
                                .clicked()
                            {
                                state.persisted.theme = theme;
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Density", |ui| {
                        for density in [Density::Comfortable, Density::Compact] {
                            if ui
                                .radio(state.persisted.density == density, density.label())
                                .clicked()
                            {
                                state.persisted.density = density;
                                ui.close();
                            }
                        }
                    });
                    ui.separator();
                    for tab in [Tab::NowPlayingArt, Tab::Queue, Tab::Debug] {
                        let present = state.persisted.dock.find_tab(&tab).is_some();
                        if ui
                            .add_enabled(
                                !present,
                                egui::Button::new(format!("Open {} panel", tab.title())),
                            )
                            .clicked()
                        {
                            state.persisted.dock.push_to_focused_leaf(tab);
                            ui.close();
                        }
                    }
                });

                ui.menu_button("Playback", |ui| {
                    ui.set_min_width(160.0);
                    let has_track = playback.track.is_some();
                    let label = if playback.playing { "Pause" } else { "Play" };
                    if ui
                        .add_enabled(has_track, egui::Button::new(label))
                        .clicked()
                    {
                        intent = Some(ShellIntent::Transport(TransportIntent::TogglePlayPause));
                        ui.close();
                    }
                    // Next / previous need the Phase 8 queue — shown but inert.
                    let _ = ui.add_enabled(false, egui::Button::new("Next track"));
                    let _ = ui.add_enabled(false, egui::Button::new("Previous track"));
                });

                ui.menu_button("Tools", |ui| {
                    ui.set_min_width(160.0);
                    if ui.button("Search").clicked() {
                        nav.push(Tab::Search);
                        ui.close();
                    }
                    if ui.button("Open Debug panel").clicked() {
                        if state.persisted.dock.find_tab(&Tab::Debug).is_none() {
                            state.persisted.dock.push_to_focused_leaf(Tab::Debug);
                        }
                        ui.close();
                    }
                });

                ui.menu_button("Help", |ui| {
                    ui.set_min_width(160.0);
                    ui.label(spottyfi_ui::components::muted(palette, "Spottyfi", 11.0));
                    ui.label(spottyfi_ui::components::muted(
                        palette,
                        concat!("Version ", env!("CARGO_PKG_VERSION")),
                        10.5,
                    ));
                    ui.separator();
                    let _ = ui.add_enabled(false, egui::Button::new("Keyboard shortcuts"));
                });

                // The right side of the menu bar: the Home shortcut, then the
                // VSCode-style background-activity indicator.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if spottyfi_ui::icons::icon_button(
                        ui,
                        palette,
                        spottyfi_ui::Icon::Home,
                        14.0,
                        false,
                        "Home",
                    )
                    .clicked()
                    {
                        nav.push(Tab::Home);
                    }
                    activity_indicator(ui, palette, &activity);
                });
            });
        });

    intent
}

/// The VSCode-style background-activity indicator on the right of the menu
/// bar: a small spinner and a label naming the current activity, with a cancel
/// affordance for the most recent in-flight task. Renders nothing when idle.
///
/// Drawn inside a `right_to_left` layout, so widgets are added rightmost-first.
fn activity_indicator(ui: &mut egui::Ui, palette: &Palette, activity: &ActivityRegistry) {
    let activities = activity.snapshot();
    let Some(current) = activities.last() else {
        // Idle — show nothing, as specified.
        return;
    };

    ui.add_space(8.0);

    // Cancel affordance for the most recent task, when it is cancellable.
    if current.cancellable {
        let cancel = spottyfi_ui::icons::icon_button(
            ui,
            palette,
            spottyfi_ui::Icon::Close,
            12.0,
            false,
            "Cancel this task",
        );
        if cancel.clicked() {
            activity.cancel(current.id);
        }
        ui.add_space(2.0);
    }

    // The activity label. When several tasks run at once, append a count.
    let label = if activities.len() > 1 {
        format!("{}  (+{})", current.label, activities.len() - 1)
    } else {
        current.label.clone()
    };
    let elapsed = current.started_at.elapsed().as_secs();
    let label = if elapsed >= 2 {
        format!("{label}  {elapsed}s")
    } else {
        label
    };
    ui.label(spottyfi_ui::components::muted(palette, label, 10.5));

    ui.add_space(4.0);
    ui.add(egui::Spinner::new().size(12.0).color(palette.accent));

    // The indicator animates the spinner and the elapsed seconds; keep the
    // menu bar repainting while work is in flight.
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(250));
}

/// The centre dock area. Returns every [`DockIntent`] raised this frame.
fn dock(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    palette: &Palette,
    playback: &PlaybackState,
    transport_ui: &mut TransportUiState,
    engine: &EngineStatus,
) -> Vec<DockIntent> {
    let Some(session) = state.session.as_mut() else {
        // No API attached yet (the post-login frame before `attach_api`).
        ui.centered_and_justified(|ui| {
            ui.add(egui::Spinner::new().size(28.0).color(palette.accent));
        });
        return Vec::new();
    };

    // Drop pages whose tabs have been closed since the last frame.
    let open_pages: Vec<Tab> = state
        .persisted
        .dock
        .iter_all_tabs()
        .filter(|(_, tab)| tab.is_page())
        .map(|(_, tab)| tab.clone())
        .collect();
    session.pages.retain_open(open_pages.iter());

    let mut viewer = ShellTabViewer {
        ctx: TabContext {
            palette: *palette,
            playback,
            transport_ui,
            engine,
            pages: &mut session.pages,
            intents: Vec::new(),
        },
    };

    egui_dock::DockArea::new(&mut state.persisted.dock)
        .style(dock_style(palette, ui.style()))
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(false)
        .show_add_buttons(false)
        .show_inside(ui, &mut viewer);

    viewer.ctx.intents
}

/// Build the flat, sharp-cornered `egui_dock` style.
///
/// Tabs are square (corner radius `0`), the active tab is a touch lighter than
/// the bar, and inactive tabs blend into it. egui_dock 0.19 always draws a tab
/// bar per leaf, so a lone leaf still shows one — see the report's open items;
/// styling keeps it as unobtrusive as possible.
fn dock_style(palette: &Palette, egui_style: &egui::Style) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(egui_style);
    let sharp = egui::CornerRadius::ZERO;

    style.dock_area_padding = None;
    style.main_surface_border_stroke = egui::Stroke::NONE;
    style.main_surface_border_rounding = sharp;

    style.separator.color_idle = palette.outline;
    style.separator.color_hovered = palette.text_muted;
    style.separator.width = 1.0;

    // The tab bar — flat, dense, sharp.
    style.tab_bar.bg_fill = palette.elevated;
    style.tab_bar.corner_radius = sharp;
    style.tab_bar.height = 26.0;
    style.tab_bar.fill_tab_bar = false;
    style.tab_bar.hline_color = palette.outline;

    // Individual tabs — square; the active tab a touch lighter than the bar,
    // inactive tabs blended into it.
    style.tab.spacing = 0.0;
    style.tab.hline_below_active_tab_name = false;
    for interaction in [
        &mut style.tab.active,
        &mut style.tab.focused,
        &mut style.tab.active_with_kb_focus,
        &mut style.tab.focused_with_kb_focus,
    ] {
        interaction.corner_radius = sharp;
        interaction.bg_fill = palette.base;
        interaction.text_color = palette.text;
        interaction.outline_color = palette.outline;
    }
    for interaction in [
        &mut style.tab.inactive,
        &mut style.tab.hovered,
        &mut style.tab.inactive_with_kb_focus,
    ] {
        interaction.corner_radius = sharp;
        interaction.bg_fill = palette.elevated;
        interaction.text_color = palette.text_muted;
        interaction.outline_color = palette.outline;
    }
    style.tab.hovered.text_color = palette.text;

    style.tab.tab_body.corner_radius = sharp;
    style.tab.tab_body.inner_margin = egui::Margin::ZERO;
    style.tab.tab_body.bg_fill = palette.base;
    style.tab.tab_body.stroke = egui::Stroke::NONE;

    // The close button on a tab.
    style.buttons.close_tab_bg_fill = palette.hover;
    style.buttons.close_tab_active_color = palette.text;
    style.buttons.close_tab_color = palette.text_muted;

    style
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
