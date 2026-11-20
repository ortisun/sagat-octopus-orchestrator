# SDK interceptors (draft for `sdks/SPEC.md` §12)

Draft normative text, to be merged into `sdks/SPEC.md` with the implementation. Names are language neutral; each SDK
uses its idiom (`interceptCall` / `InterceptCall` / `intercept_call`). Every **MUST** is held to the shared conformance
scenarios `interceptors` and `interceptor-coverage` in `tests/contract/sdk-conformance/scenarios/`. No server, protobuf
or IR change is involved. Interceptors are optional: an SDK with none configured behaves exactly as before.

Interceptors **observe and decorate** calls (logging, metrics, tracing, gateway headers, refusing a call). They never
transform payload bytes. The only payload-transforming stage of an SDK is feature 031's data converter (FR-031-016),
which sits between the interceptor chain and the transport (coordinator decision C5). This contract fixes the order
031 relies on and nothing else of 031.

## 12.1 Client

```text
ClientConfig.interceptors: ClientInterceptor[]          // default [], at most 64
ClientConfig.clock: Clock | none                         // §13; default system clock

ClientInterceptor.interceptCall(call: ClientCall, next) -> result
ClientCall { operation, metadata (read-only), value (read-only), headers (add-only) }
```

- `operation` is one of: `START`, `START_WITH_BLOB`, `UPLOAD_RUN_INPUT`, `GET`, `LIST`, `OPERATE` (cancel, pause,
  resume, rerun, saga decisions), `SIGNAL`, `PUBLISH_EVENT`, `QUERY`, `UPDATE`, `DECIDE_APPROVAL`, `LIST_APPROVALS`,
  `INSPECT_CHAIN`, `LIST_CHAIN_RUNS`, `QUERY_CHAIN`, `UPDATE_CHAIN`, `SIGNAL_CHAIN`, `CANCEL_CHAIN`, `EVENTS`,
  `WAIT_FOR_COMPLETION`, and the test-environment operations `TEST_CLOCK`, `TEST_ADVANCE`, `TEST_SKIP`,
  `TEST_REPLAY`. The set only grows; an interceptor **MUST** pass an unknown operation to `next` unchanged.
- `metadata` holds the namespace, the operation's path identifiers (workflow id, run id, chain id, approval id,
  operation name, signal name), the idempotency/update/decision key, `traceparent` and an attempt-independent call id.
- `value` is the application value the operation sends (start input, signal payload, event data, query/update
  arguments, decision body) or `none`, as a read-only view or a copy. For `UPLOAD_RUN_INPUT` and blob operations only
  the size and digest are visible, never the stream (G1402). On the way back the chain sees the decoded answer.
- `headers` may receive additional request headers: at most 16, names `[A-Za-z0-9-]{1,64}`, values printable ASCII of
  at most 1,024 bytes, never one of the protected headers (§12.4). The SDK never logs their values.
- **Order.** The first registered interceptor is the outermost. On the way out: A, B, then 031's data converter (when
  configured), serialization, the retry loop and the transport; on the way back: transport, converter, B, A. Nothing
  can be placed between the converter and the transport. `next` may be called at most once; a second call raises
  `INTERCEPTOR_FAILED`.
- **Once per logical call.** The chain runs once. The SDK then freezes the bytes and headers and enters the retry loop
  of §5: every retry **MUST** send byte-identical bodies and the same headers under the same key. A decoration that
  would differ per attempt (a timestamp) is therefore taken once.
- `waitForCompletion` is one logical call: the chain wraps the whole wait; each poll is not re-intercepted.

## 12.2 Worker host

```text
WorkerConfig.interceptors: WorkerInterceptor[]           // default [], at most 64

WorkerInterceptor.interceptExecute(task: TaskView, next) -> outcome    // forward, compensation, reconciliation
WorkerInterceptor.interceptHeartbeat(beat: HeartbeatView, next)         // metadata only
WorkerInterceptor.interceptReport(report: ReportView, next)             // complete or fail
```

- `TaskView` carries the activity context of §6.3 (read-only identity, attempt, task kind, deadline, cancellation
  token, logger) and the **decoded** step document (`input`, `steps`, `item`/`index`) as read-only views; 031's
  converter, when configured, decoded it before the chain. The outcome returned by `next` is observed read-only.
- `interceptHeartbeat` sees the task identity and the heartbeat timing; it cannot add data in this feature.
- `interceptReport` sees the report kind (`COMPLETE` or `FAIL`), the output's size (not its bytes) or the failure code,
  retryability and safe message. It runs once per report; after it, 031's converter (when configured) encodes the
  output once, and retries of the report (§5) resend identical bytes under `done-<taskId>` / `failed-<taskId>`.
- The lease, heartbeat, cancellation, stale-attempt and shutdown rules of §6 are unchanged. An interceptor cannot keep
  a task from being reported: a raised error becomes a failure report (§12.5), and a task that is never reported lapses
  as before. `ctx.secrets()` values never pass through the chain's views.

## 12.4 Protected fields and headers

The token, the tenant, the namespace, idempotency, update and decision keys, `eventId`, the lease token, the session
token, `taskId`, run id, step id, attempt, generation, execution-instance identity, the negotiated protocol version,
`traceparent` and the payload value are exposed as read-only views or copies. Where a change could be mistaken for an
effect (a mutable map), the SDK detects it and raises `VALIDATION_FAILED` with reason `PROTECTED_FIELD` before sending.
Protected request headers: `Authorization`, `X-Octopus-Namespace`, `Idempotency-Key`, `traceparent`, `tracestate`,
`Content-Type`, `Content-Length`, `User-Agent`, `Host`, and every `X-Octopus-*` and `X-Sagat-*` header; for gRPC the
same names in metadata plus `grpc-*`. Trace context keeps coming from the trace hook of §4.

## 12.5 Errors

| Where | Error raised | Result |
|---|---|---|
| Client interceptor, before sending | `INTERCEPTOR_FAILED` (origin `sdk`, `retryable: false`), message = the interceptor's type name | nothing is sent |
| Client interceptor, after the server answered | `INTERCEPTOR_FAILED` with `reason: AFTER_RESPONSE` | the effect may have been applied; the caller retries with the same key and the server replays |
| Worker execute interceptor | as if the handler raised it (§6.5): `ActivityFailure` keeps code and retryability; otherwise `ACTIVITY_ERROR`, retryable, type name | failure report |
| Worker heartbeat or report interceptor | logged by type name; the heartbeat or report proceeds unchanged | the host loop survives (§10) |

Failures of 031's converter are 031's codes. `sdks/error-catalog.json` gains `{"code": "INTERCEPTOR_FAILED", "origin":
"sdk", "http": [], "grpc": "INTERNAL", "retryable": false, "remediation": "An interceptor of the application failed: see
the application's logs for the interceptor named in the message; the call was not repeated."}` and its `version`
becomes 2.

## 12.6 Logging and telemetry

The SDK logs interceptor failures by interceptor type and operation, never a value or an added header's value. The
OpenTelemetry bridge and the C# `ActivitySource` are unchanged; an interceptor that creates spans does so under the
ambient context.

## Conformance scenarios (sketch)

- `interceptors`: two recording interceptors and one that adds a header `X-Gateway-Tag`, on client and worker; a
  `START` with `dropResponse` once and a `CompleteStep` with `dropResponse` once; observations: interceptor order per
  operation, chain entries per logical call (1), the bodies and the added header the server received per attempt
  (equal), the values observed, a protected-field or payload change refused, a protected header refused, an
  interceptor raising before and after the response, a worker interceptor raising `ActivityFailure("X",
  retryable=false)`.
- `interceptor-coverage`: one workflow and client program that exercise every client operation and every task kind
  available today; the golden lists the operations and task kinds each SDK's recording interceptor saw, in order.
- With feature 031 present, its scenarios add a converter and check that interceptors see decoded values and that
  nothing runs between the converter and the transport.
