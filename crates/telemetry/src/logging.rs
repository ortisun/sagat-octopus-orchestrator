//! Structured logging.
//!
//! Log lines are single-line JSON by default so they can be shipped and queried. Callers must not
//! log secrets, tokens, request bodies or personal data; the adapters in this repository log
//! identifiers (trace id, tenant id, route, status, latency) only.

use tracing_subscriber::EnvFilter;

/// Output format of the process logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// One JSON object per line.
    #[default]
    Json,
    /// Human-readable, for local development.
    Pretty,
}

/// Installs the global subscriber. The filter comes from `RUST_LOG`, defaulting to `info`.
///
/// Returns `false` if a subscriber was already installed (e.g. by a test harness); that is not an
/// error because logging then continues to work.
pub fn init_logging(format: LogFormat) -> bool {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true);
    match format {
        LogFormat::Json => builder.json().flatten_event(true).try_init().is_ok(),
        LogFormat::Pretty => builder.try_init().is_ok(),
    }
}
