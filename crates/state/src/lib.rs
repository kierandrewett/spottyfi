//! Application state, the `Action`/`Event` bus and the dispatcher.
//!
//! `state` owns the redux-style store so that `ui` stays a pure projection and
//! carries no business logic. See `docs/architecture.md`.
//!
//! It also hosts the [`activity`] registry: a small, `Arc`-shared snapshot of
//! the background work in flight, which the menu bar surfaces as a
//! VSCode-style activity indicator.
#![warn(missing_docs)]

pub mod activity;

pub use activity::{Activity, ActivityGuard, ActivityId, ActivityRegistry};
