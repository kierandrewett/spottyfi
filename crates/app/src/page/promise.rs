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

use std::future::Future;

use poll_promise::Promise;
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
    /// Spawn `future` onto `runtime` and return a pending [`Loadable`].
    ///
    /// `ctx` is woken with `request_repaint` when the future resolves so the
    /// page re-renders with the loaded data.
    pub fn spawn<F>(runtime: &Handle, ctx: &egui::Context, future: F) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        let ctx = ctx.clone();
        let (sender, promise) = Promise::new();
        runtime.spawn(async move {
            let value = future.await;
            sender.send(value);
            ctx.request_repaint();
        });
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
    fn a_spawned_load_resolves_to_its_value() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build runtime");
        let ctx = egui::Context::default();
        let loadable: Loadable<i32> = Loadable::spawn(runtime.handle(), &ctx, async { 7 });

        // Block until the runtime task has produced the value.
        for _ in 0..200 {
            if loadable.value().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(loadable.value(), Some(&7));
    }
}
