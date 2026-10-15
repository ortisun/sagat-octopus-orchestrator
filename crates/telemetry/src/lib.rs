//! Observability building blocks shared by the adapters (NFR-001-OBS).
//!
//! * [`TraceContext`] parses and generates W3C `traceparent` identifiers so that every response,
//!   error envelope, log line and audit record of a request carries the same trace id.
//! * [`init_logging`] installs structured (JSON) logging.
//!
//! * [`Metrics`] owns the Prometheus recorder behind the `metrics` facade.
//!
//! OTLP/HTTP JSON export supports spans plus focused log and gauge records.

mod json_log;
mod logging;
mod metrics;
mod otlp;
mod redact;
mod span;
mod trace;

pub use json_log::{LogLayer, Style};
pub use logging::{LogFormat, Telemetry, TelemetryConfig, init, init_logging};
pub use metrics::{Metrics, WorkflowPrimitiveKind, WorkflowPrimitiveOutcome};
pub use otlp::{
    AttrValue, ExportError, GaugeData, LogData, OtlpClient, OtlpConfig, OtlpConfigError, OtlpLayer,
    SpanData, SpanEvent, SpanKind, SpanLink, SpanSink, SpanStatus, start,
};
pub use redact::{MAX_VALUE_CHARS, REDACTED, is_sensitive_name, scrub};
pub use span::{SpanId, attempt_span_id, event_span_id, keeps, run_span_id, traceparent};
pub use trace::{TraceContext, TraceId};
