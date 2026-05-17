//! A one-shot async-fetch wrapper bridged onto the egui UI thread.
//!
//! Every page loads its data with a single fetch — "give me this playlist",
//! "give me the saved tracks". [`Loadable`] wraps that fetch: the page spawns
//! it onto the tokio runtime, draws a spinner while it is pending, and reads
//! the value once it is ready. This is the `Promise<T>` pattern from
//! `docs/threading.md`, built on the `poll-promise` crate.
//!
//! The runtime task calls `egui::Context::request_repaint` on completion so
//! the UI wakes to render the result without polling every frame.
//!
//! ## Activity registration
//!
//! [`Loadable::spawn_tracked`] registers the load in the shared
//! [`ActivityRegistry`] so the menu-bar indicator can show that a background
//! load is in flight, with a cancel affordance.
//!
//! ## Cancellation
//!
//! Cancelling a load must **never** abort the runtime task that owns the
//! `poll_promise::Promise`'s `Sender`: aborting drops the `Sender` unsent and
//! the next poll of the `Promise` panics (`The Promise Sender was dropped`).
//!
//! Instead the cancel hook trips a shared [`AtomicBool`] flag. The spawned task
//! observes the flag and stops early, and — crucially — [`Loadable::state`]
//! checks the flag *first*: once tripped it reports [`LoadState::Cancelled`]
//! and the `Promise` is never polled again.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use poll_promise::Promise;
use spottyfi_state::ActivityRegistry;
use tokio::runtime::Handle;

/// The observable state of a [`Loadable`] this frame.
pub enum LoadState<'a, T> {
    /// The load is still in flight.
    Pending,
    /// The load resolved; here is its value.
    Ready(&'a T),
    /// The user cancelled the load before it resolved.
    Cancelled,
}

/// A one-shot async load whose result the UI reads each frame.
///
/// `T` is typically a `Result<…, ApiError>`, so a page can render either the
/// data or an error once the load resolves.
pub struct Loadable<T: Send + 'static> {
    /// The underlying promise. `poll-promise` caches the resolved value, so
    /// [`Promise::ready`] keeps returning it once available.
    ///
    /// Never poll this once `cancelled` is set: a cancelled load's producing
    /// task may have stopped without sending, and polling would panic.
    promise: Promise<T>,
    /// Tripped by the activity-cancel hook. Once set, [`Loadable::state`]
    /// reports [`LoadState::Cancelled`] and never touches `promise` again.
    cancelled: Arc<AtomicBool>,
}

impl<T: Send + 'static> Loadable<T> {
    /// Spawn `future`, registering it in `registry` under `label` so the
    /// menu-bar activity indicator shows it while it runs.
    ///
    /// The activity is registered as cancellable: the cancel affordance trips
    /// a shared flag rather than aborting the task, so the `Promise`'s `Sender`
    /// is never dropped unsent. The activity is deregistered when the future
    /// resolves (or when the load is cancelled).
    pub fn spawn_tracked<F>(
        runtime: &Handle,
        ctx: &egui::Context,
        registry: &Arc<ActivityRegistry>,
        label: impl Into<String>,
        future: F,
    ) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        let ctx = ctx.clone();
        let (sender, promise) = Promise::new();
        let cancelled = Arc::new(AtomicBool::new(false));

        // The task races the future against the cancel flag. On cancellation
        // it returns without sending — that is safe because the owning
        // `Loadable` will never poll the promise once the flag is set.
        let task_cancelled = Arc::clone(&cancelled);
        let handle = runtime.spawn(async move {
            tokio::select! {
                value = future => {
                    sender.send(value);
                    ctx.request_repaint();
                }
                () = wait_for_flag(&task_cancelled) => {
                    // Cancelled: drop `sender` unsent. The UI never polls it.
                }
            }
        });

        // Register the activity with a cancel hook that trips the flag (and
        // wakes the task by repainting). Tripping the flag is idempotent and a
        // no-op once the task has already finished.
        let hook_cancelled = Arc::clone(&cancelled);
        let id = registry.register_cancellable(label, move || {
            hook_cancelled.store(true, Ordering::SeqCst);
        });

        // Deregister the activity once the task completes, on its own runtime
        // task so neither the load future nor the UI thread is blocked.
        let registry_for_finish = Arc::clone(registry);
        runtime.spawn(async move {
            let _ = handle.await;
            registry_for_finish.finish(id);
        });

        Self { promise, cancelled }
    }

    /// Whether the load has been cancelled by the user.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// This frame's [`LoadState`].
    ///
    /// Checks the cancel flag **before** the promise: a cancelled load's
    /// producing task may have stopped without sending a value, so its
    /// `Promise` must never be polled again.
    #[must_use]
    pub fn state(&self) -> LoadState<'_, T> {
        if self.is_cancelled() {
            return LoadState::Cancelled;
        }
        match self.promise.ready() {
            Some(value) => LoadState::Ready(value),
            None => LoadState::Pending,
        }
    }

    /// The loaded value, or `None` while the load is still in flight **or**
    /// once it has been cancelled.
    ///
    /// Prefer [`Loadable::state`] when the caller needs to distinguish a
    /// pending load from a cancelled one.
    #[must_use]
    pub fn value(&self) -> Option<&T> {
        match self.state() {
            LoadState::Ready(value) => Some(value),
            LoadState::Pending | LoadState::Cancelled => None,
        }
    }
}

/// Resolve once `flag` becomes `true`, polling it on a short interval.
///
/// A poll loop rather than a `Notify` keeps the cancel hook a plain
/// `FnOnce()`; the flag is checked a few times a second, never on a hot path.
async fn wait_for_flag(flag: &AtomicBool) {
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tracked_load_registers_and_deregisters() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build runtime");
        let ctx = egui::Context::default();
        let registry = ActivityRegistry::new();

        let loadable: Loadable<i32> =
            Loadable::spawn_tracked(runtime.handle(), &ctx, &registry, "Loading test…", async {
                42
            });

        for _ in 0..200 {
            if loadable.value().is_some() && !registry.is_busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(loadable.value(), Some(&42));
        assert!(!registry.is_busy(), "activity deregistered after load");
        assert!(matches!(loadable.state(), LoadState::Ready(&42)));
    }

    #[test]
    fn a_cancelled_load_reports_cancelled_and_never_polls_the_promise() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build runtime");
        let ctx = egui::Context::default();
        let registry = ActivityRegistry::new();

        // A load that never resolves on its own — only cancellation ends it.
        let loadable: Loadable<i32> = Loadable::spawn_tracked(
            runtime.handle(),
            &ctx,
            &registry,
            "Loading forever…",
            async { std::future::pending::<i32>().await },
        );
        assert!(matches!(loadable.state(), LoadState::Pending));

        // Cancel via the registry, exactly as the top-bar indicator does.
        let id = registry.snapshot()[0].id;
        registry.cancel(id);

        assert!(loadable.is_cancelled());
        // `state()` must report `Cancelled` and must not poll the promise —
        // the producing task stopped without sending, so a poll would panic.
        assert!(matches!(loadable.state(), LoadState::Cancelled));
        assert_eq!(loadable.value(), None);

        // Even after the task has fully wound down and deregistered, polling
        // stays safe because the flag short-circuits before the promise.
        for _ in 0..200 {
            if !registry.is_busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(matches!(loadable.state(), LoadState::Cancelled));
    }
}
