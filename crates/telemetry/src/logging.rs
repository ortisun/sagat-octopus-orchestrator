//! Structured logging and the span exporter, installed together.
//!
//! Log lines are single-line JSON by default so they can be shipped and queried, and every value on
//! them goes through the redaction rules of [`crate::redact`]. Callers still must not log secrets,
//! tokens, request bodies or personal data: the adapters in this repository log identifiers (trace
//! id, tenant id, route, status, latency) only, and the redaction is the net under that rule.

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::json_log::{LogLayer, Style};
use crate::otlp::{OtlpClient, OtlpConfig, OtlpLayer, SpanSink, start};

/// Output format of the process logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// One JSON object per line.
    #[default]
    Json,
    /// Human-readable, for local development.
    Pretty,
}

/// What a process installs at start.
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub format: LogFormat,
    /// Written on every log line and on the exported spans' resource.
    pub service: String,
    /// Where spans are exported; none means they are not.
    pub otlp: Option<OtlpConfig>,
}

impl TelemetryConfig {
    /// Logs only, under the name `service`.
    #[must_use]
    pub fn logging(format: LogFormat, service: impl Into<String>) -> Self {
        Self {
            format,
            service: service.into(),
            otlp: None,
        }
    }
}

/// What [`init`] installed. Keep it until the process ends and call [`Telemetry::shutdown`] so the
/// spans still queued are sent.
#[derive(Clone, Default)]
pub struct Telemetry {
    sink: Option<SpanSink>,
    installed: bool,
}

impl Telemetry {
    /// Whether this call installed the subscriber (a harness may have installed one already).
    #[must_use]
    pub const fn installed(&self) -> bool {
        self.installed
    }

    /// Whether spans are being exported.
    #[must_use]
    pub const fn exporting(&self) -> bool {
        self.sink.is_some()
    }

    /// Sends what is queued, waiting at most `deadline`.
    pub async fn shutdown(&self, deadline: std::time::Duration) {
        if let Some(sink) = &self.sink {
            sink.shutdown(deadline).await;
        }
    }
}

/// Installs the global subscriber: the log layer (filter from `RUST_LOG`, default `info`) and, when
/// configured, the span exporter. Exporting needs a Tokio runtime; without one the logs still work
/// and `exporting()` says no.
///
/// Returns a [`Telemetry`] whose `installed()` is false if a subscriber was already there (a test
/// harness); that is not an error because logging then continues to work.
pub fn init(config: TelemetryConfig) -> Telemetry {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let style = match config.format {
        LogFormat::Json => Style::Json,
        LogFormat::Pretty => Style::Text,
    };
    let log = LogLayer::new(std::io::stdout, config.service, style).with_filter(filter);

    let exporter = config
        .otlp
        .filter(|_| tokio::runtime::Handle::try_current().is_ok())
        .and_then(|otlp| {
            let ratio = otlp.sample_ratio;
            OtlpClient::new(otlp)
                .ok()
                .map(|client| (start(client), ratio))
        });
    let sink = exporter.as_ref().map(|(sink, _)| sink.clone());
    let layer = exporter.map(|(sink, ratio)| OtlpLayer::new(sink, ratio));

    let installed = tracing_subscriber::registry()
        .with(log)
        .with(layer)
        .try_init()
        .is_ok();
    Telemetry { sink, installed }
}

/// Installs logging alone, as a process with no exporter does.
///
/// Returns `false` if a subscriber was already installed.
pub fn init_logging(format: LogFormat) -> bool {
    init(TelemetryConfig::logging(format, "sagat")).installed()
}
