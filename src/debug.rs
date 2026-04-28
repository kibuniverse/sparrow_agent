use std::sync::atomic::{AtomicBool, Ordering};

static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

/// Initialize debug logging from the `SPARROW_DEBUG` environment variable.
/// Call once at program startup.
pub fn init() {
    let enabled = std::env::var("SPARROW_DEBUG")
        .map(|v| !v.is_empty() && v != "0" && v != "false")
        .unwrap_or(false);
    DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Returns whether debug logging is enabled.
pub fn is_enabled() -> bool {
    DEBUG_ENABLED.load(Ordering::Relaxed)
}

/// Print a debug log line if debug mode is enabled.
#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if $crate::debug::is_enabled() {
            eprintln!("[DEBUG] {}", format!($($arg)*));
        }
    };
}
