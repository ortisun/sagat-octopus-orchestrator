# otelbridge

## Overview

Optional bridge to application-owned OpenTelemetry tracing and metrics. [bridge.go:Bridge]

## Interface

- New constructs the bridge; TraceContext supplies ambient traceparent and OnEvent translates worker events. [bridge.go:New] [bridge.go:TraceContext] [bridge.go:OnEvent]

## Invariants

- Open task spans cap at 1000; error.type accepts bounded uppercase codes. Telemetry panics are contained and missing/invalid contexts yield no traceparent. [bridge.go:maxOpenTasks] [bridge.go:codePattern] [bridge.go:TraceContext] [bridge.go:OnEvent]

## Side Effects

- Records worker event counts/durations and consumer spans, using supplied or global providers without installing exporters. [bridge.go:New] [bridge.go:OnEvent]

## Verification

Run from the repository root:

```sh
cd sdks/go && go test -race ./... && go vet ./...
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
