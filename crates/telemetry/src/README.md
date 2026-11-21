# crates/telemetry/src

## Overview

Shared structured logging, bounded Prometheus metrics, W3C trace identity, deterministic span identity/sampling and OTLP/HTTP JSON export. Capacity observations are dormant metric vocabulary, not scheduler capacity operations or workload acceptance.

## Interface

- Logging exposes `LogFormat`, `TelemetryConfig`, `Telemetry`, `init`, `init_logging` and bounded `Telemetry::shutdown`; `LogLayer<W>` and `Style` provide writer-backed JSON/text formatting. [logging.rs:LogFormat] [logging.rs:TelemetryConfig] [logging.rs:Telemetry] [logging.rs:init] [logging.rs:init_logging] [json_log.rs:LogLayer] [json_log.rs:Style]
- `Metrics::{global,detached,render,prune_idle}` owns Prometheus setup/rendering/upkeep. Closed primitive kind/outcome/rejection/metric enums define labels. [metrics.rs:Metrics] [metrics.rs:WorkflowPrimitiveKind] [metrics.rs:WorkflowPrimitiveOutcome] [metrics.rs:WorkflowPrimitiveRejectionCode] [metrics.rs:WorkflowPrimitiveMetric]
- OTLP wire records include `AttrValue`, span kind/status/events/links/data, log data and gauge data. Configuration, typed errors and `OtlpClient` support encoding and direct span/log/gauge export. [otlp.rs:AttrValue] [otlp.rs:SpanData] [otlp.rs:LogData] [otlp.rs:GaugeData] [otlp.rs:OtlpConfig] [otlp.rs:OtlpClient] [otlp.rs:ExportError] [otlp.rs:OtlpConfigError]
- `SpanSink`, `start` and `OtlpLayer` connect tracing to bounded background batching and shutdown. [otlp.rs:SpanSink] [otlp.rs:start] [otlp.rs:OtlpLayer]
- Trace/span identity exposes `TraceId`, `TraceContext`, `SpanId`, run/attempt/event span IDs, traceparent formatting, deterministic `keeps` sampling and primitive span/field vocabulary. [trace.rs:TraceId] [trace.rs:TraceContext] [span.rs:SpanId] [span.rs:run_span_id] [span.rs:attempt_span_id] [span.rs:event_span_id] [span.rs:traceparent] [span.rs:keeps] [span.rs:WorkflowPrimitiveSpan] [span.rs:WorkflowPrimitiveSpanField]
- Redaction exports `REDACTED`, `MAX_VALUE_CHARS`, `is_sensitive_name` and `scrub`. Dormant capacity finish/vacate/acquire and leaf-certification outcomes expose closed metric recorders/span names. [redact.rs:REDACTED] [redact.rs:MAX_VALUE_CHARS] [redact.rs:is_sensitive_name] [redact.rs:scrub] [capacity.rs:CapacityFinishOutcome] [capacity.rs:CapacityVacateOutcome] [capacity.rs:CapacityAcquireOutcome] [leaf_certification.rs:LeafCertificationOutcome]

## Invariants

- Log collection scrubs text, moves reserved top-level names under fields.*, lets inner span fields override outer values and suppresses tokio_postgres/sqlparser events. Redaction clips values and recognizes selected credential forms; it is a backstop, not a guarantee that arbitrary secrets cannot be logged. [json_log.rs:Collector::text] [json_log.rs:RESERVED] [json_log.rs:LogLayer::on_event] [redact.rs:scrub]
- OTLP caps attributes/events/links, scrubs strings and coerces non-finite doubles. Configuration permits HTTP(S) http/json, rejects userinfo/unsupported samplers, bounds queue/timeout and redacts header values in Debug. [otlp.rs:MAX_ATTRIBUTES] [otlp.rs:MAX_SPAN_ITEMS] [otlp.rs:attribute] [otlp.rs:encode_span] [otlp.rs:OtlpConfig::from_lookup]
- The client disables redirects/environment proxies. 400/413/422 permanently reject batches; retryable transport/408/429/5xx errors back off within configured retries. [otlp.rs:OtlpClient::new] [otlp.rs:BATCH_REFUSED] [otlp.rs:retryable] [otlp.rs:OtlpClient::export_body]
- Live tracing exports sagat_ targets with explicit/inherited trace IDs, deterministic sampling and otel.sampled=false handling. Parsing accepts lowercase nonzero version-00 traceparent IDs; malformed context starts a new unsampled trace. [otlp.rs:OtlpLayer::on_new_span] [otlp.rs:Fields::record_bool] [trace.rs:TraceContext::parse_traceparent] [trace.rs:TraceContext::from_traceparent]
- Primitive labels are closed and idle metric series expire. Live spans use a bounded nonblocking queue: overflow is counted/dropped and failed live batches are abandoned; journal-backed projection has a separate contract. [metrics.rs:WorkflowPrimitiveMetric::label_keys] [metrics.rs:Metrics::IDLE_TIMEOUT] [otlp.rs:SpanSink::submit] [otlp.rs:send]
- Dormant capacity/leaf recorders count returned attempts; only acknowledged first commits increment confirmed counters. Replay/unknown commits do not imply an active capacity scheduler loop. [capacity.rs:CapacityFinishOutcome::record] [capacity.rs:CapacityVacateOutcome::record] [capacity.rs:CapacityAcquireOutcome::record] [leaf_certification.rs:LeafCertificationOutcome::record]

## Side Effects

- Initialization installs a global tracing subscriber, writes stdout logs and optionally starts live OTLP when a Tokio runtime exists. Writer failures are ignored by LogLayer. [logging.rs:init] [json_log.rs:LogLayer::on_event]
- Metrics global initialization installs the process recorder; rendering returns Prometheus text and upkeep expires idle series. [metrics.rs:Metrics::global] [metrics.rs:Metrics::render] [metrics.rs:Metrics::prune_idle]
- OTLP exports perform HTTP POST with configured headers/retries; absent log/gauge endpoints make those exports no-ops. Client setup installs the Rustls ring provider if none is selected. [otlp.rs:OtlpClient::export] [otlp.rs:OtlpClient::export_log] [otlp.rs:OtlpClient::export_gauge] [otlp.rs:install_crypto_provider]
- `start` spawns batching work; shutdown requests queue drain and waits only to the supplied deadline. Metric helpers record counters/histograms without persisting authority. [otlp.rs:start] [otlp.rs:SpanSink::shutdown] [capacity.rs:CapacityAcquireOutcome::record]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-telemetry --locked
```

These commands were not run in this documentation-only pass.
