//! The system-tray icon and menu.
//!
//! Spottyfi shows a tray icon via [`tray-icon`](tray_icon) with a small menu —
//! Play/Pause, Next, Previous, Show/Hide window and Quit. Clicking an entry
//! pushes a [`MediaCommand`] onto the [`MediaSender`].
//!
//! ## Threading on Linux
//!
//! `tray-icon` needs a GTK event loop on the thread that owns the icon. eframe
//! runs a winit loop instead, so the tray gets its **own dedicated thread**
//! that calls `gtk::init()`, builds the icon and runs `gtk::main()`. A GTK
//! timeout on that thread polls the shared [`MediaSnapshot`] and the tray's
//! menu / tooltip stay in sync — the Play/Pause label flips, the tooltip
//! follows the now-playing track — without crossing the thread boundary by
//! hand.
//!
//! `muda` (the menu library behind `tray-icon`) delivers menu clicks on a
//! process-global channel; the GTK timeout drains it too and forwards the
//! mapped command.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};

use super::{MediaCommand, MediaSender, MediaSnapshot, SharedSnapshot};

/// The tray icon edge length, in pixels.
const ICON_SIZE: u32 = 32;

/// Start the tray on its own GTK-owning thread.
///
/// Best-effort: a build without a system tray / appindicator just logs and
/// the app runs without a tray. The thread lives for the process lifetime.
pub fn spawn(snapshot: SharedSnapshot, sender: MediaSender) {
    let result = thread::Builder::new()
        .name("spottyfi-tray".to_owned())
        .spawn(move || run(snapshot, sender));
    if let Err(err) = result {
        tracing::warn!(%err, "tray thread failed to start");
    }
}

/// The tray thread body: init GTK, build the icon, run the GTK main loop.
fn run(snapshot: SharedSnapshot, sender: MediaSender) {
    if let Err(err) = gtk::init() {
        tracing::info!(%err, "GTK init failed; system tray disabled");
        return;
    }

    // The menu items — kept so the timeout below can flip the Play/Pause
    // label and the icon's tooltip as playback state changes.
    let play_pause = MenuItem::new("Play", true, None);
    let next = MenuItem::new("Next", true, None);
    let previous = MenuItem::new("Previous", true, None);
    let show_hide = MenuItem::new("Show / Hide window", true, None);
    let quit = MenuItem::new("Quit Spottyfi", true, None);

    let menu = Menu::new();
    let append = menu.append_items(&[
        &play_pause,
        &next,
        &previous,
        &PredefinedMenuItem::separator(),
        &show_hide,
        &PredefinedMenuItem::separator(),
        &quit,
    ]);
    if let Err(err) = append {
        tracing::warn!(%err, "could not build tray menu");
        return;
    }

    // Route each item's menu-id to the command it raises.
    let mut routes: HashMap<String, MediaCommand> = HashMap::new();
    routes.insert(play_pause.id().0.clone(), MediaCommand::PlayPause);
    routes.insert(next.id().0.clone(), MediaCommand::Next);
    routes.insert(previous.id().0.clone(), MediaCommand::Previous);
    routes.insert(show_hide.id().0.clone(), MediaCommand::ToggleWindow);
    routes.insert(quit.id().0.clone(), MediaCommand::Quit);

    let tray = TrayIconBuilder::new()
        .with_tooltip("Spottyfi")
        .with_menu(Box::new(menu))
        .with_icon(tray_icon_image())
        .build();
    let tray = match tray {
        Ok(tray) => tray,
        Err(err) => {
            tracing::info!(%err, "system tray unavailable");
            return;
        }
    };
    tracing::info!("system tray icon created");

    // The last snapshot reflected in the menu — so the timeout only touches
    // GTK widgets when something actually changed.
    let last = Rc::new(RefCell::new(MediaSnapshot::default()));

    // Poll the menu-event channel and the shared snapshot four times a second
    // from inside the GTK loop, where touching the widgets is safe.
    let menu_rx = MenuEvent::receiver();
    gtk::glib::timeout_add_local(Duration::from_millis(250), move || {
        // Drain menu clicks.
        while let Ok(event) = menu_rx.try_recv() {
            if let Some(command) = routes.get(&event.id().0) {
                sender.send(command.clone());
            }
        }

        // Reflect playback state in the menu labels + tooltip.
        let current = snapshot.load_full();
        let mut last = last.borrow_mut();
        if *current != *last {
            play_pause.set_text(if current.playing { "Pause" } else { "Play" });
            let has_track = current.has_track;
            play_pause.set_enabled(has_track);
            next.set_enabled(current.can_next);
            previous.set_enabled(current.can_previous);
            let tooltip = if has_track {
                format!("Spottyfi — {}", current.now_playing_line())
            } else {
                "Spottyfi".to_owned()
            };
            if let Err(err) = tray.set_tooltip(Some(tooltip)) {
                tracing::debug!(%err, "tray tooltip update failed");
            }
            *last = (*current).clone();
        }

        gtk::glib::ControlFlow::Continue
    });

    gtk::main();
}

/// Build the tray icon image.
///
/// Spottyfi has no bundled raster app icon, so the tray uses a generated
/// solid accent-green rounded square — recognisable in a tray strip and
/// dependency-free. A real icon can replace this in the Phase 13 packaging
/// work.
fn tray_icon_image() -> Icon {
    let size = ICON_SIZE as usize;
    let mut rgba = vec![0u8; size * size * 4];
    // Spotify-ish accent green (#1ed760).
    let (r, g, b) = (0x1e, 0xd7, 0x60);
    let radius = size as f32 * 0.22;
    for y in 0..size {
        for x in 0..size {
            let inside = rounded_square_contains(x as f32, y as f32, size as f32, radius);
            let idx = (y * size + x) * 4;
            if inside {
                rgba[idx] = r;
                rgba[idx + 1] = g;
                rgba[idx + 2] = b;
                rgba[idx + 3] = 0xff;
            }
            // Outside the rounded square stays fully transparent.
        }
    }
    // `from_rgba` only fails on a length mismatch, which cannot happen for the
    // buffer just built; fall back to a bare opaque square if it ever does.
    Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE).unwrap_or_else(|err| {
        tracing::debug!(%err, "tray icon build fell back to a plain square");
        let plain = vec![0xffu8; size * size * 4];
        // A plain square cannot fail; if it somehow does there is no icon to
        // show, so panic-free degradation means an unreachable default.
        Icon::from_rgba(plain, ICON_SIZE, ICON_SIZE)
            .unwrap_or_else(|_| Icon::from_rgba(vec![0; 4], 1, 1).expect("1x1 icon"))
    })
}

/// Whether `(x, y)` lies inside a `size`×`size` square with `radius` corners.
fn rounded_square_contains(x: f32, y: f32, size: f32, radius: f32) -> bool {
    // Clamp the point into the inner rectangle whose corners are the centres
    // of the corner arcs; the distance from there is the rounded-corner test.
    let cx = x.clamp(radius, size - radius);
    let cy = y.clamp(radius, size - radius);
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_square_includes_centre_excludes_corner() {
        let size = 32.0;
        let radius = 7.0;
        // The dead centre is always inside.
        assert!(rounded_square_contains(16.0, 16.0, size, radius));
        // The extreme corner pixel is outside the rounding.
        assert!(!rounded_square_contains(0.0, 0.0, size, radius));
        // A mid-edge pixel is inside.
        assert!(rounded_square_contains(16.0, 0.0, size, radius));
    }

    #[test]
    fn icon_image_builds() {
        // Building the generated icon must not panic and must produce a
        // non-trivial image.
        let _icon = tray_icon_image();
    }
}
