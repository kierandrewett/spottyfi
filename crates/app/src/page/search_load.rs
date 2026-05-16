//! The debounced, cancellable catalogue-search load.
//!
//! Search is unlike the other page loads: the query changes as the user types,
//! so a [`Loadable`](super::Loadable) — spawned once, resolved once — is the
//! wrong shape. [`SearchLoad`] instead supports being **re-dispatched**: each
//! new query aborts any in-flight request and spawns a fresh one.
//!
//! Two correctness properties matter here, both unit-tested below:
//!
//! - **Debounce.** [`Debounce`] gates dispatch: a query is only sent ~250ms
//!   after the user stops typing, not on every keystroke.
//! - **Cancellation.** Every dispatch carries a monotonic *generation*. A
//!   task writes its result into shared state only if its generation is still
//!   the latest, and the previous task's runtime handle is aborted outright.
//!   A stale, slow response can therefore never overwrite a newer query's
//!   results.
//!
//! Like the other loads, an in-flight search registers a cancellable activity
//! in the shared [`ActivityRegistry`] so the menu-bar indicator shows it.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use spottyfi_api::{ApiError, SearchType, SpotifyApi};
use spottyfi_models::SearchResults;
use spottyfi_state::ActivityRegistry;
use tokio::runtime::Handle;
use tokio::task::AbortHandle;

/// How long the user must stop typing before a query is dispatched.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// How many results of each kind a search asks Spotify for.
const RESULT_LIMIT: u32 = 20;

/// A keystroke debouncer.
///
/// The page calls [`Debounce::edited`] whenever the query text changes and
/// [`Debounce::due`] every frame; `due` returns `true` exactly once, the first
/// frame at least [`DEBOUNCE`] after the last edit.
#[derive(Debug, Default)]
pub struct Debounce {
    /// When the query was last edited, if an edit is pending dispatch.
    pending_since: Option<Instant>,
}

impl Debounce {
    /// Record that the query text changed at `now`.
    pub fn edited(&mut self, now: Instant) {
        self.pending_since = Some(now);
    }

    /// Whether a pending edit is now due for dispatch, given the current time.
    ///
    /// Returns `true` once and then clears the pending state, so the caller
    /// dispatches exactly one query per burst of typing.
    pub fn due(&mut self, now: Instant) -> bool {
        match self.pending_since {
            Some(since) if now.duration_since(since) >= DEBOUNCE => {
                self.pending_since = None;
                true
            }
            _ => false,
        }
    }

    /// Whether an edit is waiting out the debounce window.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.pending_since.is_some()
    }
}

/// The shared result slot a search task writes into and the UI reads.
struct Shared {
    /// The result of the most recently *completed* search, if any.
    result: Option<Result<SearchResults, ApiError>>,
    /// The generation of the dispatch whose result `result` holds — or whose
    /// result is awaited. A task only writes if its generation still matches.
    generation: u64,
}

/// A re-dispatchable, cancellable catalogue search.
pub struct SearchLoad {
    /// The result slot shared with the in-flight task.
    shared: Arc<Mutex<Shared>>,
    /// The abort handle of the in-flight task, if one is running.
    in_flight: Option<AbortHandle>,
    /// The query string of the most recent dispatch.
    query: String,
    /// Whether a dispatch is still awaiting its result.
    loading: bool,
}

impl Default for SearchLoad {
    fn default() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                result: None,
                generation: 0,
            })),
            in_flight: None,
            query: String::new(),
            loading: false,
        }
    }
}

/// The search types Spotify is queried for. Podcasts/shows are not in the
/// [`SearchType`] enum yet — see the Phase 6 report and `docs/questions.md`.
const SEARCH_TYPES: &[SearchType] = &[
    SearchType::Track,
    SearchType::Artist,
    SearchType::Album,
    SearchType::Playlist,
];

impl SearchLoad {
    /// Build an idle search load with no query dispatched.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Dispatch `query`, aborting any in-flight search first.
    ///
    /// The previous task is aborted and a fresh one spawned. The new task
    /// carries the next generation; only a task whose generation is still the
    /// latest writes its result, so a slow stale response is discarded.
    ///
    /// An empty / whitespace-only query clears the results without a request.
    pub fn dispatch(
        &mut self,
        query: &str,
        api: &Arc<dyn SpotifyApi>,
        runtime: &Handle,
        ctx: &egui::Context,
        activity: &Arc<ActivityRegistry>,
    ) {
        // Abort whatever is in flight; its generation is now stale anyway.
        if let Some(handle) = self.in_flight.take() {
            handle.abort();
        }
        self.query = query.to_owned();

        let trimmed = query.trim();
        if trimmed.is_empty() {
            // Clear results; bump the generation so any racing task is stale.
            if let Ok(mut shared) = self.shared.lock() {
                shared.generation += 1;
                shared.result = None;
            }
            self.loading = false;
            return;
        }

        // Bump the generation: this dispatch owns it from here on.
        let generation = {
            let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            shared.generation += 1;
            shared.generation
        };
        self.loading = true;

        let api = Arc::clone(api);
        let shared = Arc::clone(&self.shared);
        let ctx = ctx.clone();
        let query = trimmed.to_owned();

        let handle = runtime.spawn(async move {
            let result = api.search(&query, SEARCH_TYPES, RESULT_LIMIT).await;
            // Write the result only if this task is still the latest dispatch.
            if let Ok(mut shared) = shared.lock() {
                if shared.generation == generation {
                    shared.result = Some(result);
                }
            }
            ctx.request_repaint();
        });

        // Register a cancellable activity; cancelling aborts the search task.
        let abort = handle.abort_handle();
        let cancel_abort = abort.clone();
        let id = activity.register_cancellable("Searching…", move || cancel_abort.abort());
        let activity_finish = Arc::clone(activity);
        runtime.spawn(async move {
            let _ = handle.await;
            activity_finish.finish(id);
        });

        self.in_flight = Some(abort);
    }

    /// The query string of the most recent dispatch.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Whether a dispatched search is still awaiting its result.
    ///
    /// `true` between [`dispatch`](Self::dispatch) and the result landing;
    /// the UI draws a spinner while it holds.
    #[must_use]
    pub fn is_loading(&self) -> bool {
        if !self.loading {
            return false;
        }
        // The result has landed once the slot is populated for this query.
        let done = self
            .shared
            .lock()
            .map(|s| s.result.is_some())
            .unwrap_or(false);
        !done
    }

    /// Run `f` with the most recently completed search result, if any.
    ///
    /// A closure rather than a borrow because the result lives behind a
    /// `Mutex`; the lock is held only for the duration of `f`.
    pub fn with_result<R>(
        &self,
        f: impl FnOnce(Option<&Result<SearchResults, ApiError>>) -> R,
    ) -> R {
        let shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        f(shared.result.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spottyfi_api::MockSpotifyApi;
    use spottyfi_models::Page;

    /// Build a runtime, egui context and activity registry for a test.
    fn harness() -> (
        tokio::runtime::Runtime,
        egui::Context,
        Arc<ActivityRegistry>,
    ) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build runtime");
        (runtime, egui::Context::default(), ActivityRegistry::new())
    }

    /// Spin until `cond` holds or a generous timeout elapses.
    fn wait_for(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..400 {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        cond()
    }

    #[test]
    fn debounce_fires_once_after_the_window() {
        let mut debounce = Debounce::default();
        let start = Instant::now();
        debounce.edited(start);
        // Not yet due before the window elapses.
        assert!(!debounce.due(start));
        assert!(!debounce.due(start + Duration::from_millis(100)));
        assert!(debounce.is_pending());
        // Due once the window has elapsed.
        assert!(debounce.due(start + DEBOUNCE));
        // And only once — the pending edit is consumed.
        assert!(!debounce.due(start + DEBOUNCE + Duration::from_millis(50)));
        assert!(!debounce.is_pending());
    }

    #[test]
    fn debounce_resets_on_each_keystroke() {
        let mut debounce = Debounce::default();
        let start = Instant::now();
        debounce.edited(start);
        // A second edit 100ms in pushes the deadline back.
        debounce.edited(start + Duration::from_millis(100));
        assert!(!debounce.due(start + DEBOUNCE));
        // It is due 250ms after the *last* edit.
        assert!(debounce.due(start + Duration::from_millis(100) + DEBOUNCE));
    }

    #[test]
    fn an_empty_query_clears_results_without_a_request() {
        let (runtime, ctx, activity) = harness();
        // The mock has no `expect_search`: calling it would panic.
        let api: Arc<dyn SpotifyApi> = Arc::new(MockSpotifyApi::new());
        let mut load = SearchLoad::new();
        load.dispatch("   ", &api, runtime.handle(), &ctx, &activity);
        assert!(!load.is_loading());
        load.with_result(|r| assert!(r.is_none()));
    }

    #[test]
    fn a_dispatch_resolves_to_the_api_result() {
        let (runtime, ctx, activity) = harness();
        let mut mock = MockSpotifyApi::new();
        mock.expect_search().returning(|query, _, _| {
            Ok(SearchResults {
                tracks: Page {
                    items: Vec::new(),
                    total: if query == "daft punk" { 7 } else { 0 },
                    ..Page::default()
                },
                ..SearchResults::default()
            })
        });
        let api: Arc<dyn SpotifyApi> = Arc::new(mock);

        let mut load = SearchLoad::new();
        load.dispatch("daft punk", &api, runtime.handle(), &ctx, &activity);
        assert!(load.is_loading());
        assert!(wait_for(|| !load.is_loading()));
        load.with_result(|r| {
            let results = r.expect("result present").as_ref().expect("search ok");
            assert_eq!(results.tracks.total, 7);
        });
    }

    #[test]
    fn a_stale_response_never_overwrites_a_newer_query() {
        // The first query sleeps long; the second resolves immediately. The
        // slow stale response must not clobber the fast newer one.
        let (runtime, ctx, activity) = harness();
        let mut mock = MockSpotifyApi::new();
        mock.expect_search().returning(|query, _, _| {
            let slow = query == "slow";
            let total = if slow { 999 } else { 1 };
            std::thread::sleep(if slow {
                Duration::from_millis(300)
            } else {
                Duration::from_millis(0)
            });
            Ok(SearchResults {
                tracks: Page {
                    total,
                    ..Page::default()
                },
                ..SearchResults::default()
            })
        });
        let api: Arc<dyn SpotifyApi> = Arc::new(mock);

        let mut load = SearchLoad::new();
        load.dispatch("slow", &api, runtime.handle(), &ctx, &activity);
        // Immediately re-dispatch — this aborts/supersedes the slow query.
        load.dispatch("fast", &api, runtime.handle(), &ctx, &activity);

        assert!(wait_for(|| !load.is_loading()));
        load.with_result(|r| {
            let results = r.expect("result present").as_ref().expect("search ok");
            assert_eq!(results.tracks.total, 1, "newer query's result wins");
        });
        // Give the slow task time to (try to) finish; it must still not win.
        std::thread::sleep(Duration::from_millis(350));
        load.with_result(|r| {
            let results = r.expect("result present").as_ref().expect("search ok");
            assert_eq!(results.tracks.total, 1, "stale result was discarded");
        });
    }
}
