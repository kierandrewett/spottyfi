//! The dock tab model.
//!
//! Every surface in the centre dock is a [`Tab`]. Tabs split into **page tabs**
//! (navigable content: Home, Library, Liked Songs, Playlist, Album, Artist)
//! and **panel tabs** (auxiliary surfaces: Now Playing Art, Queue, Debug).
//!
//! A `Tab` is only a lightweight, serialisable *key*: the dock
//! ([`egui_dock::DockState`]) stores `Tab`s so the whole layout round-trips
//! through RON. The live, stateful [`Page`](crate::page::Page) objects — which
//! carry the in-flight loads and per-page UI state — live in a
//! [`PageRegistry`](crate::page::PageRegistry) keyed by `Tab`.

use serde::{Deserialize, Serialize};
use spottyfi_audio::{PlaybackState, QueueState, QueueTrack};
use spottyfi_ui::theme::Palette;

use spottyfi_ui::components::Density;
use spottyfi_ui::theme::Theme;

use crate::page::SettingsContext as PageSettingsContext;
use crate::page::{settings_page, PageAction, PageContext, PageRegistry, SettingsAction};
use crate::playback_controller::EngineStatus;
use crate::settings::AppSettings;
use crate::shell::Layout;
use crate::transport::{self, TransportIntent, TransportUiState};

/// A single dock tab — the serialisable key for one centre-dock surface.
///
/// Id-carrying variants ([`Tab::Playlist`], [`Tab::Album`], [`Tab::Artist`])
/// identify which object the page renders; the bare base-62 Spotify id is
/// stored, not the full URI.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    /// The Home landing page.
    Home,
    /// The Your Library page (playlists + saved albums).
    Library,
    /// The Liked Songs (saved tracks) page.
    LikedSongs,
    /// A playlist page, keyed by playlist id.
    Playlist(String),
    /// An album page, keyed by album id.
    Album(String),
    /// An artist page, keyed by artist id.
    Artist(String),
    /// The Search page (real search lands in Phase 6).
    Search,
    /// The Browse page: the genre/category grid plus Last.fm charts.
    Browse,
    /// A browse-category page, keyed by the Spotify category id.
    Category(String),
    /// The Charts page: Last.fm global top tracks and artists.
    Charts,
    /// The New Releases page (Spotify `new-releases`).
    NewReleases,
    /// The Made For You page: recommendations seeded from the user's top
    /// artists and tracks via Last.fm.
    MadeForYou,
    /// The Settings page: audio, equalizer, local files, appearance, hotkeys.
    Settings,
    /// A not-yet-built page. Carries its display name; the body is a "coming
    /// soon" placeholder until the real page is implemented.
    Placeholder(String),
    /// The Now Playing album-art panel.
    NowPlayingArt,
    /// The play queue panel.
    Queue,
    /// The debug panel: the "paste a URI and play" control.
    Debug,
}

impl Tab {
    /// The tab's static fallback title.
    ///
    /// Page tabs may show a richer, data-derived title once their page has
    /// loaded (the playlist's name, say); the registry supplies that. This is
    /// the label shown before the load resolves.
    #[must_use]
    pub fn title(&self) -> &'static str {
        match self {
            Tab::Home => "Home",
            Tab::Library => "Your Library",
            Tab::LikedSongs => "Liked Songs",
            Tab::Playlist(_) => "Playlist",
            Tab::Album(_) => "Album",
            Tab::Artist(_) => "Artist",
            Tab::Search => "Search",
            Tab::Browse => "Browse",
            Tab::Category(_) => "Category",
            Tab::Charts => "Charts",
            Tab::NewReleases => "New Releases",
            Tab::MadeForYou => "Made For You",
            Tab::Settings => "Settings",
            Tab::Placeholder(_) => "Coming soon",
            Tab::NowPlayingArt => "Now Playing",
            Tab::Queue => "Queue",
            Tab::Debug => "Debug",
        }
    }

    /// The plain, human-readable tab title shown on the dock tab bar.
    ///
    /// For an id-carrying page (playlist / album / artist) this is the
    /// object's own name, supplied by the page registry once its data has
    /// loaded; for everything else it is the static label. Never a
    /// breadcrumb path — see `docs/ui-reference.md`.
    #[must_use]
    pub fn display_title(&self, page_title: &str) -> String {
        match self {
            Tab::Playlist(_) | Tab::Album(_) | Tab::Artist(_) | Tab::Category(_) => {
                page_title.to_owned()
            }
            Tab::Placeholder(name) => name.clone(),
            _ => self.title().to_owned(),
        }
    }

    /// Whether this tab is a panel (as opposed to a navigable page).
    ///
    /// Panels are closeable; the Home page is kept open so the dock is never
    /// empty. `Placeholder` is a self-rendered surface — neither a
    /// registry-backed page nor an auxiliary panel — and is classified as a
    /// panel so the page registry never tries to build it. `Settings` is
    /// likewise self-rendered: it needs mutable shell state the registry
    /// cannot hand it, so it renders straight from the shell. `Search` is a
    /// real, registry-backed page (Phase 6).
    #[must_use]
    pub fn is_panel(&self) -> bool {
        matches!(
            self,
            Tab::NowPlayingArt | Tab::Queue | Tab::Debug | Tab::Placeholder(_) | Tab::Settings
        )
    }

    /// Whether this tab is a navigable page (rendered via the page registry).
    #[must_use]
    pub fn is_page(&self) -> bool {
        !self.is_panel()
    }
}

/// Something a dock tab raised this frame that the shell must act on.
#[derive(Debug, Clone, PartialEq)]
pub enum DockIntent {
    /// A transport command (e.g. from the debug panel, or a page's play).
    Transport(TransportIntent),
    /// Open (navigate to) a page tab, replacing the focused leaf.
    Open(Tab),
    /// Open (navigate to) a page tab in a brand-new tab (Ctrl/Cmd-click).
    OpenInNewTab(Tab),
    /// Copy a string to the system clipboard (a Spotify URI).
    CopyToClipboard(String),
    /// A tab-management command raised from a tab's right-click menu or a
    /// middle-click — applied to the dock after the `DockArea` has been drawn.
    Tab(TabCommand),
    /// A Settings-page action (apply/reset layout, audio settings changed) —
    /// applied after the `DockArea` draw, since some mutate the dock tree.
    Settings(SettingsAction),
}

/// A navigation request the shell collects this frame and applies to the dock
/// once the panels have been drawn.
///
/// `new_tab` carries the Ctrl/Cmd modifier: a plain navigation **replaces**
/// the focused tab (the `docs/docking.md` rule), Ctrl/Cmd-held **opens a new
/// tab**. `main_pane` is set for sidebar navigation, which must always land in
/// the centre tab group rather than whichever leaf happens to be focused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavRequest {
    /// The tab to navigate to.
    pub tab: Tab,
    /// Whether to open a new tab (Ctrl/Cmd-held) rather than replace.
    pub new_tab: bool,
    /// Whether to force the navigation into the main (centre) pane.
    pub main_pane: bool,
}

impl NavRequest {
    /// A plain navigation: replace the focused tab.
    #[must_use]
    pub fn replace(tab: Tab) -> Self {
        Self {
            tab,
            new_tab: false,
            main_pane: false,
        }
    }

    /// A Ctrl/Cmd-held navigation: open a new tab.
    #[must_use]
    pub fn new_tab(tab: Tab) -> Self {
        Self {
            tab,
            new_tab: true,
            main_pane: false,
        }
    }

    /// Force this navigation into the main (centre) pane — used by the sidebar
    /// so a click always opens a page in the centre tab group.
    #[must_use]
    pub fn in_main_pane(mut self) -> Self {
        self.main_pane = true;
        self
    }
}

/// A tab-management command raised from the dock's tab bar (right-click menu,
/// middle-click). Applied to the dock by the shell once the `DockArea` draw is
/// complete — the dock cannot be mutated mid-draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabCommand {
    /// Close this exact tab.
    Close(Tab),
    /// Close every tab except this one (pinned tabs are spared).
    CloseOthers(Tab),
    /// Close every tab to the right of this one in its leaf (pinned spared).
    CloseToRight(Tab),
    /// Open a second tab carrying the same surface as this one.
    Duplicate(Tab),
    /// Toggle this tab's pinned state.
    TogglePin(Tab),
}

/// The mutable shell state the self-rendered Settings tab needs.
///
/// The dock's `DockArea` borrows `PersistedShell::dock`; these are the
/// remaining fields the Settings page mutates, borrowed disjointly so both can
/// be live at once. Layout changes mutate the dock tree, so they are deferred
/// — raised as [`SettingsAction`]s and applied once the `DockArea` draw ends.
pub struct SettingsView<'a> {
    /// The selected colour theme.
    pub theme: &'a mut Theme,
    /// The selected row density.
    pub density: &'a mut Density,
    /// The currently-applied dock layout.
    pub layout: Layout,
    /// The persisted power-user settings block.
    pub settings: &'a mut AppSettings,
    /// The draft folder path being typed in the Local Files section.
    pub local_folder_draft: &'a mut String,
}

/// Everything the dock's [`egui_dock::TabViewer`] needs to render a tab's body,
/// borrowed for the duration of one `DockArea::show` call.
pub struct TabContext<'a> {
    /// The active theme palette.
    pub palette: Palette,
    /// The live playback snapshot.
    pub playback: &'a PlaybackState,
    /// The live queue snapshot, rendered by the Queue panel.
    pub queue: &'a QueueState,
    /// Mutable transport UI state (the debug text field lives here).
    pub transport_ui: &'a mut TransportUiState,
    /// The audio-engine lifecycle status, for the debug panel.
    pub engine: &'a EngineStatus,
    /// The live page objects, keyed by tab.
    pub pages: &'a mut PageRegistry,
    /// The mutable shell state the Settings tab renders against.
    pub settings_view: SettingsView<'a>,
    /// A read-only view of the pinned-tab set, so the right-click menu can
    /// show Pin vs Unpin. The dock cannot be mutated mid-draw, so pin toggles
    /// are raised as [`TabCommand`]s and applied afterwards.
    pub pinned: &'a [Tab],
    /// Any [`DockIntent`]s raised this frame, in order.
    pub intents: Vec<DockIntent>,
}

impl TabContext<'_> {
    /// Whether `tab` is currently pinned.
    fn is_pinned(&self, tab: &Tab) -> bool {
        self.pinned.iter().any(|t| t == tab)
    }
}

/// The `egui_dock` tab viewer: renders tab titles and bodies.
pub struct ShellTabViewer<'a> {
    /// The per-frame context shared with every tab body.
    pub ctx: TabContext<'a>,
}

impl egui_dock::TabViewer for ShellTabViewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        let page_title = if tab.is_page() {
            self.ctx.pages.title(tab)
        } else {
            tab.title().to_owned()
        };
        tab.display_title(&page_title).into()
    }

    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::new(("spottyfi-tab", tab.clone()))
    }

    fn is_closeable(&self, tab: &Self::Tab) -> bool {
        // Home stays open so the dock is never empty; a pinned tab keeps no
        // close button (browser behaviour) — it closes only via its menu.
        !matches!(tab, Tab::Home) && !self.ctx.is_pinned(tab)
    }

    /// Right-click a tab: the Close / Close others / Close to right / Duplicate
    /// / Pin menu. The dock cannot be mutated mid-draw, so each entry raises a
    /// [`TabCommand`] the shell applies once the `DockArea` draw completes.
    fn context_menu(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab, _path: egui_dock::NodePath) {
        ui.set_min_width(160.0);
        let is_home = matches!(tab, Tab::Home);
        let pinned = self.ctx.is_pinned(tab);

        // Home is never closeable; a pinned tab is spared by Close/others/right.
        if ui
            .add_enabled(!is_home && !pinned, egui::Button::new("Close"))
            .clicked()
        {
            self.ctx
                .intents
                .push(DockIntent::Tab(TabCommand::Close(tab.clone())));
            ui.close();
        }
        if ui.button("Close others").clicked() {
            self.ctx
                .intents
                .push(DockIntent::Tab(TabCommand::CloseOthers(tab.clone())));
            ui.close();
        }
        if ui.button("Close to the right").clicked() {
            self.ctx
                .intents
                .push(DockIntent::Tab(TabCommand::CloseToRight(tab.clone())));
            ui.close();
        }
        ui.separator();
        if ui.button("Duplicate").clicked() {
            self.ctx
                .intents
                .push(DockIntent::Tab(TabCommand::Duplicate(tab.clone())));
            ui.close();
        }
        let pin_label = if pinned { "Unpin tab" } else { "Pin tab" };
        if ui.button(pin_label).clicked() {
            self.ctx
                .intents
                .push(DockIntent::Tab(TabCommand::TogglePin(tab.clone())));
            ui.close();
        }
    }

    /// Middle-clicking a tab button closes it (browser behaviour). Home and
    /// pinned tabs are spared — the close is raised as a [`TabCommand`].
    fn on_tab_button(&mut self, tab: &mut Self::Tab, response: &egui::Response) {
        if response.middle_clicked() && !matches!(tab, Tab::Home) && !self.ctx.is_pinned(tab) {
            self.ctx
                .intents
                .push(DockIntent::Tab(TabCommand::Close(tab.clone())));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        let palette = self.ctx.palette;
        egui::Frame::new()
            .fill(palette.base)
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                match tab {
                    Tab::NowPlayingArt => now_playing_art_tab(ui, &self.ctx),
                    Tab::Queue => {
                        for intent in queue_tab(ui, &self.ctx) {
                            self.ctx.intents.push(DockIntent::Transport(intent));
                        }
                    }
                    Tab::Placeholder(name) => placeholder_tab(ui, &self.ctx, name),
                    Tab::Settings => {
                        let view = &mut self.ctx.settings_view;
                        let mut page_ctx = PageSettingsContext {
                            palette,
                            theme: view.theme,
                            density: view.density,
                            layout: view.layout,
                            settings: view.settings,
                            local_folder_draft: view.local_folder_draft,
                        };
                        for action in settings_page(ui, &mut page_ctx) {
                            self.ctx.intents.push(DockIntent::Settings(action));
                        }
                    }
                    Tab::Debug => {
                        if let Some(intent) = debug_tab(ui, &mut self.ctx) {
                            self.ctx.intents.push(DockIntent::Transport(intent));
                        }
                    }
                    page_tab => {
                        let page_ctx = PageContext {
                            palette,
                            playback: self.ctx.playback,
                        };
                        if let Some(action) = self.ctx.pages.ui(page_tab, ui, &page_ctx) {
                            // A Ctrl/Cmd-held in-page link opens a new tab; a
                            // plain click replaces the focused tab. The
                            // modifier is read at click time, this frame.
                            let new_tab = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
                            self.ctx
                                .intents
                                .push(page_action_to_intent(action, new_tab));
                        }
                    }
                }
            });
    }
}

/// Translate a [`PageAction`] into the shell-level [`DockIntent`].
///
/// `new_tab` carries the Ctrl/Cmd modifier state read at click time: an
/// [`PageAction::Open`] becomes [`DockIntent::OpenInNewTab`] when it is held,
/// [`DockIntent::Open`] (replace the focused tab) otherwise.
fn page_action_to_intent(action: PageAction, new_tab: bool) -> DockIntent {
    match action {
        PageAction::PlayContext {
            uri,
            name,
            tracks,
            offset,
        } => DockIntent::Transport(TransportIntent::PlayContext {
            uri,
            name,
            tracks,
            offset,
        }),
        PageAction::PlayNext(track) => DockIntent::Transport(TransportIntent::PlayNext(track)),
        PageAction::Enqueue(track) => DockIntent::Transport(TransportIntent::Enqueue(track)),
        PageAction::Open(tab) if new_tab => DockIntent::OpenInNewTab(tab),
        PageAction::Open(tab) => DockIntent::Open(tab),
        PageAction::CopyToClipboard(text) => DockIntent::CopyToClipboard(text),
    }
}

/// The Now Playing Art panel: the current track's album art, large.
fn now_playing_art_tab(ui: &mut egui::Ui, ctx: &TabContext<'_>) {
    let palette = ctx.palette;
    ui.vertical_centered(|ui| {
        let side = ui
            .available_width()
            .min(ui.available_height() - 80.0)
            .max(80.0);
        let art_url = ctx
            .playback
            .track
            .as_ref()
            .and_then(|t| t.art_url.as_deref());
        spottyfi_ui::components::album_art(ui, &palette, art_url, side, 10.0);
        ui.add_space(12.0);
        match &ctx.playback.track {
            Some(track) => {
                ui.label(
                    egui::RichText::new(&track.title)
                        .family(spottyfi_ui::fonts::semibold())
                        .size(18.0)
                        .color(palette.text),
                );
                ui.label(spottyfi_ui::components::muted(
                    &palette,
                    track.artist_line(),
                    13.0,
                ));
            }
            None => {
                ui.label(spottyfi_ui::components::muted(
                    &palette,
                    "Nothing playing",
                    13.0,
                ));
            }
        }
    });
}

/// The fixed height of a queue-panel row — dense and flat.
const QUEUE_ROW_HEIGHT: f32 = 44.0;

/// The Queue panel: a Now Playing block, the "Next from <context>" section and
/// the manual queue. Manual entries can be clicked to skip to them and dragged
/// to reorder. Returns every [`TransportIntent`] the user raised this frame.
fn queue_tab(ui: &mut egui::Ui, ctx: &TabContext<'_>) -> Vec<TransportIntent> {
    let palette = ctx.palette;
    let queue = ctx.queue;
    let mut intents = Vec::new();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;

            // Now Playing.
            spottyfi_ui::components::section_header(ui, &palette, "Now playing");
            match &queue.current {
                Some(track) => {
                    queue_row(ui, &palette, track, RowKind::NowPlaying);
                }
                None => {
                    ui.add_space(2.0);
                    ui.label(spottyfi_ui::components::muted(
                        &palette,
                        "Nothing playing.",
                        12.0,
                    ));
                }
            }

            // Next from <context>.
            ui.add_space(12.0);
            let context_label = queue
                .context_name
                .as_deref()
                .map_or_else(|| "Next up".to_owned(), |name| format!("Next from {name}"));
            spottyfi_ui::components::section_header(ui, &palette, &context_label);
            if queue.up_next_context.is_empty() {
                ui.add_space(2.0);
                ui.label(spottyfi_ui::components::muted(
                    &palette,
                    "No upcoming context tracks.",
                    12.0,
                ));
            } else {
                // The context cursor: the current track's absolute index. The
                // first upcoming track sits at `cursor + 1`.
                let base = queue.context_index.map_or(0, |i| i + 1);
                for (offset, track) in queue.up_next_context.iter().enumerate() {
                    if queue_row(ui, &palette, track, RowKind::UpNext).clicked() {
                        intents.push(TransportIntent::SkipToContext(base + offset));
                    }
                }
            }

            // The manual queue — click to skip, drag to reorder.
            ui.add_space(12.0);
            spottyfi_ui::components::section_header(ui, &palette, "Queue");
            if queue.manual.is_empty() {
                ui.add_space(2.0);
                ui.label(spottyfi_ui::components::muted(
                    &palette,
                    "Add tracks with “Add to queue”.",
                    12.0,
                ));
            } else {
                manual_queue(ui, &palette, &queue.manual, &mut intents);
            }
        });

    intents
}

/// Which queue section a row belongs to — controls its accent and affordances.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    /// The single Now Playing row (accent-green, not clickable).
    NowPlaying,
    /// An upcoming context row (click to skip to it).
    UpNext,
    /// A manual-queue row (click to skip, drag to reorder, remove button).
    Manual,
}

/// Render one dense, flat queue row: a small thumbnail, the title over the
/// artist line, and a trailing duration. Returns the row's click response.
fn queue_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    track: &QueueTrack,
    kind: RowKind,
) -> egui::Response {
    let sense = if kind == RowKind::NowPlaying {
        egui::Sense::hover()
    } else {
        egui::Sense::click()
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), QUEUE_ROW_HEIGHT), sense);
    if !ui.is_rect_visible(rect) {
        return response;
    }

    // A flat, full-bleed hover highlight — sharp corners, no inset.
    if response.hovered() && kind != RowKind::NowPlaying {
        ui.painter().rect_filled(rect, 0, palette.hover);
    }

    let title_color = if kind == RowKind::NowPlaying {
        palette.accent
    } else {
        palette.text
    };

    let mut content = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(6.0, 4.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    spottyfi_ui::components::album_art(&mut content, palette, track.art_url.as_deref(), 34.0, 0.0);
    content.add_space(8.0);
    content.vertical(|ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(&track.title)
                    .family(spottyfi_ui::fonts::medium())
                    .size(12.5)
                    .color(title_color),
            )
            .truncate(),
        );
        ui.add(
            egui::Label::new(spottyfi_ui::components::muted(
                palette,
                track.artist_line(),
                11.0,
            ))
            .truncate(),
        );
    });
    content.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(spottyfi_ui::components::muted(
            palette,
            fmt_duration(track.duration),
            10.5,
        ));
    });

    if kind != RowKind::NowPlaying {
        response
            .clone()
            .on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    response
}

/// Render the manual queue with click-to-skip and drag-to-reorder.
///
/// Dragging a row over another row's half emits a [`TransportIntent::Reorder
/// Manual`] on drop; a click (no drag) skips to that entry; the trailing ✕
/// removes it.
fn manual_queue(
    ui: &mut egui::Ui,
    palette: &Palette,
    manual: &[QueueTrack],
    intents: &mut Vec<TransportIntent>,
) {
    let mut drop_target: Option<usize> = None;
    let mut dragged_from: Option<usize> = None;

    for (index, track) in manual.iter().enumerate() {
        let row_id = egui::Id::new(("queue-manual", index, &track.uri));

        let response = ui
            .dnd_drag_source(row_id, index, |ui| {
                manual_row(ui, palette, track, index, intents);
            })
            .response;

        if response.dragged() || ui.ctx().is_being_dragged(row_id) {
            dragged_from = Some(index);
        }

        // When something is being dragged, treat each row as a drop slot.
        if egui::DragAndDrop::has_payload_of_type::<usize>(ui.ctx()) {
            let rect = response.rect;
            let hovered = ui.rect_contains_pointer(rect);
            if hovered {
                drop_target = Some(index);
                ui.painter().hline(
                    rect.x_range(),
                    rect.top(),
                    egui::Stroke::new(2.0, palette.accent),
                );
            }
        }
    }

    // On release, apply the reorder.
    if ui.input(|i| i.pointer.any_released()) {
        if let (Some(from), Some(to)) = (dragged_from, drop_target) {
            if from != to {
                intents.push(TransportIntent::ReorderManual { from, to });
            }
        }
    }
}

/// One manual-queue row: a clickable [`queue_row`] with a trailing remove ✕.
fn manual_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    track: &QueueTrack,
    index: usize,
    intents: &mut Vec<TransportIntent>,
) {
    let response = queue_row(ui, palette, track, RowKind::Manual);
    // A plain click (not the end of a drag) skips straight to this entry.
    if response.clicked() {
        intents.push(TransportIntent::SkipToManual(index));
    }
    response.context_menu(|ui| {
        ui.set_min_width(150.0);
        if ui.button("Play now").clicked() {
            intents.push(TransportIntent::SkipToManual(index));
            ui.close();
        }
        if ui.button("Remove from queue").clicked() {
            intents.push(TransportIntent::RemoveManual(index));
            ui.close();
        }
    });
}

/// Format a [`std::time::Duration`] as `m:ss`.
fn fmt_duration(d: std::time::Duration) -> String {
    let total = d.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}

/// A not-yet-built page (Browse, Charts, New Releases, …).
fn placeholder_tab(ui: &mut egui::Ui, ctx: &TabContext<'_>, name: &str) {
    coming_soon(
        ui,
        &ctx.palette,
        name,
        "This page is part of a later phase and isn't built yet.",
    );
}

/// A centred "coming soon" placeholder used by pages that aren't built yet.
fn coming_soon(ui: &mut egui::Ui, palette: &Palette, title: &str, detail: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.34);
        spottyfi_ui::icons::icon(ui, spottyfi_ui::Icon::Discover, 40.0, palette.text_muted);
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(title)
                .family(spottyfi_ui::fonts::semibold())
                .size(18.0)
                .color(palette.text),
        );
        ui.add_space(4.0);
        ui.label(spottyfi_ui::components::muted(palette, detail, 12.0));
    });
}

/// The Debug panel: the "paste a URI and play" control kept reachable for
/// quick playback testing.
fn debug_tab(ui: &mut egui::Ui, ctx: &mut TabContext<'_>) -> Option<TransportIntent> {
    let palette = ctx.palette;
    spottyfi_ui::components::section_header(ui, &palette, "Debug");
    ui.add_space(4.0);
    transport::debug_play_control(ui, &palette, ctx.transport_ui, ctx.engine)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_and_panels_are_classified() {
        assert!(Tab::Home.is_page());
        assert!(Tab::Playlist("x".into()).is_page());
        assert!(Tab::Album("x".into()).is_page());
        assert!(Tab::Queue.is_panel());
        assert!(Tab::Debug.is_panel());
        assert!(!Tab::Debug.is_page());
    }

    #[test]
    fn id_carrying_tabs_compare_by_id() {
        assert_eq!(Tab::Album("a".into()), Tab::Album("a".into()));
        assert_ne!(Tab::Album("a".into()), Tab::Album("b".into()));
    }

    #[test]
    fn page_actions_map_to_dock_intents() {
        assert_eq!(
            page_action_to_intent(PageAction::Open(Tab::LikedSongs), false),
            DockIntent::Open(Tab::LikedSongs)
        );
        assert_eq!(
            page_action_to_intent(PageAction::CopyToClipboard("uri".into()), false),
            DockIntent::CopyToClipboard("uri".into())
        );
    }

    #[test]
    fn ctrl_held_open_action_opens_a_new_tab() {
        // A plain click replaces the focused tab; Ctrl/Cmd-held opens a new one.
        assert_eq!(
            page_action_to_intent(PageAction::Open(Tab::Browse), false),
            DockIntent::Open(Tab::Browse)
        );
        assert_eq!(
            page_action_to_intent(PageAction::Open(Tab::Browse), true),
            DockIntent::OpenInNewTab(Tab::Browse)
        );
    }

    #[test]
    fn play_context_action_maps_to_a_transport_intent() {
        let action = PageAction::PlayContext {
            uri: "spotify:playlist:x".into(),
            name: "X".into(),
            tracks: Vec::new(),
            offset: 0,
        };
        assert!(matches!(
            page_action_to_intent(action, false),
            DockIntent::Transport(TransportIntent::PlayContext { .. })
        ));
    }
}
