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
}

/// A paginated load that streams its items into shared state incrementally.
pub struct IncrementalLoad<T: Send + 'static> {
    /// The shared state the runtime task appends to and the UI reads.
    shared: Arc<Mutex<Shared<T>>>,
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

        let registry_finish = Arc::clone(registry);

        let handle = runtime.spawn(async move {
            futures::pin_mut!(stream);
            let mut outcome = Ok(());
            // Repaint once per page (every STREAM_PAGE_SIZE items) rather than
            // per item, so a fast stream does not flood the UI with wakeups.
            let mut since_repaint = 0usize;
            const REPAINT_EVERY: usize = 50;

            while let Some(item) = stream.next().await {
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
            if let Ok(mut shared) = task_shared.lock() {
                shared.outcome = Some(outcome);
            }
            ctx.request_repaint();
        });

        // Register a cancellable activity; cancelling aborts the stream task.
        let abort = handle.abort_handle();
        let id = registry.register_cancellable(label, move || abort.abort());
        runtime.spawn(async move {
            let _ = handle.await;
            registry_finish.finish(id);
        });

        Self { shared }
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
        };
        f(snapshot)
    }

    /// The number of items streamed in so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.with(|s| s.items.len())
    }

    /// Whether the load has fully completed.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.with(|s| s.done)
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
            if load.is_done() {
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
            if load.is_done() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        load.with(|s| {
            assert_eq!(s.items, &[1, 2]);
            assert!(s.error.is_some());
        });
    }
}
