# SDK semantic contract

## Languages

The official SDKs are TypeScript, Go, Python, C#, Java and Rust. Their shared semantics are
defined in [sdks/SPEC.md](../../sdks/SPEC.md); package manifests and the
[CI workflow](../../.github/workflows/ci.yml) define their build/test toolchains.
Do not interpret a CI compiler version as the minimum consumer runtime: the Java library targets
Java 17 while CI uses JDK 21; the .NET library targets both `net8.0` and `net10.0`.
The old Node/Go/Python/.NET/Java baseline proposals are not an installation matrix.

## Mandatory common surface

`WorkflowClient.start`, `.get`, `.list`, `.cancel`, `.signal`, `.publishEvent`; `Worker.registerActivity`, `.heartbeat`, `.complete`, `.fail`; `Step.run`, `Step.sleep`, `Step.waitForEvent`, `Step.invokeChild`, `Step.parallel`; `Saga.step/compensate/reconcile`; deadline/cancel/trace propagation; idempotency headers.

## Wire protocol

REST serves clients and Studio; gRPC serves workers. The compiled worker contract is [worker.proto](../../specs/001-platform-foundation/contracts/worker.proto), consumed by `crates/api-grpc/build.rs`. No database code generation is required.

### Essential messages

`RegisterWorker`, `PollTasks` and `StreamTasks` handle registration and task reception. `Heartbeat` renews a lease and journals the first attempt start; there are no separate `AckLease` or `RenewLease` RPCs. `CompleteStep` and `FailStep` report outcomes. Capability-scoped `ResolveSecrets`, `ResolveDatabase`, `WriteBlob` and `ReadBlob` support execution. Field requirements differ by RPC; use the compiled contract and [worker protocol](worker-runtime-protocol.md), including the structural `instance` identity for scoped work.

## Compatibility

- Protocol version negotiated at handshake: incompatible major → explicit error; backward-compatible minor.
- Common SDK conformance tests run with golden fixtures; run the same scenarios in the six SDKs.
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
