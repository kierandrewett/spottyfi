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
//! load is in flight, with a cancel affordance that aborts the runtime task.

use std::future::Future;
use std::sync::Arc;

use poll_promise::Promise;
use spottyfi_state::ActivityRegistry;
use tokio::runtime::Handle;

/// A one-shot async load whose result the UI reads each frame.
///
/// `T` is typically a `Result<…, ApiError>`, so a page can render either the
/// data or an error once the load resolves.
pub struct Loadable<T: Send + 'static> {
    /// The underlying promise. `poll-promise` caches the resolved value, so
    /// [`Promise::ready`] keeps returning it once available.
    promise: Promise<T>,
}

impl<T: Send + 'static> Loadable<T> {
    /// Spawn `future`, registering it in `registry` under `label` so the
    /// menu-bar activity indicator shows it while it runs.
    ///
    /// The activity is registered as cancellable: the cancel affordance aborts
    /// the spawned runtime task. The activity is deregistered when the future
    /// resolves (or when the task is aborted).
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

        let handle = runtime.spawn(async move {
            let value = future.await;
            sender.send(value);
            ctx.request_repaint();
        });

        // Register the activity with a cancel hook that aborts the task. The
        // abort handle is cheap to clone and tripping it is a no-op once the
        // task has already finished.
        let abort = handle.abort_handle();
        let registry_for_cancel = Arc::clone(registry);
        let id = registry.register_cancellable(label, move || abort.abort());

        // Deregister the activity once the task completes, on its own runtime
        // task so neither the load future nor the UI thread is blocked.
        let registry_for_finish = Arc::clone(registry);
        runtime.spawn(async move {
            // Awaiting the join handle resolves both on success and on abort.
            let _ = handle.await;
            registry_for_finish.finish(id);
        });
        // `registry_for_cancel` is moved into the cancel closure above; this
        // keeps the registry alive for the closure's lifetime.
        let _ = &registry_for_cancel;

        Self { promise }
    }

    /// The loaded value, or `None` while the load is still in flight.
    ///
    /// `poll-promise` caches the resolved value, so once this returns `Some`
    /// it keeps doing so for the lifetime of the [`Loadable`].
    #[must_use]
    pub fn value(&self) -> Option<&T> {
        self.promise.ready()
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
    }
}
