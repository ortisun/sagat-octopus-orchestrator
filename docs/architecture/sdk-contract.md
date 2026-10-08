# SDK semantic contract

## Languages

- TypeScript: Node LTS (version pinned in lockfile when coded).
- Go: stable version selected in the workspace at the start of implementation.
- Python: 3.12+ as proposed baseline to be confirmed in CI.
- C#: .NET 10 LTS and C# 14 as proposed baseline; `IHostedService`, `Microsoft.Extensions.DependencyInjection`, cancellation and ActivitySource.
- Java: Java 21+ as initial baseline to be confirmed in CI; functional interfaces, executors/virtual threads where applicable.

## Mandatory common surface

`WorkflowClient.start`, `.get`, `.list`, `.cancel`, `.signal`, `.publishEvent`; `Worker.registerActivity`, `.heartbeat`, `.complete`, `.fail`; `Step.run`, `Step.sleep`, `Step.waitForEvent`, `Step.invokeChild`, `Step.parallel`; `Saga.step/compensate/reconcile`; deadline/cancel/trace propagation; idempotency headers.

## Wire protocol

REST for clients and dashboard, gRPC for low-latency worker↔server communication; canonical identifiers, enums and errors in `proto/sagat_octopus_orchestrator/v1`. No codegen from the database is needed.

### Essential messages

`RegisterWorker(capabilities)`, `PollOrStreamTasks(max_inflight)`, `AckLease`, `Heartbeat`, `CompleteStep`, `FailStep`, `RenewLease`; commands must accept `task_id`, `generation`, `lease_token`, `idempotency_key`, `traceparent`, output ref.

## Compatibility

- Protocol version negotiated at handshake: incompatible major → explicit error; backward-compatible minor.
- Common SDK conformance tests run with golden fixtures; run the same scenarios in the five SDKs.
- W3C trace context and OpenTelemetry; typed errors with code, retryable and causes.
- `Activity` integrates existing code (BYO worker); `Script` uses the artifact registry and platform runners; both have the same step outcome.

## C# example (conceptual API)

```csharp
services.AddSagatOctopusOrchestrator(o => o.Endpoint = new Uri("https://workflow.example.com"));
services.AddSagatOctopusOrchestratorWorker(w => w.Activity<ReserveLimitActivity>("reserve-limit"));

var run = await client.StartAsync("corporate-payment", new { amount = 100m },
    new StartOptions(IdempotencyKey: paymentId), cancellationToken);
```

## Parity does not mean the same runtime

Go/Java/C# compiled inline requires a build published in the artifact registry; inline code does not require deploying the customer's application, but **requires automatic compilation and deployment of the artifact inside the execution plane**.
