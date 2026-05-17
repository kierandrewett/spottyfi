//! App-layer dock state: tab pinning, per-tab history and the closed-tab stack.
//!
//! `egui_dock` 0.19 owns the tab *tree* (splits, sizes, the active tab per
//! leaf) and supports a per-tab right-click [`context_menu`] and a
//! [`is_closeable`] hook. It does **not** model browser-style per-tab history,
//! tab pinning or a reopen-last-closed stack — those are Spottyfi power
//! features and live here, in the app's own tab layer, keyed by [`Tab`].
//!
//! [`context_menu`]: egui_dock::TabViewer::context_menu
//! [`is_closeable`]: egui_dock::TabViewer::is_closeable
//!
//! The [`DockState`](egui_dock::DockState) and these maps are persisted side by
//! side in [`PersistedShell`](super::persist::PersistedShell); the new fields
//! all carry `#[serde(default)]` so a pre-Phase-10 `layout.ron` still loads.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::tabs::Tab;

/// One tab's browser-style navigation history.
///
/// A tab whose content is *replaced* (a plain click on a sidebar entry or an
/// in-page link, see `docs/docking.md`) keeps the surface it left behind on a
/// back stack; Back/Forward walk between them. Navigating fresh (not via
/// Back/Forward) clears the forward stack, exactly like a web browser.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabHistory {
    /// Surfaces visited before the current one, oldest last-pushed at the end.
    back: Vec<Tab>,
    /// Surfaces the user navigated *back* away from, newest at the end.
    forward: Vec<Tab>,
}

impl TabHistory {
    /// Record a navigation from `current` to a new surface.
    ///
    /// `current` is pushed onto the back stack and the forward stack is
    /// cleared — a fresh navigation invalidates any "redo" history.
    pub fn navigate(&mut self, current: Tab) {
        self.back.push(current);
        self.forward.clear();
    }

    /// Whether [`back`](Self::back) would return a surface.
    #[must_use]
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// Whether [`forward`](Self::forward) would return a surface.
    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Step back one surface.
    ///
    /// `current` (the surface being left) is pushed onto the forward stack and
    /// the previous surface is returned. Returns `None` at the start of history.
    #[must_use]
    pub fn back(&mut self, current: Tab) -> Option<Tab> {
        let previous = self.back.pop()?;
        self.forward.push(current);
        Some(previous)
    }

    /// Step forward one surface, the inverse of [`back`](Self::back).
    #[must_use]
    pub fn forward(&mut self, current: Tab) -> Option<Tab> {
        let next = self.forward.pop()?;
        self.back.push(current);
        Some(next)
    }
}

/// A tab the user closed, kept so `Cmd/Ctrl+Shift+T` can reopen it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedTab {
    /// The closed tab's key — what gets re-opened.
    pub tab: Tab,
    /// The history that tab carried, restored alongside it.
    #[serde(default)]
    pub history: TabHistory,
}

/// The bounded stack of recently-closed tabs (most-recent last).
///
/// Capped so a long session does not grow it without bound; `Cmd/Ctrl+Shift+T`
/// pops the most recent entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClosedTabStack {
    /// Closed tabs, oldest first, newest last.
    entries: Vec<ClosedTab>,
}

/// The most recently-closed tabs the stack retains.
const CLOSED_STACK_CAP: usize = 16;

impl ClosedTabStack {
    /// Record a closed tab. Panels (Queue, Debug, …) are not navigable history
    /// surfaces but are still reopenable, so every closed tab is recorded.
    pub fn push(&mut self, tab: Tab, history: TabHistory) {
        self.entries.push(ClosedTab { tab, history });
        if self.entries.len() > CLOSED_STACK_CAP {
            let overflow = self.entries.len() - CLOSED_STACK_CAP;
            self.entries.drain(0..overflow);
        }
    }

    /// Pop the most recently-closed tab, if any.
    #[must_use]
    pub fn pop(&mut self) -> Option<ClosedTab> {
        self.entries.pop()
    }

    /// Whether a reopen would do anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The app-layer companion to the dock tree: pinning, history, closed tabs.
///
/// Persisted with `#[serde(default)]` so older layout files load cleanly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DockExtras {
    /// The pinned tabs. A pinned tab survives "Close others" / "Close all" and
    /// shows a pin marker in its right-click menu.
    #[serde(default)]
    pub pinned: Vec<Tab>,
    /// Per-tab navigation history, keyed by the tab the history belongs to.
    #[serde(default)]
    pub history: HashMap<Tab, TabHistory>,
    /// The recently-closed-tab stack for `Cmd/Ctrl+Shift+T`.
    #[serde(default)]
    pub closed: ClosedTabStack,
}

impl DockExtras {
    /// Whether `tab` is pinned.
    #[must_use]
    pub fn is_pinned(&self, tab: &Tab) -> bool {
        self.pinned.iter().any(|t| t == tab)
    }

    /// Toggle `tab`'s pinned state.
    pub fn toggle_pin(&mut self, tab: &Tab) {
        if let Some(pos) = self.pinned.iter().position(|t| t == tab) {
            self.pinned.remove(pos);
        } else {
            self.pinned.push(tab.clone());
        }
    }

    /// Whether `tab` can navigate back.
    #[must_use]
    pub fn can_go_back(&self, tab: &Tab) -> bool {
        self.history.get(tab).is_some_and(TabHistory::can_go_back)
    }

    /// Whether `tab` can navigate forward.
    #[must_use]
    pub fn can_go_forward(&self, tab: &Tab) -> bool {
        self.history
            .get(tab)
            .is_some_and(TabHistory::can_go_forward)
    }

    /// Whether reopening the last-closed tab would do anything.
    #[must_use]
    pub fn can_reopen_closed(&self) -> bool {
        !self.closed.is_empty()
    }

    /// Drop pin / history state for tabs no longer present in the dock.
    ///
    /// Called once per frame with the set of live tabs so a closed tab's
    /// bookkeeping does not leak. The closed-tab stack is intentionally *not*
    /// pruned — its whole purpose is to outlive the tab.
    pub fn retain_open<'a>(&mut self, open: impl Iterator<Item = &'a Tab> + Clone) {
        self.pinned.retain(|tab| open.clone().any(|t| t == tab));
        self.history.retain(|tab, _| open.clone().any(|t| t == tab));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> Tab {
        Tab::Home
    }

    fn album(id: &str) -> Tab {
        Tab::Album(id.to_owned())
    }

    #[test]
    fn fresh_history_cannot_navigate() {
        let history = TabHistory::default();
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
    }

    #[test]
    fn navigate_then_back_then_forward_round_trips() {
        let mut history = TabHistory::default();
        // Start on Home, navigate to album A, then album B.
        history.navigate(home());
        history.navigate(album("a"));
        // Currently on album B; step back twice.
        assert_eq!(history.back(album("b")), Some(album("a")));
        assert_eq!(history.back(album("a")), Some(home()));
        assert!(!history.can_go_back());
        assert!(history.can_go_forward());
        // Step forward twice, back to album B.
        assert_eq!(history.forward(home()), Some(album("a")));
        assert_eq!(history.forward(album("a")), Some(album("b")));
        assert!(!history.can_go_forward());
    }

    #[test]
    fn fresh_navigation_clears_the_forward_stack() {
        let mut history = TabHistory::default();
        // Visit Home, then album A — now sitting on album B implicitly.
        history.navigate(home());
        history.navigate(album("a"));
        // Step back once: leaves B on the forward stack, lands on album A.
        assert_eq!(history.back(album("b")), Some(album("a")));
        assert!(history.can_go_forward());
        // A fresh navigation from album A invalidates the forward stack.
        history.navigate(album("a"));
        assert!(!history.can_go_forward());
        assert!(history.can_go_back());
    }

    #[test]
    fn back_at_history_start_returns_none() {
        let mut history = TabHistory::default();
        assert_eq!(history.back(home()), None);
        assert_eq!(history.forward(home()), None);
    }

    #[test]
    fn closed_stack_is_lifo() {
        let mut stack = ClosedTabStack::default();
        assert!(stack.is_empty());
        stack.push(home(), TabHistory::default());
        stack.push(album("a"), TabHistory::default());
        assert_eq!(stack.pop().map(|c| c.tab), Some(album("a")));
        assert_eq!(stack.pop().map(|c| c.tab), Some(home()));
        assert!(stack.is_empty());
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn closed_stack_is_capped() {
        let mut stack = ClosedTabStack::default();
        for i in 0..(CLOSED_STACK_CAP + 5) {
            stack.push(album(&i.to_string()), TabHistory::default());
        }
        // Only the cap-many most-recent survive; the oldest five were evicted.
        let mut seen = 0;
        while let Some(closed) = stack.pop() {
            seen += 1;
            // The five oldest (`album("0")`..`album("4")`) must be gone.
            if let Tab::Album(id) = closed.tab {
                assert!(id.parse::<usize>().unwrap_or(0) >= 5);
            }
        }
        assert_eq!(seen, CLOSED_STACK_CAP);
    }

    #[test]
    fn closed_stack_round_trips_history() {
        let mut history = TabHistory::default();
        history.navigate(home());
        let mut stack = ClosedTabStack::default();
        stack.push(album("a"), history.clone());
        let reopened = stack.pop().expect("one entry");
        assert_eq!(reopened.tab, album("a"));
        assert!(reopened.history.can_go_back());
    }

    #[test]
    fn pin_toggle_is_idempotent_pairwise() {
        let mut extras = DockExtras::default();
        assert!(!extras.is_pinned(&home()));
        extras.toggle_pin(&home());
        assert!(extras.is_pinned(&home()));
        extras.toggle_pin(&home());
        assert!(!extras.is_pinned(&home()));
    }

    #[test]
    fn retain_open_drops_pins_and_history_for_gone_tabs() {
        let mut extras = DockExtras::default();
        extras.toggle_pin(&album("a"));
        extras.toggle_pin(&album("b"));
        extras
            .history
            .entry(album("a"))
            .or_default()
            .navigate(home());
        extras
            .history
            .entry(album("b"))
            .or_default()
            .navigate(home());
        // Only album A survives in the dock.
        let open = [album("a")];
        extras.retain_open(open.iter());
        assert!(extras.is_pinned(&album("a")));
        assert!(!extras.is_pinned(&album("b")));
        assert!(extras.history.contains_key(&album("a")));
        assert!(!extras.history.contains_key(&album("b")));
    }

    #[test]
    fn retain_open_keeps_the_closed_stack() {
        let mut extras = DockExtras::default();
        extras.closed.push(album("gone"), TabHistory::default());
        // Even with no open tabs, the closed stack is preserved.
        extras.retain_open(std::iter::empty());
        assert!(!extras.closed.is_empty());
    }
}
