//! Startup progress indicator for the interactive REPL.
//!
//! Agent startup performs some variable-latency work — most notably spawning
//! and handshaking with the MCP filesystem server via `npx`, whose latency is
//! dominated by npm registry resolution and Node cold start. This module gives
//! the caller a tiny abstraction to surface a live spinner + status text while
//! that work runs, while non-interactive paths (server, sub-agents, tests) keep
//! the exact same stdout/stderr output they always had.

use std::io::IsTerminal;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// How startup progress is surfaced to the user.
pub struct StartupProgress(ProgressInner);

enum ProgressInner {
    /// Interactive TTY: an animated `indicatif` spinner. The current phase is
    /// shown as the spinner message; `detail`/`warn` lines print above it.
    Spinner(ProgressBar),
    /// Non-interactive default: reproduces the historical stdout (`detail`) /
    /// stderr (`warn`) output. `phase` is a no-op (no extra lines).
    Plain,
    /// Fully silent. Only `warn` still emits (to stderr) so real errors stay
    /// visible. Reserved for paths that must not emit startup chatter.
    Silent,
}

impl StartupProgress {
    /// Spinner on a TTY, plain text otherwise. Used by the REPL.
    pub fn auto() -> Self {
        if std::io::stdout().is_terminal() {
            let bar = ProgressBar::new_spinner();
            bar.set_draw_target(ProgressDrawTarget::stdout());
            bar.set_style(
                ProgressStyle::with_template("{spinner:.green} {msg}")
                    .expect("startup spinner template should be valid"),
            );
            bar.enable_steady_tick(Duration::from_millis(100));
            Self(ProgressInner::Spinner(bar))
        } else {
            Self(ProgressInner::Plain)
        }
    }

    /// Reproduces the historical stdout/stderr startup output. Used by every
    /// non-interactive `Agent` constructor (server, tests, sub-agents, inspect).
    pub fn plain() -> Self {
        Self(ProgressInner::Plain)
    }

    /// Silent except for warnings. Kept for callers that want no chatter.
    pub fn silent() -> Self {
        Self(ProgressInner::Silent)
    }

    /// Advance to a new startup phase. Only visible in spinner mode.
    pub fn phase(&self, message: &str) {
        if let ProgressInner::Spinner(bar) = &self.0 {
            bar.set_message(message.to_string());
        }
    }

    /// Print a detail line (roots, tool counts, modes, ...). Above the spinner
    /// in spinner mode; a plain `println!` otherwise; suppressed when silent.
    pub fn detail(&self, message: &str) {
        match &self.0 {
            ProgressInner::Spinner(bar) => bar.println(message),
            ProgressInner::Plain => println!("{message}"),
            ProgressInner::Silent => {}
        }
    }

    /// Print a warning. Always goes to stderr so failures are never swallowed;
    /// in spinner mode the spinner is briefly suspended to avoid clobbering.
    pub fn warn(&self, message: &str) {
        match &self.0 {
            ProgressInner::Spinner(bar) => bar.suspend(|| eprintln!("{message}")),
            ProgressInner::Plain | ProgressInner::Silent => eprintln!("{message}"),
        }
    }

    /// Clear the spinner line so the prompt starts on a clean line. Idempotent
    /// (a no-op for non-spinner modes and when called more than once).
    pub fn finish(&self) {
        if let ProgressInner::Spinner(bar) = &self.0 {
            bar.finish_and_clear();
        }
    }
}

impl Drop for StartupProgress {
    fn drop(&mut self) {
        self.finish();
    }
}
