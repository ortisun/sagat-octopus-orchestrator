//! Explicit onboarding telemetry for the short-lived `sagat` process.
//!
//! No telemetry record is emitted until an operator configures the standard OTLP endpoint or a
//! local NDJSON report. Values are deliberately from closed sets: arguments, paths, URLs, tokens
//! and payloads never become attributes or metric labels.

use std::future::Future;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sagat_telemetry::{
    AttrValue, GaugeData, LogData, OtlpClient, OtlpConfig, OtlpLayer, SpanId, SpanSink, TraceId,
    keeps, start, traceparent,
};
use tracing::Instrument;
use tracing_subscriber::layer::SubscriberExt;

use crate::args::Command;
use crate::{
    EXIT_BREAKING, EXIT_DIFFERENCES, EXIT_DONE, EXIT_FAILED, EXIT_PENDING, EXIT_TIMEOUT, EXIT_USAGE,
};

const LOCAL_REPORT: &str = "SAGAT_ONBOARDING_TELEMETRY_FILE";
const FLUSH_DEADLINE: Duration = Duration::from_secs(2);

tokio::task_local! {
    static TRACEPARENT: String;
}

/// The command's W3C context while it calls REST or gRPC adapters.
#[must_use]
pub(crate) fn current_traceparent() -> Option<String> {
    TRACEPARENT.try_with(Clone::clone).ok()
}

pub(crate) async fn under_traceparent<F>(traceparent: String, future: F) -> F::Output
where
    F: Future,
{
    TRACEPARENT.scope(traceparent, future).await
}

/// One explicitly instrumented CLI invocation.
pub struct Onboarding {
    command: &'static str,
    stage: &'static str,
    tenant_context: &'static str,
    trace_id: TraceId,
    span_id: SpanId,
    traceparent: String,
    span: tracing::Span,
    sink: Option<SpanSink>,
    exporter: Option<OtlpClient>,
    report: Option<PathBuf>,
    started: Instant,
}

impl Onboarding {
    /// Reads opt-in telemetry settings from the process. No setting means no exported telemetry;
    /// the correlation header used by platform requests remains available for failed commands.
    ///
    /// # Errors
    /// A configured exporter whose settings or destination are unsafe.
    pub fn from_process(args: &[String]) -> Result<Self, String> {
        let lookup = |name: &str| std::env::var(name).ok();
        let report = lookup(LOCAL_REPORT)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let mut otlp =
            OtlpConfig::from_lookup(&lookup, "sagat-cli").map_err(|error| error.to_string())?;
        if let Some(config) = &mut otlp {
            if std::iter::once(config.endpoint.as_str())
                .chain(config.logs_endpoint.as_deref())
                .chain(config.metrics_endpoint.as_deref())
                .any(|endpoint| !crate::client::is_safe_address(endpoint))
            {
                return Err(
                    "the OTLP endpoint must use https (plain http is allowed only on loopback)"
                        .to_owned(),
                );
            }
            // A command must finish promptly and flush before process exit. An unavailable
            // opt-in collector cannot hold a developer command through the server defaults.
            config.batch_size = 1;
            config.flush_interval = Duration::from_millis(25);
            config.timeout = Duration::from_secs(1);
            config.retries = 0;
        }
        let trace_id = TraceId::generate();
        let span_id = SpanId::generate();
        let sampled = match (&otlp, &report) {
            (Some(config), _) => keeps(trace_id, config.sample_ratio),
            (None, Some(_)) => true,
            (None, None) => false,
        };
        let (sink, exporter) = if let Some(config) = otlp {
            let ratio = config.sample_ratio;
            let exporter = OtlpClient::new(config).map_err(|error| error.to_string())?;
            let sink = start(exporter.clone());
            let subscriber =
                tracing_subscriber::registry().with(OtlpLayer::new(sink.clone(), ratio));
            tracing::subscriber::set_global_default(subscriber)
                .map_err(|_| "the telemetry subscriber is already installed".to_owned())?;
            (Some(sink), Some(exporter))
        } else {
            (None, None)
        };

        let (command, stage, tenant_context) = dimensions(args);
        let span = tracing::info_span!(
            target: "sagat_cli",
            "sagat.cli.command",
            trace_id = %trace_id,
            span_id = %span_id,
            otel.kind = "client",
            otel.sampled = sampled,
            command,
            stage,
            tenant.context = tenant_context,
            outcome = tracing::field::Empty,
            duration_ms = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
        );
        Ok(Self {
            command,
            stage,
            tenant_context,
            trace_id,
            span_id,
            traceparent: traceparent(trace_id, span_id, sampled),
            span,
            sink,
            exporter,
            report,
            started: Instant::now(),
        })
    }

    /// Runs `future` under the command span and makes its trace context available to the clients.
    pub async fn observe<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        under_traceparent(
            self.traceparent.clone(),
            future.instrument(self.span.clone()),
        )
        .await
    }

    /// The stable trace identifier shown with an instrumented failed command.
    #[must_use]
    pub fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    /// Records the bounded result, writes the local report if configured, then waits briefly for
    /// the command span to leave the exporter queue.
    pub async fn finish(self, code: u8) -> Result<(), &'static str> {
        let elapsed = self.started.elapsed();
        let duration_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        let outcome = outcome(code);
        self.span.record("outcome", outcome);
        self.span.record("duration_ms", duration_ms);
        if code != EXIT_DONE {
            self.span.record("otel.status_code", "error");
        }

        let completion = Completion {
            trace_id: self.trace_id,
            span_id: self.span_id,
            command: self.command,
            stage: self.stage,
            tenant_context: self.tenant_context,
            outcome,
            duration_ms,
            duration_seconds: elapsed.as_secs_f64(),
            timestamp_unix_nano: unix_nano_now(),
        };
        let report_result = if let Some(path) = &self.report {
            write_report(path, &completion)
        } else {
            Ok(())
        };

        // The exporter receives a span only when its final handle closes.
        drop(self.span);
        if let Some(sink) = &self.sink {
            sink.shutdown(FLUSH_DEADLINE).await;
        }
        let export_result = if let Some(exporter) = &self.exporter {
            export_completion(exporter, &completion).await
        } else {
            Ok(())
        };
        report_result.and(export_result)
    }
}

struct Completion {
    trace_id: TraceId,
    span_id: SpanId,
    command: &'static str,
    stage: &'static str,
    tenant_context: &'static str,
    outcome: &'static str,
    duration_ms: u64,
    duration_seconds: f64,
    timestamp_unix_nano: u64,
}

async fn export_completion(
    exporter: &OtlpClient,
    completion: &Completion,
) -> Result<(), &'static str> {
    let bounded = || {
        vec![
            ("command".to_owned(), AttrValue::from(completion.command)),
            ("stage".to_owned(), AttrValue::from(completion.stage)),
            ("outcome".to_owned(), AttrValue::from(completion.outcome)),
        ]
    };
    let mut log_attributes = bounded();
    log_attributes.push((
        "tenant.context".to_owned(),
        AttrValue::from(completion.tenant_context),
    ));
    log_attributes.push((
        "duration_ms".to_owned(),
        AttrValue::Int(i64::try_from(completion.duration_ms).unwrap_or(i64::MAX)),
    ));
    let log = LogData {
        trace_id: completion.trace_id,
        span_id: completion.span_id,
        unix_nano: completion.timestamp_unix_nano,
        event_name: "sagat.cli.command.completed".to_owned(),
        body: "sagat CLI command completed".to_owned(),
        error: completion.outcome != "success",
        attributes: log_attributes,
    };
    let gauge = GaugeData {
        name: "sagat_cli_command_duration_seconds".to_owned(),
        description: "Duration of one sagat CLI command.".to_owned(),
        unit: "s".to_owned(),
        unix_nano: completion.timestamp_unix_nano,
        value: completion.duration_seconds,
        attributes: bounded(),
    };

    // Each signal is attempted independently so one rejected record cannot suppress the other.
    let log_result = exporter.export_log(&log).await;
    let metric_result = exporter.export_gauge(&gauge).await;
    if log_result.is_err() || metric_result.is_err() {
        Err("the collector did not accept every completion signal")
    } else {
        Ok(())
    }
}

fn unix_nano_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
        })
}

fn write_report(path: &std::path::Path, completion: &Completion) -> Result<(), &'static str> {
    let timestamp_unix_ms = completion.timestamp_unix_nano / 1_000_000;
    let log = serde_json::json!({
        "signal": "log",
        "timestamp_unix_ms": timestamp_unix_ms,
        "event": "sagat.cli.command.completed",
        "trace_id": completion.trace_id.to_string(),
        "command": completion.command,
        "stage": completion.stage,
        "tenant_context": completion.tenant_context,
        "outcome": completion.outcome,
        "duration_ms": completion.duration_ms,
    });
    // This is one measured duration point with only bounded dimensions. Trace and tenant IDs are
    // intentionally absent from the metric record.
    let metric = serde_json::json!({
        "signal": "metric",
        "timestamp_unix_ms": timestamp_unix_ms,
        "name": "sagat_cli_command_duration_seconds",
        "unit": "s",
        "value": completion.duration_seconds,
        "command": completion.command,
        "stage": completion.stage,
        "outcome": completion.outcome,
        "duration_ms": completion.duration_ms,
    });
    let mut bytes = serde_json::to_vec(&log).map_err(|_| "the report could not be encoded")?;
    bytes.push(b'\n');
    bytes.extend(serde_json::to_vec(&metric).map_err(|_| "the report could not be encoded")?);
    bytes.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|_| "the report file could not be opened")?;
    file.write_all(&bytes)
        .map_err(|_| "the report file could not be written")
}

fn dimensions(args: &[String]) -> (&'static str, &'static str, &'static str) {
    match crate::args::parse(args) {
        Ok(invocation) => {
            let command = command_label(&invocation.command);
            let tenant = if matches!(
                invocation.command,
                Command::Help | Command::Version | Command::New { .. }
            ) {
                "local"
            } else {
                // The CLI deliberately does not read tenant claims from an unverified token and
                // does not add an auth request solely for telemetry.
                "unknown"
            };
            (command, "execute", tenant)
        }
        Err(_) => ("invalid", "parse", "local"),
    }
}

fn command_label(command: &Command) -> &'static str {
    match command {
        Command::Help => "help",
        Command::Version => "version",
        Command::Validate => "manifest.validate",
        Command::Diff { .. } => "manifest.diff",
        Command::Export { .. } => "manifest.export",
        Command::Publish { .. } => "manifest.publish",
        Command::Promote { .. } => "workflow.promote",
        Command::Rollback { .. } => "workflow.rollback",
        Command::History { .. } => "workflow.history",
        Command::Requests { .. } => "publication.requests",
        Command::Decide { approve: true, .. } => "publication.approve",
        Command::Decide { approve: false, .. } => "publication.reject",
        Command::Doctor { .. } => "doctor",
        Command::New { .. } => "new",
        Command::WorkflowValidate { .. } => "workflow.validate",
        Command::WorkflowPublish { .. } => "workflow.publish",
        Command::WorkflowRun { .. } => "workflow.run",
        Command::WorkflowRuns { .. } => "workflow.runs",
        Command::WorkflowInspect { .. } => "workflow.inspect",
        Command::WorkflowCancel { .. } => "workflow.cancel",
        Command::WorkflowRerun { .. } => "workflow.rerun",
        Command::RevealGrantsList => "namespace.reveal_grants.list",
        Command::RevealGrant { .. } => "namespace.reveal_grants.grant",
        Command::RevealRevoke { .. } => "namespace.reveal_grants.revoke",
        Command::ScriptTest { .. } => "script.test",
        Command::ScriptPublish { .. } => "script.publish",
        Command::WorkerDev { .. } => "worker.dev",
        Command::DevToken { .. } => "dev.token",
        Command::WorkerBind { .. } => "worker.bind",
    }
}

const fn outcome(code: u8) -> &'static str {
    match code {
        EXIT_DONE => "success",
        EXIT_FAILED => "failure",
        EXIT_USAGE => "usage",
        EXIT_PENDING => "pending",
        EXIT_BREAKING => "breaking",
        EXIT_DIFFERENCES => "differences",
        EXIT_TIMEOUT => "timeout",
        _ => "failure",
    }
}
