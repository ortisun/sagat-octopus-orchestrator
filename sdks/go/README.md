# Sagat Octopus Orchestrator: Go SDK

The Go SDK of the Sagat Octopus Orchestrator, a durable workflow platform. It is three things in one
module and nothing else:

| Package | What it is |
|---|---|
| `client` | The REST client: start, read, list, operate, cancel and signal runs, publish events, wait for a run to finish. |
| `worker` | The worker host (gRPC): registers activities, compensations and reconcilers, takes leases, heartbeats, reports. |
| `workflow` | Typed, side-effect-free builders of the workflow IR (`step`, `sleep`, `waitForEvent`, `invokeChild`, `parallel`, `saga`...). |
| `sagat` (the module root) | What they share: `*sagat.Error`, the error catalog, token sources, the retry policy, W3C `traceparent` helpers. |

The SDK is not a workflow runtime. It does not execute workflow code, replay anything or decide a
transition: the server owns every state and every retry decision. The behaviour every SDK shares is in
[`sdks/SPEC.md`](../SPEC.md); the five SDKs (TypeScript, Go, Python, C#, Java) are held to it by one
conformance suite ([`tests/contract/sdk-conformance`](../../tests/contract/sdk-conformance/README.md)).

Package metadata: module `github.com/ortisun/sagat-octopus-orchestrator/sdks/go`, version `0.1.0`,
license `UNLICENSED` (as the repository and the TypeScript package: there is no license file yet),
repository `https://github.com/ortisun/sagat-octopus-orchestrator`, directory `sdks/go`.

## Install

Go 1.24 or newer.

```sh
go get github.com/ortisun/sagat-octopus-orchestrator/sdks/go@latest
```

The module lives in a subdirectory of the repository, so its release tags are `sdks/go/v0.1.0`
and so on. No tag has been published yet: until one is, use a `replace` directive or a `go.work`
that points at a checkout of the repository.

The import path ends in `/go` but the root package is called `sagat`, so import it with a name:

```go
import (
	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/workflow"
)
```

The generated protobuf code is committed (`internal/workerpb`): consumers need neither `protoc` nor a plugin.

## Start a run

```go
c, err := client.New("https://api.example.com/v1", "payments", sagat.StaticToken(token))
if err != nil { ... }

run, err := c.Start(ctx, "order-flow", map[string]any{"orderId": "o-1", "amount": "19.99"},
	client.WithIdempotencyKey("order-o-1"), client.WithCorrelationID("o-1"))
if err != nil {
	var e *sagat.Error
	if errors.As(err, &e) {
		log.Println(e.Code, e.TraceID, e.Remediation) // CONFLICT, 32 hex characters, what to do about it
	}
	return err
}
done, err := c.WaitForCompletion(ctx, run.RunID, client.WaitTimeout(30*time.Second))
```

`Start` takes a workflow id or the name of a workflow of the namespace. With no key the SDK generates a
random `Idempotency-Key` per call and reuses it for every retry of that call, so a lost answer never
starts two runs. `List` returns a page; `ListAll` is a Go iterator (`for run, err := range c.ListAll(...)`).

A token is a `sagat.TokenSource`: `sagat.StaticToken(s)`, or `sagat.TokenFunc(func(ctx) (string, error))`,
asked before every request so a rotated token is used at once. The tenant is never configured: it is the
token's.

## Register an activity

```go
host, err := worker.New("https://workers.example.com:443", "payments", "payments-worker", sagat.StaticToken(token))

host.RegisterActivity("payments.charge", worker.Typed(func(ctx context.Context, in ChargeInput) (Receipt, error) {
	task := worker.TaskFrom(ctx)           // run, node, attempt, idempotency key, trace, steps, deadline...
	// send task.IdempotencyKey to the provider: the effect happens once however often this is repeated
	if in.Amount == "" {
		return Receipt{}, worker.Permanent("INVALID_AMOUNT", "the order has no amount")
	}
	return charge(ctx, task.IdempotencyKey, in)
}))
host.RegisterCompensation("payments.refund", worker.Typed(refund))   // idempotent; tolerates an effect that never happened
host.RegisterReconciler("payments.lookup", worker.TypedReconciler(lookup)) // returns worker.Applied(out), NotApplied() or Unknown()

if err := host.Start(ctx); err != nil { ... }   // registers; returns once registered
defer host.Stop(graceCtx)                       // stops polling, waits for the tasks in flight, then cancels them
```

* The handler's `ctx` is cancelled with a cause (`context.Cause(ctx)`) and what it returns afterwards means
  something different for each:
  `worker.ErrRunCancelled` (the run was cancelled; you decide what to do; returning `ctx.Err()` fails the
  step as `CANCELLED`, not retryable), `worker.ErrLeaseLost` (the lease is gone; nothing you return is
  reported), `worker.ErrShutdown` (the host is stopping and the grace period passed; returning `ctx.Err()`
  reports nothing, the lease lapses and the step is leased again; any other end is reported as usual).
* The heartbeat runs until the report is made, not only until the handler returns.
* Fail on purpose with `worker.Retryable(code, msg)`, `worker.Permanent(code, msg)` or
  `worker.OutcomeUnknown(msg)` (the platform reconciles instead of retrying blindly). Any other error is
  reported as `ACTIVITY_ERROR`, retryable, with **only its Go type name** as the message, never its text.
  A panic is reported the same way and does not stop the host.
* `Task.Secrets(ctx)` resolves the step's secrets once per attempt, on first use; the values are wiped
  when the handler returns. They are credentials: do not log them.
* `Typed` decodes the input into your type (a document that cannot be decoded fails the step as
  `INPUT_INVALID`). Decoding into `any` turns numbers into `float64`; use `int64` fields, or take a
  `json.RawMessage` (`RegisterActivity` takes the raw handler), to keep integers exact.
* The host exposes the version the server agreed to (`host.ProtocolVersion()`) and runs at it.
* Metrics: `worker.WithEventHandler(func(worker.Event))` is called for registrations, polls, tasks taken,
  completed, failed, stale or dropped, heartbeats and reconnects.

## Build a workflow

```go
doc, err := workflow.New("order-flow").
	Description("charges and ships").
	Step("charge", "payments.charge",
		workflow.Retry(workflow.MaxAttempts(3), workflow.WithStrategy(workflow.Exponential), workflow.Jitter(true)),
		workflow.CompensateWith("payments.refund")).
	Sleep("pause", 2*time.Second).
	Step("ship", "shipping.ship").
	Build()                // map[string]any; JSON() is its RFC 8785 canonical text
```

The first mistake the builder can see (bad name, duplicate id, a parallel with one branch...) is
remembered and returned by `Build`, `JSON` and `Err` as a `*sagat.Error` with the code
`VALIDATION_FAILED`; the calls after it do nothing. The canonical text is RFC 8785 and byte-identical to the one the other
SDKs write for the same program (the `builders` scenario of the conformance suite checks it).

## Errors

Every failure is a `*sagat.Error`: `Code` (see `sdks/error-catalog.json`), `Message` (safe to log),
`HTTPStatus` or `GRPCStatus`, `Retryable`, `TraceID`, `Reason`, `RetryAfter`, `Remediation`. Use
`errors.As`, or `sagat.IsCode(err, sagat.CodeNotFound)`. A cancelled context is `CANCELLED` and wraps the
context's error, so `errors.Is(err, context.Canceled)` works. No token, payload or secret is ever put in
an error, a log or an event.

## Configuration

| Setting (SPEC section 1) | Go |
|---|---|
| `apiUrl`, `namespace`, `token` | `client.New(apiURL, namespace, token, ...)` |
| `grpcUrl`, `namespace`, `token`, worker name | `worker.New(grpcURL, namespace, name, token, ...)` |
| `timeout` (30 s) | `client.WithTimeout`, `worker.WithTimeout` |
| `retry` (4, 200 ms, 5 s) | `client.WithRetry(sagat.RetryPolicy{...})`, `worker.WithRetry`; a zero field keeps its default |
| `userAgent` suffix | `client.WithUserAgent` |
| TLS (roots, mTLS) | `client.WithTLSConfig`, `worker.WithTLSConfig` (`*tls.Config`); `client.WithHTTPClient` for anything else |
| `max_inflight` (10), pools, languages | `worker.WithMaxInflight`, `WithPools`, `WithLanguages` |
| heartbeat interval | `worker.WithHeartbeatInterval` (default: a third of the lease, 500 ms to 10 s) |
| logs | `WithLogger(*slog.Logger)`; silent by default |
| trace context | `client.WithTraceContext(func(ctx) string)` returns a W3C `traceparent` (an OpenTelemetry bridge is four lines, see its doc), or pass `client.WithTraceparent` per call. No OpenTelemetry dependency |
| interceptors, proxies, fault injection | `worker.WithDialOptions(grpc.WithChainUnaryInterceptor(...))`, `client.WithHTTPClient` |

Plain `http://` outside loopback is accepted and logs one warning at start.

## Versioning (SemVer)

The SDK follows [SemVer](https://semver.org). Before 1.0 a minor version may change the Go API; a patch
version never does. The **wire** contract does not move with the SDK: REST is additive within `/v1`, the
worker protocol is additive within `1.x`. A newer SDK against an older server runs the host at the
server's protocol version and treats a call the server does not have as `NOT_FOUND` with the reason
`UNSUPPORTED_BY_SERVER`; an older SDK ignores the fields it does not know (`Run.Raw` keeps them).

## Runtime matrix

What was **actually run** for this release, and what was not:

| | Tested here | Not run |
|---|---|---|
| Go | **1.24.2** (`go test -race ./...`, `go vet`, the conformance suite) | 1.25 and later, and anything older than 1.24 (`go.mod` says `go 1.24.0` because gRPC v1.80.0 requires it, so older toolchains cannot build the module) |
| OS / arch | **macOS, Darwin 25.3.0, arm64** | Linux, Windows, amd64 |
| Server | The platform at this commit (REST `/v1`, worker protocol 1.2), through the conformance suite, which plays 13 scenarios; protocol 1.0 is also covered by the unit tests of the host, 1.1 only by the registration probe of `protocol-negotiation` | Any other server version |
| TLS | Not run: every test and every scenario is plaintext on loopback; `https://` goes through `credentials.NewTLS` and `net/http` and is **not** covered by a test | mTLS, custom roots |
| Race detector | `-race` passes on the unit tests | -- |
| Vulnerability scan | Not run (`govulncheck` was not installed) | -- |

Dependencies, all at least two weeks older than this release: `google.golang.org/grpc` v1.80.0 (the
newest that builds with Go 1.24), `google.golang.org/protobuf` v1.36.11, and what `go.sum` pins.

## Development

```sh
cd sdks/go
go test -race ./...         # unit tests, offline: a fake REST server and a fake gRPC WorkerService, in process
go vet ./... && gofmt -l .  # gofmt prints nothing
scripts/generate.sh         # copies worker.proto and error-catalog.json from the repository, regenerates internal/workerpb
conformance/prepare.sh      # builds bin/runner (git-ignored)
```

* `TestSharedFilesAreTheRepositorys` fails when the copy of `worker.proto` or of the error catalog is behind
  the repository, and, where `protoc` and the plugins in `bin/` exist, when `internal/workerpb` is not what the
  proto generates. Inside a downloaded module (no repository around) it skips, saying so.
* `conformance/prepare.sh` is idempotent and builds offline when the Go module cache has the dependencies:
  it tries `GOPROXY=off` first and runs `go mod download` once only if a module is missing. There is no
  `vendor/` directory: it would put all of gRPC in the repository for one script.
* The conformance suite is run from the repository root:
  `SAGAT_SDK_LANGS=go cargo test -p sagat-tests --test sdk_conformance -- --nocapture`.
