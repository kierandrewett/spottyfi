//! Rebindable keyboard shortcuts.
//!
//! Phase 12 turns the previously hard-coded shortcuts into a persisted,
//! user-editable [`HotkeyMap`]. Each [`HotkeyAction`] (close tab, new tab,
//! reopen, search, play/pause, next, previous) maps to a [`Hotkey`] — a key
//! plus a set of modifiers. The map serialises to RON inside
//! [`PersistedShell`](crate::shell::PersistedShell) and is surfaced, with a
//! capture-the-next-keypress editor, in the Settings page's Hotkeys section.
//!
//! The map is also the single source of truth for the **media-key fallback**:
//! [`global-hotkey`](crate::media_keys) registers the play/pause, next and
//! previous bindings system-wide, so the same configuration drives both the
//! in-window shortcuts and the desktop media keys.

use std::fmt;

use serde::{Deserialize, Serialize};

/// One user-rebindable action.
///
/// The order of [`HotkeyAction::all`] is the order the Settings page lists
/// them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HotkeyAction {
    /// Close the focused dock tab.
    CloseTab,
    /// Open a new Home tab in the centre group.
    NewTab,
    /// Reopen the most recently closed tab.
    ReopenTab,
    /// Open (and focus) the Search page.
    OpenSearch,
    /// Toggle play / pause.
    PlayPause,
    /// Skip to the next track.
    NextTrack,
    /// Skip to the previous track.
    PreviousTrack,
}

impl HotkeyAction {
    /// Every action, in Settings-page display order.
    #[must_use]
    pub fn all() -> [HotkeyAction; 7] {
        [
            HotkeyAction::CloseTab,
            HotkeyAction::NewTab,
            HotkeyAction::ReopenTab,
            HotkeyAction::OpenSearch,
            HotkeyAction::PlayPause,
            HotkeyAction::NextTrack,
            HotkeyAction::PreviousTrack,
        ]
    }

    /// A human-readable label for the action.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            HotkeyAction::CloseTab => "Close tab",
            HotkeyAction::NewTab => "New Home tab",
            HotkeyAction::ReopenTab => "Reopen closed tab",
            HotkeyAction::OpenSearch => "Open Search",
            HotkeyAction::PlayPause => "Play / pause",
            HotkeyAction::NextTrack => "Next track",
            HotkeyAction::PreviousTrack => "Previous track",
        }
    }

    /// Whether this action is a media-transport action.
    ///
    /// Transport actions ([`HotkeyAction::PlayPause`], [`HotkeyAction::
    /// NextTrack`], [`HotkeyAction::PreviousTrack`]) are also registered as
    /// system-wide media keys via [`crate::media_keys`].
    #[must_use]
    pub fn is_transport(self) -> bool {
        matches!(
            self,
            HotkeyAction::PlayPause | HotkeyAction::NextTrack | HotkeyAction::PreviousTrack
        )
    }

    /// This action's first-launch default binding.
    #[must_use]
    fn default_hotkey(self) -> Hotkey {
        let cmd = HotkeyModifiers {
            command: true,
            shift: false,
            alt: false,
        };
        match self {
            HotkeyAction::CloseTab => Hotkey::new(HotkeyKey::W, cmd),
            HotkeyAction::NewTab => Hotkey::new(HotkeyKey::T, cmd),
            HotkeyAction::ReopenTab => Hotkey::new(
                HotkeyKey::T,
                HotkeyModifiers {
                    shift: true,
                    ..cmd
                },
            ),
            HotkeyAction::OpenSearch => Hotkey::new(HotkeyKey::K, cmd),
            // Transport actions default to plain function-style keys so they
            // do not clash with the editing shortcuts; the media keys proper
            // are handled by MPRIS / `global-hotkey` regardless.
            HotkeyAction::PlayPause => Hotkey::new(HotkeyKey::Space, HotkeyModifiers::CTRL_ALT),
            HotkeyAction::NextTrack => Hotkey::new(HotkeyKey::Period, HotkeyModifiers::CTRL_ALT),
            HotkeyAction::PreviousTrack => {
                Hotkey::new(HotkeyKey::Comma, HotkeyModifiers::CTRL_ALT)
            }
        }
    }
}

/// The modifier-key combination of a [`Hotkey`].
///
/// `command` is the platform "primary" modifier — Ctrl on Linux/Windows, Cmd
/// on macOS — matching egui's `Modifiers::command`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct HotkeyModifiers {
    /// The primary modifier (Ctrl on Linux/Windows, Cmd on macOS).
    #[serde(default)]
    pub command: bool,
    /// The Shift modifier.
    #[serde(default)]
    pub shift: bool,
    /// The Alt / Option modifier.
    #[serde(default)]
    pub alt: bool,
}

impl HotkeyModifiers {
    /// Ctrl + Alt — the default modifier set for the transport shortcuts.
    pub const CTRL_ALT: HotkeyModifiers = HotkeyModifiers {
        command: true,
        shift: false,
        alt: true,
    };

    /// Whether no modifier at all is held.
    #[must_use]
    pub fn is_empty(self) -> bool {
        !self.command && !self.shift && !self.alt
    }
}

/// The (small, serialisable) set of keys a shortcut can be bound to.
///
/// A deliberately restricted subset — letters, digits, the function keys and a
/// handful of punctuation — so the persisted form is stable and the capture
/// editor cannot record something un-typeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(missing_docs)] // Each variant is a single, self-evident key.
pub enum HotkeyKey {
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Space, Comma, Period, Slash, Semicolon, Minus, Equals,
}

impl HotkeyKey {
    /// Map from an [`egui::Key`], for the capture editor.
    #[must_use]
    #[rustfmt::skip]
    pub fn from_egui(key: egui::Key) -> Option<HotkeyKey> {
        use egui::Key as K;
        Some(match key {
            K::A => HotkeyKey::A, K::B => HotkeyKey::B, K::C => HotkeyKey::C,
            K::D => HotkeyKey::D, K::E => HotkeyKey::E, K::F => HotkeyKey::F,
            K::G => HotkeyKey::G, K::H => HotkeyKey::H, K::I => HotkeyKey::I,
            K::J => HotkeyKey::J, K::K => HotkeyKey::K, K::L => HotkeyKey::L,
            K::M => HotkeyKey::M, K::N => HotkeyKey::N, K::O => HotkeyKey::O,
            K::P => HotkeyKey::P, K::Q => HotkeyKey::Q, K::R => HotkeyKey::R,
            K::S => HotkeyKey::S, K::T => HotkeyKey::T, K::U => HotkeyKey::U,
            K::V => HotkeyKey::V, K::W => HotkeyKey::W, K::X => HotkeyKey::X,
            K::Y => HotkeyKey::Y, K::Z => HotkeyKey::Z,
            K::Num0 => HotkeyKey::Num0, K::Num1 => HotkeyKey::Num1,
            K::Num2 => HotkeyKey::Num2, K::Num3 => HotkeyKey::Num3,
            K::Num4 => HotkeyKey::Num4, K::Num5 => HotkeyKey::Num5,
            K::Num6 => HotkeyKey::Num6, K::Num7 => HotkeyKey::Num7,
            K::Num8 => HotkeyKey::Num8, K::Num9 => HotkeyKey::Num9,
            K::F1 => HotkeyKey::F1, K::F2 => HotkeyKey::F2, K::F3 => HotkeyKey::F3,
            K::F4 => HotkeyKey::F4, K::F5 => HotkeyKey::F5, K::F6 => HotkeyKey::F6,
            K::F7 => HotkeyKey::F7, K::F8 => HotkeyKey::F8, K::F9 => HotkeyKey::F9,
            K::F10 => HotkeyKey::F10, K::F11 => HotkeyKey::F11, K::F12 => HotkeyKey::F12,
            K::Space => HotkeyKey::Space, K::Comma => HotkeyKey::Comma,
            K::Period => HotkeyKey::Period, K::Slash => HotkeyKey::Slash,
            K::Semicolon => HotkeyKey::Semicolon, K::Minus => HotkeyKey::Minus,
            K::Equals => HotkeyKey::Equals,
            _ => return None,
        })
    }

    /// Map to an [`egui::Key`] for in-window shortcut matching.
    #[must_use]
    #[rustfmt::skip]
    pub fn to_egui(self) -> egui::Key {
        use egui::Key as K;
        match self {
            HotkeyKey::A => K::A, HotkeyKey::B => K::B, HotkeyKey::C => K::C,
            HotkeyKey::D => K::D, HotkeyKey::E => K::E, HotkeyKey::F => K::F,
            HotkeyKey::G => K::G, HotkeyKey::H => K::H, HotkeyKey::I => K::I,
            HotkeyKey::J => K::J, HotkeyKey::K => K::K, HotkeyKey::L => K::L,
            HotkeyKey::M => K::M, HotkeyKey::N => K::N, HotkeyKey::O => K::O,
            HotkeyKey::P => K::P, HotkeyKey::Q => K::Q, HotkeyKey::R => K::R,
            HotkeyKey::S => K::S, HotkeyKey::T => K::T, HotkeyKey::U => K::U,
            HotkeyKey::V => K::V, HotkeyKey::W => K::W, HotkeyKey::X => K::X,
            HotkeyKey::Y => K::Y, HotkeyKey::Z => K::Z,
            HotkeyKey::Num0 => K::Num0, HotkeyKey::Num1 => K::Num1,
            HotkeyKey::Num2 => K::Num2, HotkeyKey::Num3 => K::Num3,
            HotkeyKey::Num4 => K::Num4, HotkeyKey::Num5 => K::Num5,
            HotkeyKey::Num6 => K::Num6, HotkeyKey::Num7 => K::Num7,
            HotkeyKey::Num8 => K::Num8, HotkeyKey::Num9 => K::Num9,
            HotkeyKey::F1 => K::F1, HotkeyKey::F2 => K::F2, HotkeyKey::F3 => K::F3,
            HotkeyKey::F4 => K::F4, HotkeyKey::F5 => K::F5, HotkeyKey::F6 => K::F6,
            HotkeyKey::F7 => K::F7, HotkeyKey::F8 => K::F8, HotkeyKey::F9 => K::F9,
            HotkeyKey::F10 => K::F10, HotkeyKey::F11 => K::F11, HotkeyKey::F12 => K::F12,
            HotkeyKey::Space => K::Space, HotkeyKey::Comma => K::Comma,
            HotkeyKey::Period => K::Period, HotkeyKey::Slash => K::Slash,
            HotkeyKey::Semicolon => K::Semicolon, HotkeyKey::Minus => K::Minus,
            HotkeyKey::Equals => K::Equals,
        }
    }

    /// A short display label for the key (`"W"`, `"Space"`, `"F5"`).
    #[must_use]
    fn label(self) -> &'static str {
        match self {
            HotkeyKey::Space => "Space",
            HotkeyKey::Comma => ",",
            HotkeyKey::Period => ".",
            HotkeyKey::Slash => "/",
            HotkeyKey::Semicolon => ";",
            HotkeyKey::Minus => "-",
            HotkeyKey::Equals => "=",
            HotkeyKey::Num0 => "0",
            HotkeyKey::Num1 => "1",
            HotkeyKey::Num2 => "2",
            HotkeyKey::Num3 => "3",
            HotkeyKey::Num4 => "4",
            HotkeyKey::Num5 => "5",
            HotkeyKey::Num6 => "6",
            HotkeyKey::Num7 => "7",
            HotkeyKey::Num8 => "8",
            HotkeyKey::Num9 => "9",
            HotkeyKey::F1 => "F1",
            HotkeyKey::F2 => "F2",
            HotkeyKey::F3 => "F3",
            HotkeyKey::F4 => "F4",
            HotkeyKey::F5 => "F5",
            HotkeyKey::F6 => "F6",
            HotkeyKey::F7 => "F7",
            HotkeyKey::F8 => "F8",
            HotkeyKey::F9 => "F9",
            HotkeyKey::F10 => "F10",
            HotkeyKey::F11 => "F11",
            HotkeyKey::F12 => "F12",
            HotkeyKey::A => "A",
            HotkeyKey::B => "B",
            HotkeyKey::C => "C",
            HotkeyKey::D => "D",
            HotkeyKey::E => "E",
            HotkeyKey::F => "F",
            HotkeyKey::G => "G",
            HotkeyKey::H => "H",
            HotkeyKey::I => "I",
            HotkeyKey::J => "J",
            HotkeyKey::K => "K",
            HotkeyKey::L => "L",
            HotkeyKey::M => "M",
            HotkeyKey::N => "N",
            HotkeyKey::O => "O",
            HotkeyKey::P => "P",
            HotkeyKey::Q => "Q",
            HotkeyKey::R => "R",
            HotkeyKey::S => "S",
            HotkeyKey::T => "T",
            HotkeyKey::U => "U",
            HotkeyKey::V => "V",
            HotkeyKey::W => "W",
            HotkeyKey::X => "X",
            HotkeyKey::Y => "Y",
            HotkeyKey::Z => "Z",
        }
    }
}

/// A single keyboard shortcut: a [`HotkeyKey`] plus its modifier set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hotkey {
    /// The bound key.
    pub key: HotkeyKey,
    /// The modifier combination held alongside it.
    #[serde(default)]
    pub modifiers: HotkeyModifiers,
}

impl Hotkey {
    /// Build a hotkey from a key and a modifier set.
    #[must_use]
    pub fn new(key: HotkeyKey, modifiers: HotkeyModifiers) -> Self {
        Self { key, modifiers }
    }

    /// Whether this binding matches the keyboard state in `input`.
    ///
    /// A match requires the key to have been *pressed* this frame and the
    /// modifier set to line up exactly — an extra held modifier is a miss, so
    /// `Ctrl+T` does not also fire on `Ctrl+Shift+T`.
    #[must_use]
    pub fn matches(self, input: &egui::InputState) -> bool {
        if !input.key_pressed(self.key.to_egui()) {
            return false;
        }
        let mods = &input.modifiers;
        let command = mods.command || mods.ctrl;
        command == self.modifiers.command
            && mods.shift == self.modifiers.shift
            && mods.alt == self.modifiers.alt
    }
}

impl fmt::Display for Hotkey {
    /// A `Ctrl + Shift + T`-style human-readable rendering.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<&str> = Vec::new();
        if self.modifiers.command {
            // "Cmd/Ctrl" reads correctly on every platform without a cfg.
            parts.push("Cmd/Ctrl");
        }
        if self.modifiers.shift {
            parts.push("Shift");
        }
        if self.modifiers.alt {
            parts.push("Alt");
        }
        let key = self.key.label();
        if parts.is_empty() {
            write!(f, "{key}")
        } else {
            write!(f, "{} + {key}", parts.join(" + "))
        }
    }
}

/// The full set of action → [`Hotkey`] bindings.
///
/// Stored as a flat list of pairs so it serialises cleanly to RON and a
/// future-added action simply falls back to its default when absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyMap {
    /// The bindings, one per action.
    bindings: Vec<(HotkeyAction, Hotkey)>,
}

impl Default for HotkeyMap {
    /// The first-launch map: every action at its [`HotkeyAction::default_hotkey`].
    fn default() -> Self {
        Self {
            bindings: HotkeyAction::all()
                .into_iter()
                .map(|action| (action, action.default_hotkey()))
                .collect(),
        }
    }
}

impl HotkeyMap {
    /// The binding for `action`, falling back to its default if (e.g. after a
    /// version bump) the persisted map has no entry for it.
    #[must_use]
    pub fn get(&self, action: HotkeyAction) -> Hotkey {
        self.bindings
            .iter()
            .find(|(a, _)| *a == action)
            .map(|(_, h)| *h)
            .unwrap_or_else(|| action.default_hotkey())
    }

    /// Rebind `action` to `hotkey`, replacing any existing entry.
    pub fn set(&mut self, action: HotkeyAction, hotkey: Hotkey) {
        if let Some(slot) = self.bindings.iter_mut().find(|(a, _)| *a == action) {
            slot.1 = hotkey;
        } else {
            self.bindings.push((action, hotkey));
        }
    }

    /// Reset every binding to its first-launch default.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The first action whose binding `hotkey` collides with, ignoring
    /// `except` (the action currently being rebound).
    ///
    /// Used by the capture editor to warn before committing a duplicate.
    #[must_use]
    pub fn conflict(&self, hotkey: Hotkey, except: HotkeyAction) -> Option<HotkeyAction> {
        self.bindings
            .iter()
            .find(|(a, h)| *a != except && *h == hotkey)
            .map(|(a, _)| *a)
    }

    /// Which action, if any, the keyboard state in `input` triggers this frame.
    ///
    /// Returns at most one action — the first match in [`HotkeyAction::all`]
    /// order, so a frame's keypress never fans out to two actions.
    #[must_use]
    pub fn triggered(&self, input: &egui::InputState) -> Option<HotkeyAction> {
        HotkeyAction::all()
            .into_iter()
            .find(|action| self.get(*action).matches(input))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_map_covers_every_action() {
        let map = HotkeyMap::default();
        for action in HotkeyAction::all() {
            // `get` must return the real stored binding, not the fallback.
            assert_eq!(map.get(action), action.default_hotkey());
        }
    }

    #[test]
    fn round_trips_through_ron() {
        let mut map = HotkeyMap::default();
        map.set(
            HotkeyAction::PlayPause,
            Hotkey::new(HotkeyKey::P, HotkeyModifiers::CTRL_ALT),
        );
        let text = ron::ser::to_string(&map).expect("serialise");
        let restored: HotkeyMap = ron::from_str(&text).expect("deserialise");
        assert_eq!(map, restored);
    }

    #[test]
    fn missing_action_falls_back_to_default() {
        // A map persisted before an action existed simply omits it.
        let map = HotkeyMap { bindings: vec![] };
        assert_eq!(
            map.get(HotkeyAction::NewTab),
            HotkeyAction::NewTab.default_hotkey()
        );
    }

    #[test]
    fn set_replaces_an_existing_binding() {
        let mut map = HotkeyMap::default();
        let new = Hotkey::new(HotkeyKey::Num1, HotkeyModifiers::default());
        map.set(HotkeyAction::OpenSearch, new);
        assert_eq!(map.get(HotkeyAction::OpenSearch), new);
        // Exactly one entry per action — no duplicate appended.
        assert_eq!(map.bindings.len(), HotkeyAction::all().len());
    }

    #[test]
    fn conflict_detects_a_duplicate_binding() {
        let map = HotkeyMap::default();
        // `Cmd/Ctrl+W` is CloseTab's default — binding NewTab to it collides.
        let close = map.get(HotkeyAction::CloseTab);
        assert_eq!(
            map.conflict(close, HotkeyAction::NewTab),
            Some(HotkeyAction::CloseTab)
        );
        // The same key against its own action is not a conflict.
        assert_eq!(map.conflict(close, HotkeyAction::CloseTab), None);
    }

    #[test]
    fn display_renders_modifiers_and_key() {
        let hk = Hotkey::new(
            HotkeyKey::T,
            HotkeyModifiers {
                command: true,
                shift: true,
                alt: false,
            },
        );
        assert_eq!(hk.to_string(), "Cmd/Ctrl + Shift + T");
        let plain = Hotkey::new(HotkeyKey::F5, HotkeyModifiers::default());
        assert_eq!(plain.to_string(), "F5");
    }

    #[test]
    fn egui_key_round_trips() {
        for action in HotkeyAction::all() {
            let key = action.default_hotkey().key;
            assert_eq!(HotkeyKey::from_egui(key.to_egui()), Some(key));
        }
    }
}
