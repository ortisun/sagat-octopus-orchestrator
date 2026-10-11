//! Observability building blocks shared by the adapters (NFR-001-OBS).
//!
//! * [`TraceContext`] parses and generates W3C `traceparent` identifiers so that every response,
//!   error envelope, log line and audit record of a request carries the same trace id.
//! * [`init_logging`] installs structured (JSON) logging.
//!
//! * [`Metrics`] owns the Prometheus recorder behind the `metrics` facade.
//!
//! OTLP export of *spans* is not implemented yet (feature 008, gap G6); the trace id is already
//! propagated so the exporter can be added without touching call sites.

mod logging;
mod metrics;
mod trace;

pub use logging::{LogFormat, init_logging};
pub use metrics::Metrics;
pub use trace::{TraceContext, TraceId};
