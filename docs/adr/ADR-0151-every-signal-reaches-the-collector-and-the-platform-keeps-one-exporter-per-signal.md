# ADR-0151: Every signal reaches an OpenTelemetry Collector, and each process keeps one exporter per signal that never blocks the work

**Status:** Accepted (2026-11-26, delegate, Owner decision 2026-11-24-4)
**Decided by:** agent proposal with design 015, accepted by the agent preparing design 015 for build under the owner's
delegation (Owner decision 2026-11-24-4); recorded as Owner decision 2026-11-26-118. The owner may overrule it.
**Refines / supersedes:** refines ADR-0008 (observability is part of the feature) and ADR-0017 (OTLP to a Collector) by
adding logs and metrics; supersedes none.
**Enforced by:** the unit tests of `crates/telemetry` for the log queue (drop, scrub, correlation) and the configuration
refusals (Tasks 015-2, 015-3); the resource-attribute test (Task 015-4); `otelcol-contrib validate` on both Collector
configurations and the attribute-filter drift test (Tasks 015-5, 015-6); the measurement of Non-functional requirement
015-001 (Task 015-9). Until they exist: review only.

## Context

Spans reach the Collector over OTLP/HTTP with JSON; metrics are a Prometheus page; logs are JSON lines on standard
output, unlinked to spans in a backend (design 015, The Problem). Operators run OTLP-native backends and expect one
pipeline.

## Decision

1. Traces and logs MUST be exported by each process over OTLP/HTTP with JSON, each through a bounded queue whose submit
   never waits: a full queue drops and counts. A log record MUST carry the trace id and span id of the span it was
   written in and MUST be scrubbed as standard output is. Standard output keeps its JSON lines.
2. Metrics MUST reach the Collector by its Prometheus receiver scraping each process's admin port. No process gains a
   second metrics pipeline; names, units and closed labels are unchanged.
3. A process MUST read the Collector's authentication header from a file when one is configured
   (`OTEL_EXPORTER_OTLP_HEADERS_FILE`).
4. No resource attribute or metric label MAY carry a tenant, namespace, run or sink id.
5. The repository MUST ship a production Collector configuration with TLS, bearer authentication, a memory limiter and
   an attribute filter generated from the platform's sensitive-name rule.

## Considered options

1. **OTLP for traces and logs, a scrape for metrics** — one implementation per signal; the process knows each line's
   span.
2. **OTLP for all three in the process** — a second metrics pipeline with its own cardinality and failure modes.
3. **A file shipper for logs** — loses the span of each line unless parsed back; depends on the container runtime.

## Consequences

A log line can now leave the process twice (standard output and OTLP); both are scrubbed the same way. A Collector that
is down costs dropped telemetry, counted, never latency. Protobuf and gRPC encodings stay a follow-up (Gap 015-001).

## Evidence

Design 015 (`docs/designs/015-opentelemetry-and-openbao.md`), `crates/telemetry/src/otlp.rs` (the span queue this
decision copies for logs), `infra/observability/otel-collector.yaml`.
