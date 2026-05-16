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
pub struct Loadable<T> {
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
    #[must_use]
    pub fn value(&self) -> Option<&T> {
        self.promise.ready()
    }

    /// Whether the load is still in flight.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.promise.ready().is_none()
    }
}
