# Runbook: the SDKs (feature 006)

What a developer or an operator needs to use the five SDKs (TypeScript, Go, Python, C#, Java) against a
platform, to upgrade either side, to read their errors and to release a new version. The behaviour of the
SDKs is `sdks/SPEC.md`; the decision behind versioning and testing is
[ADR-0015](../adr/ADR-0015-sdks-meet-the-server-by-minor-version-and-are-held-to-one-conformance-suite.md).
The commands below were run against this repository; what is **not** built is at the end.

## 1. What the SDKs are, and are not

A **client** (start, get, list, cancel, operate, signal, publish an event, read the journal, wait for a run),
a **worker host** (register activities, compensations and reconcilers; receive tasks, heartbeat, cancel, report) and
**builders** of the workflow document. They keep no journal and make no retry decision of their own about
your workflow: the platform owns every state and every timer. They retry a *call* only when repeating it is
safe (§4). External effects stay at-least-once: the SDK hands the activity an idempotency key and a
reconciliation hook, it cannot make a provider exactly-once.

## 2. Upgrading

Within protocol **1**, either side can go first.

| You upgrade | What happens | Check |
|---|---|---|
| The server, SDKs older | A worker offers the minor it speaks (`1`, `1.1`); the server answers the same and serves it at that minor. The forms of every accepted minor keep working. | The worker's `protocolVersion` (every SDK exposes it after `start`) equals what it offered. |
| The SDK, server older | The SDK offers its newest (`1.4`); an older server answers its own minor, and the SDK uses only what that minor has (against `1` no step identity in the heartbeat and no `ResolveSecrets`). | `protocolVersion` is the server's minor, not the SDK's. |
| Either, to another **major** | The server refuses with `UNSUPPORTED_PROTOCOL` (gRPC `INVALID_ARGUMENT`, trailer `x-sagat-error`); the SDK raises the canonical error and does not retry. | Upgrade or downgrade the SDK to the server's major. |

Order for a fleet: no order is required inside a major. Roll workers one at a time; a worker that stops
gets `stop()`, finishes the tasks in flight (30 s by default), and anything left keeps its lease until it
lapses and is leased again — the step is retried, not lost.

A SDK version is **SemVer**: `0.x` may change its API in a minor (and says so in its changelog); from `1.0`
the public API is SemVer. The SDK version and the protocol version are independent numbers.

## 3. Reading an error

Every SDK raises one canonical error with `code`, `status`, `retryable`, `traceId`, `reason`,
`retryAfterMs` and a `remediation` sentence (`sdks/error-catalog.json` is the list, and each SDK embeds an
identical copy). Quote the `traceId`: it finds the request in the platform's logs and traces. An error never
holds a payload, a token or a secret.

| Code | Usually means | Do |
|---|---|---|
| `UNAUTHENTICATED` | The token was refused (issuer, audience, tenant claim, expiry; a token lives at most one hour) | Fix the token source; a function that returns a fresh token is retried once |
| `FORBIDDEN` | The identity lacks the role or permission | Ask an administrator |
| `NOT_FOUND` | No such run or workflow **in this tenant and namespace** (another tenant's is also NOT_FOUND, deliberately) | Check ids and the `X-Octopus-Namespace` |
| `CONFLICT` with reason `IDEMPOTENCY_KEY_REUSED` | The key was used for a different request | New key for a new request |
| `CONFLICT` with reason `STATE_CONFLICT` | The run is not in a state that allows it (cancel of a finished run, a signal nobody waits for) | Read the run, decide |
| `VALIDATION_FAILED` | A field is invalid; `reason` names it. Also raised by the SDK itself before anything is sent | Fix the input |
| `RATE_LIMITED` / `QUOTA_EXCEEDED` | Too fast (wait `retryAfterMs`) / a quota is exhausted (not retryable) | Slow down / free capacity or raise the quota |
| `UNAVAILABLE`, `TRANSPORT` | No usable answer; retried by the SDK as far as its policy goes (4 attempts for a call, backing off for a worker) | Check the URL, the network, TLS |
| `NOT_BOUND` (worker) | The worker identity has no binding for the namespace and activity | `PUT /namespaces/{id}/worker-bindings/{subject}` ([security runbook](security-governance.md)) |
| `STALE_ATTEMPT` (worker) | The lease is gone (it lapsed, was reclaimed, or the run ended) | Nothing: the host stops the activity and drops its result; the step is retried |
| `SECRET_NOT_USABLE` (worker) | A secret the step declares is missing or its publisher may no longer use it | Check the secret and its access list |
| `PAYLOAD_TOO_LARGE` (worker) | An output over 256 KiB (inline limit) | Return less; the step is failed for good |

## 4. Retries and idempotency, in one place

- A `GET` is repeated on a transport failure, `503`/`502`/`504`, `429` and a retryable `500`.
- A `POST` is repeated only if it carries an idempotency key or an event id. The SDK **generates a key per
  call** when you give none and reuses it for every retry of that call, so a lost answer to `start` never
  makes a second run. A repeat of `start` with your key returns the run that exists.
- Never repeated: any `4xx` other than `429`, `STALE_ATTEMPT`, `IDEMPOTENCY_KEY_REUSED`.
- A worker's report (`CompleteStep`, `FailStep`) is repeated with the same key (`done-<taskId>` /
  `failed-<taskId>`), up to five times; `already_applied` is a success.
- A failure of **your activity** is not retried by the SDK; the node's `retry` policy in the workflow is, by
  the platform.

## 5. Looking at a worker

```sh
# What the platform counted of the worker protocol (engine admin listener), by method and code:
curl -s http://<engine-admin>/metrics | grep '^grpc_requests_total'
# explicit polling makes about two PollTasks a second (the server asks for 500 ms); a few `aborted` heartbeats
# are workers whose lease lapsed or whose heartbeat crossed their own completion
```

A worker that registers but never receives a task: is the workflow's activity name among the activities
it registered, is the identity bound to it (`NOT_BOUND` otherwise), is `maxInflight` reached? A worker whose
tasks are leased again and again: its heartbeat is not reaching the server (network, a token that expired
mid-task, a protocol-1 worker against a server older than the fix in `a4107cd`).

## 6. Running the conformance suite

```sh
cargo test -p sagat-tests --test sdk_conformance -- --ignored --nocapture          # five languages, every scenario (needs Docker)
SAGAT_SDK_LANGS=go,python cargo test -p sagat-tests --test sdk_conformance -- --ignored   # some languages
SAGAT_SDK_SCENARIO=lease-lost SAGAT_SDK_REUSE=1 cargo test -p sagat-tests --test sdk_conformance -- --ignored
cargo test -p sagat-tests --test e2e_sdk_multilanguage --test fault_sdk_multilanguage -- --ignored               # the user stories and the faults
cargo test -p sagat-tests --test perf_sdk_multilanguage --release -- --ignored --nocapture   # the benchmark
```

All of these are `#[ignore]` in the workspace test run: they need Node, Python, Go, .NET, a JDK and Docker. The `sdks` job of CI (`.github/workflows/ci.yml`) installs the toolchains and runs the first, second and fourth lines; the benchmark is run by hand on a quiet machine.

A red `FAIL scenario/language: differs from expected/… at /pointer` names where an SDK's result differs from
the golden file; the SDK's raw and normalized output are in `target/sdk-conformance/<scenario>/`. A golden is
changed (`SAGAT_SDK_BLESS=1`) only after reading what it now says against what the scenario claims.

## 7. Releasing an SDK

1. All scenarios pass in the SDK, three runs in a row, and its unit tests pass.
2. Its README's runtime matrix says which runtimes were run and which were only declared.
3. Bump the version (SemVer; `0.x` says so), update its changelog, check that the copies of `worker.proto` and
   `error-catalog.json` it embeds are current (its tests fail when they are not).
4. Follow [SDK packaging and publication](sdk-packaging.md). The packaging and distribution workflow validates clean consumers; public publication requires the confirmed registry identities, credentials and any registry-required signing.

## Task transport and backpressure

Protocol 1.4 adds `StreamTasks`. All five worker hosts accept `auto` (default), `poll` and `stream`.
Auto uses streaming when negotiated protocol is at least 1.4 and polling for older servers. Only a
raw gRPC `UNIMPLEMENTED` permits automatic fallback during the current registration; authorization
or transport errors do not silently downgrade. Explicit streaming on an older minor is refused.
Consult each SDK README for its language's option syntax.

Each receive spends finite credit: available local slots, at most 64 tasks, with at most 8 MiB of
serialized inputs across its frames. Completing a handler does not refill that receive's credit.
Frames are dispatched immediately. A receive error leaves earlier delivered handlers, heartbeats
and result reporting alive. Stop cancels receiving promptly and retains the existing handler drain.
The server window is five seconds; clients cap it at six seconds or a shorter configured request
timeout. An underfilled normal window gives a positive idle retry hint.

Stream admission is bounded per process (32), tenant (8) and subject within tenant (2). These are
process-local bounds, not a distributed subscription quota. Short database claims share discovery
admission with polling and preserve the heartbeat/result reserve. The implementation still scans
PostgreSQL for work while a stream waits; fewer receive RPCs do not imply fewer database queries.

Monitor `worker_task_streams_open`, `worker_task_streams_refused_total{scope}`,
`worker_task_streams_closed_total{reason}`, `worker_task_stream_duration_seconds`,
`worker_task_stream_tasks_total` and `worker_task_stream_claims_total{outcome}` alongside
`grpc_requests_total{method="StreamTasks",code}`. Initial success only opens a response; body failure
is counted separately. A stalled response that cannot finish within its receive deadline closes
its HTTP/2 connection through an independent watchdog. Other RPCs sharing that connection may
need to reconnect; heartbeat and result retries retain their normal lease and idempotency rules. Do not add tenant, worker or task identifiers as metric labels.

A repeated refusal requires checking the stream caps, ordinary admission budget, current membership,
worker binding, drain state and session limit. Do not raise limits to mask a leaked or idle client.
Use the comparative benchmark's recorded hardware and measurements before choosing transport for a
fleet; no universal throughput gain is promised.

## Distribution and validation boundaries

The repository and SDK packages use MIT, selected by the owner on 2026-10-11. Reproducible packaging,
clean-consumer smoke checks, release notes and the distribution workflow exist. Public registry
publication still depends on registry ownership, credentials and required signing configuration.

The common scenarios inject transport/session failures. `e2e_durable_execution::stream_recovery`
also stops and restarts the real engine runtime against the same PostgreSQL database: a valid
streamed lease still completes once; an abandoned lease expires and its old result is fenced after
reclaim. This is an engine-runtime restart test, not an operating-system process crash test.
