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
| After the frame is handed to the transport, lost on the wire | An unflagged lease: lapses (Gap 006-001); since feature 037 a lapsed lease of an `Activity` is an unknown outcome and goes to reconciliation or review. A lease flagged at 1.8: provisional, withdrawn at its own earlier deadline, no attempt consumed, no review ("Delivery acknowledgement (protocol 1.8)" below, Architecture decision 0099) |
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
  bounded valid work, cancel the receive, record a closed diagnostic and hand the excess leases
  back with `TASK_NOT_STARTED` (`sdks/SPEC.md` §6.2; at most eight hand-backs on their way at once).
  Never grow an unbounded queue of handlers or concatenate all frames.
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
   `NOT_STARTED`), but only from a session accepted at 1.8 or later, which reserves the code for the host, within the
   cap of four withdrawals per attempt (Decision 006-031 of the server build), and never for an acknowledged lease of 1.8, which is an attempt (the server review of
   pull request #260; "Delivery acknowledgement (protocol 1.8)" below); otherwise, and always from an older session, it is an
   ordinary retryable failure, never an unknown outcome.
6. **What is not covered.** A frame lost on the wire after it was handed to the transport, and a lost unary answer, still
   lapse (Gap 006-001) when unflagged; for a lease flagged at 1.8 delivery acknowledgement closes it (next section). A process that stops between a claim and its withdrawal leaves
   the lease to lapse (Gap 006-002); a flagged lease is provisional and is withdrawn at its own earlier deadline instead.





## Delivery acknowledgement (protocol 1.8)

Decision 006-040 to Decision 006-061 of [`research.md`](research.md); Architecture decision 0099; Owner decision 2026-11-20-12.
Minor 1.8 adds a flag on the lease (`acknowledge_before_start`), a call (`AcknowledgeTasks`) and a registration field
(`declines_acknowledgement`), and is the minor at which the hand-back code `TASK_NOT_STARTED` is reserved for the host
(Architecture decision 0098 decision 5). A flagged lease is *provisional* until its host acknowledges it; the host runs nothing and
calls nothing for it but the acknowledgement and the hand-back before the server has committed that acknowledgement; a provisional
lease whose earlier deadline passes is withdrawn (the cause `NOT_ACKNOWLEDGED`), so a frame lost on the wire and a lost `PollTasks`
answer (Gap 006-001) no longer become a review. An acknowledged lease is an attempt, with today's meaning in every respect.
Unflagged leases, and every lease of a session below 1.8, are served exactly as before.

In this section "the receive bound" is the 6 s a host gives one receive call (`sdks/SPEC.md` §10).

### Wire

Added to `specs/001-platform-foundation/contracts/worker.proto`, additively (the six SDK copies stay byte-identical):

```protobuf
message WorkerCapabilities {
  // ... fields 1 to 5 unchanged ...
  // Added in protocol 1.8. The host asks for no flagged lease (its fallback, §6.2); the server never marks one for it.
  bool declines_acknowledgement = 10;
}
message TaskLease {
  // ... fields 1 to 18 unchanged ...
  // Added in protocol 1.8. Set only on a lease the server marked provisional: the host starts nothing and calls nothing for
  // it but AcknowledgeTasks and the hand-back (FailStep TASK_NOT_STARTED) before the answer ACKNOWLEDGED.
  bool acknowledge_before_start = 23;
}

// Added in protocol 1.8. The host acknowledges the flagged leases of one frame, or of one PollTasks answer, before it runs or
// calls anything else for them. Idempotent: the same lease may be acknowledged again.
rpc AcknowledgeTasks(AcknowledgeTasksRequest) returns (AcknowledgeTasksResponse);

message TaskAcknowledgement {
  string task_id = 1; string run_id = 2; string step_id = 3;
  int32 attempt = 4; int64 generation = 5; string lease_token = 6;
  ExecutionInstanceIdentity instance = 7;
}
message AcknowledgeTasksRequest { string worker_id = 1; repeated TaskAcknowledgement tasks = 2; }
enum AcknowledgementVerdict {
  ACKNOWLEDGEMENT_VERDICT_UNSPECIFIED = 0;
  ACKNOWLEDGEMENT_VERDICT_ACKNOWLEDGED = 1;   // committed: this host holds the lease; run it
  ACKNOWLEDGEMENT_VERDICT_NOT_HELD = 2;       // not this host's provisional lease (withdrawn, replaced, too late, unbound)
  ACKNOWLEDGEMENT_VERDICT_CANCELLED = 3;      // asked to stop before it started: hand the lease back, run nothing
  ACKNOWLEDGEMENT_VERDICT_RETRY = 4;          // its row was busy: ask again for this task
}
message TaskAcknowledgementResult {
  string task_id = 1;
  AcknowledgementVerdict verdict = 2;
  int64 lease_time_left_ms = 3;               // when ACKNOWLEDGED: the time left before the lease's expiry, measured under the lock
}
message AcknowledgeTasksResponse { repeated TaskAcknowledgementResult results = 1; }
```

Field 23 of `TaskLease` and field 10 of `WorkerCapabilities` are chosen above every number the drafts 019 and 031 name for those
messages (19 to 22, and 6).

- A request carries 1 to 64 tasks (the credit of one receive window) of one namespace, with distinct `task_id`s and `attempt` of
  at least 1; each identity field has the bounds `Heartbeat` gives it, and `step_id`, when present, is checked as `verify` checks
  it. The addresses reach SQL as bound arrays. `worker_id` is validated and ignored, as `Heartbeat` does; the lease token is the
  only authority. Tasks of more than one namespace, or anything else malformed, is `INVALID_ARGUMENT` before any lock is taken.
- A response carries one result per task, in request order; another count, an unknown `task_id` or `UNSPECIFIED` is a failed
  call for the host. The acknowledgement renews nothing; `lease_time_left_ms` is a duration, not an instant, so the host never
  compares clocks with it.
- A capability call on a provisional lease answers `FAILED_PRECONDITION` with the stable reason `TASK_NOT_ACKNOWLEDGED`, listed
  in the proto's error comment; the SDKs give it the canonical code they give `FAILED_PRECONDITION` today.
- Every message that carries a `lease_token` is in `skip_debug`, with a `Debug` that prints counts only; the descriptor test of
  Task 006-052 fails for one that is not, so the older request types are closed too. Each SDK has a token canary on its forms.
- Admission: a class of its own, `Acknowledge`, admitted while the requests in flight are under the discovery threshold plus
  `max(1, (limit - discovery threshold) / 2)`, so that at least half of the reserve, and never less than one place, stays for the
  calls of the class `Lease` (`Heartbeat`, `CompleteStep`, `FailStep`, `ResolveSecrets`, `ResolveDatabase`, and the transfers that
  share its limit). `AppendStepLogs` of 1.7 is classified explicitly in the same change. A shed acknowledgement costs at worst a
  withdrawal; a shed heartbeat can cost a review.

### The hand-back at 1.8

Architecture decision 0098 decision 5, as amended by its build review (Decision 006-030 and Decision 006-031 on its branch), holds
the server's rule: a `TASK_NOT_STARTED` withdraws the lease (cause `NOT_STARTED`, no attempt consumed, no review) only from a lease
under the reserved code, while `lease-withdrawal-v1` is recorded, the engine's `SAGAT_LEASE_WITHDRAWAL` is on, the step is `LEASED`
with the issued fence, and the attempt's withdrawals are under four; otherwise it is an ordinary failure, the attempt spent, the
flag the host sent honoured, never a review. This design adds:

- **The reservation is read from the lease.** The claim writes `lease_hand_back_generation` when the claiming session was accepted
  at 1.8 or later; the hand-back decides from it, under the run lock, never from the session row, which registering again under the
  same worker name overwrites.
- **Not for an acknowledged lease**, which is an attempt (Decision 006-045).
- **Not after a capability was served**: a hand-back of a lease whose `lease_capability_generation` is its generation is an ordinary
  failure, counted and audited as a protocol violation (Decision 006-053).
- **The order of locks**: the binding checks of the report path come before the re-read of the step row `FOR UPDATE`, in every
  withdrawal path.

The host sends the code only for a lease no handler of which started: a lease over its credit, received while stopping, given up,
answered `CANCELLED`, or waiting when it falls back. The SDKs rewrite a handler-raised `TASK_NOT_STARTED` to `WORKER_ERROR` at every
minor (Decision 006-028 of the SDK build), and keep at most eight hand-backs on their way at once (Decision 006-026 of that build).

### Negotiation and activation

- The server's minor becomes 8 (`DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR = 8`) in Task 006-057, after the gate of Task 006-056,
  never before: until then the tests of the 1.8 paths raise a session's stored minor, as Decision 006-030 of the server build does.
  Minors are cumulative: `protocol_ceiling` answers 8 for an ordinary worker when it would answer 7 (design 041, Task 041-7) and the
  store proves `supports_delivery_acknowledgement()`, a constant of the adapter. Nothing else gates the minor. The database runner
  keeps 6.
- From the same task every control plane and engine of the build lists `delivery-acknowledgement-v1`. An engine with
  `SAGAT_LEASE_WITHDRAWAL=off` still lists it (it serves the call and replays the cause, as Decision 006-032 of the server build
  says of `lease-withdrawal-v1`) but marks no lease; a marked lease whose withdrawal falls to such an engine lapses as before,
  counted `disabled`.
- `control-plane capability-activate delivery-acknowledgement-v1` (the generic command of Decision 006-034 of the server build) has
  an entry for it: both kinds, and "requires `lease-withdrawal-v1` for both kinds". The dependency is a row of a small owner-only
  table, `platform_requirement_dependencies`, seeded by the migration; the command checks it under the epoch lock for a typed
  refusal, and the registry's `platform_requirements_guard` (an invoker function with schema-qualified names, which takes the epoch
  row first) reads it to refuse recording a requirement whose dependency is not recorded for every kind it needs, and removing a
  dependency while a requirement that needs it is recorded. So the hand SQL of `docs/runbooks/instance-registry.md` section 5
  cannot break it either.
- Every engine of the build serves `AcknowledgeTasks` whatever it knows of the requirement. An engine marks leases only while it
  knows the requirement is recorded; it learns of an activation or a removal within one registry heartbeat (Gap 006-044). A mark
  already written keeps its meaning until its lease ends.

### The marks

Columns of `steps`, all written for a lease's generation and counting only while it is the step's `generation` and the step is
`LEASED` or `RUNNING`:

| Column | Written by | Meaning |
|---|---|---|
| `lease_hand_back_generation bigint` | the claim, when the claiming session was accepted at 1.8 or later | the lease is under the reserved hand-back |
| `lease_offered_generation bigint` and `lease_offered_at timestamptz` | the claim, when it marks the lease provisional (and sets the flag): the generation, and the claim's time (`clock_timestamp()` after the lock) | provisional until acknowledged; the start of the delivery delay |
| `lease_acknowledged_at timestamptz` | cleared by a claim that marks; set by the acknowledgement | acknowledged |
| `lease_capability_generation bigint` | the transaction a served capability call already commits | a capability was served to this lease |
| `withdrawn_lease_token_hash bytea` | a withdrawal with the cause `NOT_ACKNOWLEDGED`; replaced by the next | evidence, read only on the stale path of a call |

and `worker_sessions.declines_acknowledgement boolean NOT NULL DEFAULT false`, written by the host's own registration.

- The claim writes them in the statement that already writes the lease, in each of the three: `upsert_step` (root),
  `upsert_scoped_instance` (a scoped instance claimed in a full-run transaction) and the path statement of `execution/path.rs`.
- It marks the lease provisional when the session was accepted at 1.8 and does not decline acknowledgement, the engine knows the
  requirement is recorded and its `SAGAT_LEASE_WITHDRAWAL` is on, the lease's earlier deadline (the earlier of the claim's time plus
  the engine's time-to-live and the claim's time plus `timeoutMs`) is at least three receive bounds (18 s) after the claim, and the
  attempt's withdrawals so far are under the cap of four (the count the domain's withdrawal command reads, from the run state the
  claim already restored). Otherwise the lease is not marked, the flag is false, and `worker_lease_marks_skipped_total{reason}`
  (`short_deadline`, `attempt_capped`, `declined`, `not_activated`, `switch_off`) counts it.
- No statement assigns a column of the marks from a value it read earlier: the engine's statements keep them (`ELSE` the column)
  or clear them with the lease, the acknowledgement sets `lease_acknowledged_at = clock_timestamp()` only, the capability paths set
  `lease_capability_generation = generation` only. A static check in `scripts/check-architecture.py` fails on any other assignment
  in `crates/infra-postgres` and the migrations; a barrier test holds a run transaction that loaded the run before an
  acknowledgement and makes it write the row through each statement that can touch a `LEASED` row.
- No `CHECK` ties the columns to the status: an older binary must still be able to write the row. A mark left by a binary that does
  not know the columns never counts, because the next lease has the next generation.
- The condition is one SQL macro of the adapter and one pure function of the application, which extends `LeaseDeadlines`
  (`crates/application/src/execution.rs`); a parity test in `integration_durable_execution` writes every combination of a row and
  compares them.
- The gauges `worker_leases_marked`, `worker_leases_provisional` and `worker_leases_provisional_overdue` are `FILTER` clauses of the
  scan the operations statistics query already makes of leased steps (`crates/infra-postgres/src/operations.rs`); no index is added.
- The marks are lease data: not journaled, not derived by the replay, and no test of replay against projection asserts them.

### The server's acknowledgement

One transaction per call, through a store port of its own (`AcknowledgementTx`: `matched`, `bound_patterns`,
`current_worker_authorized`, `lock_matched`, `acknowledge`); no run lock:

1. `worker:execute`, once (`PERMISSION_DENIED`, audited as today). `SET LOCAL lock_timeout` and `statement_timeout` to half the
   receive bound (3 s), for every wait of the transaction.
2. `matched`: one statement with no lock, by the step primary key (`steps_pkey`), reads the addressed rows whose `lease_token_hash`
   matches the request's token hash: their namespace and activity. Tasks of more than one namespace: `INVALID_ARGUMENT`. A task with
   no such row answers `NOT_HELD`. The run ids with no row in the caller's tenant are checked for existence in another tenant with
   one statement, and at most one `CrossTenantAccessDenied` record per call is written, spawned off the answer path.
3. The checks of `ensure_bound`, under its locks and in its order, for the call's one namespace: the binding's shared advisory lock
   and the namespace's bound patterns, then the namespace's shared secrets lock and the membership's row lock with
   `current_worker_authorized`. Coverage is decided per row with `first_uncovered` on its activity; a row of a managed or
   non-activity node is never marked and answers `NOT_HELD`. A refused binding answers `NOT_HELD` for its rows, and the call returns
   at most one `AccessDenial`, which the adapter audits through `spawn_audit_access_denied` whatever the call's status. These locks
   come before any row lock, as in every other path, so a revocation (which takes the binding's lock exclusively, then rows) cannot
   form a cycle with the acknowledgement.
4. `lock_matched`: one statement locks the bound rows `FOR UPDATE OF steps SKIP LOCKED`, in primary-key order, and reads, with them,
   their status, generation, attempt, token hash, marks, deadlines, `cancel_requested` and the run's stop reason. A row it could not
   lock answers `RETRY` and costs nothing; the statement never waits for a row.
5. `clock_timestamp()` is sampled after that statement, and the pure function decides each locked row: provisional with the
   request's token, attempt and generation, no stop asked, at least one receive bound left before both `lease_expires_at` and
   `attempt_deadline_at` (when set): it wins; already acknowledged with the same token: `ACKNOWLEDGED` (counted `duplicate`);
   provisional with `cancel_requested` or a run stop reason: `CANCELLED`; anything else: `NOT_HELD`, with its reason.
6. `acknowledge`: one update of the winning rows, which the transaction already holds: `lease_acknowledged_at = clock_timestamp()`.
   It returns the delay since `lease_offered_at` and the time left before `lease_expires_at`, which the answer reports.
7. A deadlock, a serialization failure, a lock or statement timeout, or an unavailable store ends the call with a retryable status;
   the host repeats it, and the rows already written answer `ACKNOWLEDGED` again. No transient error is ever answered `NOT_HELD`.

Nothing else: the acknowledgement does not restore the run, renew the lease, apply a transition, write the journal or the outbox.
About eight statements per call whatever its size (the settings, `matched`, two for the binding, two for the membership,
`lock_matched`, `acknowledge`), plus the triggers of `steps` the update fires, which
the budget's top-level count does not show and Task 006-055 itemises. Counted: `worker_task_acknowledgements_total{verdict, reason}`
with the verdicts `acknowledged`, `duplicate`, `not_held` (reasons `no_match`, `too_late`, `not_provisional`, `unbound`), `cancelled`
and `retry`.

### The withdrawals

Every withdrawal of a lease whose mark counts, under the run lock (the whole-run or the path transaction) and after its binding
checks, re-reads the step row `FOR UPDATE` and decides with the pure function on what it read:

| Under the row lock | Outcome |
|---|---|
| Provisional, and its expiry or its attempt deadline passed by `clock_timestamp()` | The lost-lease pass withdraws it (Architecture decision 0098 decision 3; root in the whole-run transaction, scoped in the path transaction, as its build does): `step.lease_withdrawn` with `cause = NOT_ACKNOWLEDGED`, the lease and its marks cleared, `withdrawn_lease_token_hash` kept, the step `READY`, the attempt set back by one, the generation kept; the settle runs, so a step the planner stops is cancelled at once (a compensation or reconciliation is kept and leased again) and a stopping collection or loop drains the instance as one never leased. The withdrawal counts toward the cap of four, which the claim checked when it marked the lease, so this one is always within it |
| Not acknowledged, a hand-back under the reserved code (the rule above) | The withdrawal of Architecture decision 0098, cause `NOT_STARTED` |
| Acknowledged, and expired or timed out | `expire_attempt`, as today: an unknown outcome, consultation or review; `worker_leases_expired_total{mark="acknowledged"}` |
| `RUNNING` with an unacknowledged mark (a host that heartbeated first) | `expire_attempt`, as today; the heartbeat was counted as a violation |
| Not marked | As today, and as Architecture decision 0098 decides |
| Anything else (acknowledged since the candidate was read, replaced) | Nothing; `fence_missed` |

The candidate query of the pass returns the marks only as a hint; the decision is the re-read's.

**Exactly one of the acknowledgement and a withdrawal commits for a lease.** The acknowledgement writes a row only while it holds
the row's lock and found it provisional with the request's fence and at least one receive bound before its deadlines; each
withdrawal takes the same row lock and writes only while it is still provisional (for the pass, past a deadline; for a hand-back,
not acknowledged). If the acknowledgement holds the row first, the withdrawal reads `lease_acknowledged_at` set and does nothing; if
a withdrawal holds it first, the acknowledgement skips the row (`RETRY`) and, asked again, finds the token cleared (`NOT_HELD`). An
acknowledgement repeated while its first attempt is still committing skips the row too and is answered `ACKNOWLEDGED` on the next
ask. The host runs a handler only on `ACKNOWLEDGED`. So a handler of a flagged lease runs only after the server committed that its
host holds it, and a lease the server withdraws was not run by a host that keeps the protocol (the trust assumption, Decision
006-053). The deadline decides when the pass looks, never whether both can happen; the losing side of the race between the deadline
(a timeout) and the acknowledgement (the host's signal) is the journal's withdrawal event or the counted `NOT_HELD` (constitution
principle I).

### Why the deadline is the lease's own

The acknowledgement deadline is the lease's earlier deadline: the earlier of the claim's time plus the time-to-live and the claim's
time plus `timeoutMs`. No other deadline is added. The derivation uses specified bounds only:

1. **The receive bound (6 s).** A host bounds each receive call to 6 s or its shorter configured call timeout (`sdks/SPEC.md` §10
   for `StreamTasks`; at 1.8 the same bound applies to `PollTasks`, Decision 006-048). It starts that clock before it sends the
   request and the server claims only after it has the request, so any lease the host reads, it reads within the receive bound of
   the claim, in real time, with no comparison of clocks.
2. **One acknowledgement attempt (6 s).** The host bounds each attempt to the receive bound and sends it as soon as the frame is read.
3. **One receive bound of margin, for each deadline and for its own reason.** Before the expiry: the acknowledgement renews nothing,
   so the host's first heartbeat must commit before the lease can expire; the host sends it at the earlier of its interval and a
   third of the `lease_time_left_ms` the answer reports, whatever interval it is configured with, so it comes at most 2 s after the
   answer and has the rest of the margin to commit. Before the attempt deadline: a heartbeat does not move it, so the margin is what
   the handler is sure to have of its timeout when it starts; a handler that started with less would most likely time out into an
   unknown outcome, while a withdrawal costs nothing.
4. **The threshold.** A read, one complete attempt and the margin fit when the earlier deadline is at least three receive bounds
   (18 s) after the claim; a lease with less is not marked and keeps today's meaning. At the default 30 s time-to-live and no attempt
   timeout, at most (30 − 6 − 6) / 6 = 3 complete attempts fit after the latest read, fewer with the backoff of §5 between them.
5. **Why not another deadline.** Safety does not depend on the deadline (the row-lock argument above). An earlier deadline would
   withdraw healthy deliveries when the database is slowest; the lease's own deadlines add no setting, no column and no scan. A lost
   lease is withdrawn at its earlier deadline plus the period of the lost-lease pass, when it lapses today.
6. **What is measured, and what must hold** (Task 006-056, before any SDK work and before the server offers 1.8). Measured on one
   machine with no network: ready to handler entry 751 to 889 ms at the 99th percentile (`tests/performance/reports/sdk-streaming.md`),
   which bounds claim to handler entry from above. The gate requires a 64-lease call within half the receive bound (3 s) at the 99th
   percentile, half of the receive bound being left for the network and the host, and the heartbeat's 99th percentile in a burst under
   a third of it (2 s), which is what the margin of item 3 assumes; the rise of ready to handler entry is reported, not gated.

### The host at 1.8

The normative host text is `sdks/SPEC.md` §5, §6.2 to §6.5 and §6.8. In short:

1. **Receive.** A `PollTasks` call is bounded to the receive bound, as a stream window is. Unflagged leases are dispatched at once.
   Flagged leases over the credit, or received while stopping, are handed back and not acknowledged.
2. **Acknowledge.** As soon as a frame or a poll answer is read, the host sends one `AcknowledgeTasks` with its flagged leases,
   concurrently with its next receive, and calls nothing else for them but the hand-back before the answer.
3. **Verdicts.** `ACKNOWLEDGED`: dispatch (§6.3) with the lease's own expiry as its deadline; the first heartbeat at the earlier of
   the interval and a third of `lease_time_left_ms`, on the monotonic clock. `RETRY`: ask again for that lease, with the backoff of
   §5. `CANCELLED`: run nothing, hand the lease back. `NOT_HELD`: drop it, run nothing, report nothing, free its slot, emit `dropped`
   with the code `NOT_HELD`.
4. **A call that fails** (a transport failure, `UNAVAILABLE`, `DEADLINE_EXCEEDED`, `CANCELLED`, `UNKNOWN`, `ABORTED`,
   `RESOURCE_EXHAUSTED` after its hint, a retryable `INTERNAL`, an unusable answer): repeat it, without the leases already decided or
   handed back, with the backoff of §5 and no limit on attempts, each attempt bounded to the receive bound; `UNAUTHENTICATED`: once
   more with a fresh token. The host gives a lease up when its time left at receipt (its expiry minus the host's clock at receipt)
   minus the receive bound has passed on its monotonic clock, clamped to between one receive bound and 120 s, and never while an
   attempt is in flight: it waits for that attempt's outcome, at most the receive bound, and runs a late `ACKNOWLEDGED`. A lease
   given up is not run and is handed back.
5. **Fallback.** `UNIMPLEMENTED`; three calls in a row refused with another status not listed in item 4 (`PERMISSION_DENIED`,
   `INVALID_ARGUMENT`, `FAILED_PRECONDITION`, `NOT_FOUND`); or three leases in a row given up with no `ACKNOWLEDGED` between them: the
   host hands back what waits, waits for the answers of its hand-backs (their retry budget of §5), logs once, and registers again at
   its own minor with `declines_acknowledgement = true` until it restarts. It keeps every feature of its minor; only the flag is
   declined.
6. **Stopping.** A stopping host waits, at most the receive bound, for the verdict of an acknowledgement in flight, runs what is
   `ACKNOWLEDGED` as it runs any task in flight, and hands back the rest.
7. **Logs.** The happy path logs nothing new; `NOT_HELD`, `CANCELLED`, a lease given up, a refusal and a fallback log one line
   each, with identifiers only.

### Scoped instances

A collection item or loop body leased to a 1.8 session is marked, flagged, acknowledged, refused capabilities and withdrawn by the
same rules. The acknowledgement addresses the step row by its full address, so it does not depend on the path. The withdrawal is the
scoped withdrawal of the build of Architecture decision 0098 (one path transaction per run and instance path, with the addressed
event), whose re-read `FOR UPDATE` is the same; the refusal is a variant of `live_path_fence`; a stopping collection or loop drains a
withdrawn instance as one never leased. Each task of the plan has a scoped row in its evidence.

### Rollback

1. Stop marking: remove `delivery-acknowledgement-v1` (the owner function, as `docs/runbooks/instance-registry.md` section 5
   describes; the dependency table keeps `lease-withdrawal-v1` while it is recorded), or set `SAGAT_LEASE_WITHDRAWAL=off` on every
   engine. Within one registry heartbeat no engine marks a lease; sessions at 1.8 go on, their new leases unflagged.
2. Wait until no mark counts (`worker_leases_marked` is 0): every marked lease ends, acknowledged, withdrawn or reported, with its
   meaning unchanged; that takes the longest earlier deadline in flight plus the period of the lost-lease pass and its backlog.
3. Roll the binary back. The floor is the build of Architecture decision 0098, and the registry enforces it: that build decodes the
   cause `NOT_ACKNOWLEDGED` (precondition (a) of the task phase), and `lease-withdrawal-v1`, which every writer of the cause needs,
   cannot be removed once a journal holds `step.lease_withdrawn` (Decision 006-032 of that build). A session at 1.8 meeting a binary
   below 1.7 is the case of design 041; an unflagged lease asks nothing of 1.8.
4. The `down` migration refuses while a mark counts, while the history names the capability, or while a live process lists it; marks
   that no longer count are dropped with the columns.

### Failure modes and their exact outcomes

| Case | Outcome |
|---|---|
| A frame or a `PollTasks` answer lost; the host crashes before acknowledging; the engine crashes between the claim and the withdrawal of Architecture decision 0098 | At the earlier deadline the lease is withdrawn (`NOT_ACKNOWLEDGED`): `READY`, attempt not consumed, no review; the next lease has the same attempt number and the next generation |
| The acknowledgement never leaves the host | Repeated; on a verdict the task runs once; if the host gives the lease up first, nothing runs and the hand-back withdraws it (`NOT_STARTED`) |
| The acknowledgement's answer is lost after its commit | The host repeats; the server answers `ACKNOWLEDGED` again (`duplicate`); the handler runs once. The host never gives up while an attempt is in flight; if no attempt can reach the server until its give-up, nothing runs, and its hand-back of the now acknowledged lease is an ordinary retryable failure; if the hand-back cannot reach the server either, the lease lapses into a review (Gap 006-041) |
| A repeat while the first attempt is still committing | The row is skipped (`RETRY`); the next ask answers `ACKNOWLEDGED` |
| An acknowledgement with less than one receive bound left, or after a deadline, before the pass | `NOT_HELD` (`too_late`), nothing written; the pass withdraws the lease |
| An acknowledgement after a withdrawal, or after the lease was replaced | `NOT_HELD`; the host drops the task; counted apart, never as a call after a withdrawal |
| Acknowledgement and withdrawal at the same moment | Exactly one holds the row; the other finds it changed |
| The host stops, or crashes, after `ACKNOWLEDGED` and before the handler starts | Stopping: it runs the task, as any task in flight. Crashing: the lease lapses into a review (Gap 006-040; the window is in-process and holds no call) |
| The step is asked to stop, or the run has a stop reason, before the acknowledgement | `CANCELLED`, nothing written; the host hands back; the withdrawal's settle cancels the step; if the hand-back is lost, the withdrawal at the deadline does |
| A binding revoked after the claim | Ordered with the acknowledgement by the binding's lock: `NOT_HELD` (`unbound`), one audit record; the lease is withdrawn at its deadline and leased to a bound worker |
| A token that does not match (forged, old, another tenant's run) | `NOT_HELD` (`no_match`), no lock taken; at most one audit record per call |
| A row held by another transaction (a busy run, a cancellation in progress) | `RETRY` for that lease alone; the call never waits for a row |
| A capability call on a provisional lease | `FAILED_PRECONDITION`, `TASK_NOT_ACKNOWLEDGED`; counted and audited as a violation |
| A `Heartbeat`, `CompleteStep` or a `FailStep` other than the hand-back on a provisional lease | Served as today; counted and audited once per step and generation as a violation |
| A hand-back after a capability was served | An ordinary retryable failure; a violation |
| A report, heartbeat or capability call carrying the token of a lease withdrawn as `NOT_ACKNOWLEDGED` | Refused as stale; counted, audited once per step and withdrawn token, alerted: a handler may have run on a lease the journal says never ran (Gap 006-042). A hand-back or an acknowledgement with such a token is counted apart, as evidence that nothing ran |
| A hand-back that lands after its host registered again | Decided from `lease_hand_back_generation`, so still withdrawn |
| The attempt's withdrawals reach the cap of four | Its next lease is not marked (`attempt_capped`) and keeps today's meaning |
| `AcknowledgeTasks` refused by a proxy or a rolled-back replica | The host hands back and registers again declining acknowledgement, at its own minor |
| A host clock skewed by more than the time-to-live | Its give-up is clamped on its monotonic clock; its first heartbeat follows `lease_time_left_ms`; it acknowledges as any other host |
| A host configured with a long heartbeat interval | Its first heartbeat of a flagged lease still comes at a third of `lease_time_left_ms` |
| An engine restarts between the claim and the acknowledgement | Nothing of the lease lives in the engine; any engine of the build answers |
| The requirement removed while marked leases exist | Their marks keep their meaning until their leases end; no new mark after one registry heartbeat |
| An engine with `SAGAT_LEASE_WITHDRAWAL=off` | It marks nothing; a marked lease whose withdrawal falls to it lapses as before (`disabled`) |
| A lease whose earlier deadline is under 18 s | Not marked (`short_deadline`); today's meaning (Gap 006-047) |
| A hand-written client at 1.8 that runs a flagged lease without acknowledging and makes no call during it | Withdrawn and run again with the same attempt number and idempotency key (Gap 006-042); a later call with its token is counted and alerted |

### Observability

- `worker_task_acknowledgements_total{verdict, reason}`; `grpc_requests_total{method="AcknowledgeTasks", code}`.
- `worker_leases_undelivered_total{cause, outcome}` (the counter of Architecture decision 0098) gains the cause `NOT_ACKNOWLEDGED`;
  `worker_lease_marks_skipped_total{reason}`; `worker_leases_expired_total{mark}` (`acknowledged`, `unmarked`), since the journal does
  not record that a lapsed lease had been acknowledged (Gap 006-046).
- `worker_task_delivery_delay_seconds`, from the claim to the acknowledgement on the database clock; and
  `worker_task_acknowledgement_call_seconds`, the server-side duration of one call by its number of leases (`1`, `2-8`, `9-64`).
- Gauges without labels: `worker_leases_marked`, `worker_leases_provisional`, `worker_leases_provisional_overdue`; and
  `worker_sessions{protocol, declines}`, the live sessions by accepted minor and whether they decline acknowledgement.
- `worker_capability_calls_refused_total{method, reason="TASK_NOT_ACKNOWLEDGED"}`, `worker_protocol_violations_total{method}`,
  `worker_calls_after_withdrawal_total{kind}` (`report`, `capability`, `hand_back`, `acknowledgement`).
- Alerts, with `promtool` cases at the default and at an 18 s time-to-live: `SagatLeasesNotAcknowledged`, withdrawals with the cause
  `NOT_ACKNOWLEDGED` above 1% of marked claims over 15 minutes (frames are being lost or hosts are not acknowledging; a threshold
  chosen as an operating limit, not measured); `SagatLeaseAttemptCapped`, skips with the reason `attempt_capped` above 1% of marked
  claims over 15 minutes; `SagatLeaseDeliverySlow`, the 99th percentile of the delivery delay over 10 minutes above twice the baseline
  Task 006-056 records, and never above the receive bound; `SagatCallsAfterWithdrawal`, any increase of the `report` or `capability`
  kind.
- The call's span links to the trace context of each lease answered `ACKNOWLEDGED` (at most 64), never as a parent. No log line,
  label or attribute carries a token, a payload or an unbounded identifier.
- SDK metrics hooks gain `acknowledgements`, `notHeld`, `givenUp` and `acknowledgementFallbacks` counters.

### Acceptance matrix

| Area | Required evidence |
|---|---|
| Exactness | Barrier tests: the acknowledgement holds the row while the pass's withdrawal waits, the reverse, the same with the hand-back's withdrawal, a repeated acknowledgement while the first commits, an acknowledgement after a deadline and before the pass, a revocation; root and scoped |
| The marks | Each of the three lease statements; a lease written with the old column list is not marked; the static check of the assignments; the barrier with a run transaction loaded before an acknowledgement |
| Withdrawal | A provisional `Activity` lease past its expiry, and one past its attempt deadline: `step.lease_withdrawn` cause `NOT_ACKNOWLEDGED`, no review, the attempt reused; an acknowledged lease that expires is an unknown outcome; the cap: four withdrawals then an unmarked lease |
| Hand-back | From a lease under the reserved code, withdrawn; from one claimed below 1.8, an ordinary failure; after the session registered again at another minor, decided by the lease; for an acknowledged lease or after a capability was served, an ordinary failure; a handler-raised code reaches the server as `WORKER_ERROR` |
| Capabilities | The descriptor-driven test, root and scoped; a report on a provisional lease is served; a wrong token gets the ordinary stale answer |
| Gate | The capability unlisted and the minor at 7 before Task 006-057; no mark while the requirement is not recorded; the trigger and the command refuse both orders, with the race held open |
| Cost | `LEASE` and both completion lines unchanged; `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` pinned, the former with no run lock, no scan of the run's steps, no journal or outbox insert, fewer statements than the latter; a 64-lease call itemised with its triggers |
| Throughput (gate, before the SDKs and before 1.8 is offered) | The transitions scenario at 1.8 against 1.7, median of five interleaved runs each, network between workers and services: no more than 15% lower; the 64-lease call within 3 s at the 99th percentile; no heartbeat shed while acknowledgements are admitted in a burst; the heartbeat's 99th percentile in the burst under 2 s; the rise of ready to handler entry and the share of claims left unmarked as `short_deadline` reported |
| Six SDKs | `delivery-acknowledgement` passes in six languages with a reviewed golden; under the override every host scenario but `protocol-negotiation` and those that hand back runs at 1.8 with its existing golden unblessed and acknowledges at least one flagged lease |
| Released v0.2 | `provisional-lease-cutover` forward and its rollback case |
