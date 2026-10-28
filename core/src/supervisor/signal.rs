//! Ctrl-C / SIGTERM handling with a flag, using the C library's `signal` through a
//! plain FFI declaration (no crate). The standard library has no signal API.

use std::sync::atomic::{AtomicBool, Ordering};

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" {
    /// `sig_t signal(int, sig_t)`; the handler is passed as an address so that
    /// `SIG_DFL` (0) can be passed through the same declaration.
    fn signal(sig: i32, handler: usize) -> usize;
}

/// Restore the default SIGPIPE disposition so `psl echo | head` ends quietly instead of
/// panicking on a closed pipe (Rust ignores SIGPIPE at startup).
pub fn default_sigpipe() {
    #[cfg(unix)]
    // SAFETY: installing SIG_DFL (0) for SIGPIPE (13) has no preconditions.
    unsafe {
        signal(13, 0);
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
        signal(2, on_signal as usize);
        signal(15, on_signal as usize);
    }
}

/// True once per received signal.
pub fn take() -> bool {
    STOP_REQUESTED.swap(false, Ordering::SeqCst)
}
