# opentelemetry

## Overview

Opt-in OpenTelemetry adapter. [OpenTelemetryBridge.java:OpenTelemetryBridge]

## Interface

- `OpenTelemetryBridge` accepts global or supplied OpenTelemetry; `traceContext` supplies traceparent and `onEvent` supplies a worker-event consumer. [OpenTelemetryBridge.java:OpenTelemetryBridge] [OpenTelemetryBridge.java:traceContext] [OpenTelemetryBridge.java:onEvent]

## Invariants

- The adapter retains at most 1,000 task spans and emits error.type only for bounded machine codes. [OpenTelemetryBridge.java:accept] [OpenTelemetryBridge.java:CODE]

## Side Effects

- Creates counters, histograms and spans through the selected OpenTelemetry API; provider/exporter setup remains application-owned. [OpenTelemetryBridge.java:OpenTelemetryBridge] [OpenTelemetryBridge.java:accept]

## Verification

Run from the repository root:

```sh
cd sdks/java && ./mvnw -q verify
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
