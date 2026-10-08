//! Observability building blocks shared by the adapters (NFR-001-OBS).
//!
//! * [`TraceContext`] parses and generates W3C `traceparent` identifiers so that every response,
//!   error envelope, log line and audit record of a request carries the same trace id.
//! * [`init_logging`] installs structured (JSON) logging.
//!
//! OTLP export of spans and metrics is **not implemented yet** (tracked in
//! `specs/001-platform-foundation/research.md`, gap G6); the trace id is already propagated so the
//! exporter can be added without touching call sites.

mod logging;
mod trace;

pub use logging::{LogFormat, init_logging};
pub use trace::{TraceContext, TraceId};
