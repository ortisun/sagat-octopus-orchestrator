# SDK OpenTelemetry integration

The five SDKs emit or bridge the same bounded signals. The application owns the OpenTelemetry provider, sampler, readers, processors, exporters, shutdown, and any global registration. The SDKs do not install an exporter and do not read exporter credentials.

Worker attempts use the fixed span name `sagat.worker.task` and kind `CONSUMER`. They are children of the lease's W3C `traceparent` when it is valid. Span attributes are limited to the numeric attempt, the fixed outcome, and a validated canonical error code. Worker metrics are `sagat.worker.events` and `sagat.worker.task.duration`; their labels contain only the fixed event/outcome and validated error code. Payloads, activity names, task IDs, tokens, lease/session tokens, secrets, and exception messages are excluded.

## TypeScript

Install `@opentelemetry/api` and configure the OpenTelemetry SDK and exporters in the application. Import the adapter only when telemetry is enabled:

```ts
import { OpenTelemetryBridge } from '@sagat-octopus-orchestrator/sdk/opentelemetry';

const telemetry = new OpenTelemetryBridge();
const client = new SagatClient({ ...config, traceContext: telemetry.traceContext });
const worker = new WorkerHost({ ...workerConfig, onEvent: telemetry.onEvent });
```

The API package is an optional peer dependency. An application may instead pass its own tracer provider, meter provider, and propagator to the bridge constructor.

## Python

Install the optional extra, configure providers in the application, and pass the two existing hooks:

```console
pip install 'sagat-octopus-orchestrator[opentelemetry]'
```

```python
from sagat_octopus_orchestrator.opentelemetry import OpenTelemetryBridge

telemetry = OpenTelemetryBridge()
client_config = ClientConfig(..., trace_context=telemetry.trace_context)
worker_config = WorkerConfig(..., on_event=telemetry.on_event)
```

Providers may also be passed directly to `OpenTelemetryBridge` without changing global providers.

## Go

Import the opt-in subpackage. It uses only OpenTelemetry APIs; the application's provider setup remains separate.

```go
telemetry := otelbridge.New(otel.GetTracerProvider(), otel.GetMeterProvider())
client.New(apiURL, namespace, token, client.WithTraceContext(telemetry.TraceContext))
worker.New(grpcURL, namespace, name, token, worker.WithEventHandler(telemetry.OnEvent))
```

## Java

Add `io.opentelemetry:opentelemetry-api` in the application and pass either the application `OpenTelemetry` instance or the global instance installed by the Java agent:

```java
var telemetry = new OpenTelemetryBridge(openTelemetry);
var client = ClientConfig.builder().traceContext(telemetry.traceContext());
var worker = WorkerConfig.builder().onEvent(telemetry.onEvent());
```

The SDK declares the API dependency optional and never initializes `GlobalOpenTelemetry`.

## .NET

The .NET SDK emits native `ActivitySource` and `Meter` signals, so no bridge package is needed. Register the published names with the application-owned OpenTelemetry providers:

```csharp
services.AddOpenTelemetry()
    .WithTracing(builder => builder.AddSource(SdkInfo.ActivitySourceName))
    .WithMetrics(builder => builder.AddMeter(SdkInfo.MeterName));
```

An ambient `Activity.Current` supplies client propagation. A lease starts the fixed worker span and makes it current while the handler runs.

## Failure behavior

Telemetry is observational. A missing/no-op provider produces no exported signal. Exceptions and panics from providers, processors, exporters, or metric listeners are contained at the SDK hook boundary and do not change HTTP results, polling, heartbeats, handler execution, or task reporting. Exporter shutdown and flushing remain application responsibilities.

## Worker hook contract

The `task.taken`/`TaskTaken` event adds an optional lease `traceparent` for parenting the attempt
span. Existing event consumers may ignore the additive field. It is propagation context, never
an attribute or metric label. Other task events retain their existing shape.

## Local verification (2026-10-12)

Real exporter tests passed in all ecosystems: TypeScript 2, Python 2, Java 2 and .NET 5 telemetry
cases, plus Go bridge/worker/client tests and bridge vet. Five packaged clean-consumer smoke checks
also passed. Full TS/Java suites reported protocol-copy drift being reconciled with protocol 1.3;
a Java heartbeat-stop timing failure remains under investigation. These targeted results do not
stand in for the final integrated SDK conformance gate.
