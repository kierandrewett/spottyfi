//! The background-activity registry.
//!
//! Long-running background work — playlist loads, search, image-heavy page
//! loads — registers an [`Activity`] here when it starts and deregisters it
//! when it finishes. The UI reads a cheap snapshot of the live activities once
//! per frame and surfaces them as a VSCode-style status area in the menu bar.
//!
//! The registry is shared as an [`Arc<ActivityRegistry>`]; it is internally
//! synchronised, so background tasks on the tokio runtime and the egui UI
//! thread can both touch it freely. It follows the same snapshot shape as the
//! rest of the app: the UI never mutates, it only reads a [`Vec<Activity>`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A unique handle identifying one registered activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActivityId(u64);

/// One in-flight background task, as seen by the UI.
#[derive(Clone)]
pub struct Activity {
    /// The activity's unique id.
    pub id: ActivityId,
    /// A short human label, e.g. `"Loading playlist…"`.
    pub label: String,
    /// When the activity was registered — lets the UI show an elapsed time.
    pub started_at: Instant,
    /// `true` when the activity can be cancelled by the user.
    pub cancellable: bool,
}

/// The interior of one activity entry, including its cancel hook.
struct Entry {
    activity: Activity,
    /// Invoked when the user cancels the activity. It should signal the
    /// underlying task to stop — typically by tripping a shared cancel flag
    /// the task observes — rather than aborting it outright, so a task that
    /// owns a `poll_promise` `Sender` is never dropped mid-flight.
    cancel: Option<Box<dyn FnOnce() + Send>>,
}

/// A shared registry of in-flight background activities.
///
/// Cloned cheaply behind an `Arc`. Background work calls [`register`] /
/// [`finish`] (or holds an [`ActivityGuard`] that deregisters on drop); the UI
/// calls [`snapshot`] once per frame.
///
/// [`register`]: ActivityRegistry::register
/// [`finish`]: ActivityRegistry::finish
/// [`snapshot`]: ActivityRegistry::snapshot
#[derive(Default)]
pub struct ActivityRegistry {
    /// The live activities. A `Mutex` is plenty — touched a handful of times
    /// per second, never on a hot path.
    entries: Mutex<Vec<Entry>>,
    /// The monotonically increasing source of [`ActivityId`]s.
    next_id: AtomicU64,
}

impl ActivityRegistry {
    /// Build an empty registry.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a new activity with the given label. Returns its id.
    ///
    /// `cancellable` is `false`: use [`register_cancellable`] to attach a
    /// cancel hook.
    ///
    /// [`register_cancellable`]: ActivityRegistry::register_cancellable
    pub fn register(&self, label: impl Into<String>) -> ActivityId {
        self.insert(label.into(), None)
    }

    /// Register a new activity that the user may cancel.
    ///
    /// `cancel` is invoked at most once — when the user clicks the cancel
    /// affordance, or never if the activity finishes first. It should signal
    /// the underlying task to stop, typically by tripping a shared cancel flag
    /// the task observes. It must **not** abort a task that owns a
    /// `poll_promise` `Sender`: dropping that `Sender` unsent panics the next
    /// poll of the owning `Promise`.
    pub fn register_cancellable(
        &self,
        label: impl Into<String>,
        cancel: impl FnOnce() + Send + 'static,
    ) -> ActivityId {
        self.insert(label.into(), Some(Box::new(cancel)))
    }

    /// Insert an entry and return its id.
    fn insert(&self, label: String, cancel: Option<Box<dyn FnOnce() + Send>>) -> ActivityId {
        let id = ActivityId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let activity = Activity {
            id,
            label,
            started_at: Instant::now(),
            cancellable: cancel.is_some(),
        };
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(Entry { activity, cancel });
        }
        id
    }

    /// Deregister the activity with `id`. A no-op if it is already gone.
    pub fn finish(&self, id: ActivityId) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|entry| entry.activity.id != id);
        }
    }

    /// Cancel the activity with `id`: run its cancel hook (if any) and remove
    /// it. Called by the UI when the user clicks the cancel affordance.
    pub fn cancel(&self, id: ActivityId) {
        let cancel = {
            let Ok(mut entries) = self.entries.lock() else {
                return;
            };
            let Some(pos) = entries.iter().position(|e| e.activity.id == id) else {
                return;
            };
            entries.remove(pos).cancel
        };
        // Run the hook outside the lock — it may be arbitrary user code.
        if let Some(cancel) = cancel {
            cancel();
        }
    }

    /// A cheap snapshot of the live activities, oldest first. Read by the UI
    /// once per frame.
    #[must_use]
    pub fn snapshot(&self) -> Vec<Activity> {
        self.entries
            .lock()
            .map(|entries| entries.iter().map(|e| e.activity.clone()).collect())
            .unwrap_or_default()
    }

    /// Whether any background activity is currently in flight.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.entries
            .lock()
            .map(|entries| !entries.is_empty())
            .unwrap_or(false)
    }
}

/// An RAII guard that deregisters its activity when dropped.
///
/// Holding the guard for the lifetime of a background task guarantees the
/// activity disappears from the indicator when the task ends — including on an
/// early return or a panic.
pub struct ActivityGuard {
    registry: Arc<ActivityRegistry>,
    id: ActivityId,
}

impl ActivityGuard {
    /// Register `label` on `registry` and return a guard that deregisters it.
    #[must_use]
    pub fn new(registry: &Arc<ActivityRegistry>, label: impl Into<String>) -> Self {
        let id = registry.register(label);
        Self {
            registry: Arc::clone(registry),
            id,
        }
    }

    /// The id of the activity this guard owns.
    #[must_use]
    pub fn id(&self) -> ActivityId {
        self.id
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        self.registry.finish(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_finish_round_trips() {
        let registry = ActivityRegistry::new();
        assert!(!registry.is_busy());
        let id = registry.register("Loading playlist…");
        assert!(registry.is_busy());
        assert_eq!(registry.snapshot().len(), 1);
        assert_eq!(registry.snapshot()[0].label, "Loading playlist…");
        registry.finish(id);
        assert!(!registry.is_busy());
    }

    #[test]
    fn a_guard_deregisters_on_drop() {
        let registry = ActivityRegistry::new();
        {
            let _guard = ActivityGuard::new(&registry, "Searching…");
            assert!(registry.is_busy());
        }
        assert!(!registry.is_busy());
    }

    #[test]
    fn cancel_runs_the_hook_and_removes_the_entry() {
        use std::sync::atomic::AtomicBool;

        let registry = ActivityRegistry::new();
        let fired = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&fired);
        let id = registry.register_cancellable("Loading…", move || {
            flag.store(true, Ordering::SeqCst);
        });
        assert!(registry.snapshot()[0].cancellable);
        registry.cancel(id);
        assert!(fired.load(Ordering::SeqCst));
        assert!(!registry.is_busy());
    }

    #[test]
    fn ids_are_unique() {
        let registry = ActivityRegistry::new();
        let a = registry.register("a");
        let b = registry.register("b");
        assert_ne!(a, b);
    }
}
