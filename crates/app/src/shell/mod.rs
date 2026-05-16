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
use spottyfi_ui::components::Density;
use spottyfi_ui::theme::{Palette, Theme};
use tokio::runtime::Handle;

pub use persist::{default_dock, PersistedShell};
pub use tabs::{DockIntent, Tab};

use crate::page::{Loadable, PageRegistry, PageServices};
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

/// The sidebar's playlist list, or the error explaining why it is missing.
pub(super) type SidebarPlaylists = Result<Vec<spottyfi_models::SimplifiedPlaylist>, String>;

/// The session-scoped services and live page state, attached after login.
struct ActiveSession {
    /// The page registry — the live, stateful pages keyed by tab.
    pages: PageRegistry,
    /// The async load of the sidebar's playlist list.
    sidebar_playlists: Loadable<SidebarPlaylists>,
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
    /// The session-scoped page state, present once the API is attached.
    session: Option<ActiveSession>,
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
            session: None,
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
        let services = PageServices {
            api: Arc::clone(&api),
            runtime: runtime.clone(),
            ctx: ctx.clone(),
        };
        let sidebar_playlists = Loadable::spawn(&runtime, &ctx, async move {
            let mut playlists = Vec::new();
            let mut offset = 0u32;
            loop {
                match api.user_playlists(offset, 50).await {
                    Ok(page) => {
                        let count = page.items.len() as u32;
                        playlists.extend(page.items);
                        if !page.has_next || count == 0 {
                            break;
                        }
                        offset += count;
                    }
                    Err(err) => return Err(err.to_string()),
                }
            }
            Ok(playlists)
        });
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

    // Top bar — fixed height, drawn first so panels below dock under it.
    if let Some(i) = top_bar(ui, state, &palette, profile, avatar, &mut nav) {
        intent = Some(i);
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

/// The ~48px top bar: navigation, Home, omni-search and the profile menu.
fn top_bar(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    palette: &Palette,
    profile: &UserProfile,
    avatar: Option<&egui::TextureHandle>,
    nav: &mut Vec<Tab>,
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
                if spottyfi_ui::components::icon_button(
                    ui,
                    palette,
                    "\u{1f3e0}",
                    14.0,
                    false,
                    "Home",
                )
                .clicked()
                {
                    nav.push(Tab::Home);
                }

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
        for tab in [
            Tab::Home,
            Tab::Library,
            Tab::NowPlayingArt,
            Tab::Queue,
            Tab::Debug,
        ] {
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

    let mut dock_style = egui_dock::Style::from_egui(ui.style());
    dock_style.tab_bar.fill_tab_bar = true;
    dock_style.dock_area_padding = None;
    dock_style.separator.color_idle = palette.outline;

    egui_dock::DockArea::new(&mut state.persisted.dock)
        .style(dock_style)
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(false)
        .show_inside(ui, &mut viewer);

    viewer.ctx.intents
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
