//! Ctrl-C / SIGTERM handling with a flag, using the C library's `signal` through a
//! plain FFI declaration (no crate). The standard library has no signal API.

use std::sync::atomic::{AtomicBool, Ordering};

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

/// A C signal handler, or `None` for `SIG_DFL` (the null pointer). `Option` of a
/// function pointer has the same representation as a nullable C function pointer.
#[cfg(unix)]
type Handler = Option<extern "C" fn(i32)>;

#[cfg(unix)]
extern "C" {
    /// `sig_t signal(int, sig_t)`.
    fn signal(sig: i32, handler: Handler) -> Handler;
}

/// Restore the default SIGPIPE disposition so `psl echo | head` ends quietly instead of
/// panicking on a closed pipe (Rust ignores SIGPIPE at startup).
pub fn default_sigpipe() {
    #[cfg(unix)]
    // SAFETY: installing SIG_DFL (0) for SIGPIPE (13) has no preconditions.
    unsafe {
        signal(13, None);
    }
}

#[cfg(unix)]
extern "C" fn on_signal(_sig: i32) {
    STOP_REQUESTED.store(true, Ordering::SeqCst);
}

/// Install handlers for SIGINT (2) and SIGTERM (15). Safe to call more than once.
pub fn install() {
    #[cfg(unix)]
    // SAFETY: `signal` is async-signal-safe to install, and the handler only touches an atomic.
    unsafe {
        signal(2, Some(on_signal));
        signal(15, Some(on_signal));
    }
}

/// True once per received signal.
pub fn take() -> bool {
    STOP_REQUESTED.swap(false, Ordering::SeqCst)
}
