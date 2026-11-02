# crates/telemetry

## Overview

sagat-telemetry provides structured logging, Prometheus metrics, trace identity and OTLP export.

## Interface

- Telemetry, Metrics, OtlpClient, trace/span identifiers and redaction helpers form the observability boundary. [src/logging.rs:Telemetry] [src/metrics.rs:Metrics] [src/otlp.rs:OtlpClient] [src/trace.rs:TraceId] [src/redact.rs:scrub]
- [src](./src/README.md) — Shared structured logging, bounded Prometheus metrics, W3C trace identity, deterministic span identity/sampling and OTLP/HTTP JSON export. Capacity observations are dormant metric vocabulary, not scheduler capacity operations or workload acceptance. [src/logging.rs:LogFormat]

## Invariants

- Attributes/events/links/queues/timeouts/metric labels are bounded; redaction is a backstop rather than proof arbitrary secrets cannot be logged. Capacity/leaf-certification recorders are dormant vocabulary and do not establish scheduler/workload acceptance. [src/otlp.rs:MAX_ATTRIBUTES] [src/metrics.rs:WorkflowPrimitiveMetric::label_keys] [src/redact.rs:scrub] [src/capacity.rs:CapacityAcquireOutcome::record] [src/leaf_certification.rs:LeafCertificationOutcome::record]

## Side Effects

- Initialization installs global tracing/metrics state, writes logs and may spawn batching work performing OTLP HTTP POSTs. [src/logging.rs:init] [src/metrics.rs:Metrics::global] [src/otlp.rs:start] [src/otlp.rs:OtlpClient::export]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-telemetry --locked
```

These commands were not run in this documentation-only pass.
