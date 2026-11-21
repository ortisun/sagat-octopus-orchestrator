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
   `NOT_STARTED`), except an acknowledged lease of protocol 1.8, which is an attempt (decision D006-045); otherwise it is an
   ordinary retryable failure, never an unknown outcome.
6. **What is not covered.** A frame lost on the wire after it was handed to the transport, and a lost unary answer, still
   lapse (gap G006-001) for a session below 1.8; at 1.8 delivery acknowledgement closes it (next section). A process that stops between a claim and its withdrawal leaves
   the lease to lapse (G006-002); at 1.8 that lease is provisional and is withdrawn at its expiry instead.



## Delivery acknowledgement (protocol 1.8)

Decisions D006-040 to D006-060 of [`research.md`](research.md); ADR-0099; owner decision OD-2026-11-20-12. A lease of a session
accepted at 1.8 is *provisional* until its host acknowledges it; the host runs nothing and calls nothing for it before the
server has committed that acknowledgement; a provisional lease that reaches its expiry is withdrawn (ADR-0098's event, the cause
`NOT_ACKNOWLEDGED`), so a frame lost on the wire and a lost `PollTasks` answer (gap G006-001) no longer become a review. An
acknowledged lease is an attempt, with today's meaning in every respect. Sessions below 1.8 are served exactly as before.

### Wire

Added to `specs/001-platform-foundation/contracts/worker.proto`, additively (the six SDK copies stay byte-identical):

```protobuf
// Added in protocol 1.8. The host acknowledges the leases of one frame, or of one PollTasks answer,
// before it runs or calls anything for them. Idempotent: the same lease may be acknowledged again.
rpc AcknowledgeTasks(AcknowledgeTasksRequest) returns (AcknowledgeTasksResponse);

message TaskAcknowledgement {
  string task_id = 1; string run_id = 2; string step_id = 3;
  int32 attempt = 4; int64 generation = 5; string lease_token = 6;
  ExecutionInstanceIdentity instance = 7;
}
message AcknowledgeTasksRequest { string worker_id = 1; repeated TaskAcknowledgement tasks = 2; }
enum AcknowledgementVerdict {
  ACKNOWLEDGEMENT_VERDICT_UNSPECIFIED = 0;
  ACKNOWLEDGEMENT_VERDICT_ACKNOWLEDGED = 1;   // the server has committed that this host holds the lease: run it
  ACKNOWLEDGEMENT_VERDICT_NOT_HELD = 2;       // not this host's lease (expired, withdrawn, replaced, foreign, unbound): drop it
  ACKNOWLEDGEMENT_VERDICT_CANCELLED = 3;      // its run asked to stop before it started; withdrawn and cancelled: drop it
}
message TaskAcknowledgementResult {
  string task_id = 1;
  AcknowledgementVerdict verdict = 2;
  int64 lease_expires_at_epoch_ms = 3;        // the renewed expiry when ACKNOWLEDGED; 0 otherwise
  bool cancel_requested = 4;                  // as on HeartbeatResponse when ACKNOWLEDGED; false otherwise
}
message AcknowledgeTasksResponse { repeated TaskAcknowledgementResult results = 1; }
```

- A request carries 1 to 64 tasks (the credit of one receive window) with distinct `task_id`s and `attempt` of at least 1;
  each identity field has the bounds `Heartbeat` gives it (the instance path at most 8 segments), so a request stays far below
  the server's message limit. Anything else is `INVALID_ARGUMENT` before any task is read. `worker_id` is validated and
  ignored, as `Heartbeat` does; no field but the lease token is authority.
- A response carries one result per task, in request order. A host that receives another count, an unknown `task_id` or
  `ACKNOWLEDGEMENT_VERDICT_UNSPECIFIED` treats the answer as a failed call.
- `TaskLease`, `PollTasksResponse`, `HeartbeatRequest` and every other message are unchanged.
- The call is in admission class `Lease` (`crates/api-grpc/src/limits.rs`), beside `Heartbeat`, `CompleteStep` and `FailStep`:
  it is the call that makes leased work runnable. It holds one database connection at a time. Its request, its response and the
  element `TaskAcknowledgement`, which carries a token, join `skip_debug`, with a `Debug` that prints counts only.
- A capability call on a provisional lease answers `FAILED_PRECONDITION` with the stable reason `TASK_NOT_ACKNOWLEDGED`, listed
  in the proto's error comment; the SDKs give it the canonical code they give `FAILED_PRECONDITION` today. A host that keeps the
  protocol never meets it.

### Negotiation and activation

- The server's minor becomes 8 (`DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR = 8`). Minors are cumulative: `protocol_ceiling` answers 8
  for an ordinary worker only when it would answer 7 (design 041, task T041-7), the store proves
  `supports_delivery_acknowledgement()` (a constant of the adapter, as the proofs of 1.5 and 1.6 are, true in the build whose
  migrations add the columns) and the engine knows the requirement `delivery-acknowledgement-v1` is recorded. The database runner
  keeps 6. `GET /version` reports the ceiling as it is gated at that moment, not the build's constant.
- An engine lists the capability `delivery-acknowledgement-v1` only when its lease time-to-live is at least 12 s (ADR-0082
  decision 7: a capability is what the configuration will honour); every control plane of the build lists it. So the activation
  refuses while a live engine runs a shorter time-to-live, and an engine restarted with one after the activation is refused and
  closes its gate.
- `control-plane delivery-acknowledgement-activate` (owner role, one bounded transaction, `--operator`, `--expect-live N`,
  `--dry-run`, no step of its own) records the requirement. The registry's `ENABLE ALWAYS` trigger refuses recording it while
  `lease-withdrawal-v1` is not recorded, and refuses removing `lease-withdrawal-v1` while it is recorded (a guard in the existing
  trigger function, `SECURITY DEFINER` with its `search_path` pinned like its siblings), so the hand SQL of
  `docs/runbooks/instance-registry.md` section 5 cannot break the dependency either.
- `control-plane delivery-acknowledgement-remove` removes it in one bounded transaction; then, once the registry's learning bound
  has passed (one registry heartbeat, about 12 s, plus the call: ADR-0082 decision 4), it expires in a second bounded transaction
  every worker session accepted at 1.8, so that their hosts are answered `UNAUTHENTICATED`, register again and are offered at most
  7. It prints how many sessions it expired and how many provisional leases are in flight.
- Every engine of the build serves `AcknowledgeTasks` whatever it knows of the requirement. An engine offers 1.8 and writes new
  marks only while it knows the requirement is recorded; it learns of an activation or a removal within one registry heartbeat
  (gap G006-044). A mark already written keeps its meaning until its lease ends, whatever the requirement says by then.

### The provisional mark

Columns of `steps`, written by the statements that already write a lease (the claim's root statement and the scoped path
statement), never by a statement of their own:

| Column | Written | Meaning |
|---|---|---|
| `lease_offered_generation bigint` | by a claim that marks the lease: the lease's generation | the mark counts only while it equals `generation` |
| `lease_offered_at timestamptz` | with it: the claim transaction's time (`clock_timestamp()` after the lock, before the commit) | the start of the delivery delay |
| `lease_acknowledged_at timestamptz` | cleared by every claim that marks; set by the first acknowledgement | acknowledged when set and the mark counts |
| `lease_unacknowledged_withdrawals smallint NOT NULL DEFAULT 0` | incremented by a `NOT_ACKNOWLEDGED` withdrawal; cleared by an acknowledgement and by any report | the bound of D006-052 |
| `withdrawn_lease_token_hash bytea` | the token hash of a lease withdrawn as `NOT_ACKNOWLEDGED`, replaced by the next such withdrawal | evidence of D006-053, read only on the stale path of a report or heartbeat |

and `worker_sessions.acknowledgement_kept boolean NOT NULL DEFAULT true`, cleared by a protocol violation (D006-053).

- *Provisional* is: the step `LEASED`, `lease_offered_generation = generation`, `lease_acknowledged_at` empty. *Acknowledged* is:
  `lease_offered_generation = generation`, `lease_acknowledged_at` set. A lease written later by any binary has the next
  generation, so a mark left behind by a binary that does not know the columns never counts. No `CHECK` ties the columns to the
  status: an older binary must still be able to write the row.
- A claim marks a lease only when the session was accepted at 1.8, `acknowledgement_kept` holds for it, the engine knows the
  requirement is recorded, its own time-to-live is at least 12 s, and `lease_unacknowledged_withdrawals` is under 3.
- The condition is written once, as a SQL macro of the adapter (as `scoped_timer_metadata_valid!()` is), and the store returns
  the marks as typed values; the application decides from them with one pure function, unit-tested over every combination.
- The marks are lease data, like the token hash and the expiry: they are not journaled and the replay does not derive them.
  Adding nullable columns, and a `smallint` with a constant default, changes the catalogue only.

### The server's acknowledgement

1. Check `worker:execute` once, before anything else (`PERMISSION_DENIED`, audited as today).
2. Read, with one statement, no lock and the unique index of the step address, the rows of the call's addresses (tenant, run,
   node, instance path) whose `lease_token_hash` matches the request's token hash and whose status is `LEASED` or `RUNNING`.
   Read the caller's bound patterns and authorization once. A task with no matching row answers `NOT_HELD` and costs no lock;
   the run ids of the call that do not exist in the caller's tenant are checked with one statement for existence in another
   tenant, and at most one `CrossTenantAccessDenied` audit record is written per call, through the bounded denial gate.
3. For each run with a match, in request order and one after another (never two run locks at once): take the continuation-chain
   lock and then the run row's lock, as `begin` and `begin_path` take them, without restoring any state; write each matched step
   row with one conditional statement: the same tenant, address, token hash, `attempt` and `generation` as the request, the step
   `LEASED` or `RUNNING`, and, when the lease is provisional, `lease_expires_at` still in the future. It sets
   `lease_acknowledged_at = now()` if it was empty, clears `lease_unacknowledged_withdrawals`, renews the lease as a heartbeat
   renews it (`least(now() + time-to-live, attempt_deadline_at)`), and returns the step's activity, node kind, status,
   `cancel_requested`, whether the mark was already set, and the delay `now() - lease_offered_at`. Check the binding on the
   returned activity (for a managed node, the executor's ownership); a refused binding rolls that run's transaction back and
   answers `NOT_HELD`, and one bounded denial audit record per call says so.
4. A provisional lease whose `cancel_requested` holds is not acknowledged (decision D006-051): the transaction restores that
   run (or that path), withdraws the lease with the cause `NOT_STARTED`, and runs the settle; the answer is `CANCELLED`.
5. Commit the run. A run that cannot be opened for a reason other than an unavailable store answers `NOT_HELD` for its tasks and
   the call goes on; an unavailable store ends the call with its canonical status after the runs already committed, and the
   host's repeat answers those `ACKNOWLEDGED` again.
6. Answer. A call of 64 leases over 64 runs must be answered within 3 s (R / 2) at the 99th percentile on the measured topology;
   task T006-064 measures it and the build stops if it is not.

Counted: `worker_task_acknowledgements_total{verdict}` with `acknowledged` (the mark set now), `duplicate` (already set),
`renewed` (a lease with no mark that counts), `not_held` and `cancelled`.

### The withdrawal and the attempt timeout

The lost-lease pass (`crates/infra-postgres/src/maintenance.rs` `lost_leases`, `crates/scheduler/src/passes.rs` `leases`, root
and path) keeps its candidate query; the candidates are only a hint. Under the run lock (the whole-run or the path transaction)
it re-reads the step's status, generation, marks, expiry and attempt deadline (`lease_deadlines_instance`, extended), and the
pure decision function answers one of:

| Under the lock | Outcome |
|---|---|
| Provisional, `lease_expires_at` passed, `attempt_deadline_at` not passed | Withdrawn (ADR-0098 decision 3; root in the whole-run transaction, scoped in the path transaction, as the ADR-0098 build does): `step.lease_withdrawn` with `cause = NOT_ACKNOWLEDGED`, the lease and its marks cleared, `lease_unacknowledged_withdrawals` incremented, `withdrawn_lease_token_hash` kept, the step `READY`, the attempt set back by one, the generation kept; the settle runs, so a step of a stopping run is cancelled at once and a stopping collection or loop drains the instance as one never leased; the step's write is fenced on what was read, and a fence that fails rolls the transaction back and counts `fence_missed` |
| Provisional, `attempt_deadline_at` passed (whether or not the expiry has) | The attempt ends as a known retryable failure, code `ATTEMPT_TIMEOUT_BEFORE_START`: consumed by the step's own timeout, retried by its policy, never consultation or review |
| Acknowledged, or not marked, and expired or timed out | `expire_attempt`, as today: an unknown outcome, consultation or review for external-capable work |
| Neither deadline passed (renewed since the candidate was read) | Nothing |

Counted as `worker_leases_undelivered_total{cause="NOT_ACKNOWLEDGED", outcome}` (`withdrawn`, `fence_missed`) and
`worker_attempts_timed_out_before_start_total`. ADR-0098's own withdrawals also require the lease not to be acknowledged: a
`TASK_NOT_STARTED` for an acknowledged lease is an ordinary retryable failure with a known outcome (decision D006-045).

**Exactly one of the acknowledgement and the withdrawal commits for a lease.** Both hold the run lock. The acknowledgement writes
only while the token, attempt and generation match a `LEASED` or `RUNNING` step and a provisional lease is not past its expiry;
the withdrawal writes only while the lease it re-read under the lock is still provisional and past its expiry. If the
acknowledgement commits first, the withdrawal reads `lease_acknowledged_at` set, or an expiry renewed into the future, and does
nothing; if the withdrawal commits first, the acknowledgement finds the token cleared and answers `NOT_HELD`; an acknowledgement
that arrives after the expiry but before the pass answers `NOT_HELD` and writes nothing, so the deadline is the expiry. The host
runs a handler only on `ACKNOWLEDGED`. So a handler of a 1.8 lease runs only after the server committed that its host holds it,
and a lease the server withdraws was not run by a host that keeps the protocol (the trust assumption, D006-053). No clock enters
the argument; the deadline decides when the pass looks, never whether both can happen.

### Why the deadline is the lease expiry

The acknowledgement deadline is the lease's expiry at the claim: the claim's time plus the lease time-to-live
(`SAGAT_LEASE_TTL_SECS`, default 30 s). The derivation uses specified bounds only:

1. **R = 6 s, the receive bound.** A host bounds each receive call to 6 s or its shorter configured call timeout (`sdks/SPEC.md`
   §10 for `StreamTasks`; at 1.8 the same bound applies to `PollTasks`, D006-048). It starts that clock before it sends the
   request and the server claims only after it has the request, so any lease the host reads, it reads within R of the lease's
   claim, in real time, with no comparison of clocks. A lease not read by then is never read: the host has cancelled the call.
2. **R again, for one acknowledgement attempt.** A host of 1.8 bounds each `AcknowledgeTasks` attempt to R as well, and sends it
   as soon as the frame is read. So one complete attempt ends within 2R = 12 s of the claim.
3. **The threshold.** One complete attempt fits before the expiry when the time-to-live is at least 2R = 12 s; below that a host
   that reads its frame at the end of its receive bound has no full attempt left. An engine whose time-to-live is under 12 s does
   not list the capability (above). At the default 30 s, (30 − 6) / 6 = 4 complete attempts fit after the latest read.
4. **The interval of safe deadlines.** Safety does not depend on the deadline (the run-lock argument above). Between the healthy
   delivery time and the expiry the deadline trades only how long a lost lease holds its step and its session's slot (longer
   with a later deadline) against how many healthy deliveries are withdrawn when the database is slow (more with an earlier one),
   and each such withdrawal claims and delivers the task again, adding work when the system is slowest.
5. **The choice: the upper end.** The expiry adds no setting, no column and no scan, and it is the bound a host already meets
   today with its first heartbeat, so no host that keeps its lease today loses it. A lost lease is withdrawn at the claim's time
   plus the time-to-live plus the period of the lost-lease pass, when it lapses today.
6. **What is measured, and what must hold.** Measured on one machine with no network: ready to handler entry 751 to 889 ms at
   the 99th percentile (the ten burst rows of `tests/performance/reports/sdk-streaming.md`), which bounds the claim to handler
   entry from above; a completion 6 to 103 ms at the 99th percentile (`tests/performance/reports/scalability-ha.md`), which bounds
   one acknowledgement transaction from above, since a completion restores the run and applies a transition and the
   acknowledgement does neither. The server must answer a full call (64 leases over 64 runs) within R / 2 = 3 s at the 99th
   percentile, measured by task T006-064 with a network between the workers and the services; R / 2 leaves the other half of an
   attempt for the network and the host. At 103 ms per run, which overstates an acknowledgement, 64 runs one after another take
   6.592 s, so the requirement is not met by assumption: it is the measurement's to show, and the build stops if it does not.

### The host at 1.8

The normative host text is `sdks/SPEC.md` §6.2 to §6.4 and §6.8. In short:

1. **Receive.** A `PollTasks` call is bounded to R, as a stream window is. The excess over the credit and the leases received
   while stopping are handed back with `TASK_NOT_STARTED` and are not acknowledged.
2. **Acknowledge.** As soon as a frame or a poll answer is read, the host sends one `AcknowledgeTasks` with all its leases that
   it will run, concurrently with its next receive. Nothing else is called for those leases before the answer.
3. **Verdicts.** `ACKNOWLEDGED`: dispatch the task (§6.3) with the result's expiry as its deadline; the heartbeat starts now, its
   interval a third of the time left on that expiry. `ACKNOWLEDGED` with `cancel_requested = true` (the lease was already
   acknowledged, or carries no mark): dispatch, with the token already cancelled with the reason `run-cancelled`, as a heartbeat
   would have done. `NOT_HELD` or `CANCELLED`: drop the task, run nothing, report nothing, free its slot, emit `dropped` with the
   code `NOT_HELD` or `CANCELLED`.
4. **A call that fails.** `UNAVAILABLE`, `RESOURCE_EXHAUSTED` (after its retry hint), a retryable `INTERNAL`, a transport
   failure, a deadline or an unusable answer: repeat the same call, without the leases already decided or handed back, with the
   backoff of §5, each attempt bounded to R. `UNAUTHENTICATED`: repeat once with a fresh token; the lease does not depend on the
   session. The host stops trying for a lease when the lease's time left at receipt (its expiry minus the host's clock at receipt,
   clamped to between 12 s and 120 s) has passed on its monotonic clock; that lease is not run and is handed back. A verdict that
   arrives after the host gave a lease up is ignored.
5. **A call that is refused.** `UNIMPLEMENTED`, `PERMISSION_DENIED`, `INVALID_ARGUMENT` or any other status not listed in item 4:
   hand the call's leases back, log once, and register again offering 1.7 for the rest of the host's life (the precedent of
   ADR-0029 decision 6 for `StreamTasks`).
6. **Logs.** The happy path logs nothing new; `NOT_HELD`, `CANCELLED`, a lease given up and a refusal log one line each, with
   identifiers only.

### Scoped instances

A collection item or loop body leased to a 1.8 session is marked, acknowledged, refused capabilities and withdrawn by the same
rules. The pre-read and the conditional write address the step row by its full address (the instance path columns), so they do
not depend on the path; the lock is the run row's, which `begin_path` takes too. The withdrawal is the scoped withdrawal of the
ADR-0098 build (its decision on the grain of the transaction: one path transaction per run and instance path, with the addressed
event); the capability refusal is a variant of `live_path_fence`; a stopping collection or loop drains a withdrawn instance as
one never leased. Each of these has a scoped row in the evidence of its task.

### Rollback

1. `control-plane delivery-acknowledgement-remove`: the requirement is removed; after the learning bound the sessions accepted at
   1.8 are expired; from then on no engine offers 1.8 or writes a new mark.
2. Wait until `worker_leases_provisional` is 0, at most one lease time-to-live after step 1: every mark already written ends with
   its lease (acknowledged, withdrawn or timed out), with its meaning unchanged.
3. Roll the binary back, no further than a build that decodes the cause `NOT_ACKNOWLEDGED`: the ADR-0098 build if it adds the
   cause to its closed set before it merges (D006-059), otherwise this build. A journal holding the cause cannot be replayed by
   an older binary; that is why the floor exists.
4. The `down` migration refuses while any mark counts (a provisional or acknowledged lease of a 1.8 session); marks that no
   longer count are dropped with the columns.

### Failure modes and their exact outcomes

| Case | Outcome |
|---|---|
| A frame or a `PollTasks` answer lost on the wire; the host crashes before acknowledging; the engine crashes between the claim and ADR-0098's withdrawal | No acknowledgement; at the expiry the lease is withdrawn (`NOT_ACKNOWLEDGED`): `READY`, attempt not consumed, no review; the next lease has the same attempt number and the next generation |
| The acknowledgement never leaves the host | Repeated (host item 4); on a verdict the task runs once; if the host gives the lease up first, nothing runs and it hands the lease back (`TASK_NOT_STARTED` on a provisional lease: withdrawn, `NOT_STARTED`) |
| The acknowledgement's answer is lost after its commit | The host repeats; the server answers `ACKNOWLEDGED` again (`duplicate`); the handler runs once. If the host gives up before any answer, nothing runs and the hand-back of the now acknowledged lease is an ordinary retryable failure (attempt consumed, known outcome); if the hand-back cannot reach the server either, the lease lapses into a review (gap G006-041) |
| A duplicate acknowledgement | `ACKNOWLEDGED` with the current expiry; only the renewal is written |
| An acknowledgement after the expiry, before the pass | `NOT_HELD`; nothing written; the pass withdraws the lease |
| An acknowledgement after the withdrawal, or after the lease was replaced | `NOT_HELD`; the host drops the task |
| Acknowledgement and withdrawal at the same moment | Exactly one commits; either the task runs once or it is withdrawn and the host drops it |
| The host stops, or crashes, after `ACKNOWLEDGED` and before the handler starts | Stopping: it hands the lease back, an ordinary retryable failure. Crashing: the lease lapses into a review (gap G006-040; the window is in-process and holds no call) |
| The run's cancellation is requested before the acknowledgement | `CANCELLED`: withdrawn (`NOT_STARTED`) and cancelled in the acknowledgement's transaction; the handler never runs; no attempt, no review. A compensation or reconciliation is never asked to stop, so this never happens to one |
| The run is cancelled while the lease is provisional and the host never answers | Withdrawn at the expiry; the settle cancels the step, except a compensation or reconciliation, which the planner keeps and leases again |
| The attempt deadline passes before the acknowledgement | `ATTEMPT_TIMEOUT_BEFORE_START`: a known retryable failure, the attempt consumed by the step's own timeout; a later acknowledgement is `NOT_HELD` |
| A reset while the lease is provisional | The lease is replaced or cleared, with a new generation; the acknowledgement answers `NOT_HELD` |
| A task whose token does not match (forged, old, another tenant's run) | `NOT_HELD`, no lock taken; at most one audit record per call; the real holder's acknowledgement is unaffected |
| A binding revoked after the claim | `NOT_HELD`, one bounded audit record; the lease is withdrawn at its expiry and leased to a bound worker |
| A capability call on a provisional lease | `FAILED_PRECONDITION`, `TASK_NOT_ACKNOWLEDGED`, after the token and binding matched; nothing handed out; counted |
| A `Heartbeat`, `CompleteStep` or `FailStep` on a provisional lease | Served as today (the lease becomes `RUNNING` or reported, so it is never withdrawn); counted as a protocol violation; the session's later leases are not provisional |
| A report or heartbeat carrying the token of a lease withdrawn as `NOT_ACKNOWLEDGED` | Refused as stale, as today; counted `worker_reports_after_withdrawal_total`, audited and alerted: a handler ran on a lease the journal says never ran (gap G006-042) |
| Three withdrawals of one step in a row | The next lease is not provisional; a fourth loss lapses as today |
| `AcknowledgeTasks` refused (`UNIMPLEMENTED` from a proxy, a rolled-back replica) | The host hands the leases back and registers again offering 1.7 for the rest of its life |
| A host clock skewed by more than the time-to-live | Its give-up time is clamped to between 12 s and 120 s on its monotonic clock; it acknowledges as any other host |
| One busy run in a frame | The call waits for that run's lock; the call's 3 s bound is measured with such a run (task T006-064) |
| An engine restarts or fails over between the claim and the acknowledgement | The lease and its marks are in PostgreSQL; any engine of the build serves the acknowledgement |
| The requirement is removed while provisional leases exist | Their marks keep their meaning until their leases end; no new mark after one registry heartbeat |
| A hand-written client at 1.8 that runs a handler without acknowledging and makes no call during the lease (a slow handler, a lost report, a crash) | Withdrawn and run again with the same attempt number and idempotency key (gap G006-042); a later call with its token is counted and alerted |

### Observability

- `worker_task_acknowledgements_total{verdict}` (`acknowledged`, `duplicate`, `renewed`, `not_held`, `cancelled`) and
  `grpc_requests_total{method="AcknowledgeTasks", code}`.
- `worker_leases_undelivered_total{cause, outcome}` gains the cause `NOT_ACKNOWLEDGED` (ADR-0098's counter).
- `worker_task_delivery_delay_seconds`: the database-clock delay from the claim to the acknowledgement
  (`lease_acknowledged_at - lease_offered_at`), returned by the acknowledgement's own statement.
- `worker_task_acknowledgement_call_seconds`: the server-side duration of one call, by the number of runs in it (`1`, `2-8`,
  `9-64`): the quantity of item 6 of the derivation.
- `worker_leases_provisional` and `worker_leases_provisional_expired`: gauges without labels from the operations statistics query
  (`crates/infra-postgres/src/operations.rs`), not the hot path; `worker_lease_ttl_seconds`, the configured time-to-live.
- `worker_leases_expired_total{mark}` (`acknowledged`, `unmarked`) for the leases that lapse into today's meaning, since the
  journal does not record that a lapsed lease had been acknowledged (gap G006-046).
- `worker_attempts_timed_out_before_start_total`, `worker_capability_calls_refused_total{method, reason="TASK_NOT_ACKNOWLEDGED"}`,
  `worker_protocol_violations_total{method}` and `worker_reports_after_withdrawal_total`.
- Alerts, with `promtool` cases: `SagatLeasesNotAcknowledged`, withdrawals with the cause `NOT_ACKNOWLEDGED` above 1 a minute
  over 10 minutes (frames are being lost, or hosts are not acknowledging), or with the cause `NOT_STARTED` above 1 a minute for
  30 minutes (longer than a rolling deploy, whose stopping hosts hand leases back);
  `SagatLeaseWithdrawalBound`, any step reaching the bound of three withdrawals; `SagatLeaseDeliverySlow`, the 99th percentile of
  the delivery delay over 10 minutes above R, computed in the rule (a healthy host reads within R, so a higher figure means the
  acknowledgements themselves are slow); `SagatReportsAfterWithdrawal`, any increase of that counter.
- The call's span links to the trace context of each lease answered `ACKNOWLEDGED` (at most 64), never as a parent. No log line,
  metric label or span attribute carries a token, a payload or an unbounded identifier.
- SDK metrics hooks gain `acknowledgements`, `notHeld` and `acknowledgementFallbacks` counters.

### Acceptance matrix

| Area | Required evidence |
|---|---|
| Exactness | Barrier tests: the acknowledgement holds the run lock while the withdrawal waits, the reverse, and the candidate read taken before an acknowledgement that renews the lease: exactly one commits, the handler would run once; an acknowledgement after the expiry and before the pass is `NOT_HELD` |
| Withdrawal | A provisional `Activity` lease, root and scoped, that expires: `step.lease_withdrawn` cause `NOT_ACKNOWLEDGED`, no `step.needs_review`, the attempt not consumed and reused by the next lease; an acknowledged lease that expires is an unknown outcome; a `TASK_NOT_STARTED` for an acknowledged lease is a retryable failure |
| The mark | A lease written with the old column list after a mark is not provisional and is not withdrawn; the bound of three; a session that broke the rule gets no mark |
| Attempt timeout | `timeoutMs` counted from the claim at 1.8 and below; a provisional lease past its attempt deadline ends `ATTEMPT_TIMEOUT_BEFORE_START`, known, retried |
| Cancellation | Cancel before the acknowledgement, with and without declared secrets: `CANCELLED`, the run ends `CANCELLED`, no handler, no review |
| Capabilities | The descriptor-driven test: every lease-token method but the four is refused on a provisional lease, root and scoped, and served after the acknowledgement; a report on a provisional lease is served; a wrong token gets the ordinary stale answer |
| Gate | No mark, no 1.8 and no new `NOT_ACKNOWLEDGED` while the requirement is not recorded; the trigger refuses both orders; an engine under 12 s does not list the capability; the removal expires the 1.8 sessions |
| Older minors | A session at 1.7 and below gets no mark; the scenarios pinned at 1.4 and 1 keep their goldens |
| Cost | `LEASE` and both completion lines unchanged at 1.8; `ACKNOWLEDGEMENT` (one task) and `FIRST_HEARTBEAT` pinned, the former with no step scan and no journal or outbox insert and fewer statements than the latter; a call of 64 runs measured |
| Six SDKs | `delivery-acknowledgement` passes in six languages with a reviewed golden; the 26 scenarios that pin no minor and the 29 pinned at 1.5, run at 1.8 by the harness override, pass with their existing goldens unblessed; only `protocol-negotiation` and the new scenario have new goldens |
| Released v0.2 | `provisional-lease-cutover` forward and its rollback case |
| Throughput | The transitions scenario at 1.8 against 1.7, same machine and day, with a network between workers and services: no more than 15% lower, the call bound met, the delays recorded |
