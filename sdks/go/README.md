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

Package metadata: module `github.com/ortisun/sagat-octopus-orchestrator/sdks/go`, version `0.2.0`,
license `MIT` (the module source artifact includes `LICENSE`),
repository `https://github.com/ortisun/sagat-octopus-orchestrator`, directory `sdks/go`.

## Install

Go 1.25 or newer (gRPC v1.83 requires it).

```sh
go get github.com/ortisun/sagat-octopus-orchestrator/sdks/go@latest
```

The module lives in a subdirectory of the repository, so its release tags are `sdks/go/v0.2.0`
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

For a large run input, call `UploadRunInput(ctx, onePassReader, options)`, then pass its verified
`client.BlobDescriptor` to `StartWithBlob`. The upload is streamed once and never retried; the descriptor
is accepted only after source EOF and matching local size/SHA-256. `StartWithBlob` uses the same call options
and idempotency behavior as `Start`.

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
* At protocol 1.5, `worker.TaskFrom(ctx).Instance` is a typed copy of the lease identity with its
  ordered scope path and definition node. It is `nil` at protocol 1.4 or older; reports continue to
  use the original lease identity.
* For a collection body, `Task.Item` is the captured JSON item and `Task.Index` points to its stable
  input index. Both are absent from root and non-collection work; the handler input remains the run input.
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
SDKs write for the same program (the `builders` scenario of the conformance suite checks it). `JSON()` refuses, by value, a whole number beyond 2^53 - 1 that ECMAScript prints in digits (`VALIDATION_FAILED`: send it as a string): `9007199254740993`, `9007199254740992.0`, `1e20`, a `float64` `1e20` or an `int64` past 2^53 - 1 are all refused, while `1e21` and above and every fraction are numbers written as RFC 8785 writes them. No language but TypeScript's can print the nearest double and call it exact. `Build()` keeps the document. A `Flow` is read when `Build()` is called, not when it is joined: a step added to a branch or to a saga body after it was joined is part of the document, and a mistake in it is raised by `Build()`; an id is checked when its node is added to its flow and again by `Build()` over the whole document. A JSON text with a lone surrogate escape (`\ud800`) is refused on the way out, as invalid UTF-8 is.

## Errors

Every failure is a `*sagat.Error`: `Code` (see `sdks/error-catalog.json`), `Message` (safe to log),
`HTTPStatus` or `GRPCStatus`, `Retryable`, `TraceID`, `Reason`, `RetryAfter`, `Remediation`. Use
`errors.As`, or `sagat.IsCode(err, sagat.CodeNotFound)`. A cancelled context is `CANCELLED` and wraps the
context's error, so `errors.Is(err, context.Canceled)` works. No token, payload or secret is ever put in
an error, a log or an event.

**Retries are by code, the same in every SDK** (SPEC section 5): a call is repeated, with the same idempotency key and
body, when the error's code is `TRANSPORT`, `UNAVAILABLE`, `RATE_LIMITED` or `INTERNAL` *and* the error says it may be
(the server's own `retryable`, else the catalog's default). The server's `408` is `UNAVAILABLE` and is repeated; a `429`
that is a quota (`QUOTA_EXCEEDED`) never is; a body that claims `retryable: true` on any other code is not believed. The
same function decides for the REST client, for the registration and for the reports of the worker host.

**A token source that fails is repeated too**, because nothing was sent: in every call the SDK repeats, in the registration and in the
report of a result (a result that was computed is not lost to a hiccup of the identity provider). When the attempts are
used up the error is `UNAUTHENTICATED` with the reason `TOKEN_PROVIDER_FAILED`; its message never repeats what the source
said, and `errors.Is` reaches what it returned. That it was the source is something the SDK keeps in a type of its own
(`internal/tokenfail`): a `reason` in a server's answer, even that one, never makes a call repeatable.

## Safety

* **No credential is printed.** A `sagat.StaticToken`, a `*client.Client`, a `*worker.Host` and a `*worker.Secrets` print as
  `<redacted>` (or as the host and the namespace only) with every `fmt` verb, as JSON and in `log/slog`; a URL's
  userinfo and query are never printed. A panic in a token source, a logger, an event handler or a trace hook is
  recovered and never repeated: the heartbeat and the host go on.
* **What the server says is bounded.** An answer is read up to 32 MiB (then `PROTOCOL`); a wait the server asks for
  (`Retry-After`, `retry-after-ms`, the pause after an empty poll) is capped at 60 s; the `message` of a server error
  is cut to 512 characters and its `reason` to 128; a `traceId` that is not 32 hexadecimal characters is not
  repeated; a poll answer with more tasks than the host asked for runs only the first ones and logs how many were
  left to lapse. A server that registers a worker and refuses it at once is not hammered: the second refusal in a
  row waits for the backoff first.
* **Secrets end with the handler.** `Task.Secrets` and the methods of a `*Secrets` kept past the end of the handler return `VALIDATION_FAILED` ("the attempt is over"); the values are wiped.
* **Ids and names in a URL path.** A run id is a UUID (`VALIDATION_FAILED` otherwise, before any request); a signal
  name of `""`, `"."` or `".."` is refused; any other name is percent-encoded (`/` is `%2F`).
* A redirect is never followed (see Configuration).

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
| trace context | `otelbridge.New(...).TraceContext` returns the ambient W3C `traceparent`, or pass `client.WithTraceparent` per call; the same bridge's `OnEvent` emits worker spans and metrics |
| interceptors, proxies, fault injection | `worker.WithDialOptions(grpc.WithChainUnaryInterceptor(...))`, `client.WithHTTPClient` |

Plain `http://` outside loopback is accepted and logs one warning at start. A redirect is never followed (the request carries a bearer token, the namespace and an idempotency key): a `3xx` is a `PROTOCOL` error, "the server redirected the call", whatever its target; a client you pass to `WithHTTPClient` is copied and its `CheckRedirect` replaced, and its `Transport` must not follow redirects either.

## TLS

| Option | Client | Worker host | Tested |
|---|---|---|---|
| `https://` with the system roots (the default) | yes | yes | only that an untrusted certificate is refused |
| Trust roots of your own: `tls.Config{RootCAs: pool}` | `client.WithTLSConfig` | `worker.WithTLSConfig` | yes, both channels |
| Client certificate (mTLS): `tls.Config{Certificates: ...}` | `client.WithTLSConfig` | `worker.WithTLSConfig` | yes, both channels; refusal without a certificate and with an unknown one |
| Name to verify the server certificate against: `tls.Config{ServerName: ...}` | `client.WithTLSConfig` | `worker.WithTLSConfig` | yes, both channels |
| Anything else of `*tls.Config` (`MinVersion`, `GetClientCertificate`...) | passed through | passed through | no |
| Your own `http.Client` / transport | `client.WithHTTPClient` (then `WithTLSConfig` is ignored) | n/a (`worker.WithDialOptions` for the channel) | no |

**What you hand the SDK is yours.** `WithTLSConfig` and `WithHTTPClient` (and `worker.WithDialOptions`) give the SDK a
`*tls.Config` / `*http.Client` / gRPC dial options that you built, so what they do with certificate and host-name
verification is your choice, not the SDK's: the SDK never builds such an object with verification off, has no setting
that turns it off, and does not look inside the ones you give it (a `*tls.Config{InsecureSkipVerify: true}` that you pass
is honoured, as Go's own packages would). The only things the SDK adds to a client of yours are a copy of it and a
`CheckRedirect` that never follows a redirect.

The configuration is copied when the client or the host is made. Without one, the default is the system
roots and TLS 1.2 or newer. **Certificate verification is never switched off**: there is no option for it, no
`InsecureSkipVerify` anywhere in the library (a test keeps those words out of the sources), and a server
certificate that is not trusted, or is for another name, is a `TRANSPORT` error (`errors.As` reaches a
`*tls.CertificateVerificationError` or `x509.HostnameError` as the cause). Plain `http://` is accepted for
development (any other scheme in `grpcURL` or `apiURL` is `VALIDATION_FAILED`, never plaintext); outside loopback it logs one warning at start, and `https://` logs none.

## Versioning (SemVer)

The SDK follows [SemVer](https://semver.org). Before 1.0 a minor version may change the Go API; a patch
version never does. The **wire** contract does not move with the SDK: REST is additive within `/v1`, the
worker protocol is additive within `1.x`. A newer SDK against an older server runs the host at the
server's protocol version and treats a call the server does not have (a `501`, a `404` that says `ROUTE_NOT_FOUND`, a `405` that says
`METHOD_NOT_ALLOWED`, or a `404` that is not the platform's error envelope) as `NOT_FOUND` with the reason
`UNSUPPORTED_BY_SERVER`; an older SDK ignores the fields it does not know (`Run.Raw` keeps them).
This SDK offers worker protocol `1.5`; when `1.4` or older is negotiated, lease-bound requests omit
scoped execution identity and remain root compatible.

## Runtime matrix

What was **actually run** for this release, and what was not:

| | Tested here | Not run |
|---|---|---|
| Go | **1.25.14** (`go test -race ./...`, `go vet`, `gofmt`, `govulncheck`, the conformance suite) | Every other Go: `go.mod` says `go 1.25.0` because gRPC v1.83 requires it, so **1.24 and older cannot build the module** (the first release of this SDK was run on 1.24.2, before that change); 1.26 was not run as the SDK's toolchain |
| OS / arch | **macOS, Darwin 25.3.0, arm64** | Linux, Windows, amd64 |
| Server | The platform at this commit (REST `/v1`, worker protocol 1.5), through the shared conformance suite; protocol 1.0 is also covered by the unit tests of the host, 1.1 by the registration probe of `protocol-negotiation` | Any other server version |
| TLS | Tested with throw-away certificates (ECDSA, generated in the tests) against an in-process TLS server, on both channels, on loopback: an untrusted server certificate is refused as `TRANSPORT` and never reaches the handler; a configured CA makes the same server work (a task goes through end to end on the worker channel); a certificate for another name is refused; with mTLS the server sees the client certificate, and refuses a client with none or with one of another authority; no warning is logged for `https://`. See "TLS" below for what is **not** covered | A trusted server certificate from the *system* roots (only the untrusted case is tested), revocation, TLS versions and cipher suites, certificate rotation on a running host |
| Race detector | `-race` passes on the unit tests | -- |
| Vulnerability scan | `govulncheck` v1.7.0 on `./...` with Go 1.25.14 and the vulnerability database of 2026-10-08: 9 reachable findings, all from 2026-10-08, and 4 in packages that are imported but not called | See "Known findings" below |

Dependencies, all at least two weeks older than this release (checked against the proxy): `google.golang.org/grpc`
v1.83.2 (2026-08-25), `google.golang.org/protobuf` v1.36.11, `golang.org/x/net` v0.58.0 (2026-08-12), `golang.org/x/sys`
v0.47.0, `golang.org/x/text` v0.41.0, OpenTelemetry Go v1.46.0 for the opt-in `otelbridge` package and its real SDK tests, and what `go.sum` pins. No provider or exporter is configured by the SDK.

### Known findings (`govulncheck ./...`, database of 2026-10-08, re-run on 2026-10-10 with the same result)

* **Nothing left from the advisories that an independent review found reachable on gRPC v1.80.0 and `x/net` v0.49.0**: GO-2026-6348 (heap
  exhaustion through HTTP/2 DATA fragmentation, reachable through `ResolveSecrets`) is fixed in gRPC 1.83.1,
  GO-2026-6061 in 1.82.1, GO-2026-4918 in `x/net` 0.53.0, GO-2026-6443 (server panic, not called) in 1.83.2.
* **9 findings remain, all published on 2026-10-08, two days ago.** They are advisories of `net/http`, `crypto/tls` and
  `net/textproto` of the Go standard library (fixed in Go 1.26.9, released 2026-10-02) and, for five of them
  (GO-2026-6603, 6610, 6611, 6612, 6617), of the same HTTP/2 code in `golang.org/x/net` (fixed in v0.60.0, released
  2026-10-08). Both fixes are younger than the two-week rule, and `x/net` v0.59.0 and later require Go 1.26, so
  taking them would raise the minimum Go of the SDK for nothing `govulncheck` can see (v0.59.0 does not fix any of
  these). The standard-library findings are those of the toolchain that compiled the SDK, not of the SDK: whoever
  builds with Go 1.26.9 or newer does not have the standard-library ones. Judging by their titles, most concern HTTP/2 *servers* (HPACK encoder race, flow-control refund, Trailer memory,
CONNECT desynchronization), which the SDK is not; 6605 (HTTP/1 client desynchronization after a CONNECT rejection), 6607
(`crypto/tls` ECH), 6608 (MIME headers), 6610 (an HTTP/2 transport accepting malformed framing headers) and 6611
(CPU from repeated initial window changes) can concern a client, and I did not establish whether the SDK is exposed to
them. The call traces `govulncheck` prints go through interface dispatch (for example `rand.Read` to
`http2.ConnectionError.Error`) and are not real call paths.
* 4 more advisories are in packages the SDK imports but does not call (`html/template`, `os` on Windows, `net/http` Range headers).

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
  `SAGAT_SDK_LANGS=go cargo test -p sagat-tests --test sdk_conformance -- --ignored --nocapture`.

## Task transport

`worker.WithTransport(worker.TransportAuto)` is the default. It uses finite-credit task streams
on negotiated protocol 1.4 and unary polling on older servers. `TransportPoll` selects polling
for comparisons; `TransportStream` requires 1.4. Only an UNIMPLEMENTED stream triggers automatic
poll fallback for that registration. Other failures keep their canonical classification.

Each receive asks for available handler slots, at most 64, with an aggregate 8 MiB input bound.
Frames dispatch immediately; completed handlers do not replenish that receive's credit.
Tasks already received continue heartbeating/reporting after a receive failure. Stop cancels
only the pending receive promptly, then follows the existing handler drain. A lost response
can leave a committed lease until expiry; critical effects still need idempotency.

Streaming receive failures emit `worker.EventReceiveFailed` (`receive.failed`) with the canonical `Code` and `Retryable` fields, without remote error text or payloads. Explicit streaming remains refused if re-registration negotiates an older minor; it never silently switches to polling.
