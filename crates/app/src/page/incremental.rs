//! An incrementally-streamed async load bridged onto the egui UI thread.
//!
//! [`Loadable`](super::Loadable) resolves once, all-or-nothing — fine for a
//! single fetch, but wrong for a paginated collection: a 500-track playlist
//! would block first paint on five sequential round-trips.
//!
//! [`IncrementalLoad`] instead consumes an [`ItemStream`] and pushes each item
//! into shared state *as it arrives*. The page reads a snapshot every frame,
//! so the first page of tracks renders the instant it lands and the rest
//! stream in underneath. The UI thread never blocks; it only reads a `Vec`.
//!
//! Like [`Loadable`](super::Loadable) it registers a cancellable activity in
//! the shared [`ActivityRegistry`] so the menu-bar indicator reflects the load.
//!
//! ## Cancellation
//!
//! Like [`Loadable`](super::Loadable), cancelling trips a shared
//! [`AtomicBool`] rather than aborting the task. The streaming task observes
//! the flag between items and stops; the snapshot exposes `cancelled` so the
//! page can render a calm cancelled state instead of an endless spinner.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures::stream::Stream;
use futures::StreamExt as _;
use spottyfi_api::ApiError;
use spottyfi_state::ActivityRegistry;
use tokio::runtime::Handle;

/// The shared, growing state of an in-flight incremental load.
struct Shared<T> {
    /// Every item streamed in so far, in arrival order.
    items: Vec<T>,
    /// Set once the stream ends: `Ok` on clean completion, `Err` if a page
    /// request failed (the partial `items` are still kept and shown).
    outcome: Option<Result<(), ApiError>>,
}

impl<T> Default for Shared<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            outcome: None,
        }
    }
}

/// A snapshot of an incremental load, taken once per frame by the UI.
pub struct LoadSnapshot<'a, T> {
    /// Every item streamed in so far.
    pub items: &'a [T],
    /// `true` once the stream has fully completed (success or error).
    pub done: bool,
    /// `Some(err)` if the stream ended on a page error.
    pub error: Option<&'a ApiError>,
    /// `true` once the user has cancelled the load. The streaming task stops
    /// promptly; `items` holds whatever arrived before the cancellation.
    pub cancelled: bool,
}

/// A paginated load that streams its items into shared state incrementally.
pub struct IncrementalLoad<T: Send + 'static> {
    /// The shared state the runtime task appends to and the UI reads.
    shared: Arc<Mutex<Shared<T>>>,
    /// Tripped by the activity-cancel hook; observed by the streaming task.
    cancelled: Arc<AtomicBool>,
}

impl<T: Send + 'static> IncrementalLoad<T> {
    /// Spawn `stream` onto `runtime`, streaming each item into shared state.
    ///
    /// `ctx` is repainted as items arrive (throttled to a repaint per page,
    /// not per item) so the page re-renders incrementally. The load registers
    /// a cancellable activity in `registry` under `label`.
    pub fn spawn<S>(
        runtime: &Handle,
        ctx: &egui::Context,
        registry: &Arc<ActivityRegistry>,
        label: impl Into<String>,
        stream: S,
    ) -> Self
    where
        S: Stream<Item = Result<T, ApiError>> + Send + 'static,
    {
        let shared: Arc<Mutex<Shared<T>>> = Arc::new(Mutex::new(Shared::default()));
        let task_shared = Arc::clone(&shared);
        let ctx = ctx.clone();
        let cancelled = Arc::new(AtomicBool::new(false));

        let registry_finish = Arc::clone(registry);

        // The task observes the cancel flag between items and stops cleanly
        // rather than being aborted, so it never leaves shared state torn.
        let task_cancelled = Arc::clone(&cancelled);
        let handle = runtime.spawn(async move {
            futures::pin_mut!(stream);
            let mut outcome = Ok(());
            // Repaint once per page (every STREAM_PAGE_SIZE items) rather than
            // per item, so a fast stream does not flood the UI with wakeups.
            let mut since_repaint = 0usize;
            const REPAINT_EVERY: usize = 50;

            loop {
                if task_cancelled.load(Ordering::SeqCst) {
                    // Cancelled: leave `outcome` unset so the snapshot reports
                    // neither done nor errored, just cancelled.
                    ctx.request_repaint();
                    return;
                }
                tokio::select! {
                    item = stream.next() => {
                        let Some(item) = item else { break };
                        match item {
                            Ok(value) => {
                                if let Ok(mut shared) = task_shared.lock() {
                                    shared.items.push(value);
                                }
                                since_repaint += 1;
                                if since_repaint >= REPAINT_EVERY {
                                    since_repaint = 0;
                                    ctx.request_repaint();
                                }
                            }
                            Err(err) => {
                                outcome = Err(err);
                                break;
                            }
                        }
                    }
                    () = wait_for_flag(&task_cancelled) => {
                        ctx.request_repaint();
                        return;
                    }
                }
            }
            if let Ok(mut shared) = task_shared.lock() {
                shared.outcome = Some(outcome);
            }
            ctx.request_repaint();
        });

        // Register a cancellable activity; cancelling trips the flag so the
        // streaming task stops without an abort.
        let hook_cancelled = Arc::clone(&cancelled);
        let id = registry.register_cancellable(label, move || {
            hook_cancelled.store(true, Ordering::SeqCst);
        });
        runtime.spawn(async move {
            let _ = handle.await;
            registry_finish.finish(id);
        });

        Self { shared, cancelled }
    }

    /// Run `f` with a snapshot of the load's current state.
    ///
    /// A closure rather than a returned borrow because the items live behind a
    /// `Mutex`; the lock is held only for the duration of `f`.
    pub fn with<R>(&self, f: impl FnOnce(LoadSnapshot<'_, T>) -> R) -> R {
        let shared = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let snapshot = LoadSnapshot {
            items: &shared.items,
            done: shared.outcome.is_some(),
            error: shared.outcome.as_ref().and_then(|o| o.as_ref().err()),
            cancelled: self.cancelled.load(Ordering::SeqCst),
        };
        f(snapshot)
    }

    /// The number of items streamed in so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.with(|s| s.items.len())
    }
}

/// Resolve once `flag` becomes `true`, polling it on a short interval.
async fn wait_for_flag(flag: &AtomicBool) {
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_stream_in_and_the_load_completes() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build runtime");
        let ctx = egui::Context::default();
        let registry = ActivityRegistry::new();

        let stream = futures::stream::iter(vec![Ok(1), Ok(2), Ok(3)]);
        let load: IncrementalLoad<i32> =
            IncrementalLoad::spawn(runtime.handle(), &ctx, &registry, "Streaming…", stream);

        for _ in 0..200 {
            if load.with(|s| s.done) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        load.with(|s| {
            assert_eq!(s.items, &[1, 2, 3]);
            assert!(s.done);
            assert!(s.error.is_none());
        });
        assert!(!registry.is_busy());
    }

    #[test]
    fn a_stream_error_keeps_partial_items() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build runtime");
        let ctx = egui::Context::default();
        let registry = ActivityRegistry::new();

        let stream = futures::stream::iter(vec![
            Ok(1),
            Ok(2),
            Err(ApiError::NotFound("boom".to_owned())),
        ]);
        let load: IncrementalLoad<i32> =
            IncrementalLoad::spawn(runtime.handle(), &ctx, &registry, "Streaming…", stream);

        for _ in 0..200 {
            if load.with(|s| s.done) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        load.with(|s| {
            assert_eq!(s.items, &[1, 2]);
            assert!(s.error.is_some());
        });
    }

    #[test]
    fn cancelling_reports_cancelled_and_stops_the_stream() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build runtime");
        let ctx = egui::Context::default();
        let registry = ActivityRegistry::new();

        // A stream that never ends on its own — only cancellation stops it.
        let stream = futures::stream::pending::<Result<i32, ApiError>>();
        let load: IncrementalLoad<i32> =
            IncrementalLoad::spawn(runtime.handle(), &ctx, &registry, "Streaming…", stream);
        assert!(!load.with(|s| s.cancelled));

        let id = registry.snapshot()[0].id;
        registry.cancel(id);

        assert!(load.with(|s| s.cancelled));
        load.with(|s| {
            assert!(s.cancelled, "snapshot reports cancelled");
            assert!(!s.done, "a cancelled load is not 'done'");
        });

        // The streaming task winds down and the activity deregisters.
        for _ in 0..200 {
            if !registry.is_busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!registry.is_busy());
        assert!(load.with(|s| s.cancelled));
    }
}
