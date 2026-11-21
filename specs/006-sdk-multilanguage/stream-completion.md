# Worker task streaming completion contract

Status: **Implemented and verified; five-SDK integration, comparative measurement and clean-consumer packages recorded** (2026-10-12).
Traces: NFR-006-PERF, NFR-006-SEC/DUR/OBS/COMP, FR-006-001/003/005/006,
FR-001-003/007, FR-002-003/004/008. Decision:
[ADR-0029](../../docs/adr/ADR-0029-worker-task-streams-spend-finite-receive-credit.md).

Feature 001 owns the canonical protobuf/authentication boundary and feature 002 owns durable claim
admission. Both are approved dependencies for this feature-006 completion. This document defines
the shared behavior before five SDK implementations are assigned; it does not itself close G125.

## Wire contract

Add to the existing service, without removing or renumbering anything:

```protobuf
// Added in protocol 1.4. max_tasks is total credit for this finite receive window.
rpc StreamTasks(PollTasksRequest) returns (stream PollTasksResponse);
```

- Registration offers/accepts protocol `1.4` using ADR-0015's existing minor negotiation. The server
  keeps serving every old accepted minor. A streaming call using a session negotiated below 1.4 is
  refused with the existing `UNSUPPORTED_PROTOCOL` classification before a lease is granted.
- The request carries the same worker/session identity and bearer metadata as a poll. The new method
  requires `1 <= max_tasks <= 64`; invalid credit is a canonical validation error. Its bound is
  checked before claims. Existing `PollTasks` input behavior is unchanged.
- `max_tasks` means the maximum number of leases delivered or committed for this call, cumulatively
  across all frames. A stream never grants replacement credit because a handler completed, a lease
  expired or the network accepted a frame.
- A nonempty frame contains committed `TaskLease` values and `retry_after_ms = 0`. A consumer
  dispatches each frame as it arrives; the entire stream must not be collected before execution.
- The window may send several nonempty frames before it ends. After exhausting its credit it ends
  normally. If it ends normally while underfilled, its final frame is empty and has a positive
  `retry_after_ms` derived from the worker idle retry setting. No empty progress/heartbeat frames
  are emitted before that terminal frame, and nothing follows it.
- Underfilled EOF without that terminal hint is treated conservatively by SDKs as an interrupted
  receive: retain prior deliveries and back off before reopening. EOF after all requested credits
  were delivered is normal. No acknowledgment of EOF changes any task state.
- Total serialized task inputs committed across the entire window are at most 8 MiB, as for a unary
  poll. Each gRPC frame stays within the existing 16 MiB client receive allowance including bounded
  metadata. Unused byte budget is passed into canonical claim admission before leasing. A task that
  fits a fresh 8 MiB budget stays ready when the current window is too small; only a task exceeding
  the absolute context limit receives the existing durable `RUN_DATA_LIMIT` preflight failure.
- The receive window lasts at most five seconds from admission, additionally limited by bearer
  expiry, request cancellation/deadline and process shutdown. Ordinary expiration of a healthy
  receive window uses the normal underfilled termination above. Authentication/refusal/failure
  yields the canonical gRPC status and metadata, including after successful earlier frames. A
  stalled body which cannot finish normally must end through the bounded cancellation/error path.
  The adapter retains body-owned permits and arms an independent connection watchdog. After a
  10 ms terminal-status transport grace it closes the whole HTTP/2 connection, including other
  multiplexed RPCs, if flow control prevents body completion. No new claim is admitted in that
  grace. Normal completion/drop disarms the watchdog; existing RPC retries handle reconnects.
- Lease tokens, task generations, idempotency keys, deadline, trace context, compensation kind and
  blob descriptors retain exactly their existing meanings. No stream identifier is authority to
  execute or report a task.

## Server admission and authority

1. Validate the request and initially authenticate through `authenticate_bearer`. Acquire separate
   lifetime stream permits after authentication: process 32, tenant 8 and subject-within-tenant 2
   by default. An exhausted cap returns `RESOURCE_EXHAUSTED`, the canonical rate-limit metadata
   and a bounded positive retry hint; it does not queue. Share permit state across cloned services.
2. Drop the initial ordinary tenant concurrency permit before idle waiting or returning the
   streaming body. Hold the dedicated permits through body completion, drop, cancellation, timeout
   and producer failure. An HTTP/2 header response does not release these permits. Default limits
   are explicit process bounds; a cluster-wide subscription quota is not claimed.
3. Before any subsequent claim attempt, obtain normal discovery admission, execute the existing
   bearer authentication pipeline again, obtain a fresh membership/role principal and check token
   expiry. Per-subject/per-tenant request budgets continue charging claim attempts. Never borrow a
   permit reserved for heartbeat/results and never keep a request permit while sleeping or waiting
   for an output queue slot. Reauthentication failure ends the stream; do not use cached authority.
4. Use the canonical application session validation and claim path, constrained by remaining credit
   and bytes. At claim time enforce tenant/namespace/subject ownership, current session token and
   negotiated minor, current worker bindings, drain state and existing activity/tenant quotas.
5. Make session `max_inflight` admission exact across simultaneous unary/stream calls and replicas.
   The current standalone `inflight()` read is only an optimization; recheck under a shared
   transaction fence before each claim commits. Count earlier claims in the same transaction.
   Session replacement/limit change and binding revocation must coordinate with the same admission
   boundary. A missing/replaced/expired session must not grant a lease from a stale snapshot.
   Review lock order against run locks, the binding editor, worker drain and activity policy
   fences. Do not hold a transaction across the entire receive window. A process mutex alone is
   insufficient. Record the chosen concrete lock order in the feature-002 design documentation.
6. Reserve the single producer queue place before beginning a claim. Carry at most one queued frame;
   keep total owned serialized inputs within the window budget. Do not start another claim while
   the queue is full. Every awaited claim/queue operation participates in stream cancellation and
   deadline handling; no detached producer survives its response body or retains permits forever.
7. After an empty claim, release all request/database resources and wait the configured idle retry
   interval before looking again, interruptible by cancellation and expiry. The baseline still
   polls PostgreSQL internally. It does not require a durable NATS subscription or promise fewer
   database queries than the polling transport.
8. On token expiry or a stream error, stop admitting new claims and stop exposing buffered leases
   under invalid credentials. Already committed leases still recover by expiry; they are never
   requeued immediately merely because their response was suppressed.

Revocation is checked at each admission boundary. A claim admitted before a membership change can
finish under the existing transaction semantics; the next claim must observe the changed authority.
JWT validation provides the existing issuer/JWKS behavior, not an invented immediate issuer-side
revocation service. The stream must not silently extend those semantics.

## Cancellation and partial delivery

| Point of interruption | Required result |
|---|---|
| Waiting for work or queue capacity | No new claim; all temporary/lifetime resources released when the stream ends |
| Before the claim transaction commits | Transaction rollback; no leased event or task token exposed |
| After commit, before the frame is handed to the transport | Withdrawn once every live process honours it ("Leases that never reach a host" below): the step goes back to `READY`, no attempt consumed, no review; until then the lease lapses |
| After the frame is handed to the transport, lost on the wire | Below 1.8: lapses (gap G006-001); since feature 037 a lapsed lease of an `Activity` is an unknown outcome and goes to reconciliation or review. At 1.8: the lease is provisional and is withdrawn at its expiry, no attempt consumed, no review ("Delivery acknowledgement (protocol 1.8)" below, ADR-0099) |
| After one or more frames reached SDK | Those tasks keep their handler, heartbeat and reporting lifecycle; only future receiving is retried |
| Server error after a successful frame | Preserve that frame; classify trailer error normally and back off/re-register as required |
| SDK stop during an idle receive | Cancel the receive promptly; preserve existing bounded handler drain |
| SDK stop with active tasks | Stop new admission, drain handlers up to deadline, then existing shutdown/lease-expiry semantics |
| Body producer panic or unexpected exit | Canonical failure or detectable incomplete stream; no success manufactured from underfilled EOF |
| Engine restart | New receive reconnects; committed states and leases remain PostgreSQL authority |

A stream failure does not mean a provider side effect failed. No handler is replayed from a network
buffer, no lease is acknowledged by a transport read, and external effects remain at-least-once.

## Shared SDK host behavior

- Add one idiomatic worker transport option with the same semantic choices in all five SDKs:
  `auto` (default), `poll`, `stream`. `auto` streams for accepted minor >= 1.4 and polls otherwise.
  Explicit `stream` requires minor >= 1.4 and returns `UNSUPPORTED_PROTOCOL` on an older negotiated
  server; `poll` remains available on every compatible server. Normal package versions remain a
  separate SemVer release decision; only protocol constants automatically advance to 1.4 here.
- At most one poll/receive call is outstanding for one host. Open a stream only when local free
  capacity is positive, requesting `min(free, 64)`. Keep local slots charged until the existing
  handler/report lifecycle finishes, including across reconnection, lease expiry and registration.
- Dispatch a frame immediately and start heartbeats using the same activity path as unary polling.
  Defend against a server emitting more than the remaining credit or local capacity: accept only
  bounded valid work, cancel the receive, record a closed diagnostic and leave excess leases to
  expire. Never grow an unbounded queue of handlers or concatenate all frames.
- Normal filled EOF may reopen immediately if capacity remains. Underfilled termination obeys the
  positive retry hint with existing jitter and the host's bounded wait rules; task completion can
  wake that wait. A malformed empty frame with no positive hint still receives a minimum wait.
- On `UNAUTHENTICATED`, use the existing bounded re-registration behavior; on
  `RESOURCE_EXHAUSTED`, honor its retry hint; on transport/`UNAVAILABLE`, use the existing capped
  backoff. Partial successes survive every one of those paths. Reset backoff only on meaningful
  successful receiving, not a rapidly failing registration loop.
- In `auto`, `UNIMPLEMENTED` disables streaming for that registration and falls back to polling,
  retaining already delivered tasks and waiting before the fallback call. Do not downgrade on
  authorization, validation or arbitrary server failures. Explicit `stream` reports unsupported
  transport instead of silently changing the requested benchmark/diagnostic mode.
- Read the token source for each new window. Cancel and dispose its stream on stop/reconnect/failure
  in the runtime's idiomatic way. Do not abort user handlers merely to close the receive operation.
- Transport-specific diagnostics contain no token, payload, worker identity, arbitrary remote
  exception text or unbounded attributes. Reuse the existing safe logger and error catalog.

## Observability and audit

Retain `grpc_requests_total{method="StreamTasks",code}` for initial calls, and measure the response
body separately so its later failures are visible. Required bounded signals:

- `worker_task_streams_open` and `worker_task_streams_refused_total{scope}` with scope limited to
  `process`, `tenant`, `subject`, `discovery`;
- `worker_task_streams_closed_total{reason}` with a closed set covering `credit`, `bytes`, `window`,
  `cancelled`, `unauthenticated`, `refused`, `unavailable`, `internal`;
- `worker_task_stream_duration_seconds`, `worker_task_stream_tasks_total` and
  `worker_task_stream_claims_total{outcome}` (`ready`, `empty`, `error`).

A scope label names the scope category, never its identifier. If implementation reuses existing
metrics for an equivalent signal, document the mapping and test it. Preserve each lease's W3C
trace context; an idle receive with no parent must not emit a new externally exported trace every
five seconds. Request spans/status and structured completion logs are bounded and payload-free.
The canonical claim journal and existing denial audit remain the evidence; no extra audit record
is required for every empty stream tick. Permission or admission changes retain their existing
transactional audit contract. Cancellation/drop paths must return open gauges and permits to zero.

## Acceptance matrix

All items below require named executed evidence before completion. Test doubles can exercise SDK
fault classification, but cannot replace real PostgreSQL/tonic tests for admission and recovery.

| Area | Required evidence |
|---|---|
| Wire compatibility | Descriptor remains additive; 1.0–1.3 unary scenarios unchanged; 1.4 register/stream; unsupported minor rejected without lease; five canonical copies/generated clients agree |
| Successive delivery | A single window receives at least two distinct nonempty frames after work arrives in separate waves; execution starts before EOF |
| Credit and bytes | Credit 1/64, invalid credit, aggregate 8 MiB across frames, task left READY when only the remainder is insufficient; no extra attempt or lease |
| Exact admission | Mixed poll/stream calls on one session across two store/service instances cannot exceed max_inflight; replacement/lowered limit, revoke, drain and activity quota races |
| Tenant/identity | Foreign session/token/namespace, role removed, membership removed, binding narrowed and token expires while window waits; no newly authorized claim after refusal |
| Saturation | Dedicated process/tenant/subject bounds held past response headers; idle streams hold no ordinary tenant request permit; heartbeats/results progress; other tenant remains serviceable |
| Slow consumer | Real stopped HTTP/2 reader, bounded producer memory/queue, deadline/drop cleanup and no ongoing claims after cancellation |
| Recovery | Cancel before commit, disconnect after commit before delivery, partial frames then status/error, stale report after reaping, server restart; consistent journal and no duplicated committed transition |
| SDK parity | Each official public host runs streaming success, partial disconnect, shutdown, capacity=1, backoff, explicit poll and older-server auto fallback; normalized golden outcomes agree |
| Telemetry | Closed metric labels, no secret/payload canaries, preserved task trace, gauges/permits released on success/error/panic/drop |

The comparative release-mode benchmark runs the same idle, sustained ready-work and backpressure
workloads with `poll` and `stream` in all five SDKs on recorded hardware/configuration. Record network
receive RPC rate, server claim attempts, p50/p95/p99 task delivery latency, completed throughput,
peak local in-flight count, and bounded queue/memory observations. Add a stream-saturation case
showing heartbeat completion. Report regressions and unmet hypotheses. The requirement is honest
measurement of a working transport; no universal throughput improvement is assumed.

## Implementation ownership and integration order

1. **Canonical server owner:** canonical proto and Rust descriptor baseline; application/store
   admission and tests; `api-grpc` stream/auth/body bounds; runtime settings if needed. Publish the
   settled proto first. Do not regenerate or edit SDK-owned outputs. Preserve existing unary tests.
2. **SDK owners, after the canonical schema is settled:** one owner per SDK subtree (or one owner
   for a named disjoint pair). Each updates its own canonical proto copy/codegen, protocol constant,
   RPC adapter, host lifecycle/options, unit tests and conformance runner. Never hand-edit another
   SDK's generated output, canonical proto, shared scenarios, shared goldens or `sdks/SPEC.md`.
3. **Shared conformance/integration owner:** `sdks/SPEC.md`, common harness/control/scenarios and
   reviewed goldens, architecture/runbook text, comparative benchmark/report and final traceability.
   Define scenario inputs before SDK runners implement them. Do not bless from one language alone.
4. **Final integrator:** compare all five copied schemas byte-for-byte with the canonical file, run
   their generation/drift checks, then server contracts, mixed-version/concurrency/fault tests,
   five-language conformance and benchmark. Record actual counts, skips and blockers; run required
   workspace/architecture/contracts/SDK checks after integration. Rebuild package artifacts affected
   by SDK changes before any registry publication.

Independent SDK implementation can proceed against the agreed schema while server tests are being
completed. Shared files have one owner. Closing this document's gates requires the integrated
server and SDKs, not five isolated unit-test results.


## Leases that never reach a host

Decisions D006-001 to D006-010 of [`research.md`](research.md); ADR-0098. A committed lease that no host received used to be
left to lapse; since feature 037 a lapse of an `Activity` opens a review for work that never ran. The rules:

1. **The server withdraws what it knows it did not send.** A lease is *handed to the transport* when `TaskBody::poll_next`
   returns the frame that carries it (for a unary `PollTasks`, when the handler returns its answer). Every lease committed by a
   claim and not handed to the transport is withdrawn: the leases of a claim that ends after the producer was told to stop, of
   a frame queued when the window deadline passes, of a frame drained at token expiry or when the body is dropped, of a frame
   sent into a closed queue, and of a unary poll whose handler is dropped before it returns.
2. **The producer is told to stop, never aborted in a claim.** The claim it is in finishes (it is bounded by the store's
   timeouts); nothing more is queued; what it holds goes to the withdrawal.
3. **A withdrawal is fenced.** One transaction per run, under the run lock; it acts only on a step that is still `LEASED` with
   the generation and lease-token hash the server issued. It appends `step.lease_withdrawn` (`node`, `attempt`, `generation`,
   `cause`: `NOT_SENT` or `NOT_STARTED`), clears the lease, returns the step to `READY` and sets its attempt back by one, so
   the next lease reuses that attempt number with the next generation. A fence that does not hold changes nothing and is
   counted (`worker_leases_undelivered_total{cause, outcome="fence_missed"}`).
4. **Only when every live process honours it.** The engine withdraws only while the requirement `lease-withdrawal-v1` is
   recorded for control planes and engines (instance-capability registry, ADR-0082; `control-plane lease-withdrawal-activate`).
   Until then the lease lapses as before and `worker_leases_undelivered_total{cause, outcome="lapsed"}` counts it.
5. **A host hands back what it drops** (`sdks/SPEC.md` §6.2): `FailStep` with `TASK_NOT_STARTED`, retryable, before any
   handler of the task runs. With the requirement recorded and the step still `LEASED`, the server withdraws it (cause
   `NOT_STARTED`); otherwise it is an ordinary retryable failure, never an unknown outcome.
6. **What is not covered.** A frame lost on the wire after it was handed to the transport, and a lost unary answer, still
   lapse (gap G006-001) for a session below 1.8; at 1.8 delivery acknowledgement closes it (next section). A process that stops between a claim and its withdrawal leaves
   the lease to lapse (G006-002).


## Delivery acknowledgement (protocol 1.8)

Decisions D006-011 to D006-024 of [`research.md`](research.md); ADR-0099; owner decision OD-2026-11-20-12. A lease of a
session accepted at 1.8 is *provisional* until its host acknowledges it; the host runs nothing for it before the server has
committed that acknowledgement; a provisional lease that reaches its expiry unacknowledged is withdrawn (ADR-0098's event, the
cause `NOT_ACKNOWLEDGED`), so a frame lost on the wire and a lost `PollTasks` answer (gap G006-001) no longer become a review.
Sessions below 1.8 are served exactly as before.

### Wire

Added to `specs/001-platform-foundation/contracts/worker.proto`, additively (the six SDK copies stay byte-identical):

```protobuf
// Added in protocol 1.8. The host acknowledges the leases of one frame, or of one PollTasks answer,
// before it runs any of them. Idempotent: the same lease may be acknowledged again.
rpc AcknowledgeTasks(AcknowledgeTasksRequest) returns (AcknowledgeTasksResponse);

message TaskAcknowledgement {
  string task_id = 1; string run_id = 2; string step_id = 3;
  int32 attempt = 4; int64 generation = 5; string lease_token = 6;
  ExecutionInstanceIdentity instance = 7;
}
message AcknowledgeTasksRequest { string worker_id = 1; repeated TaskAcknowledgement tasks = 2; }
enum AcknowledgementVerdict {
  ACKNOWLEDGEMENT_VERDICT_UNSPECIFIED = 0;
  ACKNOWLEDGEMENT_VERDICT_ACKNOWLEDGED = 1;   // the server has committed that this host holds the lease
  ACKNOWLEDGEMENT_VERDICT_NOT_HELD = 2;       // the lease is not this host's (withdrawn, replaced, foreign, unbound)
}
message TaskAcknowledgementResult {
  string task_id = 1;
  AcknowledgementVerdict verdict = 2;
  int64 lease_expires_at_epoch_ms = 3;        // the renewed expiry when ACKNOWLEDGED; 0 otherwise
  bool cancel_requested = 4;                  // as on HeartbeatResponse; false when NOT_HELD
}
message AcknowledgeTasksResponse { repeated TaskAcknowledgementResult results = 1; }
```

- A request carries 1 to 64 tasks (the credit of one receive window) with distinct `task_id`s; anything else is
  `INVALID_ARGUMENT` before any task is read. Each field is validated as `Heartbeat` validates it; `instance` is echoed as at
  1.5. A response carries one result per task, in request order; a host that receives another count, an unknown `task_id`
  or `ACKNOWLEDGEMENT_VERDICT_UNSPECIFIED` treats the whole answer as a failed call.
- `TaskLease`, `PollTasksResponse`, `HeartbeatRequest` and every other message are unchanged. No stream identifier, frame
  number or session token is authority: the lease token is, as for every other worker call.
- The call is in admission class `Lease` (`crates/api-grpc/src/limits.rs`), the capacity reserved for the calls that keep
  leased work alive, beside `Heartbeat`, `CompleteStep` and `FailStep`. Its request and response join `skip_debug`; their
  `Debug` prints counts only.
- `ResolveSecrets`, `ResolveDatabase`, `WriteBlob`, `ReadBlob` and `AppendStepLogs` for a lease that is provisional and
  unacknowledged answer `FAILED_PRECONDITION` with the stable reason `TASK_NOT_ACKNOWLEDGED`.

### Negotiation and activation

- The server's minor becomes 8 (`DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR = 8`). Minors are cumulative: `protocol_ceiling`
  answers 8 for an ordinary worker only when it would answer 7 (design 041, task T041-7) **and** the store proves
  `supports_delivery_acknowledgement()` **and** the engine knows the requirement `delivery-acknowledgement-v1` is recorded
  **and** the engine's lease time-to-live is at least 7 s (the threshold derived below). Otherwise it answers what it answers
  today. The database runner keeps 6.
- Every control plane and engine of the build lists `delivery-acknowledgement-v1` (the control plane because it replays the
  journal, the engine because it also serves the worker API and runs the lost-lease pass). The operator records it with
  `control-plane delivery-acknowledgement-activate` (`--operator`, `--expect-live N`, `--dry-run`, owner role, one bounded
  transaction, no step of its own), which refuses while `lease-withdrawal-v1` (ADR-0098) is not recorded. The removal of
  `lease-withdrawal-v1` refuses while `delivery-acknowledgement-v1` is recorded.
- A claim marks a lease provisional only when the session was accepted at 1.8 **and** the claiming engine knows the
  requirement is recorded. An engine learns of an activation or a removal within one registry heartbeat (about 12 s,
  ADR-0082 decision 4); during that window a 1.8 session may receive a lease that is not provisional (gap G006-008), which
  keeps today's meaning.

### The provisional mark

Two nullable columns of `steps`, written by the statements that already write the lease (no added statement):

| Column | Set | Cleared |
|---|---|---|
| `lease_offered_at` | by the claim, to the commit time, when the lease is provisional | whenever the lease is cleared or replaced (every write whose status is not `LEASED` or `RUNNING`, and every new lease) |
| `lease_acknowledged_at` | by the first acknowledgement of that lease | as above |

A lease is *provisional and unacknowledged* when the step is `LEASED`, `lease_offered_at` is set and `lease_acknowledged_at`
is empty. The lease's expiry at the claim is unchanged (commit plus the lease time-to-live). The marks are lease data, like
the token hash and the expiry: they are not journaled, the replay does not derive them, and a reset clears them with the lease.

### The server's acknowledgement

For each run of the call, in the order its first task appears:

1. Open the run as `Heartbeat` does (`begin(tenant, run, Lock::Wait)`, which takes the continuation-chain lock too). A run
   that does not exist in the caller's tenant answers `NOT_HELD` for its tasks; a run of another tenant is audited
   `CrossTenantAccessDenied`, as `missing_task_error` audits it.
2. For each task of that run: the step identity belongs to the run and instance (`verify`), the lease-token hash matches a
   step `LEASED` or `RUNNING` (`lease_matches_instance`), and the binding still holds (`ensure_bound`). A check that fails
   answers `NOT_HELD` and writes nothing for that task.
3. For each task whose checks hold, write the step row: if `lease_acknowledged_at` is empty and `lease_offered_at` is set,
   set `lease_acknowledged_at = now()` and move `attempt_deadline_at` later by `now() - lease_offered_at`; then renew the lease
   as a heartbeat renews it (`least(now() + time-to-live, attempt_deadline_at)`). Answer `ACKNOWLEDGED` with the renewed
   expiry and the step's `cancel_requested`.
4. Commit. Nothing else: no run state is loaded, no transition applied, no journal event, no outbox row. A store failure ends
   the call with its canonical status after the runs already committed; the host repeats the call and those runs answer
   `ACKNOWLEDGED` again (a duplicate).

An acknowledgement of a lease that is not provisional (a session that was accepted at 1.8 before a removal, or a lease marked
before the claiming engine learned of an activation) is answered the same way and is a renewal. Counted:
`worker_task_acknowledgements_total{verdict}` with `acknowledged`, `duplicate` (the mark was already set) and `not_held`.

### The withdrawal

The lost-lease pass (`crates/infra-postgres/src/maintenance.rs` `lost_leases`, `crates/scheduler/src/passes.rs` `leases`):

- Its `EXPIRED` arm also returns the two marks; its `TIMED_OUT` arm skips a lease that is provisional and unacknowledged (the
  attempt has not begun: decision D006-019).
- Under the run lock, its condition (`Condition::LeaseLost`) is checked again; then a lease still `LEASED`, provisional,
  unacknowledged and expired, with the requirement known to be recorded, is withdrawn as ADR-0098 decision 3 withdraws it:
  one fenced transaction, `step.lease_withdrawn` (`node`, `attempt`, `generation`, `cause = NOT_ACKNOWLEDGED`), the lease and
  its marks cleared, the step back to `READY`, the attempt set back by one, the generation kept. The engine's settle runs in
  the same transaction, so a step of a stopping run is cancelled at once. Counted as
  `worker_leases_undelivered_total{cause="not_acknowledged", outcome="withdrawn"}`; a fence that no longer holds is
  `outcome="fence_missed"`.
- An acknowledged lease that expires, and a provisional one that expires while the requirement is not known to be recorded
  (`outcome="lapsed"`), go to `expire_attempt` as today: an unknown outcome, consultation or review.
- ADR-0098's withdrawal with the cause `NOT_SENT` also requires `lease_acknowledged_at` to be empty (a host cannot have
  acknowledged a lease it never received; the condition costs nothing and closes the case by construction).

**Exactly one of the acknowledgement and the withdrawal commits for a lease.** Both hold the run lock; the acknowledgement
writes only while the token hash matches a `LEASED` or `RUNNING` step; the withdrawal writes only while the step is `LEASED`
and unacknowledged. If the acknowledgement commits first, the withdrawal finds `lease_acknowledged_at` set and withdraws
nothing; if the withdrawal commits first, the acknowledgement finds the token cleared and answers `NOT_HELD`. The host runs a
handler only on `ACKNOWLEDGED`. So a handler of a 1.8 lease runs only after the server committed that its host holds it, and a
lease the server withdraws was not run by a host that keeps the protocol. No clock enters the argument: the deadline below
decides when the reaper looks, never whether the two can both happen.

### Why the deadline is the lease expiry

The acknowledgement deadline is the lease's expiry at the claim: commit plus the lease time-to-live (`SAGAT_LEASE_TTL_SECS`,
default 30 s). The derivation:

1. **R, the receive bound: 6 s.** A host bounds each receive call to 6 s or its shorter configured call timeout
   (`sdks/SPEC.md` §10 for `StreamTasks`; at 1.8 the same bound applies to `PollTasks`, decision D006-018). The host starts
   that clock before it sends the request and the server commits a lease only after it has the request, so any lease the
   host reads, it reads within 6 s of the lease's commit in real time, with no comparison of clocks. A lease not read by then
   is never read: the host has cancelled the call. Measured, ready to handler entry is 751 to 889 ms at the 99th percentile in
   the ten burst rows (`tests/performance/reports/sdk-streaming.md`), which includes the commit to handler entry.
2. **A, one acknowledgement: at most 0.103 s measured.** No acknowledgement exists to measure yet. A completion does strictly
   more (it opens the run the same way, then loads the run's state, applies the transition, routes and appends events); its
   99th percentile is 6 to 103 ms over the nine rows of 2026-11-12 (`tests/performance/reports/scalability-ha.md`). So A is
   at most 0.103 s at those loads. Task T006-064 measures A itself (`worker_task_acknowledgement_delay_seconds`) and
   re-derives the threshold of item 5 from it.
3. **The interval of safe deadlines.** A healthy host of 1.8 has its acknowledgement committed within L = R + A = 6.103 s of
   the commit. A deadline below L withdraws healthy deliveries; a deadline above the lease expiry is never reached, because the
   lease is reaped at its expiry. Any deadline in [L, time-to-live] is equally safe, because safety is the run-lock argument
   above. Inside the interval the deadline trades two things only: how long a lost lease holds its step and its session's
   slot (longer with a later deadline) and how many healthy deliveries are withdrawn when the database is slow (more with an
   earlier one). A withdrawn healthy delivery claims and delivers the same task again, which adds work exactly when the system
   is slowest; a later deadline costs time only after a loss, which is rare (three observed cases since feature 037: two CI jobs and one play of feature 022).
4. **The choice: the upper end.** The deadline is the expiry: at the default it leaves a margin of 30 − 6.103 = 23.897 s over
   L, almost four times L; it adds no setting, no column and no scan; and it is the bound a host already has to meet today with
   its first heartbeat, so no host that keeps its lease today loses it. A lost lease is withdrawn at commit plus the
   time-to-live plus the period of the lost-lease pass, the time after which it lapses today.
5. **The threshold.** The time-to-live is configured in whole seconds, so the server offers 1.8 only when it is at least
   R + ⌈A⌉ = 6 + 1 = 7 s; below that it caps the ordinary ceiling at 7 and logs once at start which setting caused it.

### The host at 1.8

Additions to `sdks/SPEC.md` §6.2 to §6.4 (the normative host text is there):

1. **Receive.** A `PollTasks` call is bounded to 6 s or the shorter configured call timeout, as a stream window is. The excess
   over the credit and the leases received while stopping are handed back with `TASK_NOT_STARTED` (decision D006-005) and are
   not acknowledged.
2. **Acknowledge.** As soon as a frame or a poll answer is read, the host sends one `AcknowledgeTasks` with all its leases that
   it will run, concurrently with its next receive. It starts no handler, no heartbeat and no other call of those leases
   before the answer.
3. **Verdicts.** `ACKNOWLEDGED`: dispatch the task (§6.3), with the result's `lease_expires_at_epoch_ms` as its deadline; the
   heartbeat starts now, its interval a third of the time left on that expiry (500 ms to 10 s). `ACKNOWLEDGED` with
   `cancel_requested = true`: do not start the handler; hand the lease back with `TASK_NOT_STARTED`. `NOT_HELD`: drop the task,
   run nothing, report nothing, free its slot, emit the event `dropped` with the code `NOT_HELD`.
4. **Failure of the call.** `UNAVAILABLE`, `RESOURCE_EXHAUSTED` (after its retry hint), `INTERNAL` that says it may be
   repeated, transport failures, a deadline, and an answer the host cannot use: repeat the same call with the backoff of §5,
   with no limit on attempts, each attempt bounded by the shorter of the call timeout and the time left until the latest
   expiry among the leases still waiting (by the host's clock). `UNAUTHENTICATED`: repeat once with a fresh token (the lease
   does not depend on the session). A lease whose expiry passes without a verdict is not run and is handed back with
   `TASK_NOT_STARTED` (§6.5 report retries). Any other status: run nothing, log once, hand every lease of the call back.
5. **Logs.** The happy path logs nothing new; the verdict `NOT_HELD` and a call that is given up log one line each, with
   identifiers only.

### Failure modes and their exact outcomes

| Case | Outcome |
|---|---|
| A frame or a `PollTasks` answer lost on the wire, or the host crashes before acknowledging | No acknowledgement arrives; at the expiry the lost-lease pass withdraws the lease (`NOT_ACKNOWLEDGED`): `READY`, attempt not consumed, no review; the next lease has the same attempt number and the next generation |
| The acknowledgement never leaves the host | Repeated (host item 4); on a verdict the task runs once; if the leases expire first, nothing runs, the leases are handed back, and the server withdraws whichever it still holds |
| The acknowledgement's answer is lost after its commit | The host repeats; the server answers `ACKNOWLEDGED` again (`duplicate`); the handler runs once. If no answer arrives before the host's view of the expiry, nothing runs and the host hands the lease back: `TASK_NOT_STARTED` on an acknowledged `LEASED` step is withdrawn (`NOT_STARTED`). If the hand-back cannot reach the server either, the acknowledged lease lapses at its renewed expiry into a review (gap G006-005) |
| A duplicate acknowledgement (same lease, also after the first heartbeat) | `ACKNOWLEDGED` with the current expiry; only the renewal is written |
| A late acknowledgement after the withdrawal, or after the lease was replaced | `NOT_HELD`; the host drops the task without running or reporting; counted `not_held` |
| Acknowledgement and withdrawal at the same moment | Exactly one commits (run lock, fences); either the task runs once or it is withdrawn and the host drops it |
| The host stops, or crashes, after `ACKNOWLEDGED` and before the handler starts | The acknowledged lease lapses at its renewed expiry: unknown outcome, consultation or review (gap G006-004; the window is in-process, with no call in it). A stopping host instead hands the lease back (`TASK_NOT_STARTED`): withdrawn |
| The run's cancellation is requested before the acknowledgement | `ACKNOWLEDGED` with `cancel_requested = true`; the host hands the lease back; the withdrawal's settle cancels the step; no attempt, no review, the handler never runs |
| The run is cancelled while the lease is provisional and the host never answers | Withdrawn at the expiry; the settle cancels the step |
| A reset of the run while the lease is provisional | The lease is replaced or cleared with its marks; the acknowledgement answers `NOT_HELD` |
| A foreign tenant's run, a forged or old token, a binding revoked after the claim | `NOT_HELD`; the foreign run is audited; the lease is withdrawn at its expiry and leased to a bound worker |
| The step's attempt deadline passes before the acknowledgement | Not an attempt timeout: the attempt deadline is moved by the acknowledgement; only the expiry applies until then |
| A capability call (`ResolveSecrets`, `ResolveDatabase`, `WriteBlob`, `ReadBlob`, `AppendStepLogs`) on a provisional lease | `FAILED_PRECONDITION`, reason `TASK_NOT_ACKNOWLEDGED`; counted; nothing handed out |
| A `Heartbeat`, `CompleteStep` or `FailStep` on a provisional lease (a host that broke the rule) | Served as today; the step leaves the reach of the withdrawal (`RUNNING` or reported) |
| An engine restarts or fails over between the claim and the acknowledgement | The lease and its marks are in PostgreSQL; any engine of the build serves the acknowledgement |
| The requirement is removed while provisional leases exist | Within one registry heartbeat no engine withdraws with `NOT_ACKNOWLEDGED`; such a lease that expires lapses as before (`outcome="lapsed"`) |
| A hand-written client at 1.8 that runs a handler without acknowledging, uses no capability and dies before any call | Withdrawn and run again with the same attempt number and idempotency key (gap G006-006); external effects stay at least once |

### Observability

- `worker_task_acknowledgements_total{verdict}` (`acknowledged`, `duplicate`, `not_held`) and the method's entry in
  `grpc_requests_total{method="AcknowledgeTasks", code}`.
- `worker_leases_undelivered_total{cause, outcome}` gains the cause `not_acknowledged` (ADR-0098's counter).
- `worker_task_acknowledgement_delay_seconds`, a histogram on the database clock of `lease_acknowledged_at -
  lease_offered_at`, written by the acknowledgement's own statement (`RETURNING`): the measured A of the derivation.
- `worker_leases_provisional` and `worker_leases_provisional_expired`, gauges from the operations statistics query
  (`crates/infra-postgres/src/operations.rs`), not from the hot path.
- `worker_capability_calls_refused_total{method, reason="TASK_NOT_ACKNOWLEDGED"}`.
- Alerts: `SagatLeasesNotAcknowledged` when withdrawals with the cause `not_acknowledged` happen in every 5-minute window for
  15 minutes (frames are being lost, or a host is not acknowledging); `SagatLeaseAcknowledgementSlow` when the 99th percentile
  of the delay over 10 minutes exceeds half of the time-to-live minus R (at the default, 12 s), the point at which the margin of
  the derivation has halved.
- The call's span carries a link to each lease's trace context (at most 64), never a parent; logs and metric labels carry no
  token, payload or identifier with unbounded values.
- SDK metrics hooks gain `acknowledgements` and `notHeld` counters.

### Acceptance matrix

| Area | Required evidence |
|---|---|
| Exactness | Barrier tests in both orders (acknowledgement holds the run lock while the withdrawal waits, and the reverse): exactly one commits, a late acknowledgement is `NOT_HELD`, the handler runs once |
| Withdrawal | A provisional lease of an `Activity` that expires: `step.lease_withdrawn` cause `NOT_ACKNOWLEDGED`, no `step.needs_review`, the attempt not consumed, the next lease reuses it; an acknowledged lease that expires is still an unknown outcome |
| Older minors | A session at 1.7 and below gets no mark; its expiry is unchanged; the whole conformance suite passes at its pinned minors with the existing goldens |
| Gate | No mark, no 1.8 and no `NOT_ACKNOWLEDGED` while the requirement is not recorded; the activation refuses without `lease-withdrawal-v1`; the removal of `lease-withdrawal-v1` refuses while this one is recorded; 1.8 not offered under a 6 s time-to-live |
| Capabilities | Each of the five capability calls refused on a provisional lease and served after the acknowledgement |
| Attempt deadline | A step whose timeout is shorter than a held delivery is not timed out before the acknowledgement and is timed out from it |
| Cost | `LEASE`, `COMPLETION_WITH_A_NEXT_STEP` and `COMPLETION_THAT_ENDS_THE_RUN` unchanged at 1.8; `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` pinned; `ACKNOWLEDGEMENT` has fewer statements than `FIRST_HEARTBEAT` |
| Six SDKs | The scenario `delivery-acknowledgement` passes in six languages with a reviewed golden; every other scenario passes at 1.8 with its existing golden, unblessed |
| Released v0.2 | The upgrade scenario `provisional-lease-cutover`: a v0.2 worker's lease across the upgrade keeps its meaning; no mark before the activation; the current build replays a journal holding `NOT_ACKNOWLEDGED` |
| Throughput | The transitions scenario at 1.8 against 1.7, same machine and day, recorded with the delay histogram, and the threshold re-derived |
