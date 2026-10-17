# SDK specification (v1)

The normative description of what every Sagat Octopus Orchestrator SDK does — TypeScript, Go, Python,
C#/.NET and Java — so that the five behave the same where it matters and differ only in idiom
(`FR-006-001..008`, ADR-0015). A statement with **MUST** is checked by the conformance suite
(`tests/contract/sdk-conformance/`) unless it says otherwise. Naming below is language neutral; each
SDK uses its own conventions (`startRun` / `StartRun` / `start_run` / `StartAsync`).

An SDK is three things in one package and **nothing else**:

1. a **client** (REST): start, read, list, cancel and signal runs, publish events;
2. a **worker host** (gRPC, `worker.proto`): register activities, take leases, heartbeat, complete or
   fail, with cancellation and the saga/reconciliation task kinds;
3. **workflow builders**: typed, side-effect-free constructors of the workflow IR
   (`workflow.schema.json`) with the primitives `step`, `sleep`, `waitForEvent`, `invokeChild`,
   `parallel`, `saga`, and the `compensateWith` / `reconcileWith` of a step.

An SDK is **not** a workflow runtime. It does not execute workflow code, replay anything, keep a
journal, or decide a transition: the server owns every state and every retry decision (ADR-0001,
ADR-0002). The builders only write a document the server validates and versions.

## 1. Configuration and credentials

| Setting | Meaning | Default |
|---|---|---|
| `apiUrl` | Base URL of the REST API including `/v1`, e.g. `https://api.example/v1` | required |
| `grpcUrl` | URL of the worker service, `https://host:port` (`http://` only for loopback or development; any other scheme is `VALIDATION_FAILED`, never plaintext) | required for a worker |
| `namespace` | Slug of the namespace every call is made in | required |
| `token` | A bearer token, **or** a provider called before every request that returns a fresh one | required |
| `timeout` | Deadline of one REST call or one unary RPC | 30 s |
| `retry` | `maxAttempts`, `initialDelay`, `maxDelay` (see §5) | 4, 200 ms, 5 s |
| `userAgent` suffix | appended to `sagat-sdk-<language>/<version>` | none |
| TLS | trust roots; client certificate for mTLS | system roots |

- A **token provider** (a function) is called for every request so that a rotated token is used at
  once; a static token is the simple case. The SDK never writes a token to a log, an error message or a
  metric label.
- The tenant is **never** configured: it is the token's (`tenant` claim). A tenant is never read from a
  header or a body (ADR-0005).
- Plain `http://` and `grpc` without TLS outside loopback is accepted but logs one warning at start. The
  warning names the host and nothing else (a URL can carry a credential in its userinfo or query).
- **An HTTP redirect is never followed.** A `Location` can point anywhere, and the request carries a bearer
  token, the namespace and an idempotency key; a `3xx` is raised as `PROTOCOL` ("the server redirected the
  call"), whatever its target. A transport that follows redirects by default is configured not to.
- The SDK has no setting of its own, and no code, that turns certificate or host-name verification off. Where an
  ecosystem lets the caller hand the SDK its own transport object (Go's `*http.Client` and `*tls.Config`, .NET's
  `HttpMessageHandler`, Java's `HttpTransport`), what that object does is the caller's choice and the README says
  so; the SDK never builds such an object with verification off. A server name
  other than the URL's host can be given for verification (`tls.serverName`); it is verified, not skipped.

## 2. Canonical errors

Every failure an SDK raises to its caller is **one** type, `SagatError`, with these fields (the
language's native shape: exception, error value, result):

| Field | Meaning |
|---|---|
| `code` | A stable machine code from the catalog below |
| `message` | Safe, human-readable. Never a payload, a token, a secret or another tenant's data |
| `status` | The HTTP status, or the gRPC status name; `null` for a failure before any answer |
| `retryable` | Whether the same call may be made again as it is. From the server's own `retryable` when it sent one |
| `traceId` | 32 lowercase hex characters from `X-Trace-Id` / the error envelope; empty when the server did not answer |
| `reason` | The server's stable sub-reason (`details.reason`, or the gRPC `x-sagat-error` value), if any |
| `retryAfterMs` | From `Retry-After` / `retry-after-ms`, if any |
| `remediation` | What to do about it: the fixed sentence of the catalog for `code` |

The catalog is `sdks/error-catalog.json`; each SDK embeds a copy and a test proves it is equal to the
file. Codes:

- from the REST envelope: `BAD_REQUEST`, `UNAUTHENTICATED`, `FORBIDDEN`, `NOT_FOUND`, `CONFLICT`,
  `VALIDATION_FAILED`, `RATE_LIMITED`, `QUOTA_EXCEEDED`, `UNAVAILABLE`, `INTERNAL`;
- from the worker protocol: `STALE_ATTEMPT` (gRPC `ABORTED`), `IDEMPOTENCY_KEY_REUSED`, `NOT_BOUND`,
  `SECRET_NOT_USABLE` (both gRPC `PERMISSION_DENIED`, told apart by `x-sagat-error`),
  `UNSUPPORTED_PROTOCOL` (registration refused the version);
- raised by the SDK itself: `TRANSPORT` (no answer: connection refused, reset, timed out),
  `CANCELLED` (the caller's own cancellation), `PROTOCOL` (an answer that does not match the
  contract), `PAYLOAD_TOO_LARGE` (an output over 256 KiB, before it is sent).

gRPC mapping (the trailer `x-sagat-error` wins when present): `UNAUTHENTICATED` → `UNAUTHENTICATED`;
`PERMISSION_DENIED` → `NOT_BOUND` / `SECRET_NOT_USABLE` / `FORBIDDEN`; `INVALID_ARGUMENT` →
`VALIDATION_FAILED` (`UNSUPPORTED_PROTOCOL` when `x-sagat-error` says so: `RegisterWorker` refused the
version); `ABORTED` → `STALE_ATTEMPT` or `IDEMPOTENCY_KEY_REUSED`;
`RESOURCE_EXHAUSTED` → `RATE_LIMITED` (or `QUOTA_EXCEEDED` when the trailer says so); `UNAVAILABLE`
and `DEADLINE_EXCEEDED` → `UNAVAILABLE` / `TRANSPORT`; `NOT_FOUND` → `NOT_FOUND`; anything else →
`INTERNAL`.

## 3. Client

All calls are `<apiUrl><path>` with `Authorization: Bearer <token>`, `Accept: application/json`,
`X-Octopus-Namespace: <namespace>`, `User-Agent`, and — when there is one — `traceparent`. A body is
`application/json`. Unknown fields in an answer **MUST** be ignored (a newer server).

| Operation | Request | Success |
|---|---|---|
| `start(workflowId, input, options)` | `POST /runs` with `{workflowId, versionId?, input, correlationId?, deadline?}`, header `Idempotency-Key` | `202` Run |
| `uploadRunInput(onePassSource, options)` | `POST /run-input-blobs`, raw streaming body; optional `X-Octopus-Blob-Size` and `X-Octopus-Blob-Sha256` preconditions | `201` BlobDescriptor |
| `startWithBlob(workflowId, descriptor, options)` | `POST /runs` with `{workflowId, versionId?, inputBlobRef: descriptor.id, correlationId?, deadline?}`, header `Idempotency-Key` | `202` Run |
| `get(runId)` | `GET /runs/{runId}` | `200` Run |
| `list(options)` | `GET /runs?workflowId=&status=&limit=&cursor=` | `200` `{items, nextCursor}`; `listAll` pages for the caller |
| `cancel(runId, reason, options)` | `POST /runs/{runId}/operations` with `{action: "CANCEL", reason, expectedRunVersion?}`, `Idempotency-Key` | `202` OperationResult |
| `signal(runId, name, payload, options)` | `POST /runs/{runId}/signals/{name}` with the payload object, `Idempotency-Key` | `202` `{deliveredTo}` |
| `publishEvent(event)` | `POST /events` with `{eventId, type, data, subject?, occurredAt?, correlationKey?, dedupeTtlSeconds?}` | `202` `{eventId, source, status: ACCEPTED \| DUPLICATE}` |
| `waitForCompletion(runId, options)` | polls `get` every `pollInterval` (default 500 ms, ±20 % jitter) until the status is terminal or `timeout` | the final Run |

Also exposed, thin: `operate(runId, action, reason, options)` for `PAUSE`, `RESUME` and the saga
decisions, `events(runId, cursor)` for the journal. Terminal statuses: `SUCCEEDED`, `FAILED`,
`CANCELLED`, `TIMED_OUT`, `COMPENSATED`, `PARTIAL`, `COMPENSATION_FAILED`.

Rules:

- `Idempotency-Key` is 8 to 160 visible ASCII characters (0x21 to 0x7e: no space, no control character). The caller may give one; when it does not,
  the SDK generates a random one **per call** (UUID) and reuses it for every retry of that call. Two
  separate calls with no key are two runs.
- `publishEvent`'s `eventId` works the same way: the caller's, or generated per call and kept across
  retries, so an event is admitted once per window and a retry is answered `DUPLICATE`.
- `start` with a `deadline` sends an RFC 3339 instant in UTC (`...Z`); the SDK **MUST NOT** send a local
  offset it computed. A `deadline` option as a duration is converted by the SDK from the clock at the
  call, once, before the first attempt.
- Run, Page and the other answers are returned as typed values **and** keep the raw JSON (`raw`) so a
  field added by a newer server is reachable.
- Pagination: `list` returns `nextCursor` as given; `listAll` follows it until it is null.
- A run-input upload consumes its source once while measuring size and SHA-256. It has no automatic
  retry, even after authentication failure: no second stream exists to send. A successful answer is
  accepted only after the source reached EOF and its descriptor exactly matches the locally measured
  bytes. The descriptor is opaque and scoped by the server to its tenant, namespace and uploader.
  `startWithBlob` sends only its id and uses the normal run-start idempotency key; `input` and
  `inputBlobRef` are never sent together. Uploading alone does not start a run.

## 4. Trace context, correlation, deadline

- **traceparent.** An explicit `traceparent` option wins; otherwise the SDK uses the ambient trace
  context of its ecosystem when it has one (C#: `Activity.Current`; Java/Go/Python/TypeScript: OpenTelemetry
  context if the application installed the API, through an optional hook `traceContext()` the caller
  can set); otherwise none is sent and the server makes one. The value is sent as the `traceparent`
  header, validated as `00-<32 hex>-<16 hex>-<2 hex>` first (a malformed one is a `BAD_REQUEST` raised by the
  SDK, not sent).
- The run started that way has `traceId` equal to the trace id of that header; the worker's `TaskLease`
  carries `traceparent` of the run, and the SDK exposes it on the activity context and sends it back on
  `Heartbeat`, `CompleteStep`, `FailStep` and `ResolveSecrets`.
- **correlationId** is the `start` option of that name, sent as `correlationId` in the body.
- **idempotency key** — §3 for the client; for a worker the key of a report is `done-<taskId>` /
  `failed-<taskId>`, stable across retries of the report (8–160 ASCII).
- **deadline** — a run's `deadline` is the client option above. An activity's deadline is the expiry of
  its lease, `expiresAtEpochMs`, extended by every heartbeat (§6); the context exposes it as an instant
  and cancels the activity's cancellation token when the lease is lost.

## 5. Retries — only when safe

An SDK retries a call **only** when repeating it cannot do harm, with exponential backoff and full
jitter (`min(maxDelay, initialDelay * 2^n)` scaled by a random factor in `[0.5, 1.0]`), up to
`maxAttempts` total attempts (default 4), honouring `Retry-After` / `retry-after-ms` when larger:

| Call | Retried on |
|---|---|
| `GET` (REST) | `TRANSPORT`, `UNAVAILABLE`, `RATE_LIMITED`, `INTERNAL` with `retryable` true |
| `POST` that carries an idempotency key or event id | the same; **never** after any other answer |
| `uploadRunInput` | never: its one-pass body is not replayable |
| worker `PollTasks`, `Heartbeat` | `UNAVAILABLE`, `RESOURCE_EXHAUSTED`, transport failures |
| worker `CompleteStep`, `FailStep` | the same, with the **same** idempotency key and body; at most 5 times |
| `RegisterWorker` | the same; on `UNAUTHENTICATED` once, with a fresh token |

The rule is by **code**, the same in every SDK: a call is repeated when the error's code is `TRANSPORT`,
`UNAVAILABLE`, `RATE_LIMITED` or `INTERNAL` *and* it says it may be. The server's `408` is `UNAVAILABLE` (retryable),
its `429` is `RATE_LIMITED` or, when a quota is exhausted, `QUOTA_EXCEEDED` (never repeated). A body that claims
`retryable: true` on any other `4xx` is not believed.

**A token provider that fails is repeated too** — before anything is sent, so repeating cannot do harm — in every call
the SDK repeats and in the report of a result (a result that was computed is not lost to a hiccup of the identity
provider): the error is `UNAUTHENTICATED` with the reason `TOKEN_PROVIDER_FAILED` once the attempts are used up. That
it is the provider that failed is something the SDK knows and keeps privately; a `reason` in a server's answer never
makes a call repeatable.

**Never** retried: any `4xx` other than `408` and `429`; `STALE_ATTEMPT` (stop: the lease is gone, do not retry
the report); `IDEMPOTENCY_KEY_REUSED`; `VALIDATION_FAILED`; a call with no key and a non-idempotent
effect. The SDK never retries a failure of *the user's activity code*; that is the server's retry
policy (the node's `retry`).

## 6. Worker host

### 6.1 Registration and negotiation

`register` sends `RegisterWorkerRequest{worker_name, namespace, capabilities{languages, activities,
pools, max_inflight, protocol_version}}`:

- `languages`: one entry naming the SDK's language (`typescript`, `go`, `python`, `csharp`, `java`).
- `activities`: every activity name with a handler (forward, compensation and reconciliation alike),
  1 to 256.
- `max_inflight`: the host's concurrency limit, 1 to 1000 (default 10).
- `protocol_version`: the newest minor version of protocol 1 the SDK speaks — a constant of the SDK,
  `1.4` for SDK 0.1 (`1` is `1.0`). The server answers `accepted_protocol_version`: the newest both
  speak (the worker's own if it is not newer than the server's, the server's otherwise). The host **MUST**
  use only the features of the accepted version: against `1` it sends `Heartbeat` without the step
  identity (`run_id`, `step_id`, `attempt`) and does not call `ResolveSecrets`; against `1.1` it does
  not read `task_kind` at all (every task is a forward task). A major version other than `1` is refused by
  the server (`UNSUPPORTED_PROTOCOL`); the host does not retry it.
- The host exposes the accepted version (`host.protocolVersion`).
- There is one live session per worker name and identity: registering again replaces it (§6.2).

### 6.2 The loop

```text
register → loop:
  free = max_inflight - in flight
  if free > 0: PollTasks(max_tasks = min(free, 64))
     tasks → dispatch each (§6.3), concurrently
     none → wait retryAfterMs from the answer (default 1000, minimum 50), with ±20 % jitter, or
            until a task finishes, whichever comes first
  UNAUTHENTICATED (session gone or expired) → register again, then continue; if the next poll is
            UNAUTHENTICATED again, wait with the backoff of §5 before the next registration (reset by a
            successful poll): a server that registers a worker and refuses it at once must not be hammered
  RESOURCE_EXHAUSTED → wait retry-after-ms
  UNAVAILABLE / transport → wait with the backoff of §5 (no limit on attempts, cap 30 s), reconnect
```

The host runs at most as many tasks as it asked for (`max_tasks`): if a poll answer carries more, it runs the
first ones, logs the rest and leaves their leases to lapse. A wait derived from the server (`retry_after_ms`,
`Retry-After`) is capped at 60 s, whatever it says.

The host never busy-polls: an empty answer is always followed by a wait. It opens its connection when
it starts, not before. A connection that was idle may be closed by the server; `UNAVAILABLE` on it is
"reconnect and retry", not an outage.

### 6.3 Running a task

For a `TaskLease` the host:

1. finds the handler by `activity_name`; none → `FailStep(error_code = "NO_HANDLER", retryable = false)`
   (the server would not have leased it to a worker that did not advertise it, so this is a bug report);
2. parses `input_json` — the **step document** `{run: {id}, node, input, steps: {<node>: {output}}}` —
   and hands the handler `input` (the run's input) typed by the handler's declared type, plus the context
   (`ctx`): `activityName`, `taskId`, `runId`, `node` (the node id of the workflow definition, `node` of the step document;
   the lease's `step_id` when the document has none), `stepId` (the lease's `step_id`, a UUID, which the reports carry), `attempt`, `generation`,
   `idempotencyKey`, `traceparent`, `deadline`, `taskKind` (`FORWARD` when empty, `COMPENSATION`,
   `RECONCILIATION`), `targetNode`, `targetIdempotencyKey`, `steps` (the outputs of earlier steps,
   by node), a cancellation token, a logger and `secrets()`;
3. starts the **heartbeat** before the handler and keeps it until the report is made (§6.4);
4. runs the handler with the lease as its deadline; its result is the output;
5. reports (§6.5). If the lease was lost or the call answered `STALE_ATTEMPT`, it stops, drops the
   result, records a metric, and reports nothing.

A handler's input type that cannot be built from the document is not a crash: the host reports
`FailStep(error_code = "INPUT_INVALID", retryable = false)`.

### 6.4 Heartbeat and cancellation

- Interval: one third of the time left on the lease when the task is taken (the lease's `expires_at` minus the
  SDK's clock at receipt: the lease carries no TTL), never under 500 ms and never over 10 s (a third of the
  platform's default 30 s lease). A clock skewed by more than the lease's TTL distorts the interval, which is
  why it is clamped; a host whose clock is not synchronized sets `heartbeatIntervalMs` explicitly. The
  interval is then counted on the SDK's monotonic clock.
- `Heartbeat` carries `task_id, worker_id, generation, lease_token, run_id, step_id, attempt` (the last
  three only from protocol 1.1). The answer's `lease_expires_at_epoch_ms` becomes the context's new deadline.
- `cancel_requested = true` → the cancellation token of the activity is cancelled with the reason
  `run-cancelled`. The handler decides what to do; when it ends, the host reports whatever it returned
  or raised (a handler that raises its cancellation error is reported as `FailStep(error_code =
  "CANCELLED", retryable = false)`). A compensation or reconciliation task is never cancelled by the
  server (it answers `false`).
- **The host cancels the token and nothing else.** It never interrupts, aborts or kills a handler (in a runtime
  where cancellation is delivered into the running code, such as Python's `asyncio`, the handler's own awaits are
  not cancelled either: the SDK exposes the token and an awaitable on it). A handler that ignores the token runs
  to its end and what it returned is reported as usual, unless the lease was lost.
- **The lease is not fenced locally.** The SDK does not cancel a handler because the lease ran out by its own
  clock (the clocks differ); only a `STALE_ATTEMPT` to a heartbeat or a report does. A worker cut off from the
  platform keeps running its handler and its report is refused as stale; external effects stay at-least-once.
- **What a cancelled handler's end means depends on why it was cancelled.** `run-cancelled`: the platform asked
  for it, so a handler that raises its cancellation error is reported as `CANCELLED` (above) and one that
  returns is reported as what it returned. `lease-lost`: nothing is reported. `shutdown` (the host is
  stopping, §6.8): a handler that then raises its cancellation error is **not reported** — the lease lapses and
  the platform leases the step again; failing a step for good because a deploy stopped its worker would be
  wrong. A handler that returns a value, or fails with another error, after `shutdown` is reported as usual.
- `ABORTED` (`STALE_ATTEMPT`) or `FAILED_PRECONDITION` from a heartbeat → the lease is lost: the token is
  cancelled with the reason `lease-lost` and the eventual result is dropped.
- A heartbeat that fails with a transport error is retried on the next beat; the lease is still ours until
  it runs out.

### 6.5 Reporting

- Success: `CompleteStep{task_id, run_id, step_id, attempt, generation, lease_token, idempotency_key =
  "done-<task_id>", output_json, traceparent}`. `output_json` is the JSON of the handler's result (`null` →
  `"null"`; no result → empty string means "no value"). More than 256 KiB: not sent;
  `FailStep(error_code = "PAYLOAD_TOO_LARGE", retryable = false)` instead.
- Protocol 1.3 exposes `ctx.writeBlob(onePassSource, options)` and `ctx.readBlob(descriptor)` only while
  the handler is active. Upload sends one header followed by chunks no larger than 256 KiB, measures the
  source once, and accepts the returned descriptor only when its size and SHA-256 match locally. Read requires
  the complete descriptor, validates the response header, size and SHA-256 incrementally, and claims success
  only at clean gRPC EOF; partial or closed streams are unverified. An explicit native `BlobOutput(descriptor)`
  result sends `output_blob_ref` plus `output_sha256` with empty `output_json`. A JSON object that merely looks
  like a descriptor remains ordinary inline JSON. Protocol 1.2 and older raise `UNSUPPORTED_PROTOCOL` locally.
- Failure: `FailStep{..., idempotency_key = "failed-<task_id>", error_code, safe_message, retryable,
  traceparent}`. A handler fails by raising an **`ActivityFailure(code, message, retryable)`**; helpers:
  `retryable(code, message)`, `permanent(code, message)` (non-retryable), `outcomeUnknown(message)`
  (`OUTCOME_UNKNOWN`, non-retryable, reserved: the platform reconciles instead of retrying blindly).
  Any other raised error is reported as `error_code = "ACTIVITY_ERROR"`, `retryable = true`, and the
  `safe_message` is only its type name — never its text, which may hold the data it was handed.
  `safe_message` is cut to 512 Unicode code points (never inside a surrogate pair, and a lone surrogate becomes
  U+FFFD, so what is sent is always valid UTF-8); a code that is not `[A-Z0-9_]{1,64}` is replaced by
  `WORKER_ERROR`.
- The report is retried per §5. `already_applied = true` in the answer is a success. The host never
  reports one attempt twice with different content.

### 6.6 Compensation and reconciliation

- `registerCompensation(name, handler)` and `registerReconciler(name, handler)` are `registerActivity`
  with the same lease handling; they differ in the context (`taskKind`) and in what the host does with
  the result.
- A **compensation** handler undoes the effect of `ctx.targetNode`; its `idempotencyKey` is the key of
  *this* compensation and `targetIdempotencyKey` the key that identifies the original effect at the
  provider (send the latter to the provider). It **MUST** be idempotent and tolerate an effect that never
  happened (the platform compensates a step whose outcome stayed unknown). Its output is discarded by
  the server.
- A **reconciler** answers a verdict: `applied(output?)`, `notApplied()` or `unknown()`. The host
  completes the task with exactly `{"effect": "applied" | "not_applied" | "unknown", "output": …}`
  (`output` only for `applied`) and nothing else. A handler of another shape is a programming error
  reported as `FailStep(error_code = "RECONCILIATION_ANSWER_INVALID", retryable = false)`.
- The three kinds are told apart at run time by `task_kind` (protocol 1.2); a server at 1.0 or 1.1 never
  sends the others.

### 6.7 Secrets

`ctx.secrets()` calls `ResolveSecrets` once per attempt, lazily, with the task's identity, and returns a
read-only map of name → value (bytes; text on request). The values are credentials: the SDK never logs
them, never includes them in an error, never hands out its own buffer (the caller gets copies), wipes and drops
them when the handler returns (a later `secrets()` raises), and does not cache a failed resolution for the rest of
the attempt. Against protocol 1 it
raises `UNSUPPORTED_PROTOCOL` instead of calling.

### 6.8 Lifecycle

`start()` registers and begins the loop and returns once registered; `stop(timeout)` stops polling,
cancels nothing, waits for the tasks in flight up to `timeout` (default 30 s), then cancels their tokens
with the reason `shutdown` and returns. A task that did not finish keeps its lease until it lapses (the
server leases it again). The host is restartable. Metrics hooks (counters for tasks taken, completed,
failed, stale, heartbeats, polls, reconnects; a histogram of activity duration) are optional callbacks
or the ecosystem's standard (C#: `System.Diagnostics.Metrics`).

## 7. Workflow builders

Pure, deterministic constructors of the IR. `build()` returns the document (the language's JSON value)
and `toJson()` the canonical text: **RFC 8785** (JSON Canonicalization Scheme) — UTF-8, no insignificant
whitespace, the keys of every object sorted by their **UTF-16 code units**, numbers as ECMAScript prints them
(`1.0` is `1`, `600000` stays `600000`, never `6e5`). The same program in the five languages **MUST** produce
byte-identical canonical text. (Keys that differ only above U+FFFF sort differently by code point than by UTF-16
code unit; the rule is the UTF-16 one.)

```text
workflow(name).description(text).label(key, value)
  .step(id, activity, {retry, timeoutMs, compensateWith, reconcileWith, idempotencyTemplate, secrets,
                       compensationRetry, compensationTimeoutMs, irreversible, highRisk, onError})
  .database(id, {connectionRef, operation, statement, parameters?, limits?, timeoutMs?, onError?})
  .sleep(id, durationMs)
  .waitForEvent(id, {eventType, correlationExpr, timeoutMs, onTimeout?, lookbackMs?})
  .invokeChild(id, workflow, {version?, input?, cancelPolicy?})
  .condition(id, expression, ifTrue, ifFalse)
  .parallel(id, join, [branch, branch, ...])          // a branch is a flow built the same way
  .saga(id, {onCompensationFailure?, cancelPolicy?}, body)   // body is a flow
  .end(id, {outputExpr?})
  .trigger(kind, {...}) .inputSchema(schema) .outputSchema(schema)
  .build()
```

A `Flow` (the body of a saga, a branch of a parallel) is **read when `build()` is called**, not when it is joined: a
step added to it after it was joined is part of the document. Ids are checked when a node is added to its flow and again
by `build()` over the whole document.

Linking (the whole algorithm — it is why the output is the same everywhere):

- A flow is an ordered list of nodes. Node *i* gets `next = id of node i+1`, except a `condition` (it names
  its two branches) and an `end` (it ends), which never get one; the last node of a flow has no `next`.
- `entrypoint` is the id of the first node of the top flow.
- `parallel(id, join, branches)`: `branches` lists the id of the first node of each branch flow (at least
  two, in the order given); the last node of every branch has no `next`; the `parallel` node's own `next`
  is the next node of the parent flow.
- `saga(id, options, body)`: `begin` is the id of the first node of the body; the last node of the body has
  no `next`; the `saga` node's `next` is the next node of the parent flow. The body holds no `end`.
- `compensateWith` and `reconcileWith` are the names of activities (strings) on the step.
- Node ids are unique across the whole document, including nested flows; a duplicate, an empty flow, a
  parallel with fewer than two branches, a `saga` with an empty body, a bad name (`^[a-z][a-z0-9-]{2,63}$`)
  or an `irreversible` step with `compensateWith` is raised when the node is added (`VALIDATION_FAILED`),
  not when the server refuses the document.
- Options that are absent are omitted from the document (never `null`).

### 7.1 Managed database operations (feature 017)

All five SDKs emit the same canonical `type: database` node. `connectionRef` is
`{name, revision?}` within the workflow namespace, never a DSN, password or provider
path. `operation` is `query` or `execute`. Parameters are either
`{literal: {type, value}}` or `{type, expression}`. Tags are `null`, `boolean`,
`integer`, `decimal`, `float`, `text`, `timestamp`, `date`, `binary` and `json`;
integer/decimal values use exact strings, binary uses base64, timestamp/date use
RFC3339/ISO text. JSON literals obey the same canonical fidelity rules as other IR.

The builder snapshots a database node's supplied values when it is added, validates
basic shapes and finite contract ceilings, and raises `VALIDATION_FAILED` with a
fixed message that contains no SQL or values. It adds neither retry policy nor
secret references. SQL parsing, permission checks, connection/credential pinning,
execution and unknown-write handling are server responsibilities. Writes have no
automatic retry. Ordinary SDK workers continue negotiating 1.4; only the dedicated
managed executor uses 1.5. The server rejects unsupported IR before execution.

Inline results retain `{columns, rows, affectedRows, truncated}` with ordered tagged cells,
exact numeric strings and explicit nulls. They use the ordinary retained steps map
for downstream execution. The expression environment wraps each output, so a
binding may read `steps.query.output.rows[0][0].value`. Results over 256 KiB use the
existing `$blob` descriptor and explicit lease-bound blob reading; contexts do not
auto-expand them, and database expressions cannot read blob contents. Interactive result reading
requires the dedicated result permission. See
[the five-language guide](../docs/guides/managed-database-sdk.md) and the shared
`database-connections` conformance scenario.

## 8. Serialization

- JSON only. Integers up to 2^63-1 round-trip where the language can hold them; TypeScript holds up to
  2^53-1 and raises `PRECISION_LOSS`-style `VALIDATION_FAILED` on an unsafe integer instead of rounding.
  For a worker, an unsafe integer in the handler's own `input` fails the step (`INPUT_INVALID`); one in the output of
  an earlier step (`ctx.steps`) is raised when the handler *reads* `steps`, not before: a step that never reads it is
  not failed by another step's 64-bit id.
- Other numbers are not reformatted: `0.1` stays `0.1`. Amounts of money are strings (`"19.99"`) by
  convention of the platform's examples; the SDK has no decimal type of its own and never converts a
  string to a number.
- Instants the SDK writes are RFC 3339 in UTC with a `Z`; instants it reads may carry any offset and are
  returned as the language's instant type, in UTC.
- `NaN`, infinities, duplicate keys and invalid UTF-8 (a lone surrogate included) are errors on the way out and
  are not produced.
- The canonical text of a workflow (`toJson()`, §7) refuses an integer beyond 2^53 - 1 with `VALIDATION_FAILED`, in
  every SDK: the text must be the same bytes everywhere and no language but TypeScript's can print the nearest
  double and call it exact. The rule is on the **value**, not on how it was written: a number that is a whole number
  beyond 2^53 - 1 and that ECMAScript prints in digits (anything below 1e21: `9007199254740993`, `9007199254740992.0`,
  `1e20`, `2.5e19`) is refused; one it prints with an exponent (`1e21` and above, and every number with a fraction)
  is a number and is written as RFC 8785 writes it. A number is written as RFC 8785 writes it; an exponent is never expanded in memory
  (`1e999999999` is a number, not a billion digits).
- A `Retry-After` is read in seconds only; an HTTP date is ignored (no wait).

## 9. Versioning and compatibility

- The SDK follows **SemVer**. Before 1.0 a minor version may change an API; the **wire** contract does
  not move with it: REST is additive within `/v1`, the worker protocol is additive within `1.x`.
- **Newer SDK, older server.** The registration negotiation (§6.1) runs the host at the server's version;
  REST answers missing a field the SDK knows are accepted (fields are optional unless the OpenAPI marks
  them required); a call the older server does not have is raised as
  `NOT_FOUND` with the reason `UNSUPPORTED_BY_SERVER`: a `501`, a `404` whose envelope says
  `details.reason = ROUTE_NOT_FOUND`, a `405` that says `METHOD_NOT_ALLOWED`, or a `404` that is not the
  platform's envelope at all. (A `404` for a run that does not exist carries the envelope without that reason.)
- **Older SDK, newer server.** Unknown fields are ignored; the worker is served at its own version.
- Runtime matrix and package metadata: `sdks/README.md`.

## 10. Observability and safety

- Logs are structured and never contain a payload, a token, a lease token, a session token or a secret.
  Identifiers (run id, task id, node, attempt, trace id) are fine.
- Every request and RPC propagates `traceparent`; the C# SDK emits `Activity` spans from an
  `ActivitySource` named `SagatOctopusOrchestrator.Sdk`, and the others expose a tracing hook.
- Lease tokens and session tokens live in memory only and are dropped with the task / session.
- **A config, a client or a host never prints a credential**: its string form, its JSON and its debugger/inspect
  form show no bearer token, session token, lease token or TLS private key (a test builds one with a canary token
  and looks for it).
- **What the server says is bounded.** A REST body is read up to 32 MiB, then the call fails as `PROTOCOL`; the
  `message` of a server error is cut to 512 characters; a `traceId` that is not 32 hexadecimal characters is not
  repeated; the heartbeat, the poll loop and a task in flight survive an exception of the caller's hooks and of the
  runtime's observers (a token provider, a logger, an event or metrics callback, a trace listener) instead of dying
  with it or dropping a leased task; a token provider that throws is `UNAUTHENTICATED` with a reason, never the
  provider's own exception.
- **Ids and names in a URL path** (run id, signal name, event type...) are percent-encoded; a run id is a UUID and
  a name of `.`, `..` or the empty string is refused as `VALIDATION_FAILED` before anything is sent.
- Errors follow §2: a message that could carry a payload is replaced by its class.

## 11. What each SDK ships

Client, worker host, builders, errors (catalog), a README with a minimal example per capability, unit
tests, the conformance runner (`sdks/<language>/conformance`), a package manifest with the runtime
matrix, and nothing that needs a network to be tested except the conformance suite itself.

## 10. Task streaming (worker protocol 1.4)

The normative receive contract is
[`stream-completion.md`](../specs/006-sdk-multilanguage/stream-completion.md), ADR-0029.
Each public WorkerHost MUST support auto (default), poll and stream transport selection.
Auto MUST use StreamTasks for a negotiated minor >= 1.4 and PollTasks for older accepted minors.
An UNIMPLEMENTED stream permits auto fallback for that registration; permission/authentication
errors do not. Explicit stream on an unsupported minor fails with UNSUPPORTED_PROTOCOL.

A host MUST request only available execution slots, at most 64 total leases per receive window,
dispatch each frame immediately, keep partial deliveries alive across receive failures and
cancel pending receiving promptly on stop while preserving its existing handler drain.
The stream has a five-second server lifetime; clients additionally bound receiving to six
seconds or their shorter configured call timeout. A normal underfilled window ends with a
positive empty retry hint. An unexpected underfilled EOF MUST wait before reconnecting.
Protocol version changes do not themselves change package SemVer.
