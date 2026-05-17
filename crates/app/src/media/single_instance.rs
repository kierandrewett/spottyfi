//! Single-instance enforcement.
//!
//! A second `spottyfi` launch should not start a new process — it should bring
//! the already-running window to the front. This module:
//!
//! 1. Holds a process-wide lock via [`single-instance`](single_instance) for
//!    the lifetime of the [`InstanceGuard`].
//! 2. When the lock is already held, asks the running instance to raise its
//!    window over D-Bus — the MPRIS `Raise` method Spottyfi already publishes
//!    (see [`super::mpris`]) — then signals the caller to exit.
//!
//! Using MPRIS `Raise` for the "focus the running window" signal means no
//! extra IPC socket: the running instance's MPRIS server is the channel.

/// The lock name — also the D-Bus path component, kept in one place.
const LOCK_NAME: &str = "dev.drewett.spottyfi.instance";

/// The running instance's MPRIS well-known bus name.
const MPRIS_BUS: &str = "org.mpris.MediaPlayer2.spottyfi";

/// The standard MPRIS object path and root interface for the `Raise` call.
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
/// The MPRIS root interface that carries `Raise`.
const MPRIS_IFACE: &str = "org.mpris.MediaPlayer2";

/// The outcome of the single-instance check.
pub enum InstanceCheck {
    /// This is the only / first instance — carries the lock guard, which must
    /// be held for the process lifetime.
    Primary(InstanceGuard),
    /// Another instance is already running; it has been asked to raise its
    /// window. The caller should exit immediately.
    AlreadyRunning,
}

/// Holds the process-wide single-instance lock.
///
/// The lock is released when this guard is dropped — so it must live as long
/// as the application does. `app` keeps it in [`SpottyfiApp`](crate::app::
/// SpottyfiApp). `None` when the lock backend was unavailable and the app is
/// running unprotected.
pub struct InstanceGuard {
    /// The underlying OS lock. Never read — held purely for its `Drop`.
    _inner: Option<single_instance::SingleInstance>,
}

/// Run the single-instance check at startup.
///
/// Returns [`InstanceCheck::Primary`] (with the guard to keep) when this is
/// the first launch, or [`InstanceCheck::AlreadyRunning`] when another
/// instance holds the lock — in which case the running instance has been
/// pinged to raise its window and `main` should exit.
///
/// Fails open: if the lock cannot be created at all (an unsupported platform
/// or a transient OS error), this instance is treated as primary so the app
/// still launches.
pub fn check() -> InstanceCheck {
    let instance = match single_instance::SingleInstance::new(LOCK_NAME) {
        Ok(instance) => instance,
        Err(err) => {
            // The lock backend is unavailable — run unprotected rather than
            // refusing to launch.
            tracing::warn!(%err, "single-instance lock unavailable; launching anyway");
            return InstanceCheck::Primary(InstanceGuard { _inner: None });
        }
    };

    if instance.is_single() {
        InstanceCheck::Primary(InstanceGuard {
            _inner: Some(instance),
        })
    } else {
        tracing::info!("another Spottyfi instance is running; raising its window");
        raise_running_instance();
        InstanceCheck::AlreadyRunning
    }
}

/// Ask the already-running instance to raise its window via MPRIS `Raise`.
///
/// Best-effort and synchronous (this runs before the tokio runtime exists):
/// a blocking `zbus` connection makes one method call. Any failure is logged
/// — the second instance exits regardless.
fn raise_running_instance() {
    // A short-lived blocking zbus connection — `mpris-server` re-exports zbus,
    // so the dependency is already in the tree.
    let result = (|| -> Result<(), mpris_server::zbus::Error> {
        let connection = mpris_server::zbus::blocking::Connection::session()?;
        let proxy = mpris_server::zbus::blocking::Proxy::new(
            &connection,
            MPRIS_BUS,
            MPRIS_PATH,
            MPRIS_IFACE,
        )?;
        proxy.call_method("Raise", &())?;
        Ok(())
    })();
    match result {
        Ok(()) => tracing::debug!("asked the running instance to raise its window"),
        Err(err) => {
            // The running instance may not have claimed the MPRIS name yet
            // (it is published a moment after startup); nothing more to do.
            tracing::info!(%err, "could not signal the running instance");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_name_is_namespaced() {
        // A regression guard: the lock name must stay reverse-DNS namespaced
        // so it cannot collide with another app's lock.
        assert!(LOCK_NAME.starts_with("dev.drewett.spottyfi"));
    }

    #[test]
    fn mpris_constants_are_the_standard_paths() {
        assert_eq!(MPRIS_PATH, "/org/mpris/MediaPlayer2");
        assert_eq!(MPRIS_IFACE, "org.mpris.MediaPlayer2");
    }
}
