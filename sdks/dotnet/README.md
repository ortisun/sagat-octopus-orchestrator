# SagatOctopusOrchestrator.Sdk (C# / .NET)

The .NET SDK of the Sagat Octopus Orchestrator, version **0.1.0**. It is three things in one package and nothing else
(`sdks/SPEC.md`):

1. a **client** for the REST API: start, read, list, operate, cancel and signal runs, publish events, wait for a run;
2. a **worker host** for the gRPC worker protocol: register activities, compensations and reconcilers, take leases,
   heartbeat, cancel, report;
3. **builders** of the workflow IR (`workflow.schema.json`): `Step`, `Sleep`, `WaitForEvent`, `InvokeChild`,
   `Condition`, `Parallel`, `Saga`, `End`, with `CompensateWith` / `ReconcileWith`.

It is not a workflow runtime. It does not execute workflow code, replay anything or decide a transition: the server owns
every state and every retry decision. It integrates with `Microsoft.Extensions.DependencyInjection`, `IHostedService`,
`CancellationToken`, `ILogger` and `ActivitySource` (FR-006-007).

## Install

```text
dotnet add package SagatOctopusOrchestrator.Sdk --version 0.1.0
```

The package is not published to a feed yet. `dotnet pack src/SagatOctopusOrchestrator.Sdk -c Release -o ./nupkg` makes it
(libraries for `net8.0` and `net10.0`, this README and `LICENSE.txt`), and a local folder works as a NuGet source.

## Start a run

With the dependency injection that an ASP.NET Core service already has:

```csharp
builder.Services.AddSagatOctopusOrchestrator(options =>
{
    options.ApiUrl = "https://api.example/v1";
    options.Namespace = "payments";
    options.TokenProvider = async ct => await tokens.GetAsync(ct);   // called before every request; or options.Token = "…"
});

app.MapPost("/orders/{id}", async (string id, ISagatClient sagat, CancellationToken ct) =>
{
    var run = await sagat.StartAsync(
        "order-flow",                                   // a workflow name, or its id
        new { orderId = id },                           // any System.Text.Json value; a JsonNode keeps every number as it is
        new StartOptions { IdempotencyKey = $"order-{id}", CorrelationId = id },
        ct);
    return Results.Accepted($"/runs/{run.RunId}", new { run.RunId, run.Status });
});
```

The run is created with the idempotency key, and the trace context of the request (`Activity.Current`) travels in the
`traceparent` header, so the run has the trace id of the request. Without dependency injection:

```csharp
using var sagat = new SagatClient(new SagatOptions { ApiUrl = "...", Namespace = "payments", Token = "..." });
var run = await sagat.StartAsync("order-flow", new { orderId = "o-1" });
var done = await sagat.WaitForCompletionAsync(run.RunId, new WaitOptions { Timeout = TimeSpan.FromMinutes(5) });
```

Also: `GetAsync`, `ListAsync` / `ListAllAsync` (`IAsyncEnumerable<Run>`, in the order the server gave), `OperateAsync`
(`RunAction.Pause`, `Resume`, `Cancel`, …), `CancelAsync`, `SignalAsync`, `PublishEventAsync`, `EventsAsync` (the
journal) and `ResolveWorkflowAsync`. Every call takes a `CancellationToken`.

## Register an activity

As part of the application (`IHostedService`: it registers with the worker service when the host starts and stops
gracefully when the host stops):

```csharp
builder.Services
    .AddSagatOctopusOrchestrator(options =>
    {
        options.GrpcUrl = "https://worker.example:443";
        options.Namespace = "payments";
        options.Token = "...";
        options.Worker.Name = "payments-worker";        // defaults to the machine name (a pod, for a replica)
        options.Worker.MaxInflight = 10;
    })
    .AddActivity<Charge, Receipt>("payments.charge", async (input, context, ct) =>
    {
        // context.IdempotencyKey is stable across attempts: send it to the provider.
        return await provider.ChargeAsync(input.Amount, context.IdempotencyKey, ct);
    })
    .AddCompensation<Charge, Refund>("payments.refund", (input, context, ct) => /* context.TargetNode, context.TargetIdempotencyKey */ …)
    .AddReconciler<Charge>("payments.lookup", async (input, context, ct) =>
        await provider.FindAsync(context.TargetIdempotencyKey, ct) is { } found ? Verdict.Applied(found) : Verdict.NotApplied())
    .AddActivity<ShipOrder>("shipping.ship");           // a class: IActivity<TInput, TOutput>, resolved in a new scope per task
```

Without the generic host: `new WorkerHost(options)`, then `RegisterActivity` / `RegisterCompensation` /
`RegisterReconciler`, `await host.StartAsync()` and, when done, `await host.StopAsync()`.

- A handler raises `ActivityFailure.Retryable(code, message)`, `ActivityFailure.Permanent(code, message)` or
  `ActivityFailure.OutcomeUnknown(message)`. Any other exception is reported as `ACTIVITY_ERROR` with only its **type
  name** (its text may hold the data it was handed). The server's retry policy decides what happens next, not the SDK.
- The handler's `CancellationToken` is cancelled when the run is cancelled (`context.CancellationReason` is
  `run-cancelled`, seen at the next heartbeat), when the lease is lost (`lease-lost`: the result is dropped, nothing is
  reported) and when the host stops while the task is still running (`shutdown`: a handler that then raises its
  cancellation error is not reported and the lease lapses; the step is leased again). `context.Deadline` is the end of
  the lease and moves with every heartbeat.
- The heartbeat runs from before the handler until the report is made. Its interval is a third of the time left on the
  lease when the task is taken, between 0.5 s and 10 s (`Worker.HeartbeatInterval` overrides it: more than 0 and at most one hour, else
  `ValidationFailed` when the host is created). A beat that fails, whatever the failure, pauses 100 ms before the next one, so the loop cannot spin.
- A failure message the handler reports is cut at 512 Unicode code points (never inside a surrogate pair; a lone surrogate becomes U+FFFD). The
  `task_kind`, `target_node` and `target_idempotency_key` of a lease are read from protocol 1.2 only.
- `await context.GetSecretsAsync(ct)` resolves the secrets the step declares, once per attempt (protocol 1.2 or newer).
  `Get` hands out a copy (clear it when you are done); the SDK wipes its own buffer when the handler returns, before the result is reported, and a later
  request raises. A failed resolution is not remembered for the rest of the attempt. Never log them.
- Against a server at protocol 1.0 or 1.1 the host runs at that version (`host.ProtocolVersion`); against 1.0/1.1 it does not read `task_kind` (every
  task is a forward one).
- The host cancels the token and nothing else: it never interrupts a handler, and it does not fence the lease by its own clock (only a `STALE_ATTEMPT`
  ends a task). The host is restartable (`StopAsync` then `StartAsync` on the same object); `StopAsync` also ends a pause the poll loop is parked in.
- A session the server drops (`UNAUTHENTICATED` to a poll) is registered again at once; if the next poll is refused too, the registrations back off (SPEC
  section 5) until a poll succeeds. The same task leased again while it runs is another task in flight (`StopAsync` waits for and cancels both).

## Build a workflow

```csharp
using SagatOctopusOrchestrator.Sdk.Workflows;

var document = Workflow.Create("order-flow")
    .Description("charges and ships")
    .Saga("settlement", new SagaOptions { CancelPolicy = SagaCancelPolicy.CompensateCompleted },
        Workflow.Flow()
            .Step("reserve", "limit.reserve", new StepOptions { CompensateWith = "limit.release" })
            .Step("charge", "payments.charge", new StepOptions
            {
                CompensateWith = "payments.refund",
                ReconcileWith = "payments.lookup",
                HighRisk = true,
                Timeout = TimeSpan.FromSeconds(30),
                Retry = new StepRetry { MaxAttempts = 3, InitialDelay = TimeSpan.FromMilliseconds(200), Strategy = RetryStrategy.Exponential },
            }))
    .Sleep("pause", TimeSpan.FromSeconds(1))
    .Step("ship", "shipping.ship")
    .Build();                                            // a JsonObject; builder.ToJson() is the canonical text
```

The builders are pure: no I/O, no clock, no randomness. `ToJson()` is the canonical text of RFC 8785 (keys sorted by UTF-16
code unit, numbers as ECMAScript writes them, no whitespace), byte-identical to the one the other four SDKs write for the same
program. What the builder can see the server would refuse (a duplicate id, a `Parallel` with fewer than two branches, an empty
`Saga`, `Irreversible` with `CompensateWith`, a bad name, a reference to a node that does not exist) is a
`SagatException` with `ValidationFailed` when the node is added (or at `Build()` for a reference). A flow joined into another (a `Parallel`
branch, the body of a `Saga`) is read at `Build()`, not when it was joined: a step added to it afterwards is in the document, and an `End` added to
a saga body afterwards is refused at `Build()`.

## Errors

Every failure of a call is a `SagatException` with `Code` (`SagatErrorCode`; `code.ToWireName()` is the catalog's
`RATE_LIMITED`), `HttpStatus` or `GrpcStatus`, `Retryable`, `TraceId`, `Reason`, `RetryAfter` and `Remediation` (the catalog's
sentence). `Message` is safe to log: it never holds a payload, a token or a secret. The one exception is **cancellation by the
caller**: a call cancelled with the caller's own `CancellationToken` raises `OperationCanceledException`, as every .NET API does,
and not a `SagatException` (SPEC section 2 lists `CANCELLED` as one error type; here it is the idiom of the platform that wins
for the caller's own token).

**Retries are by code** (SPEC section 5): a call is repeated only when its code is `Transport`, `Unavailable`, `RateLimited` or
`Internal` *and* the error says it may be (`Retryable`; `Internal` only when the server said so). A body that claims `retryable: true`
on any other code (a 409, a 422, a 403, `QuotaExceeded`) is not believed. The server's `408` is `Unavailable`. Only calls that cannot do
harm are repeated: `GET`s, and `POST`s that carry an idempotency key or an event id (a start without a key gets a generated one, the same
on every retry of that call). A token provider that throws is `Unauthenticated` with the reason `TOKEN_PROVIDER_FAILED` (its own exception
is not kept, and it is asked again like a transport failure, because nothing was sent). That is the SDK's own knowledge, not a reason string: a
server that answers with that reason on a 400, 401, 403 or 409, or in a gRPC trailer, is not believed and the call is not repeated. A call the server does not have (a `501`, a `404` whose envelope says
`ROUTE_NOT_FOUND`, a `405` that says `METHOD_NOT_ALLOWED`, or a `404` that is not the platform's envelope) is `NotFound` with the reason
`UNSUPPORTED_BY_SERVER`; a `404` envelope without those is a plain `NotFound`.

**What the server says is bounded** (SPEC section 10): a wait it asks for (`Retry-After` in seconds, `retry-after-ms`, the pause after an
empty poll) is honoured up to 60 s whatever it says (an HTTP date is ignored); a REST body is read up to 32 MiB and the call then fails as
`Protocol`; a `Message` is cut to 512 characters and a `Reason` to 128; a `TraceId` that is not 32 lowercase hexadecimal characters is
not repeated; a run id is a UUID and a name in a path is never empty, `.` or `..` (`ValidationFailed`, before anything is sent); a poll
answer with more tasks than the host asked for runs the first ones and leaves the leases of the others to lapse.

## Configuration (`SagatOptions`)

| Option | Meaning | Default |
|---|---|---|
| `ApiUrl` | REST base URL including `/v1` | required for a client |
| `GrpcUrl` | worker service URL, `https://host:port` | required for a worker |
| `Namespace` | slug of the namespace | required |
| `Token` / `TokenProvider` | a bearer token, or a provider called before every request | one is required |
| `Timeout` | deadline of one REST call or unary RPC | 30 s |
| `Retry` | `MaxAttempts`, `InitialDelay`, `MaxDelay` | 4, 200 ms, 5 s |
| `UserAgent` | suffix appended to `sagat-sdk-csharp/0.1.0` | none |
| `Tls` | `TrustedRoots`, `ServerName`, `ClientCertificate` | system roots, the host of the URL, no client certificate |
| `Json` | `JsonSerializerOptions` for typed payloads | camel case, case-insensitive on read |
| `HttpMessageHandler` | the transport of the client (a proxy, a test, fault injection); the caller owns it and must turn redirects off | a pooled `SocketsHttpHandler` |
| `Worker.*` | `Name`, `MaxInflight` (10), `Languages`, `Pools`, `ShutdownTimeout` (30 s), `ProtocolVersion`, `HeartbeatInterval` (positive), `Interceptors` | |

`SagatOptions`, `SagatTlsOptions`, `SagatClient` and `WorkerHost` never show a credential: their `ToString()`, their JSON and the debugger show
no bearer token, token provider or private key, and a URL is shown by its host only (`ToString()`) or not at all (JSON: the URLs can carry user-info or a
query, so they are not serialized). The session token of a `WorkerHost` is hidden from the debugger and dropped when the host stops. A test builds each
with a canary token and looks for it.

`SagatOptions` binds from configuration (`services.Configure<SagatOptions>(configuration.GetSection("Sagat"))`). The tenant is
never configured: it is the token's. `http://` (or a gRPC channel without TLS) outside loopback works but logs one warning at
start.

### TLS and redirects

Three TLS options exist: `Tls.TrustedRoots` (an `X509Certificate2Collection` used instead of the system roots; an *empty* collection is refused,
it is not "the system roots": leave it null for those), `Tls.ServerName` (the name the certificate is verified against, and the name sent for
the server name indication, when it is not the host of the URL; it must be a bare host name: a space, a control character, a port, a path or
user-info is `ValidationFailed` when the client or host is created, and so is a `ServerName` with an `http://` URL; the port of the URL stays in the `Host` header) and `Tls.ClientCertificate` (the certificate, with its private key, presented for
mTLS). They apply to both channels (the `HttpClient` of the REST client and the gRPC channel of the worker host). The roots are handed to the
platform's own validation as a chain policy (`SslClientAuthenticationOptions.CertificateChainPolicy`), so the chain, the validity dates and the
host name are still verified. `ServerName` is verified too, never skipped: the request names that host (the `Host` header, which is where the
platform's TLS stack takes its server name from), so the certificate must carry it. There is no option to skip verification and no certificate
callback: a test reads `src/` and fails on `DangerousAcceptAnyServerCertificateValidator`, any `*CertificateValidationCallback` and any other
override of the host name. A server whose certificate is not trusted, is expired or names another host fails as `Transport`. Revocation is not checked.

**A transport you hand the SDK is your choice.** `SagatOptions.HttpMessageHandler` lets you supply the `HttpMessageHandler` of the REST client; what that
object does about certificates, proxies and redirects is yours (the SDK never builds a handler with verification off, but it cannot see inside one it was
given, so it checks only what it can: redirects, below). `Tls` (roots, server name, client certificate) applies to the transports the SDK builds, which are the
worker channel (always) and the REST client when you do not supply a handler: with your own `HttpMessageHandler` the REST client's TLS is whatever you configured
on it. `Worker.Interceptors` sees only calls.

A redirect is **never followed**: it could carry the token to another host. The handler the SDK creates has
`AllowAutoRedirect = false`; a 3xx is `SagatException` `Protocol` ("the server redirected the call"). A handler you supply with
`SagatOptions.HttpMessageHandler` must be configured the same way (the SDK cannot change a handler it does not own): the client
refuses an `HttpClientHandler` or `SocketsHttpHandler`, also inside `DelegatingHandler`s, that follows redirects, and cannot tell
for a handler of its own kind.

Tested here, against in-process Kestrel servers with throw-away certificates, for **both** channels: an untrusted self-signed
server certificate is refused (`Transport`), also with the root of another server configured; the configured root makes the same
server work; with mTLS the server sees the client certificate, and a server that requires one refuses a client without it; REST
also: an expired certificate and a certificate for another host are refused even when their root is trusted; no plain-text warning
is logged for `https`; a 301/302/303/307/308 is `Protocol`, is not retried and the server it points to sees no request; `ServerName` makes a certificate
for another name work when it is the name given and fail when it is not (REST and gRPC).
**Not tested:** certificate revocation, client certificates from a hardware store or with a password-protected key (the caller
loads the `X509Certificate2`), a real CA chain with intermediates, TLS versions and ciphers (the platform's defaults), and `https`
against a real platform (the conformance suite runs over loopback `http`).

## Observability

- **Hooks that fail**: a logger, a metrics listener, a trace listener or a token provider of the application that throws never kills the heartbeat, the poll loop
  or a task in flight (tests arm each of them).
- **Logs**: `ILogger<SagatClient>` / the `ILoggerFactory` given to `WorkerHost`, with fixed templates and identifiers only (method,
  path template, code, task, run, attempt). Never a token, a lease token, a session token, a secret or a payload (tests capture
  every line to prove it). `context.Logger` is a logger for the handler with the task in its scope.
- **Traces**: an `ActivitySource` named `SagatOctopusOrchestrator.Sdk`. A client call is a `Client` span (a child of
  `Activity.Current`) whose `traceparent` is sent to the server; an explicit `Traceparent` option wins. A lease's `traceparent` is
  the parent of a `Consumer` span around the handler, so `Activity.Current` inside the handler is in the run's trace and the
  requests the handler makes carry it; with no listener on the SDK's source it is a plain `Activity` with the same parent (the trace
  still reaches the handler), and with an empty or invalid `traceparent` the SDK sets none. Worker attempts use the fixed
  span name `sagat.worker.task`; tags carry identifiers only.
- **Metrics**: a `Meter` named `SagatOctopusOrchestrator.Sdk`: `sagat.worker.tasks.taken`, `.completed`, `.failed`, `.stale`,
  `.dropped`, `sagat.worker.heartbeats`, `.polls`, `.reconnects` (counters) and `sagat.worker.activity.duration` (histogram, s),
  tagged with `sagat.worker`.

## Serialization

JSON only, `System.Text.Json`. A `JsonNode` / `JsonElement` keeps every number exactly as it was read (`600000` is never
`600000.0`, a 64-bit or bigger integer is never turned into a double, `"19.99"` stays a string). A .NET number is written as the
shortest text that reads back, as JavaScript does. Instants the SDK writes are RFC 3339 in UTC with a `Z`; instants it reads may
carry any offset and come back as UTC `DateTimeOffset`s (nothing depends on the time zone of the process). `NaN`, infinities and
text that is not valid Unicode (a lone surrogate) are `ValidationFailed` before anything is sent. An activity output over 256 KiB fails the step
with `PAYLOAD_TOO_LARGE`. The canonical text of a workflow (`ToJson()`) also refuses an integer beyond 2^53 - 1 (write it as a string) and a
duration that is not a whole number of milliseconds (`TimeSpan.FromTicks(15_000)` is not 1 ms, and is never truncated to it).

## Versioning

SemVer. Before 1.0 a minor version may change the .NET API; the **wire** contract does not move with it: REST is additive within
`/v1` and the worker protocol is additive within `1.x` (the host negotiates; a newer SDK against an older server runs at the
server's version, an older SDK ignores fields it does not know). After 1.0 a breaking change of the API is a major version.
`SdkInfo.Version` is the version, `SdkInfo.ProtocolVersion` (`1.2`) the newest worker protocol minor the SDK speaks.

## Runtime version matrix

What was **run** on 2026-10-10 on one machine (macOS 26, arm64), and what was not:

| Target | Built | Unit tests | Conformance suite (15 scenarios) |
|---|---|---|---|
| `net10.0` (runtime 10.0.12, SDK 10.0.401) | yes | 410 passed | passed, three runs in a row |
| `net8.0` (runtime 8.0.31, installed next to it) | yes (a `net8.0` asset in the package) | **not run**: the test project targets `net10.0` (ASP.NET Core `TestServer` of 10) | passed, one run, on an earlier revision of the SDK and of the suite |
| .NET 9 | not built for; it uses the `net8.0` asset | **not run** | **not run** |
| Windows, Linux, x64 | n/a | **not run** | **not run** |
| .NET Framework, `netstandard`, Mono, AOT, trimming | not supported | n/a | n/a |

The conformance suite (`tests/contract/sdk-conformance`, 15 scenarios, a real platform with PostgreSQL) runs the `net10.0`
runner by default. `SAGAT_DOTNET_TFM=net8.0 SAGAT_DOTNET=<a dotnet with the .NET 8 runtime> SAGAT_SDK_LANGS=dotnet …` runs the
`net8.0` build of the SDK instead.

Not covered by any test: a real (non-fake) platform other than the conformance suite's, long-running soak, load, a server whose
clock is skewed, and the NuGet package consumed from a feed (it was packed and its contents inspected, not restored from one).

## Dependencies

Central package management with pinned versions, each published at least two weeks before the choice (2026-10-10):
`Grpc.Net.Client` 2.83.0 and `Grpc.Core.Api` 2.83.0 (2026-08-03), `Google.Protobuf` 3.36.1 (2026-08-31), `Grpc.Tools` 2.84.0
(build time, 2026-09-14), `Microsoft.Extensions.{DependencyInjection.Abstractions, Hosting.Abstractions, Logging.Abstractions,
Options}` 10.0.11 (2026-08-11) for `net10.0` and 8.0.x for `net8.0`. Tests: `Microsoft.NET.Test.Sdk` 18.10.0, `xunit` 2.9.3,
`xunit.runner.visualstudio` 3.1.5, `Grpc.AspNetCore.Server` 2.83.0, `Microsoft.AspNetCore.TestHost` 10.0.11, and
OpenTelemetry plus its in-memory exporter 1.19.0 for integration tests only.

## Development

```text
bash scripts/sync-shared.sh      # copies worker.proto and error-catalog.json from the repository (a test fails when a copy is behind)
dotnet test                      # 410 unit tests; offline after the first restore (NuGet's vulnerability audit, NU1900, is a warning, not an error)
dotnet format --verify-no-changes
bash conformance/prepare.sh      # builds the runner; the harness calls it, and conformance/run.sh
```

The runner has one mode besides `build` and `run`: `conformance/run.sh aspnet <config.json> <out.json>`, the acceptance test of
US01-AC01 against the real platform (`tests/e2e/sdk-multilanguage`, `an_aspnet_core_service…`). It starts a minimal ASP.NET Core
service on a loopback port that registers the SDK with `AddSagatOctopusOrchestrator`, with `POST /orders` taking `ISagatClient` from
dependency injection and calling `StartAsync` with the request's `Idempotency-Key` header and **no explicit trace**, calls it twice
with the same key and one `traceparent`, and writes what came back. The trace reaches the platform through the ambient `Activity`
that ASP.NET Core made from the request: it makes one only when something observes it (an OpenTelemetry SDK, which listens to
`Microsoft.AspNetCore` and, with `AddSource("SagatOctopusOrchestrator.Sdk")`, to this SDK; the mode registers an `ActivityListener`
that stands in for it). The runner needs the ASP.NET Core shared runtime (it is part of the .NET SDK).

Layout: `src/SagatOctopusOrchestrator.Sdk` (the library), `tests/SagatOctopusOrchestrator.Sdk.Tests` (xUnit, with an in-process
fake REST server and an in-process fake `WorkerService`), `conformance` (the runner, which uses only the public API of the SDK
plus plain HTTP for the control endpoint), `proto/worker.proto` (a copy). The generated protocol types are internal.

## License

MIT. `LICENSE.txt` is included in the NuGet package.
