//! The bundled [Lucide](https://lucide.dev) line-icon set, rendered as real
//! SVGs.
//!
//! Every icon Spottyfi uses is committed under `assets/icons/` and embedded
//! into the binary with [`include_bytes!`]. They are monochrome line icons
//! whose stroke is plain white in the source SVG, so a per-widget
//! [`egui::Image::tint`] recolours them to any theme colour.
//!
//! Rasterisation goes through `egui_extras`' `svg` feature (resvg). The
//! `egui_extras::install_image_loaders` call the `app` crate makes at startup
//! registers the SVG loader; [`Icon::image`] then hands egui a `bytes://` URI
//! plus the embedded bytes and egui caches the rasterised texture per size.

/// One bundled line icon.
///
/// The enum is the single source of truth for which icons exist; each variant
/// maps to a committed `assets/icons/<name>.svg` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    /// Home / house.
    Home,
    /// Magnifying glass — search.
    Search,
    /// Compass — Browse.
    Browse,
    /// Bar chart — Charts.
    Charts,
    /// Calendar — New Releases.
    NewReleases,
    /// Sparkles — Discover.
    Discover,
    /// Podcast mic — Podcasts.
    Podcast,
    /// Sparkles — Made For You.
    MadeForYou,
    /// Clock — Recently Played.
    RecentlyPlayed,
    /// Heart — Liked Songs / save.
    Heart,
    /// Disc — albums.
    Disc,
    /// User — artists / account.
    User,
    /// Music note — generic track glyph.
    Music,
    /// Library — Your Library.
    Library,
    /// Radio — local files / radio.
    Radio,
    /// List — local files / generic list.
    List,
    /// Plus — add / new playlist.
    Plus,
    /// Caret pointing down — an expanded tree section.
    ChevronDown,
    /// Caret pointing right — a collapsed tree section.
    ChevronRight,
    /// Caret pointing left — back / collapse.
    ChevronLeft,
    /// Triangle play.
    Play,
    /// Pause bars.
    Pause,
    /// Skip to previous.
    SkipBack,
    /// Skip to next.
    SkipForward,
    /// Shuffle arrows.
    Shuffle,
    /// Repeat arrows.
    Repeat,
    /// Speaker at full volume.
    Volume,
    /// Speaker muted.
    VolumeMuted,
    /// Cog — settings.
    Settings,
    /// Monitor + speaker — devices / Connect.
    Devices,
    /// Cross — close.
    Close,
    /// Down arrow — download / offline.
    Download,
    /// Left arrow — navigate back.
    ArrowLeft,
    /// Right arrow — navigate forward.
    ArrowRight,
    /// Music-note list — playlists / the queue.
    Queue,
}

impl Icon {
    /// The stable `bytes://` URI egui caches this icon's texture under.
    fn uri(self) -> &'static str {
        match self {
            Icon::Home => "bytes://spottyfi-icon-home",
            Icon::Search => "bytes://spottyfi-icon-search",
            Icon::Browse => "bytes://spottyfi-icon-browse",
            Icon::Charts => "bytes://spottyfi-icon-charts",
            Icon::NewReleases => "bytes://spottyfi-icon-new-releases",
            Icon::Discover => "bytes://spottyfi-icon-discover",
            Icon::Podcast => "bytes://spottyfi-icon-podcast",
            Icon::MadeForYou => "bytes://spottyfi-icon-made-for-you",
            Icon::RecentlyPlayed => "bytes://spottyfi-icon-recently-played",
            Icon::Heart => "bytes://spottyfi-icon-heart",
            Icon::Disc => "bytes://spottyfi-icon-disc",
            Icon::User => "bytes://spottyfi-icon-user",
            Icon::Music => "bytes://spottyfi-icon-music",
            Icon::Library => "bytes://spottyfi-icon-library",
            Icon::Radio => "bytes://spottyfi-icon-radio",
            Icon::List => "bytes://spottyfi-icon-list",
            Icon::Plus => "bytes://spottyfi-icon-plus",
            Icon::ChevronDown => "bytes://spottyfi-icon-chevron-down",
            Icon::ChevronRight => "bytes://spottyfi-icon-chevron-right",
            Icon::ChevronLeft => "bytes://spottyfi-icon-chevron-left",
            Icon::Play => "bytes://spottyfi-icon-play",
            Icon::Pause => "bytes://spottyfi-icon-pause",
            Icon::SkipBack => "bytes://spottyfi-icon-skip-back",
            Icon::SkipForward => "bytes://spottyfi-icon-skip-forward",
            Icon::Shuffle => "bytes://spottyfi-icon-shuffle",
            Icon::Repeat => "bytes://spottyfi-icon-repeat",
            Icon::Volume => "bytes://spottyfi-icon-volume",
            Icon::VolumeMuted => "bytes://spottyfi-icon-volume-muted",
            Icon::Settings => "bytes://spottyfi-icon-settings",
            Icon::Devices => "bytes://spottyfi-icon-devices",
            Icon::Close => "bytes://spottyfi-icon-close",
            Icon::Download => "bytes://spottyfi-icon-download",
            Icon::ArrowLeft => "bytes://spottyfi-icon-arrow-left",
            Icon::ArrowRight => "bytes://spottyfi-icon-arrow-right",
            Icon::Queue => "bytes://spottyfi-icon-queue",
        }
    }

    /// The embedded SVG bytes for this icon.
    fn bytes(self) -> &'static [u8] {
        match self {
            Icon::Home => include_bytes!("../assets/icons/house.svg"),
            Icon::Search => include_bytes!("../assets/icons/search.svg"),
            Icon::Browse => include_bytes!("../assets/icons/compass.svg"),
            Icon::Charts => include_bytes!("../assets/icons/chart-bar.svg"),
            Icon::NewReleases => include_bytes!("../assets/icons/calendar.svg"),
            Icon::Discover => include_bytes!("../assets/icons/sparkles.svg"),
            Icon::Podcast => include_bytes!("../assets/icons/podcast.svg"),
            Icon::MadeForYou => include_bytes!("../assets/icons/sparkles.svg"),
            Icon::RecentlyPlayed => include_bytes!("../assets/icons/clock.svg"),
            Icon::Heart => include_bytes!("../assets/icons/heart.svg"),
            Icon::Disc => include_bytes!("../assets/icons/disc-3.svg"),
            Icon::User => include_bytes!("../assets/icons/user.svg"),
            Icon::Music => include_bytes!("../assets/icons/music.svg"),
            Icon::Library => include_bytes!("../assets/icons/library.svg"),
            Icon::Radio => include_bytes!("../assets/icons/radio.svg"),
            Icon::List => include_bytes!("../assets/icons/list.svg"),
            Icon::Plus => include_bytes!("../assets/icons/plus.svg"),
            Icon::ChevronDown => include_bytes!("../assets/icons/chevron-down.svg"),
            Icon::ChevronRight => include_bytes!("../assets/icons/chevron-right.svg"),
            Icon::ChevronLeft => include_bytes!("../assets/icons/chevron-left.svg"),
            Icon::Play => include_bytes!("../assets/icons/play.svg"),
            Icon::Pause => include_bytes!("../assets/icons/pause.svg"),
            Icon::SkipBack => include_bytes!("../assets/icons/skip-back.svg"),
            Icon::SkipForward => include_bytes!("../assets/icons/skip-forward.svg"),
            Icon::Shuffle => include_bytes!("../assets/icons/shuffle.svg"),
            Icon::Repeat => include_bytes!("../assets/icons/repeat.svg"),
            Icon::Volume => include_bytes!("../assets/icons/volume-2.svg"),
            Icon::VolumeMuted => include_bytes!("../assets/icons/volume-x.svg"),
            Icon::Settings => include_bytes!("../assets/icons/settings.svg"),
            Icon::Devices => include_bytes!("../assets/icons/monitor-speaker.svg"),
            Icon::Close => include_bytes!("../assets/icons/x.svg"),
            Icon::Download => include_bytes!("../assets/icons/download.svg"),
            Icon::ArrowLeft => include_bytes!("../assets/icons/arrow-left.svg"),
            Icon::ArrowRight => include_bytes!("../assets/icons/arrow-right.svg"),
            Icon::Queue => include_bytes!("../assets/icons/list-music.svg"),
        }
    }

    /// An [`egui::Image`] for this icon, sized to a `size`×`size` square and
    /// tinted `color`.
    ///
    /// The image resolves through the stock `egui_extras` SVG loader; egui
    /// caches one rasterised texture per (uri, size) pair.
    pub fn image(self, size: f32, color: egui::Color32) -> egui::Image<'static> {
        egui::Image::from_bytes(self.uri(), self.bytes())
            .fit_to_exact_size(egui::vec2(size, size))
            .tint(color)
    }
}

/// Draw a bare, tinted icon at `size` points. Returns the hover response.
pub fn icon(ui: &mut egui::Ui, glyph: Icon, size: f32, color: egui::Color32) -> egui::Response {
    ui.add(glyph.image(size, color))
}

/// A frameless, clickable icon button.
///
/// `active` tints the icon with the accent colour (a toggled control); an
/// inactive button uses the muted text colour and brightens on hover.
pub fn icon_button(
    ui: &mut egui::Ui,
    palette: &crate::theme::Palette,
    glyph: Icon,
    size: f32,
    active: bool,
    tooltip: &str,
) -> egui::Response {
    let pad = egui::vec2(6.0, 6.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(size, size) + pad * 2.0, egui::Sense::click());

    let color = if active {
        palette.accent
    } else if response.hovered() {
        palette.text
    } else {
        palette.text_muted
    };
    if ui.is_rect_visible(rect) {
        let image_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(size, size));
        glyph.image(size, color).paint_at(ui, image_rect);
    }

    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}
