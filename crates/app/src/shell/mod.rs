//! The Spottyfi application shell: menu bar (with the top-right account menu),
//! left sidebar and the centre dock. The Settings page is a dock tab; see
//! [`crate::page::settings_page`].
//!
//! The shell is the logged-in surface. It is a pure projection: it reads the
//! playback snapshot and auth profile and returns a [`ShellIntent`] describing
//! what the user asked for; `app` applies it. The bottom transport is rendered
//! separately by [`crate::transport`].

mod dock_model;
mod nav;
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

pub use persist::{Layout, PersistedShell};
pub use tabs::{DockIntent, Tab, TabCommand};

use crate::page::{IncrementalLoad, PageRegistry, PageServices};
use crate::playback_controller::EngineStatus;
use crate::transport::{TransportIntent, TransportUiState};
use spottyfi_audio::{PlaybackState, QueueState, SpectrumAnalyzer};
use tabs::{NavRequest, ShellTabViewer, TabContext};

/// Something the user asked the shell to do this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ShellIntent {
    /// Log out and return to the login screen.
    Logout,
    /// Issue a transport command (e.g. from the debug panel or a page).
    Transport(TransportIntent),
    /// The audio engine settings changed on the Settings page. `app` restarts
    /// the engine so librespot picks up the new `PlayerConfig` (bitrate /
    /// normalisation are baked in at connect and cannot change live).
    AudioSettingsChanged,
    /// The equaliser settings changed on the Settings page. `app` pushes the
    /// new gains straight to the running audio engine — no restart.
    EqualizerChanged,
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
    /// The currently applied theme, tracked so we re-`apply` only on change.
    applied_theme: Option<Theme>,
    /// The session-scoped page state, present once the API is attached.
    session: Option<ActiveSession>,
    /// The shared background-activity registry, surfaced in the menu bar.
    activity: Arc<ActivityRegistry>,
    /// The draft folder path being typed in the Settings page's Local Files
    /// section — non-persisted, scoped to this session.
    local_folder_draft: String,
}

impl ShellState {
    /// Build the shell state, restoring the persisted layout from disk.
    #[must_use]
    pub fn load() -> Self {
        Self {
            persisted: PersistedShell::load(),
            applied_theme: None,
            session: None,
            activity: ActivityRegistry::new(),
            local_folder_draft: String::new(),
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

/// Apply one [`NavRequest`] to the persisted dock.
///
/// A plain request replaces the focused tab (recording history); a
/// Ctrl/Cmd-held request opens a new tab. A `main_pane` request (sidebar
/// navigation) always targets the centre tab group instead of the focused
/// leaf. See [`nav`].
fn apply_nav(persisted: &mut PersistedShell, request: NavRequest) {
    let PersistedShell {
        dock, dock_extras, ..
    } = persisted;
    match (request.new_tab, request.main_pane) {
        (false, false) => nav::navigate_replace(dock, dock_extras, request.tab),
        (true, false) => nav::open_new_tab(dock, request.tab),
        (false, true) => nav::navigate_replace_main(dock, dock_extras, request.tab),
        (true, true) => nav::open_new_tab_main(dock, request.tab),
    }
}

/// Apply one [`TabCommand`] to the persisted dock — the right-click menu's
/// close family, duplicate and pin toggle.
fn apply_tab_command(persisted: &mut PersistedShell, command: TabCommand) {
    let PersistedShell {
        dock, dock_extras, ..
    } = persisted;
    match command {
        TabCommand::Close(tab) => nav::close_tab(dock, dock_extras, &tab),
        TabCommand::CloseOthers(tab) => nav::close_others(dock, dock_extras, &tab),
        TabCommand::CloseToRight(tab) => nav::close_to_right(dock, dock_extras, &tab),
        TabCommand::Duplicate(tab) => nav::duplicate_tab(dock, &tab),
        TabCommand::TogglePin(tab) => dock_extras.toggle_pin(&tab),
    }
}

/// Apply a predefined [`Layout`] to the persisted shell.
///
/// Rebuilds the dock tree and records the selection. The **Power user** layout
/// also nudges the density to compact, matching its dense-tables intent; the
/// other layouts leave density alone.
fn apply_layout(persisted: &mut PersistedShell, layout: Layout) {
    persisted.dock = layout.build_dock();
    persisted.layout = layout;
    // Pin / history bookkeeping for tabs the new tree no longer holds is stale
    // — drop it. The closed-tab stack is preserved (it outlives tabs).
    let open: Vec<Tab> = persisted
        .dock
        .iter_all_tabs()
        .map(|(_, tab)| tab.clone())
        .collect();
    persisted.dock_extras.retain_open(open.iter());
    if layout == Layout::PowerUser {
        persisted.density = Density::Compact;
    }
}

/// A menu-bar navigation icon button that reflects availability: when
/// `enabled` is false it renders muted and does not report clicks.
fn nav_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: spottyfi_ui::Icon,
    enabled: bool,
    tooltip: &str,
) -> bool {
    if enabled {
        spottyfi_ui::icons::icon_button(ui, palette, icon, 13.0, false, tooltip).clicked()
    } else {
        // Disabled — draw the glyph dimmed, allocate the same space, ignore
        // any interaction so it reads as unavailable.
        let pad = egui::vec2(6.0, 6.0);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(13.0, 13.0) + pad * 2.0, egui::Sense::hover());
        let glyph_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(13.0, 13.0));
        icon.image(13.0, palette.outline).paint_at(ui, glyph_rect);
        false
    }
}

/// Apply the `Cmd/Ctrl+W` / `+T` / `+Shift+T` keyboard shortcuts.
///
/// Returns any [`NavRequest`] raised (a new Home tab), already handled close
/// and reopen mutate the dock directly.
fn apply_shortcuts(ui: &egui::Ui, persisted: &mut PersistedShell) {
    let (close, new_tab, reopen) = ui.input(|i| {
        let cmd = i.modifiers.command || i.modifiers.ctrl;
        (
            cmd && i.key_pressed(egui::Key::W),
            cmd && !i.modifiers.shift && i.key_pressed(egui::Key::T),
            cmd && i.modifiers.shift && i.key_pressed(egui::Key::T),
        )
    });
    if close {
        if let Some(tab) = nav::focused_tab(&persisted.dock) {
            let PersistedShell {
                dock, dock_extras, ..
            } = persisted;
            nav::close_tab(dock, dock_extras, &tab);
        }
    }
    if new_tab {
        // A new tab belongs in the centre tab group, not a side panel.
        nav::open_new_tab_main(&mut persisted.dock, Tab::Home);
    }
    if reopen {
        nav::reopen_last_closed(&mut persisted.dock, &mut persisted.dock_extras);
    }
}

/// Render the whole logged-in shell, returning any [`ShellIntent`].
///
/// `ui` is eframe's root UI. The shell adds the top bar, sidebar and dock; the
/// caller adds the bottom transport panel before calling this.
#[allow(clippy::too_many_arguments)]
pub fn shell(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    profile: &UserProfile,
    avatar: Option<&egui::TextureHandle>,
    playback: &PlaybackState,
    queue: &QueueState,
    transport_ui: &mut TransportUiState,
    engine: &EngineStatus,
    spectrum: &SpectrumAnalyzer,
) -> Option<ShellIntent> {
    let palette = state.persisted.theme.palette();
    let mut intent = None;
    // Navigation requests collected this frame, applied to the dock after the
    // panels have been drawn (the dock and sidebar both borrow `state`).
    let mut nav: Vec<NavRequest> = Vec::new();
    // Tab-management commands raised from the dock's tab bar this frame.
    let mut tab_commands: Vec<TabCommand> = Vec::new();
    // Settings-page actions raised this frame, applied after the dock draw.
    let mut settings_actions: Vec<crate::page::SettingsAction> = Vec::new();
    let mut copy_to_clipboard: Option<String> = None;

    // `Cmd/Ctrl+W` / `+T` / `+Shift+T` — close, new Home tab, reopen closed.
    apply_shortcuts(ui, &mut state.persisted);

    // Menu bar — fixed height, drawn first so panels below dock under it.
    if let Some(i) = menu_bar(ui, state, &palette, profile, avatar, playback, &mut nav) {
        intent = Some(i);
    }

    // Ctrl/Cmd+K opens the Search page (the search box moved to the sidebar).
    let open_search =
        ui.input(|i| i.key_pressed(egui::Key::K) && (i.modifiers.command || i.modifiers.ctrl));
    if open_search {
        nav.push(NavRequest::replace(Tab::Search));
    }

    // Left sidebar — resizable, collapsible, real playlists.
    sidebar::sidebar(ui, state, &palette, &mut nav);

    // Centre — the dock area fills the remaining space.
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(palette.base))
        .show_inside(ui, |ui| {
            for dock_intent in dock(
                ui,
                state,
                &palette,
                playback,
                queue,
                transport_ui,
                engine,
                spectrum,
            ) {
                match dock_intent {
                    DockIntent::Transport(t) => intent = Some(ShellIntent::Transport(t)),
                    DockIntent::Open(tab) => nav.push(NavRequest::replace(tab)),
                    DockIntent::OpenInNewTab(tab) => nav.push(NavRequest::new_tab(tab)),
                    DockIntent::CopyToClipboard(text) => copy_to_clipboard = Some(text),
                    DockIntent::Tab(command) => tab_commands.push(command),
                    DockIntent::Settings(action) => settings_actions.push(action),
                }
            }
        });

    // Apply tab-management commands before navigation so a "Close others"
    // followed by a navigation behaves predictably.
    for command in tab_commands {
        apply_tab_command(&mut state.persisted, command);
    }
    // Apply Settings-page actions — layout changes mutate the dock tree, so
    // they could not be applied mid-draw; an audio change bubbles up to `app`.
    for action in settings_actions {
        match action {
            crate::page::SettingsAction::ApplyLayout(layout) => {
                apply_layout(&mut state.persisted, layout);
            }
            crate::page::SettingsAction::ResetLayout => {
                apply_layout(&mut state.persisted, Layout::Default);
            }
            crate::page::SettingsAction::AudioChanged => {
                intent = Some(ShellIntent::AudioSettingsChanged);
            }
            crate::page::SettingsAction::EqualizerChanged => {
                intent = Some(ShellIntent::EqualizerChanged);
            }
        }
    }
    // Apply navigation requests gathered from the sidebar, menu bar and pages.
    for request in nav {
        apply_nav(&mut state.persisted, request);
    }
    if let Some(text) = copy_to_clipboard {
        ui.ctx().copy_text(text);
    }

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
    nav: &mut Vec<NavRequest>,
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
                    if ui.button("Settings").clicked() {
                        nav.push(NavRequest::replace(Tab::Settings));
                        ui.close();
                    }
                    ui.separator();
                    // Account actions live in the top-right account menu —
                    // this is just a pointer so the File menu is not a dead
                    // end for someone who looks here first.
                    ui.add_enabled(
                        false,
                        egui::Button::new("Account: see the avatar, top-right"),
                    );
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close();
                    }
                });

                ui.menu_button("View", |ui| {
                    ui.set_min_width(200.0);
                    ui.menu_button("Layout", |ui| {
                        ui.set_min_width(160.0);
                        for layout in Layout::all() {
                            let selected = state.persisted.layout == layout;
                            if ui.radio(selected, layout.label()).clicked() {
                                apply_layout(&mut state.persisted, layout);
                                ui.close();
                            }
                        }
                    });
                    if ui.button("Reset layout to default").clicked() {
                        apply_layout(&mut state.persisted, Layout::Default);
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
                    for tab in [Tab::NowPlayingArt, Tab::Queue, Tab::Visualiser, Tab::Debug] {
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
                    if ui
                        .add_enabled(has_track, egui::Button::new("Next track"))
                        .clicked()
                    {
                        intent = Some(ShellIntent::Transport(TransportIntent::Next));
                        ui.close();
                    }
                    if ui
                        .add_enabled(has_track, egui::Button::new("Previous track"))
                        .clicked()
                    {
                        intent = Some(ShellIntent::Transport(TransportIntent::Previous));
                        ui.close();
                    }
                });

                ui.menu_button("Tools", |ui| {
                    ui.set_min_width(180.0);
                    if ui.button("Search").clicked() {
                        nav.push(NavRequest::replace(Tab::Search));
                        ui.close();
                    }
                    // Reopen the most recently closed tab — the menu twin of
                    // `Cmd/Ctrl+Shift+T`, disabled when nothing was closed.
                    let can_reopen = state.persisted.dock_extras.can_reopen_closed();
                    if ui
                        .add_enabled(can_reopen, egui::Button::new("Reopen closed tab"))
                        .clicked()
                    {
                        nav::reopen_last_closed(
                            &mut state.persisted.dock,
                            &mut state.persisted.dock_extras,
                        );
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
                    if ui.button("Keyboard shortcuts").clicked() {
                        nav.push(NavRequest::replace(Tab::Settings));
                        ui.close();
                    }
                });

                // The right side of the menu bar: the account menu (the
                // single entry point for user info / Settings / Log out),
                // then the back / forward / Home navigation shortcuts, then
                // the VSCode-style background-activity indicator. Widgets are
                // added rightmost-first in a `right_to_left` layout, so the
                // visual order reads account, Home, forward, back.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(i) = account_menu(ui, palette, profile, avatar, nav) {
                        intent = Some(i);
                    }
                    ui.add_space(6.0);

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
                        nav.push(NavRequest::replace(Tab::Home));
                    }

                    // Forward then Back — both reflect availability for the
                    // focused tab and are disabled when its history is empty.
                    let can_forward =
                        nav::can_go_forward(&state.persisted.dock, &state.persisted.dock_extras);
                    let can_back =
                        nav::can_go_back(&state.persisted.dock, &state.persisted.dock_extras);
                    let forward = nav_button(
                        ui,
                        palette,
                        spottyfi_ui::Icon::ArrowRight,
                        can_forward,
                        "Forward",
                    );
                    if forward {
                        nav::go_forward(
                            &mut state.persisted.dock,
                            &mut state.persisted.dock_extras,
                        );
                    }
                    let back =
                        nav_button(ui, palette, spottyfi_ui::Icon::ArrowLeft, can_back, "Back");
                    if back {
                        nav::go_back(&mut state.persisted.dock, &mut state.persisted.dock_extras);
                    }

                    activity_indicator(ui, palette, &activity);
                });
            });
        });

    intent
}

/// The account control on the right of the menu bar: the signed-in user's
/// avatar and display name, which open a menu with the user's info, a
/// **Settings** entry (opens the Settings page) and **Log out**.
///
/// This is the single account entry point — the `File` menu only points here.
/// Drawn inside a `right_to_left` layout, so it lands at the bar's far right.
fn account_menu(
    ui: &mut egui::Ui,
    palette: &Palette,
    profile: &UserProfile,
    avatar: Option<&egui::TextureHandle>,
    nav: &mut Vec<NavRequest>,
) -> Option<ShellIntent> {
    let mut intent = None;
    let name = profile.display_name.as_deref().unwrap_or("Spotify user");

    // The clickable trigger: the avatar thumbnail then the display name.
    let label = egui::RichText::new(name)
        .family(spottyfi_ui::fonts::medium())
        .size(11.5)
        .color(palette.text);
    let button = match avatar {
        Some(texture) => egui::Button::image_and_text(
            egui::Image::new((texture.id(), egui::vec2(18.0, 18.0))),
            label,
        ),
        None => egui::Button::new(label),
    }
    .fill(palette.elevated)
    .corner_radius(0);

    egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        ui.set_min_width(200.0);
        // The user-info block: avatar, display name, Spotify id.
        ui.horizontal(|ui| {
            if let Some(texture) = avatar {
                ui.add(egui::Image::new((texture.id(), egui::vec2(36.0, 36.0))));
            }
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(name)
                        .family(spottyfi_ui::fonts::semibold())
                        .size(12.5)
                        .color(palette.text),
                );
                ui.label(spottyfi_ui::components::muted(
                    palette,
                    profile.id.to_string(),
                    10.0,
                ));
            });
        });
        ui.separator();
        if ui.button("Settings").clicked() {
            nav.push(NavRequest::replace(Tab::Settings));
            ui.close();
        }
        if ui.button("Log out").clicked() {
            intent = Some(ShellIntent::Logout);
            ui.close();
        }
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
#[allow(clippy::too_many_arguments)]
fn dock(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    palette: &Palette,
    playback: &PlaybackState,
    queue: &QueueState,
    transport_ui: &mut TransportUiState,
    engine: &EngineStatus,
    spectrum: &SpectrumAnalyzer,
) -> Vec<DockIntent> {
    // Borrow the session and the persisted state as disjoint fields so the
    // page registry, the dock tree, the dock extras and the Settings view can
    // all be used together.
    let ShellState {
        persisted,
        session,
        local_folder_draft,
        ..
    } = state;
    let Some(session) = session.as_mut() else {
        // No API attached yet (the post-login frame before `attach_api`).
        ui.centered_and_justified(|ui| {
            ui.add(egui::Spinner::new().size(28.0).color(palette.accent));
        });
        return Vec::new();
    };

    // Drop pages — and pin / history bookkeeping — for tabs closed since the
    // last frame. The closed-tab stack is intentionally left alone.
    let all_tabs: Vec<Tab> = persisted
        .dock
        .iter_all_tabs()
        .map(|(_, tab)| tab.clone())
        .collect();
    session
        .pages
        .retain_open(all_tabs.iter().filter(|t| t.is_page()));
    persisted.dock_extras.retain_open(all_tabs.iter());

    // Destructure the persisted state into disjoint field borrows so the
    // `DockArea` (which borrows `dock`) and the Settings view (which borrows
    // `theme` / `density` / `settings`) can both be live this frame.
    let PersistedShell {
        dock,
        theme,
        density,
        dock_extras,
        layout,
        settings,
        visualiser_mode,
        ..
    } = persisted;

    let mut viewer = ShellTabViewer {
        ctx: TabContext {
            palette: *palette,
            playback,
            queue,
            transport_ui,
            engine,
            pages: &mut session.pages,
            spectrum,
            visualiser_mode,
            settings_view: tabs::SettingsView {
                theme,
                density,
                layout: *layout,
                settings,
                local_folder_draft,
            },
            pinned: &dock_extras.pinned,
            intents: Vec::new(),
        },
    };

    egui_dock::DockArea::new(dock)
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
