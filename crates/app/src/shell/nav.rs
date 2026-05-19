//! Tab navigation and tab-management: the rules behind plain-click-replaces,
//! Ctrl/Cmd-click-new-tab, the right-click menu's close family, duplicate and
//! reopen-last-closed.
//!
//! `egui_dock` 0.19 owns the dock tree and supplies the per-tab right-click
//! menu *hook* and a closeable hook, but the *policy* — what "Close others"
//! spares, where a replaced tab's old surface goes, that a pinned tab is never
//! collateral — is Spottyfi's, and lives here. Everything operates on the
//! [`DockState`](egui_dock::DockState) plus the app-layer
//! [`DockExtras`](super::dock_model::DockExtras).

use egui_dock::DockState;

use super::dock_model::DockExtras;
use super::tabs::Tab;

/// Open `tab`, replacing the focused tab's content (the `docs/docking.md`
/// plain-click rule).
///
/// If `tab` is already open anywhere, it is simply focused — never duplicated.
/// Otherwise the focused tab is *replaced*: the surface it held is pushed onto
/// its per-tab history (so Back returns to it) and the leaf now shows `tab`.
/// The Home tab is never replaced — replacing it would lose the always-open
/// anchor — so navigating from a focused Home falls back to opening a new tab.
pub fn navigate_replace(dock: &mut DockState<Tab>, extras: &mut DockExtras, tab: Tab) {
    navigate_replace_in(dock, extras, tab, NavTarget::Focused);
}

/// Open `tab` in the **main (centre) pane**, replacing the main pane's active
/// tab — the rule for sidebar navigation.
///
/// A sidebar entry must always open a page in the centre tab group, never
/// inside a side-panel leaf, regardless of which leaf currently has focus.
/// This resolves the main leaf (the one holding the always-open Home anchor)
/// and replaces its active tab there.
pub fn navigate_replace_main(dock: &mut DockState<Tab>, extras: &mut DockExtras, tab: Tab) {
    navigate_replace_in(dock, extras, tab, NavTarget::Main);
}

/// Open `tab` in a new tab inside the **main (centre) pane** — the Ctrl/Cmd
/// -click rule for sidebar navigation.
pub fn open_new_tab_main(dock: &mut DockState<Tab>, tab: Tab) {
    let leaf = main_leaf_path(dock);
    insert_new_tab(dock, leaf, tab);
}

/// Which leaf a navigation should target.
#[derive(Clone, Copy)]
enum NavTarget {
    /// The currently-focused leaf (in-page links, the menu bar).
    Focused,
    /// The main (centre) pane — the leaf holding the Home anchor.
    Main,
}

/// The node path of the **main pane**: the leaf containing the Home tab.
///
/// Home is the always-open anchor of the centre tab group, so the leaf that
/// holds it *is* the main pane. Falls back to the focused (or first) leaf if
/// Home somehow is not present, so navigation always resolves somewhere sane.
fn main_leaf_path(dock: &DockState<Tab>) -> Option<egui_dock::NodePath> {
    dock.find_tab(&Tab::Home)
        .map(|path| path.node_path())
        .or_else(|| active_leaf_path(dock))
}

/// Shared replace-navigation, parameterised by which leaf to target.
fn navigate_replace_in(
    dock: &mut DockState<Tab>,
    extras: &mut DockExtras,
    tab: Tab,
    target: NavTarget,
) {
    // Already open — just focus it.
    if let Some(path) = dock.find_tab(&tab) {
        dock.set_focused_node_and_surface(path.node_path());
        let _ = dock.set_active_tab(path);
        return;
    }

    let leaf = match target {
        NavTarget::Focused => active_leaf_path(dock),
        NavTarget::Main => main_leaf_path(dock),
    };

    // Resolve the target leaf and its active-tab slot — the surface there is
    // the one being replaced.
    let replaced = leaf.and_then(|leaf_path| {
        if let Ok(egui_dock::Node::Leaf(leaf)) = dock.node(leaf_path) {
            let active = leaf.active.0;
            leaf.tabs
                .get(active)
                .cloned()
                .map(|current| (leaf_path, active, current))
        } else {
            None
        }
    });

    if let Some((leaf_path, active, current)) = replaced {
        // Home is the always-open anchor and a pinned tab is sticky — neither
        // is ever replaced; navigating from one opens a new tab instead.
        if current != Tab::Home && !extras.is_pinned(&current) {
            // Carry the replaced tab's history onto the new surface, then
            // record the navigation step.
            let mut history = extras.history.remove(&current).unwrap_or_default();
            history.navigate(current);
            extras.history.insert(tab.clone(), history);
            if let Ok(egui_dock::Node::Leaf(leaf)) = dock.node_mut(leaf_path) {
                if let Some(slot) = leaf.tabs.get_mut(active) {
                    *slot = tab;
                }
            }
            dock.set_focused_node_and_surface(leaf_path);
            return;
        }
    }

    // No replaceable tab in the target leaf (Home, a pinned tab, or nothing) —
    // open `tab` as a new tab in that same leaf.
    let leaf = match target {
        NavTarget::Focused => None,
        NavTarget::Main => main_leaf_path(dock),
    };
    insert_new_tab(dock, leaf, tab);
}

/// Reveal a side-panel tab (Queue, Now Playing Art, Visualiser): focus it if
/// already open, otherwise dock it next to any existing side panel, or — when
/// no side-panel column exists at all — split a fresh right-hand column off
/// the main pane.
///
/// This gives a sensible default home for panels; the user is free to drag
/// them elsewhere afterwards.
pub fn reveal_side_panel(dock: &mut DockState<Tab>, tab: Tab) {
    if focus_if_open(dock, &tab) {
        return;
    }
    if let Some(leaf) = side_panel_leaf_path(dock) {
        insert_new_tab(dock, Some(leaf), tab);
        return;
    }
    // No side-panel column exists — carve one off the right of the main pane.
    match main_leaf_path(dock).filter(|path| path.surface == egui_dock::SurfaceIndex::main()) {
        Some(main) => {
            dock[main.surface].split_right(main.node, 0.74, vec![tab.clone()]);
            let _ = focus_if_open(dock, &tab);
        }
        None => insert_new_tab(dock, None, tab),
    }
}

/// Reveal a centre tab (Settings, Devices, a page): focus it if already open,
/// otherwise add it as a new tab in the main (centre) pane rather than in
/// whichever side-panel leaf happens to hold focus.
pub fn reveal_centre(dock: &mut DockState<Tab>, tab: Tab) {
    if focus_if_open(dock, &tab) {
        return;
    }
    open_new_tab_main(dock, tab);
}

/// Focus `tab` if it is already open anywhere; returns whether it was found.
fn focus_if_open(dock: &mut DockState<Tab>, tab: &Tab) -> bool {
    if let Some(path) = dock.find_tab(tab) {
        dock.set_focused_node_and_surface(path.node_path());
        let _ = dock.set_active_tab(path);
        true
    } else {
        false
    }
}

/// The leaf of the first open side-panel tab, if any — the column new side
/// panels should join.
fn side_panel_leaf_path(dock: &DockState<Tab>) -> Option<egui_dock::NodePath> {
    [Tab::Queue, Tab::NowPlayingArt, Tab::Visualiser]
        .iter()
        .find_map(|tab| dock.find_tab(tab).map(|path| path.node_path()))
}

/// Open `tab` in a brand-new tab (the Ctrl/Cmd-click rule), focusing it.
///
/// Unlike [`navigate_replace`] this always adds a tab even when `tab` is
/// already open elsewhere — Ctrl/Cmd-click is an explicit "give me another
/// one" gesture.
pub fn open_new_tab(dock: &mut DockState<Tab>, tab: Tab) {
    insert_new_tab(dock, None, tab);
}

/// Insert `tab` as a new tab and focus it.
///
/// When `leaf` is `Some`, the tab is pushed into that exact leaf (used to keep
/// sidebar navigation in the main pane); when `None`, it goes to the focused
/// leaf, `egui_dock`'s default.
fn insert_new_tab(dock: &mut DockState<Tab>, leaf: Option<egui_dock::NodePath>, tab: Tab) {
    match leaf {
        Some(leaf_path) => {
            if let Ok(egui_dock::Node::Leaf(node)) = dock.node_mut(leaf_path) {
                node.append_tab(tab.clone());
            } else {
                dock.push_to_focused_leaf(tab.clone());
            }
        }
        None => dock.push_to_focused_leaf(tab.clone()),
    }
    if let Some(path) = dock.find_tab(&tab) {
        // Focus the leaf the tab landed in, then make it the active tab —
        // `set_active_tab` alone does not move the *leaf* focus, which Back /
        // Forward and replace-navigation rely on.
        dock.set_focused_node_and_surface(path.node_path());
        let _ = dock.set_active_tab(path);
    }
}

/// The node path of the focused leaf, falling back to the first leaf.
///
/// A freshly-built or freshly-deserialised [`DockState`] has no focused leaf
/// until the user clicks inside the `DockArea`; navigation must still work
/// before that first click, so this resolves a sensible leaf either way.
fn active_leaf_path(dock: &DockState<Tab>) -> Option<egui_dock::NodePath> {
    if let Some(path) = dock.focused_leaf() {
        return Some(path);
    }
    dock.iter_leaves().next().map(|(path, _)| path)
}

/// The currently focused (or, lacking focus, first-leaf-active) tab.
#[must_use]
pub fn focused_tab(dock: &DockState<Tab>) -> Option<Tab> {
    let path = active_leaf_path(dock)?;
    if let Ok(egui_dock::Node::Leaf(leaf)) = dock.node(path) {
        leaf.tabs.get(leaf.active.0).cloned()
    } else {
        None
    }
}

/// Whether `tab` may be closed: Home is the always-open anchor and pinned tabs
/// are spared by the close family.
fn is_closeable(extras: &DockExtras, tab: &Tab) -> bool {
    !matches!(tab, Tab::Home) && !extras.is_pinned(tab)
}

/// Close `tab`, recording it on the closed-tab stack so it can be reopened.
///
/// A no-op if `tab` is not closeable (Home, or pinned). The tab's per-tab
/// history travels with it onto the closed stack.
pub fn close_tab(dock: &mut DockState<Tab>, extras: &mut DockExtras, tab: &Tab) {
    if !is_closeable(extras, tab) {
        return;
    }
    if let Some(path) = dock.find_tab(tab) {
        if dock.remove_tab(path).is_some() {
            let history = extras.history.remove(tab).unwrap_or_default();
            extras.closed.push(tab.clone(), history);
        }
    }
}

/// Close every tab except `keep` — sparing Home and every pinned tab.
///
/// `keep` itself is also spared even if it would otherwise be closeable.
pub fn close_others(dock: &mut DockState<Tab>, extras: &mut DockExtras, keep: &Tab) {
    let victims: Vec<Tab> = dock
        .iter_all_tabs()
        .map(|(_, tab)| tab.clone())
        .filter(|tab| tab != keep && is_closeable(extras, tab))
        .collect();
    for victim in victims {
        close_tab(dock, extras, &victim);
    }
}

/// Close every tab positioned to the right of `anchor` within its own leaf —
/// sparing Home and pinned tabs.
pub fn close_to_right(dock: &mut DockState<Tab>, extras: &mut DockExtras, anchor: &Tab) {
    let Some(anchor_path) = dock.find_tab(anchor) else {
        return;
    };
    // Snapshot the anchor's leaf tab list, then close the ones after it.
    let victims: Vec<Tab> = match dock.node(anchor_path.node_path()) {
        Ok(egui_dock::Node::Leaf(leaf)) => leaf
            .tabs
            .iter()
            .skip(anchor_path.tab.0 + 1)
            .filter(|tab| is_closeable(extras, tab))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    for victim in victims {
        close_tab(dock, extras, &victim);
    }
}

/// Open a second tab carrying the same surface as `tab`, focused.
pub fn duplicate_tab(dock: &mut DockState<Tab>, tab: &Tab) {
    open_new_tab(dock, tab.clone());
}

/// Reopen the most recently closed tab, restoring its history. A no-op when the
/// closed stack is empty.
pub fn reopen_last_closed(dock: &mut DockState<Tab>, extras: &mut DockExtras) {
    let Some(closed) = extras.closed.pop() else {
        return;
    };
    // Already open again (the user reopened it another way) — just focus it.
    if let Some(path) = dock.find_tab(&closed.tab) {
        let _ = dock.set_active_tab(path);
        return;
    }
    extras.history.insert(closed.tab.clone(), closed.history);
    open_new_tab(dock, closed.tab);
}

/// Navigate the **main pane's** active tab back one step in its history.
///
/// The menu-bar Back / Forward buttons are a global navigation control, so
/// they always act on the centre tab group — never on whichever side panel
/// happens to hold focus (a panel has no history, which made the buttons
/// silently do nothing). The surface swapped out goes onto the forward stack.
pub fn go_back(dock: &mut DockState<Tab>, extras: &mut DockExtras) {
    step_history(dock, extras, HistoryStep::Back);
}

/// Navigate the main pane's active tab forward one step in its history.
pub fn go_forward(dock: &mut DockState<Tab>, extras: &mut DockExtras) {
    step_history(dock, extras, HistoryStep::Forward);
}

/// The active tab of the main (centre) pane — the leaf holding Home.
fn main_pane_tab(dock: &DockState<Tab>) -> Option<Tab> {
    let path = main_leaf_path(dock)?;
    if let Ok(egui_dock::Node::Leaf(leaf)) = dock.node(path) {
        leaf.tabs.get(leaf.active.0).cloned()
    } else {
        None
    }
}

/// Whether the main pane's active tab can step back in its history.
#[must_use]
pub fn can_go_back(dock: &DockState<Tab>, extras: &DockExtras) -> bool {
    main_pane_tab(dock).is_some_and(|tab| extras.can_go_back(&tab))
}

/// Whether the main pane's active tab can step forward in its history.
#[must_use]
pub fn can_go_forward(dock: &DockState<Tab>, extras: &DockExtras) -> bool {
    main_pane_tab(dock).is_some_and(|tab| extras.can_go_forward(&tab))
}

/// Which direction a history step goes.
#[derive(Clone, Copy)]
enum HistoryStep {
    Back,
    Forward,
}

/// Shared back/forward implementation: swap the main pane's active tab for its
/// history neighbour, in place, and re-key its history under the new surface.
fn step_history(dock: &mut DockState<Tab>, extras: &mut DockExtras, step: HistoryStep) {
    // Resolve the main (centre) leaf and the *exact* active-tab slot within it,
    // so the swap targets the right tab even if its surface is open twice.
    let Some(leaf_path) = main_leaf_path(dock) else {
        return;
    };
    let Ok(egui_dock::Node::Leaf(leaf)) = dock.node(leaf_path) else {
        return;
    };
    let active = leaf.active.0;
    let Some(current) = leaf.tabs.get(active).cloned() else {
        return;
    };
    let Some(mut history) = extras.history.remove(&current) else {
        return;
    };
    let neighbour = match step {
        HistoryStep::Back => history.back(current.clone()),
        HistoryStep::Forward => history.forward(current.clone()),
    };
    match neighbour {
        Some(target) => {
            // Swap the surface in place so the tab keeps its slot in the bar.
            if let Ok(egui_dock::Node::Leaf(leaf)) = dock.node_mut(leaf_path) {
                if let Some(slot) = leaf.tabs.get_mut(active) {
                    *slot = target.clone();
                }
            }
            extras.history.insert(target, history);
        }
        None => {
            // Nothing to step to — put the history back untouched.
            extras.history.insert(current, history);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dock_with(tabs: Vec<Tab>) -> DockState<Tab> {
        DockState::new(tabs)
    }

    fn open_tabs(dock: &DockState<Tab>) -> Vec<Tab> {
        dock.iter_all_tabs().map(|(_, t)| t.clone()).collect()
    }

    #[test]
    fn navigate_replace_swaps_the_focused_tab() {
        let mut dock = dock_with(vec![Tab::Home]);
        let mut extras = DockExtras::default();
        // Open an album, then navigate from it — it should be replaced, not
        // stacked, and Home stays the only other tab.
        open_new_tab(&mut dock, Tab::Album("a".into()));
        navigate_replace(&mut dock, &mut extras, Tab::Artist("z".into()));
        let tabs = open_tabs(&dock);
        assert!(tabs.contains(&Tab::Home));
        assert!(tabs.contains(&Tab::Artist("z".into())));
        assert!(!tabs.contains(&Tab::Album("a".into())));
        // The replaced album is now reachable via Back.
        assert!(extras.can_go_back(&Tab::Artist("z".into())));
    }

    #[test]
    fn sidebar_navigation_lands_in_the_main_pane() {
        use egui_dock::{NodeIndex, SurfaceIndex};
        // A two-leaf dock: Home in the centre, a Queue panel in a right leaf.
        let mut dock = dock_with(vec![Tab::Home]);
        let surface = SurfaceIndex::main();
        dock[surface].split_right(NodeIndex::root(), 0.7, vec![Tab::Queue]);
        let mut extras = DockExtras::default();

        // Focus the side-panel leaf — as if the user last clicked the Queue.
        let queue_leaf = dock.find_tab(&Tab::Queue).expect("queue tab").node_path();
        dock.set_focused_node_and_surface(queue_leaf);

        // A sidebar click navigates to Browse via the main-pane rule.
        navigate_replace_main(&mut dock, &mut extras, Tab::Browse);

        // Browse must have replaced Home in the *centre* leaf, leaving the
        // Queue panel untouched in its own leaf.
        let main_leaf = main_leaf_path(&dock);
        let browse_leaf = dock.find_tab(&Tab::Browse).map(|p| p.node_path());
        assert_eq!(browse_leaf, main_leaf, "Browse opened in the main pane");
        let queue_still = dock.find_tab(&Tab::Queue).map(|p| p.node_path());
        assert_eq!(
            queue_still,
            Some(queue_leaf),
            "the Queue panel leaf is left alone"
        );
    }

    #[test]
    fn revealed_side_panel_joins_an_existing_panel_column() {
        use egui_dock::{NodeIndex, SurfaceIndex};
        let mut dock = dock_with(vec![Tab::Home]);
        let surface = SurfaceIndex::main();
        dock[surface].split_right(NodeIndex::root(), 0.7, vec![Tab::Queue]);
        let queue_leaf = dock.find_tab(&Tab::Queue).expect("queue").node_path();

        // The Visualiser is a side panel — it must dock beside the Queue, not
        // in the centre Home leaf.
        reveal_side_panel(&mut dock, Tab::Visualiser);
        let vis_leaf = dock.find_tab(&Tab::Visualiser).map(|p| p.node_path());
        assert_eq!(vis_leaf, Some(queue_leaf), "Visualiser joined the column");
        assert_ne!(vis_leaf, main_leaf_path(&dock));
    }

    #[test]
    fn revealed_side_panel_splits_a_column_when_none_exists() {
        // A single-leaf dock with only the centre Home tab.
        let mut dock = dock_with(vec![Tab::Home]);
        reveal_side_panel(&mut dock, Tab::Queue);
        // The Queue gets its own leaf, distinct from the Home (centre) leaf.
        let queue_leaf = dock.find_tab(&Tab::Queue).map(|p| p.node_path());
        assert!(queue_leaf.is_some(), "Queue was docked");
        assert_ne!(
            queue_leaf,
            main_leaf_path(&dock),
            "Queue is not in the centre"
        );
    }

    #[test]
    fn revealed_centre_tab_lands_in_the_main_pane() {
        use egui_dock::{NodeIndex, SurfaceIndex};
        let mut dock = dock_with(vec![Tab::Home]);
        let surface = SurfaceIndex::main();
        dock[surface].split_right(NodeIndex::root(), 0.7, vec![Tab::Queue]);
        // Focus the side panel, then reveal Settings — a centre tab.
        let queue_leaf = dock.find_tab(&Tab::Queue).expect("queue").node_path();
        dock.set_focused_node_and_surface(queue_leaf);

        reveal_centre(&mut dock, Tab::Settings);
        let settings_leaf = dock.find_tab(&Tab::Settings).map(|p| p.node_path());
        assert_eq!(
            settings_leaf,
            main_leaf_path(&dock),
            "Settings opened in the centre pane"
        );
    }

    #[test]
    fn sidebar_new_tab_lands_in_the_main_pane() {
        use egui_dock::{NodeIndex, SurfaceIndex};
        let mut dock = dock_with(vec![Tab::Home]);
        let surface = SurfaceIndex::main();
        dock[surface].split_right(NodeIndex::root(), 0.7, vec![Tab::Queue]);

        // Focus the side panel, then Ctrl/Cmd-click a sidebar entry.
        let queue_leaf = dock.find_tab(&Tab::Queue).expect("queue").node_path();
        dock.set_focused_node_and_surface(queue_leaf);
        open_new_tab_main(&mut dock, Tab::Charts);

        // Charts lands in the Home (main) leaf, not the focused panel leaf.
        let home_leaf = dock.find_tab(&Tab::Home).expect("home").node_path();
        assert_eq!(
            dock.find_tab(&Tab::Charts).map(|p| p.node_path()),
            Some(home_leaf),
        );
    }

    #[test]
    fn navigate_replace_focuses_an_already_open_tab() {
        let mut dock = dock_with(vec![Tab::Home]);
        let mut extras = DockExtras::default();
        open_new_tab(&mut dock, Tab::Browse);
        open_new_tab(&mut dock, Tab::Album("a".into()));
        // Browse is already open — navigating to it focuses, never duplicates.
        navigate_replace(&mut dock, &mut extras, Tab::Browse);
        let count = open_tabs(&dock)
            .iter()
            .filter(|t| **t == Tab::Browse)
            .count();
        assert_eq!(count, 1);
        assert_eq!(open_tabs(&dock).len(), 3);
    }

    #[test]
    fn navigate_from_home_opens_a_new_tab() {
        let mut dock = dock_with(vec![Tab::Home]);
        let mut extras = DockExtras::default();
        // Home is focused and must never be replaced.
        navigate_replace(&mut dock, &mut extras, Tab::Search);
        let tabs = open_tabs(&dock);
        assert!(tabs.contains(&Tab::Home));
        assert!(tabs.contains(&Tab::Search));
    }

    #[test]
    fn open_new_tab_always_adds_even_if_open() {
        let mut dock = dock_with(vec![Tab::Home]);
        open_new_tab(&mut dock, Tab::Browse);
        open_new_tab(&mut dock, Tab::Browse);
        let count = open_tabs(&dock)
            .iter()
            .filter(|t| **t == Tab::Browse)
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn close_others_spares_home_and_pinned() {
        let mut dock = dock_with(vec![Tab::Home]);
        let mut extras = DockExtras::default();
        open_new_tab(&mut dock, Tab::Browse);
        open_new_tab(&mut dock, Tab::Album("a".into()));
        open_new_tab(&mut dock, Tab::Album("b".into()));
        extras.toggle_pin(&Tab::Album("a".into()));
        // Close everything but Browse.
        close_others(&mut dock, &mut extras, &Tab::Browse);
        let tabs = open_tabs(&dock);
        assert!(tabs.contains(&Tab::Home), "Home is spared");
        assert!(tabs.contains(&Tab::Browse), "the kept tab survives");
        assert!(
            tabs.contains(&Tab::Album("a".into())),
            "the pinned tab is spared"
        );
        assert!(!tabs.contains(&Tab::Album("b".into())), "others close");
    }

    #[test]
    fn close_tab_will_not_close_home_or_a_pinned_tab() {
        let mut dock = dock_with(vec![Tab::Home]);
        let mut extras = DockExtras::default();
        open_new_tab(&mut dock, Tab::Browse);
        extras.toggle_pin(&Tab::Browse);
        close_tab(&mut dock, &mut extras, &Tab::Home);
        close_tab(&mut dock, &mut extras, &Tab::Browse);
        let tabs = open_tabs(&dock);
        assert!(tabs.contains(&Tab::Home));
        assert!(tabs.contains(&Tab::Browse));
    }

    #[test]
    fn close_tab_records_the_closed_stack() {
        let mut dock = dock_with(vec![Tab::Home]);
        let mut extras = DockExtras::default();
        open_new_tab(&mut dock, Tab::Album("a".into()));
        close_tab(&mut dock, &mut extras, &Tab::Album("a".into()));
        assert!(!open_tabs(&dock).contains(&Tab::Album("a".into())));
        assert!(!extras.closed.is_empty());
        // Reopening brings it back.
        reopen_last_closed(&mut dock, &mut extras);
        assert!(open_tabs(&dock).contains(&Tab::Album("a".into())));
    }

    #[test]
    fn close_to_right_spares_left_and_pinned() {
        let mut dock = dock_with(vec![Tab::Home]);
        let mut extras = DockExtras::default();
        // A single leaf: Home, Browse, Album(a), Album(b), Charts.
        open_new_tab(&mut dock, Tab::Browse);
        open_new_tab(&mut dock, Tab::Album("a".into()));
        open_new_tab(&mut dock, Tab::Album("b".into()));
        open_new_tab(&mut dock, Tab::Charts);
        extras.toggle_pin(&Tab::Album("b".into()));
        // Close everything to the right of Browse.
        close_to_right(&mut dock, &mut extras, &Tab::Browse);
        let tabs = open_tabs(&dock);
        assert!(tabs.contains(&Tab::Home));
        assert!(tabs.contains(&Tab::Browse));
        assert!(
            tabs.contains(&Tab::Album("b".into())),
            "the pinned tab is spared"
        );
        assert!(!tabs.contains(&Tab::Album("a".into())));
        assert!(!tabs.contains(&Tab::Charts));
    }

    #[test]
    fn back_and_forward_walk_a_tabs_history() {
        let mut dock = dock_with(vec![Tab::Home]);
        let mut extras = DockExtras::default();
        // Open an album tab, then navigate twice within it.
        open_new_tab(&mut dock, Tab::Album("a".into()));
        navigate_replace(&mut dock, &mut extras, Tab::Album("b".into()));
        navigate_replace(&mut dock, &mut extras, Tab::Artist("c".into()));
        // The focused tab is now Artist(c); step back to b, then a.
        go_back(&mut dock, &mut extras);
        assert_eq!(focused_tab(&dock), Some(Tab::Album("b".into())));
        go_back(&mut dock, &mut extras);
        assert_eq!(focused_tab(&dock), Some(Tab::Album("a".into())));
        assert!(!can_go_back(&dock, &extras));
        assert!(can_go_forward(&dock, &extras));
        // Step forward again.
        go_forward(&mut dock, &mut extras);
        assert_eq!(focused_tab(&dock), Some(Tab::Album("b".into())));
    }

    #[test]
    fn history_navigation_targets_the_main_pane() {
        use egui_dock::{NodeIndex, SurfaceIndex};
        // Centre leaf with Home; navigate it twice so it has back history.
        let mut dock = dock_with(vec![Tab::Home]);
        let surface = SurfaceIndex::main();
        dock[surface].split_right(NodeIndex::root(), 0.7, vec![Tab::Queue]);
        let mut extras = DockExtras::default();

        navigate_replace_main(&mut dock, &mut extras, Tab::Browse);
        navigate_replace_main(&mut dock, &mut extras, Tab::Charts);

        // Focus the side panel — Back must still walk the *main pane*.
        let queue_leaf = dock.find_tab(&Tab::Queue).expect("queue").node_path();
        dock.set_focused_node_and_surface(queue_leaf);

        assert!(can_go_back(&dock, &extras), "main pane has history");
        go_back(&mut dock, &mut extras);
        assert_eq!(main_pane_tab(&dock), Some(Tab::Browse));
    }

    #[test]
    fn duplicate_adds_a_second_copy() {
        let mut dock = dock_with(vec![Tab::Home]);
        open_new_tab(&mut dock, Tab::Browse);
        duplicate_tab(&mut dock, &Tab::Browse);
        let count = open_tabs(&dock)
            .iter()
            .filter(|t| **t == Tab::Browse)
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn reopen_with_empty_stack_is_a_no_op() {
        let mut dock = dock_with(vec![Tab::Home]);
        let mut extras = DockExtras::default();
        reopen_last_closed(&mut dock, &mut extras);
        assert_eq!(open_tabs(&dock), vec![Tab::Home]);
    }
}
