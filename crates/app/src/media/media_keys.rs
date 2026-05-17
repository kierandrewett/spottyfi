//! System-wide media keys via [`global-hotkey`](global_hotkey).
//!
//! On most Linux desktops the dedicated XF86Audio* keys are routed to the
//! active MPRIS player by the desktop's own media-key handler, so [`mpris`]
//! already covers them. This module is the **fallback** for window managers
//! that do not do that (minimal/tiling WMs): it registers the XF86Audio*
//! codes — and the user's configurable transport hotkeys — globally and turns
//! each press into a [`MediaCommand`].
//!
//! [`global-hotkey`] delivers events on a process-global channel; [`spawn`]
//! drains it on a dedicated background thread and forwards commands onto the
//! [`MediaSender`], so the egui/winit loop never has to poll it.
//!
//! [`mpris`]: super::mpris

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use super::{MediaCommand, MediaSender};
use crate::hotkeys::{Hotkey, HotkeyKey, HotkeyMap, HotkeyModifiers};

/// Map one of our [`HotkeyKey`]s to a `global-hotkey` [`Code`].
fn key_code(key: HotkeyKey) -> Code {
    match key {
        HotkeyKey::A => Code::KeyA,
        HotkeyKey::B => Code::KeyB,
        HotkeyKey::C => Code::KeyC,
        HotkeyKey::D => Code::KeyD,
        HotkeyKey::E => Code::KeyE,
        HotkeyKey::F => Code::KeyF,
        HotkeyKey::G => Code::KeyG,
        HotkeyKey::H => Code::KeyH,
        HotkeyKey::I => Code::KeyI,
        HotkeyKey::J => Code::KeyJ,
        HotkeyKey::K => Code::KeyK,
        HotkeyKey::L => Code::KeyL,
        HotkeyKey::M => Code::KeyM,
        HotkeyKey::N => Code::KeyN,
        HotkeyKey::O => Code::KeyO,
        HotkeyKey::P => Code::KeyP,
        HotkeyKey::Q => Code::KeyQ,
        HotkeyKey::R => Code::KeyR,
        HotkeyKey::S => Code::KeyS,
        HotkeyKey::T => Code::KeyT,
        HotkeyKey::U => Code::KeyU,
        HotkeyKey::V => Code::KeyV,
        HotkeyKey::W => Code::KeyW,
        HotkeyKey::X => Code::KeyX,
        HotkeyKey::Y => Code::KeyY,
        HotkeyKey::Z => Code::KeyZ,
        HotkeyKey::Num0 => Code::Digit0,
        HotkeyKey::Num1 => Code::Digit1,
        HotkeyKey::Num2 => Code::Digit2,
        HotkeyKey::Num3 => Code::Digit3,
        HotkeyKey::Num4 => Code::Digit4,
        HotkeyKey::Num5 => Code::Digit5,
        HotkeyKey::Num6 => Code::Digit6,
        HotkeyKey::Num7 => Code::Digit7,
        HotkeyKey::Num8 => Code::Digit8,
        HotkeyKey::Num9 => Code::Digit9,
        HotkeyKey::F1 => Code::F1,
        HotkeyKey::F2 => Code::F2,
        HotkeyKey::F3 => Code::F3,
        HotkeyKey::F4 => Code::F4,
        HotkeyKey::F5 => Code::F5,
        HotkeyKey::F6 => Code::F6,
        HotkeyKey::F7 => Code::F7,
        HotkeyKey::F8 => Code::F8,
        HotkeyKey::F9 => Code::F9,
        HotkeyKey::F10 => Code::F10,
        HotkeyKey::F11 => Code::F11,
        HotkeyKey::F12 => Code::F12,
        HotkeyKey::Space => Code::Space,
        HotkeyKey::Comma => Code::Comma,
        HotkeyKey::Period => Code::Period,
        HotkeyKey::Slash => Code::Slash,
        HotkeyKey::Semicolon => Code::Semicolon,
        HotkeyKey::Minus => Code::Minus,
        HotkeyKey::Equals => Code::Equal,
    }
}

/// Map our [`HotkeyModifiers`] to a `global-hotkey` [`Modifiers`] bitset.
fn key_modifiers(mods: HotkeyModifiers) -> Modifiers {
    let mut out = Modifiers::empty();
    if mods.command {
        out |= Modifiers::CONTROL;
    }
    if mods.shift {
        out |= Modifiers::SHIFT;
    }
    if mods.alt {
        out |= Modifiers::ALT;
    }
    out
}

/// Build a `global-hotkey` [`HotKey`] from one of our [`Hotkey`]s.
fn global_hotkey(hotkey: Hotkey) -> HotKey {
    HotKey::new(Some(key_modifiers(hotkey.modifiers)), key_code(hotkey.key))
}

/// Register the media keys and start the event-pump thread.
///
/// Best-effort: a platform without a global-hotkey backend (e.g. a headless
/// session) logs and returns — the in-window shortcuts still work.
///
/// Registers two sets of bindings:
///
/// 1. The dedicated **XF86Audio* media keys** — `MediaPlayPause`,
///    `MediaTrackNext`, `MediaTrackPrevious`, `MediaStop` — with no modifier.
/// 2. The **user's configured transport hotkeys** from [`HotkeyMap`], so the
///    play/pause/next/previous shortcuts work even when the window is not
///    focused.
///
/// The [`GlobalHotKeyManager`] is moved onto the pump thread and kept alive
/// for the process lifetime — dropping it unregisters every key.
pub fn spawn(sender: MediaSender, hotkeys: &HotkeyMap) {
    let manager = match GlobalHotKeyManager::new() {
        Ok(manager) => manager,
        Err(err) => {
            tracing::info!(%err, "global media-key fallback unavailable");
            return;
        }
    };

    // id → command, so an incoming event can be routed without re-deriving.
    let mut routes: HashMap<u32, MediaCommand> = HashMap::new();

    // The dedicated media keys — these are what a desktop's own handler would
    // normally consume; registering them is the fallback for WMs that do not.
    let media: [(Code, MediaCommand); 4] = [
        (Code::MediaPlayPause, MediaCommand::PlayPause),
        (Code::MediaTrackNext, MediaCommand::Next),
        (Code::MediaTrackPrevious, MediaCommand::Previous),
        (Code::MediaStop, MediaCommand::Stop),
    ];
    for (code, command) in media {
        let hotkey = HotKey::new(None, code);
        match manager.register(hotkey) {
            Ok(()) => {
                routes.insert(hotkey.id(), command);
            }
            Err(err) => {
                // A key already grabbed by the desktop is expected — that is
                // exactly the case where MPRIS already handles it.
                tracing::debug!(%err, ?code, "media key not registered (already grabbed?)");
            }
        }
    }

    // The user's configured transport hotkeys.
    for action in crate::hotkeys::HotkeyAction::all() {
        if !action.is_transport() {
            continue;
        }
        let command = match action {
            crate::hotkeys::HotkeyAction::PlayPause => MediaCommand::PlayPause,
            crate::hotkeys::HotkeyAction::NextTrack => MediaCommand::Next,
            crate::hotkeys::HotkeyAction::PreviousTrack => MediaCommand::Previous,
            _ => continue,
        };
        let hotkey = global_hotkey(hotkeys.get(action));
        match manager.register(hotkey) {
            Ok(()) => {
                routes.insert(hotkey.id(), command);
            }
            Err(err) => {
                tracing::debug!(%err, ?action, "transport hotkey not registered");
            }
        }
    }

    if routes.is_empty() {
        tracing::info!("no global media keys registered");
        return;
    }
    tracing::info!(count = routes.len(), "global media keys registered");

    thread::Builder::new()
        .name("spottyfi-media-keys".to_owned())
        .spawn(move || pump(manager, routes, sender))
        .map(|_| ())
        .unwrap_or_else(|err| tracing::warn!(%err, "media-key pump thread failed to start"));
}

/// The event-pump loop: drain `global-hotkey`'s channel and forward commands.
///
/// `manager` is held only to keep the registrations alive for the loop's
/// lifetime; it is otherwise unused.
fn pump(manager: GlobalHotKeyManager, routes: HashMap<u32, MediaCommand>, sender: MediaSender) {
    let receiver = GlobalHotKeyEvent::receiver();
    loop {
        // A short timed recv keeps the thread responsive without a busy spin.
        // `global-hotkey` delivers events over a `crossbeam-channel`, whose
        // `recv_timeout` yields `Ok` on an event and `Err` on timeout or a
        // closed channel; the only fatal case is a disconnect.
        match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(event) => {
                // Only act on key-press, not release, so a single tap fires
                // the command exactly once.
                if event.state() == HotKeyState::Pressed {
                    if let Some(command) = routes.get(&event.id()) {
                        sender.send(command.clone());
                    }
                }
            }
            Err(err) if err.is_disconnected() => {
                tracing::debug!("media-key channel disconnected; pump exiting");
                break;
            }
            Err(_) => {
                // A plain timeout — loop and poll again.
            }
        }
    }
    // Keep `manager` alive until the loop ends, then unregister everything.
    drop(manager);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_map_to_bitset() {
        assert!(key_modifiers(HotkeyModifiers::default()).is_empty());
        let all = HotkeyModifiers {
            command: true,
            shift: true,
            alt: true,
        };
        let bits = key_modifiers(all);
        assert!(bits.contains(Modifiers::CONTROL));
        assert!(bits.contains(Modifiers::SHIFT));
        assert!(bits.contains(Modifiers::ALT));
    }

    #[test]
    fn every_hotkey_key_maps_to_a_code() {
        // A round-trip sanity check — `key_code` must be total over the
        // default map's keys (it is `match`-exhaustive, this guards intent).
        for action in crate::hotkeys::HotkeyAction::all() {
            let _ = global_hotkey(HotkeyMap::default().get(action));
        }
    }
}
