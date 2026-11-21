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
   `NOT_STARTED`), but only for a lease claimed by a session at 1.8 or later, which reserves the code for the host, within the
   cap of four withdrawals per attempt (Decision 006-031 of the server build), and never for an acknowledged lease of 1.8, which is an attempt (the server review of
   pull request #260; "Delivery acknowledgement (protocol 1.8)" below); otherwise, and always from an older session, it is an
   ordinary retryable failure, never an unknown outcome.
6. **What is not covered.** A frame lost on the wire after it was handed to the transport, and a lost unary answer, still
   lapse (Gap 006-001) when unflagged; for a lease flagged at 1.8 delivery acknowledgement closes it (next section). A process that stops between a claim and its withdrawal leaves
   the lease to lapse (Gap 006-002); a flagged lease is provisional and is withdrawn at its own earlier deadline instead.





## Delivery acknowledgement (protocol 1.8)

Decision 006-040 to Decision 006-064 of [`research.md`](research.md); Architecture decision 0099 and Architecture decision 0100; Owner
decision 2026-11-20-12.
Minor 1.8 adds a flag on the lease (`acknowledge_before_start`), a call (`AcknowledgeTasks`) and a registration field
(`declines_acknowledgement`), and is the minor at which the hand-back code `TASK_NOT_STARTED` is reserved for the host
(Architecture decision 0098 decision 5). A flagged lease is *provisional* until its host acknowledges it; the host runs nothing and
calls nothing for it but the acknowledgement and the hand-back before the server has committed that acknowledgement; a provisional
lease whose earlier deadline passes is withdrawn (the cause `NOT_ACKNOWLEDGED`), so a frame lost on the wire and a lost `PollTasks`
answer (Gap 006-001) no longer become a review. An acknowledged lease is an attempt, with today's meaning in every respect.
Unflagged leases, and every lease of a session below 1.8, are served exactly as before.

In this section "the receive bound" is the 6 s a host gives one receive call (`sdks/SPEC.md` §10).

### Wire

Added to `specs/001-platform-foundation/contracts/worker.proto`, additively; the six SDK copies and their checked-in stubs are
synchronised in the same task (Task 006-043), so they stay byte-identical, and the host behaviour comes later with each SDK:

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
  ACKNOWLEDGEMENT_VERDICT_CANCELLED = 3;      // run nothing, hand the lease back (asked to stop, or too late to start)
  ACKNOWLEDGEMENT_VERDICT_RETRY = 4;          // its row was busy: ask again for this task
}
message TaskAcknowledgementResult {
  string task_id = 1;
  AcknowledgementVerdict verdict = 2;
  int64 lease_time_left_ms = 3;               // when ACKNOWLEDGED: the time left before the lease's expiry at the clock sample; never below 0
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
- Every message with a field whose name ends in `_token` (the lease tokens and `session_token`) is in `skip_debug`, with a
  `Debug` that prints counts only; the descriptor test of Task 006-052 fails for one that is not, so the older request types are
  closed too. Each SDK has a token canary on its forms.
- Admission: a class of its own, `Acknowledge`, admitted while the requests in flight are under `max(D, min(L - 1, D + max(1,
  (L - D) / 2)))`, where `L` is the limit and `D` the discovery threshold, so that the requests in flight never exceed `L` and, when
  the reserve `L - D` is at least one place, at least `max(1, ceil((L - D) / 2))` of it stays for the calls of the class `Lease`
  (`Heartbeat`, `CompleteStep`, `FailStep`, `ResolveSecrets`, `ResolveDatabase`, and the transfers that share its limit); with a
  reserve of one place the acknowledgement shares the discovery band. The test of Task 006-049 asserts these properties for limits 1
  to 64, not the formula. `AppendStepLogs` of 1.7 is classified explicitly in the same change. A shed acknowledgement costs at worst a
  withdrawal; a shed heartbeat can cost a review.

### The hand-back at 1.8

Architecture decision 0098, as amended by its build review (its decisions 5, 7 and 8; Decision 006-030 to Decision 006-039 on its
branch), holds the server's rule: a `TASK_NOT_STARTED` withdraws the lease (cause `NOT_STARTED`, no attempt consumed, no review)
only from a lease under the reserved code, while `lease-withdrawal-v1` is recorded, the step is `LEASED` with the issued fence, and
the attempt's withdrawals are under four; otherwise it is an ordinary failure, the attempt spent, the flag the host sent honoured,
never a review. This design adds:

- **The reservation is read from the lease.** The claim writes `lease_hand_back_generation` when the claiming session was accepted
  at 1.8 or later; the hand-back decides from it, under the run lock, never from the session row, which registering again under the
  same worker name overwrites. This build's code does it (Task 006-050).
- **Only while `delivery-acknowledgement-v1` is recorded.** The hand-back reads it under the lock its removal waits for; while it is
  recorded no live process is of a build that serves a capability without recording it, so `lease_capability_generation` is
  complete. Before the activation, or after a removal, a 1.8 hand-back is an ordinary failure that spends the attempt, as below 1.8.
- **Not for a lease claimed on another timeline**, whose capability record may have been lost with the timeline: an ordinary
  failure.
- **The build of Architecture decision 0098 never honours a 1.8 hand-back.** It decides the holder's minor as the smaller of the
  stored minor and its own compiled ceiling, so with its ceiling at 7 or below it reads no session as 1.8, and its tests raise both
  (Decision 006-059 (3)). The first build that withdraws on `TASK_NOT_STARTED` is this one, which reads the lease's columns; a
  hand-back that reaches an engine of the older build in a mixed fleet, or after a rollback, is an ordinary failure, the safe
  direction.
- **A provisional lease's hand-back is withdrawn after a deadline too.** For a lease whose mark counts and that is not acknowledged,
  the expiry condition of the hand-back is lifted: nothing ran, and the pass would withdraw it anyway. So a hand-back that lands
  after the expiry and before the pass, which the host's give-up makes common when every acknowledgement attempt goes unanswered,
  costs no attempt. An unmarked lease keeps the rule of Architecture decision 0098.
- **The engine's switch does not reach a marked lease.** `SAGAT_LEASE_WITHDRAWAL=off` stops new marks and the withdrawals of
  Architecture decision 0098 for unmarked leases; the hand-back and the pass's withdrawal of a lease whose mark counts need only
  `lease-withdrawal-v1`, read under the lock its removal waits for, so a mark keeps its meaning whichever engine handles it
  (Decision 006-059 (6) states it in Architecture decision 0098 decision 8).
- **Not for an acknowledged lease**, which is an attempt (Decision 006-045).
- **Not after a capability was served**: a hand-back of a lease whose `lease_capability_generation` is its generation is an ordinary
  failure, counted and audited as a protocol violation (Decision 006-053).
- **The order of locks**: the binding checks of the report path come before the re-read of the step row `FOR UPDATE`, in every
  withdrawal path.

The host sends the code only for a lease no handler of which started: a lease over its credit, received while stopping, given up,
answered `CANCELLED`, flagged while it declines, or waiting when it falls back. The SDKs rewrite a handler-raised `TASK_NOT_STARTED`
to `WORKER_ERROR` at every minor (Decision 006-028 of the SDK build), and keep at most eight hand-backs on their way at once
(Decision 006-026 of that build).

### Negotiation and activation

- The server's minor becomes 8 (`DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR = 8`) in Task 006-057, after the gate of Task 006-056,
  never before: until then the tests of the 1.8 paths raise a session's stored minor, as Decision 006-030 of the server build does.
  Minors are cumulative: `protocol_ceiling` answers 8 for an ordinary worker when it would answer 7 (design 041, Task 041-7) and the
  store proves `supports_delivery_acknowledgement()`, a constant of the adapter. Nothing else gates the minor. The database runner
  keeps 6.
- From the same task every control plane and engine of the build lists `delivery-acknowledgement-v1`. For an engine it means: it
  serves `AcknowledgeTasks`, replays the cause `NOT_ACKNOWLEDGED`, refuses capabilities to provisional leases, and withdraws a marked
  provisional lease at its deadline and on its hand-back; for a control plane, which serves no worker call and runs no pass, it
  means it replays and shows the cause (the event reads and the Studio's closed list). Architecture decision 0082 decision 7: nothing
  is listed that the process will not do. An engine with `SAGAT_LEASE_WITHDRAWAL=off` does all of it and lists it; the switch only
  stops it marking new leases and withdrawing unmarked ones (the hand-back above).
- `control-plane capability-activate delivery-acknowledgement-v1` (the generic command of Decision 006-034 of the server build)
  gains its entry in Task 006-057, with `empty_fleet: false`, so the command refuses it before the build lists the capability. The
  dependency on `lease-withdrawal-v1` for both kinds is Architecture decision 0100: declared once as `Capability::requires()` in the
  application (from Task 006-046, where the capability is already dormant), read by the entry, refused by the command with
  `DependencyNotRecorded` and `RequiredBy`, and enforced in SQL by the trigger `platform_requirements_guard_dependency`, which fires
  after the guard of Architecture decision 0082 decision 1 inside the activation functions and reads without locking. Its pairs are a
  constant of its function, which `scripts/check-architecture.py` holds equal to `Capability::requires()`. So the hand SQL of
  `docs/runbooks/instance-registry.md` section 5 cannot break it either, and there is no table to edit.
- Every engine of the build serves `AcknowledgeTasks` whatever it knows of the requirement. An engine marks leases only while it
  knows the requirement is recorded; it learns of an activation or a removal within one registry heartbeat (Gap 006-044). A mark
  already written keeps its meaning until its lease ends, whatever the switch; removing the requirement ends the protection of the
  marks in flight (their withdrawals become `expire_attempt`), which is why a rollback drains them first.
- Every withdrawal of a lease whose mark counts, the pass's included, reads `lease-withdrawal-v1` and `delivery-acknowledgement-v1`
  under the locks their removals wait for, in its transaction before it writes (one statement); when either is not recorded it writes
  no withdrawal, and the pass ends the attempt with `expire_attempt` instead, counted `lapsed`. So the removal of both requirements
  in a row, while a journal holds no withdrawal yet, cannot leave a journal with the event and no floor, and a floor process admitted
  after a removal can never serve a lease that this build then withdraws.

### The marks

Columns of `steps`, all written for a lease's generation and counting only while it is the step's `generation`, the step is
`LEASED` or `RUNNING`, and `lease_claim_timeline` is not null and equals the database's current timeline:

| Column | Written by | Meaning |
|---|---|---|
| `lease_hand_back_generation bigint` and `lease_claim_timeline bigint` | the claim, for a session accepted at 1.8 or later: the generation, and `sagat_database_timeline()` | the lease is under the reserved hand-back; the timeline both marks belong to |
| `lease_offered_generation bigint` and `lease_offered_at timestamptz` | the claim, when it marks the lease provisional (and sets the flag): the generation and the claim's time (`clock_timestamp()` after the lock) | provisional until acknowledged; the start of the delivery delay |
| `lease_acknowledged_at timestamptz` | cleared by a claim that marks; set by the acknowledgement | acknowledged |
| `lease_capability_generation bigint` | the first capability call served to the lease, before it is served | a capability was served to this lease |
| `withdrawn_lease_token_hash bytea` | a withdrawal with the cause `NOT_ACKNOWLEDGED`; replaced by the next | evidence, read only on the stale path of a call |

and `worker_sessions.declines_acknowledgement boolean NOT NULL DEFAULT false`, written by the host's own registration (the
session upsert sets it from the registration, so one that does not decline clears it).

- **The timeline.** `sagat_database_timeline()` (`LANGUAGE sql`, `VOLATILE`, an invoker function with `pg_catalog` names, its
  execute right revoked from `PUBLIC` and granted to `sagat_app`) returns the timeline of the current write-ahead log position, the
  first eight hexadecimal digits of `pg_walfile_name(pg_current_wal_lsn())`, or null when it cannot (`pg_is_in_recovery()`, or no
  execute right on those functions), never an error. A promotion picks its new timeline before the first write it accepts, and an
  archive recovery ends on a timeline above every one it replayed, so the value changes at once with either; `pg_control_checkpoint()`
  would lag after a fast promotion until the next checkpoint, which is why it is not used. A statement evaluates it once, as a scalar
  subquery. A null current timeline means no mark counts, which is today's meaning. Task 006-044 checks on the pinned PostgreSQL
  image that the runtime role reads it, and that a role without the right gets null and every statement still works.
- **A mark of another timeline, or none, does not count.** For the acknowledgement such a lease is an attempt (it answers
  `ACKNOWLEDGED`, counted `recovered`, with the same margin and stop rule as any other); for every withdrawal it is an unmarked
  lease, under Architecture decision 0098's rules, except that its hand-back is an ordinary failure, because a capability record of
  the earlier timeline may have been lost. So an acknowledgement committed before a failover to an asynchronous standby, or before
  the point a recovery restored to, and lost by it, can never become a false "nothing ran".
- **Durable commit.** The acknowledgement's settings statement raises `synchronous_commit` to `local` for its transaction when it
  is `off` (it never lowers a stronger value), so a crash of the primary and its restart, which keep the timeline, cannot lose a
  committed acknowledgement; a lost claim, withdrawal or capability record is safe or covered by it. The engine's probe also reads
  `fsync` and marks nothing while it is off (`no_durability`).
- **The probe** runs at start and on each registry heartbeat's tick, on the request pool and outside the heartbeat's call, so its
  failure sets `no_timeline` and never touches the registry gate or readiness (Architecture decision 0082 decision 5).
- **After any restore.** `control-plane end-lease-marks --operator NAME`, run as the owner role in one bounded transaction before
  the engines start (section 7 of `docs/runbooks/disaster-recovery.md`, after every restore of any kind), calls the owner function
  `end_lease_marks()` (`SECURITY INVOKER`, `search_path = pg_catalog, pg_temp`, `public.steps` by full name, revoked from `PUBLIC`
  and granted to no role), which sets `lease_claim_timeline` to null where it is not, and writes one audit record with the operator,
  the database role and the number of marks it ended. It is idempotent. It gives the leases in flight today's meaning and costs only
  their protection; the timeline does the same by itself for a promotion, when no operator is in time.

- The claim writes them in the statement that already writes the lease, in each of the three: `upsert_step` (root),
  `upsert_scoped_instance` (a scoped instance claimed in a full-run transaction) and the path statement of `execution/path.rs`.
- It marks the lease provisional when the session was accepted at 1.8 or later and does not decline acknowledgement, the engine knows
  the requirement is recorded, its `SAGAT_LEASE_WITHDRAWAL` is on and its probe finds the timeline readable and `fsync` on, and the
  lease's earlier deadline (the earlier of the claim's time plus the engine's time-to-live and the claim's time plus `timeoutMs`) is
  at least three receive bounds (18 s) after the claim. The claim does not read the attempt's withdrawals: the cap of four is applied
  by the withdrawal command, in its own transaction, from the journal (decision 5 of Architecture decision 0099). Otherwise the lease
  is not marked, the flag is false, and `worker_lease_marks_skipped_total{reason}` counts it with the first reason that holds, in
  this order: `not_activated`, `switch_off`, `no_timeline`, `no_durability`, `declined`, `short_deadline`.
- No statement assigns a column of the marks from a value it read earlier. The allowed forms are: the claim's, in SQL conditioned
  on a new lease (the stored `generation` differs from the one written), never on a bound flag alone; the engine's other statements
  keep them (`ELSE` the column) or clear them with the lease; the acknowledgement sets `lease_acknowledged_at = clock_timestamp()`
  only; the capability paths set `lease_capability_generation = generation` only; the withdrawal sets `withdrawn_lease_token_hash`
  from the row it holds; `end_lease_marks()` sets `lease_claim_timeline` to null. A static check in `scripts/check-architecture.py` fails on any other form in `crates/infra-postgres` and the migrations,
  with a negative test for a bound-flag form; a barrier test holds a run transaction that loaded the run before an acknowledgement
  and makes it write the row through each statement that can touch a `LEASED` row.
- No `CHECK` ties the columns to the status: an older binary must still be able to write the row. A mark left by a binary that does
  not know the columns never counts, because the next lease has the next generation.
- The condition is one SQL macro of the adapter and one pure function of the application, which extends `LeaseDeadlines`
  (`crates/application/src/execution.rs`) and receives "the offer is on the current timeline" as a boolean the adapter computes, not
  a PostgreSQL timeline; a parity test in `integration_durable_execution` writes every combination of a row (the timeline equal,
  lower, higher and null among them) and compares them. The gauges, the rollback's drain and the `down` migration's refusal use the
  same macro.
- The gauges `worker_leases_marked`, `worker_leases_provisional` and `worker_leases_provisional_overdue` are `FILTER` clauses of the
  scan the operations statistics query already makes of leased steps (`crates/infra-postgres/src/operations.rs`); no index is added.
- The marks are lease data: not journaled, not derived by the replay, and no test of replay against projection asserts them.

### The server's acknowledgement

One transaction per call, through a store port of its own (`AcknowledgementTx`: `matched`, `bound_patterns`,
`current_worker_authorized`, `lock_matched`, `acknowledge`); no run lock:

1. The call has a deadline of half the receive bound (3 s) from its arrival, which covers waiting for a pooled connection. When it
   passes before the commit is sent, the transaction is abandoned (the connection is discarded, so it rolls back) and the call
   answers `UNAVAILABLE` with the reason `ACKNOWLEDGEMENT_DEADLINE` in its details, never `DEADLINE_EXCEEDED`, which the host keeps
   for its own bound; once the commit is sent, the answer waits for it (a commit waiting on a synchronous standby is outside the
   bound, and the host's give-up covers it). `worker:execute`, once (`PERMISSION_DENIED`, audited as today). One statement sets, for
   the transaction (`set_config(..., true)`), `lock_timeout` and `statement_timeout` to the smaller of 1 s and the time left, and
   `synchronous_commit` to `local` when it is `off`.
2. `matched`: one statement with no lock, by the step primary key (`steps_pkey`), reads the addressed rows whose `lease_token_hash`
   matches the request's token hash: their namespace and activity. Tasks of more than one namespace: `INVALID_ARGUMENT`. A task with
   no such row answers `NOT_HELD`. The run ids with no row in the caller's tenant are checked for existence in another tenant with
   one statement, and at most one `CrossTenantAccessDenied` record per call is written, spawned off the answer path through a
   bounded gate (32 places and 5 s, as `crates/api-http/src/denial.rs` has, moved to the application layer); past the gate the
   record is counted and dropped.
3. The checks of `ensure_bound`, under its locks and in its order, for the call's one namespace, through the same function the run
   transactions call, over a narrow trait that both they and `AcknowledgementTx` implement (a test makes both refuse the same
   narrowed binding): the binding's shared advisory lock and the namespace's bound patterns, then the namespace's shared secrets
   lock and the membership's row lock with `current_worker_authorized`. Coverage is decided per row with `first_uncovered` on its
   activity; a row of a managed or non-activity node is never marked and answers `NOT_HELD`. A refused binding answers `NOT_HELD`
   for its rows, and the call returns at most one `AccessDenial`, which the adapter audits through `spawn_audit_access_denied`
   whatever the call's status. These locks come before any row lock, as in every other path, so a revocation (which takes the
   binding's lock exclusively, then rows) cannot form a cycle with the acknowledgement.
4. `lock_matched`: one statement selects the bound rows by primary key alone (`tenant_id`, `run_id`, `step_id`), joined with `runs`
   on `runs.tenant_id = steps.tenant_id AND runs.id = steps.run_id`, and locks them `FOR NO KEY UPDATE OF steps SKIP LOCKED` in
   primary-key order (the update changes no key, and this mode conflicts with every withdrawal's `FOR UPDATE` and with every run
   transaction's update, not with the foreign-key checks of `blobs` and the others). It reads, with them, their status, generation,
   attempt, token hash, marks, deadlines, `cancel_requested`, the run's stop reason, `sagat_database_timeline()` and
   `clock_timestamp()`; with `SKIP LOCKED` the statement never waits, so the clock is sampled when the locks are held. A row it
   could not lock answers `RETRY` and costs nothing. The `runs` row is read, not locked: a stop committed after that read is ordered
   after the acknowledgement and reaches the host through its first heartbeat, as any stop of a running task does.
5. The pure function decides each locked row: provisional with the request's token, attempt and generation, no stop asked, at least
   one receive bound left before both `lease_expires_at` and `attempt_deadline_at` (when set): it wins. With the same token, an
   acknowledged lease (counted `duplicate`) or a lease of another timeline (counted `recovered`) answers `ACKNOWLEDGED` under the same
   margin and stop rule and writes nothing. A provisional lease with `cancel_requested` or a run stop reason, and an acknowledged or
   recovered lease with a stop asked or less than the margin left, answer `CANCELLED` (reasons `stop_asked`, `too_late`): run
   nothing, hand back. Anything else answers `NOT_HELD`, with its reason. The token is compared here, not in the statement's `WHERE`,
   so a row skipped is always a row held by another transaction.
6. `acknowledge`: one update of the winning rows, which the transaction already holds: `lease_acknowledged_at = clock_timestamp()`.
   It returns the delay since `lease_offered_at` and the time left before `lease_expires_at`, which the answer reports, clamped at 0.
7. A deadlock, a serialization failure, a lock or statement timeout, the call's deadline, or an unavailable store ends the call with
   `UNAVAILABLE` (retryable); the host repeats it, and the rows already written answer `ACKNOWLEDGED` again. No transient error is ever
   answered `NOT_HELD`.

Nothing else: the acknowledgement does not restore the run, renew the lease, apply a transition, write the journal or the outbox.
About eight statements per call whatever its size (the settings, `matched`, two for the binding, two for the membership,
`lock_matched`, `acknowledge`), plus the triggers of `steps` the update fires, which the budget's top-level count does not show and
Task 006-055 itemises. Counted: `worker_task_acknowledgements_total{verdict, reason}` with the verdicts `acknowledged`, `duplicate`,
`recovered`, `not_held` (reasons `no_match`, `too_late`, `not_provisional`, `unbound`), `cancelled` (reasons `stop_asked`,
`too_late`), `retry` and `deadline`.

### The withdrawals

Every withdrawal of a lease whose mark counts, under the run lock (the whole-run or the path transaction), after the report path's
binding checks (the lost-lease pass, a system actor, has none) and after reading `lease-withdrawal-v1` and
`delivery-acknowledgement-v1` with the functions that take the locks their removals wait for (`lease_withdrawal_required()` of
Architecture decision 0098 decision 8, and its twin `delivery_acknowledgement_required()`, one statement for both), re-reads the
step row `FOR UPDATE` and decides with the pure function on what it read. The withdrawal command of Architecture decision 0098
then applies the cap of four per attempt from the journal count it already reads (`lease_withdrawals`, one statement), as for every
other cause. The lost-lease pass is the only pass that ends expired leases, so in it every outcome other than a withdrawal ends the
attempt with `expire_attempt` in the same transaction, as today; it never leaves an expired lease for a later pass.

| Under the row lock | Outcome |
|---|---|
| Provisional, and its expiry or its attempt deadline passed by `clock_timestamp()`, both requirements recorded, under the cap | The lost-lease pass withdraws it (Architecture decision 0098 decision 3; root in the whole-run transaction, scoped in the path transaction, as its build does): `step.lease_withdrawn` with `cause = NOT_ACKNOWLEDGED`, the lease and its marks cleared, `withdrawn_lease_token_hash` kept, the step `READY`, the attempt set back by one, the generation kept; the settle runs, so a step the planner stops is cancelled at once (a compensation or reconciliation is kept and leased again) and a stopping collection or loop drains the instance as one never leased |
| The same, at the cap | `expire_attempt` in the same transaction: an unknown outcome, consultation or review; `worker_leases_undelivered_total{cause="NOT_ACKNOWLEDGED", outcome="capped"}`. The pass has only silence, four times over; a hand-back at the cap is the host's word and is an ordinary failure |
| The same, a requirement not recorded | `expire_attempt` in the same transaction, counted `lapsed` |
| Provisional, a hand-back under the reserved code, whether or not a deadline passed, both requirements recorded, under the cap | The withdrawal of Architecture decision 0098, cause `NOT_STARTED` |
| The same, at the cap or a requirement not recorded | An ordinary retryable failure, the attempt spent (Architecture decision 0098 decision 7), counted `capped` or `lapsed` |
| Acknowledged, and expired or timed out | `expire_attempt`, as today: an unknown outcome, consultation or review; `worker_leases_expired_total{mark="acknowledged"}` |
| A mark of another timeline, or none | An unmarked lease under Architecture decision 0098's rules, its switch included, except that its hand-back is an ordinary failure; at a deadline `expire_attempt`, counted `worker_leases_expired_total{mark="recovered"}` |
| `RUNNING` with an unacknowledged mark (a host that heartbeated first) | `expire_attempt`, as today; the heartbeat was counted as a violation |
| A 1.8 lease that is not marked (`lease_hand_back_generation` only), handed back | Withdrawn as Architecture decision 0098 decides, only while `delivery-acknowledgement-v1` is recorded; otherwise an ordinary failure |
| Not marked, below 1.8 | As today, and as Architecture decision 0098 decides |
| Anything else (acknowledged since the candidate was read, replaced) | Nothing; `fence_missed` (the lease is no longer this one) |

The candidate query of the pass returns the marks only as a hint; the decision is the re-read's. The engine's
`SAGAT_LEASE_WITHDRAWAL` is not consulted for a lease whose mark counts.

**Exactly one of the acknowledgement and a withdrawal commits for a lease.** The acknowledgement writes a row only while it holds
the row's lock and found it provisional with the request's fence and at least one receive bound before its deadlines; each
withdrawal takes the same row lock and writes only while it is still provisional (for the pass, past a deadline; for a hand-back,
not acknowledged). If the acknowledgement holds the row first, the withdrawal reads `lease_acknowledged_at` set and does nothing; if
a withdrawal holds it first, the acknowledgement skips the row (`RETRY`) and, asked again, finds the token cleared (`NOT_HELD`). An
acknowledgement repeated while its first attempt is still committing skips the row too and is answered `ACKNOWLEDGED` on the next
ask. The host runs a handler only on `ACKNOWLEDGED`. So a handler of a flagged lease runs only after the server committed that its
host holds it, and a lease the server withdraws was not run by a host that keeps the protocol (the trust assumption, Decision
006-053), on a database that keeps what it committed (the timeline rule above, Decision 006-062). The deadline decides when the pass
looks, never whether both can happen; the losing side of the race between the deadline (a timeout) and the acknowledgement (the
host's signal) is the journal's withdrawal event or the counted `NOT_HELD` (constitution principle I).

### Why the deadline is the lease's own

The acknowledgement deadline is the lease's earlier deadline: the earlier of the claim's time plus the time-to-live and the claim's
time plus `timeoutMs`. No other deadline is added. The derivation uses specified bounds only:

1. **The receive bound (6 s).** A host bounds each receive call to 6 s or its shorter configured call timeout (`sdks/SPEC.md` §10
   for `StreamTasks`; at 1.8 the same bound applies to `PollTasks`, Decision 006-048). It starts that clock before it sends the
   request and the server claims only after it has the request, so any lease the host reads, it reads within the receive bound of
   the claim, in real time, with no comparison of clocks.
2. **One acknowledgement attempt (6 s).** The host bounds each attempt to the receive bound and sends it as soon as the frame is read.
3. **One receive bound of margin, for each deadline and for its own reason.** Before the expiry: the acknowledgement renews nothing,
   so the host's first heartbeat must commit before the lease can expire. `lease_time_left_ms` is measured at the clock sample, before
   the update, the commit and the answer's transit (together `p`); the host sends the first beat at the earlier of its interval and
   a third of (`lease_time_left_ms` less the time its attempt took on its monotonic clock, which covers `p`), whatever interval it is
   configured with, and at once when that third is under 1.5 s. With `T` left at the sample (at least 6 s), the beat leaves at most
   (`T` − `p`) / 3 after the answer and has at least two thirds of `T` − `p` to commit: 2 s when `T` is 6 s and `p` is the call's 3 s
   bound, which the gate's heartbeat bound (2 s at the 99th percentile) fits, and more in every other case. Before the attempt deadline: a heartbeat does not move it, so the margin is what
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
   which bounds claim to handler entry from above. The gate requires the successful 64-lease calls within half the receive bound
   (3 s) at the 99th percentile, measured on successful calls only since the deadline ends the others, with fewer than 0.1% of calls
   ended by the deadline, half of the receive bound being left for the network and the host, and the heartbeat's 99th percentile in a burst under
   a third of it (2 s), which is what the margin of item 3 assumes (the two together fit: (6 − 3) × 2 / 3 = 2); the rise of ready to
   handler entry is reported, not gated.

### The host at 1.8

The normative host text is `sdks/SPEC.md` §5, §6.2 to §6.5 and §6.8. In short:

1. **Receive.** A `PollTasks` call is bounded to the receive bound, as a stream window is. Unflagged leases are dispatched at once.
   Flagged leases over the credit, or received while stopping, are handed back and not acknowledged. A flagged lease is never
   dispatched without `ACKNOWLEDGED`, whatever the host has declined: one received while it declines (claimed before its
   registration changed) is handed back and triggers nothing further.
2. **Acknowledge.** As soon as a frame or a poll answer is read, the host sends one `AcknowledgeTasks` with its flagged leases,
   concurrently with its next receive, and calls nothing else for them but the hand-back before the answer.
3. **Verdicts.** `ACKNOWLEDGED`: dispatch (§6.3) with the lease's own expiry as its deadline; the first heartbeat at the earlier of
   the interval and a third of `lease_time_left_ms` less the attempt's duration, at once when that is under 1.5 s, on the monotonic
   clock. `RETRY`: the leases of a frame answered `RETRY` are asked for again together in one call, after a backoff of their own
   (200 ms doubling to 1 s, never past the earliest of their give-ups), which the user's retry options do not change; `RETRY` does
   not advance the backoff of §5 and is not a failed call. `CANCELLED`: run nothing, hand the lease back. `NOT_HELD`: drop it, run
   nothing, report nothing, free its slot, emit `dropped` with the code `NOT_HELD`.
4. **A call that fails** (a transport failure, `UNAVAILABLE`, `DEADLINE_EXCEEDED`, `CANCELLED`, `UNKNOWN`, `ABORTED`,
   `RESOURCE_EXHAUSTED` after its hint, a retryable `INTERNAL`, an unusable answer): repeat it, without the leases already decided or
   handed back, with the backoff of §5 and no limit on attempts, each attempt bounded to the receive bound; `UNAUTHENTICATED`: once
   more with a fresh token. The host gives a lease up when its time left at receipt (its expiry minus the host's clock at receipt)
   minus the receive bound has passed on its monotonic clock, clamped to between one receive bound and 120 s, and never while an
   attempt is in flight: it waits for that attempt's outcome, at most the receive bound, and runs a late `ACKNOWLEDGED`. A lease
   given up is not run and is handed back; a hand-back that lands after its expiry, before the pass, is still a withdrawal.
5. **Fallback, bounded.** The host declines acknowledgement only on evidence that the call cannot work on its path, while the server
   is otherwise answering it: `UNIMPLEMENTED`; three calls in a row refused with `PERMISSION_DENIED`, `INVALID_ARGUMENT`,
   `FAILED_PRECONDITION` or `NOT_FOUND` (a closed list, the same in `sdks/SPEC.md` §6.2); or three calls in a row given up (every
   lease of the call given up with no verdict; a `RETRY` is a verdict) while at least one other call of the host to the same server (a
   receive, a heartbeat, a report) was answered during each, whatever status their attempts ended with. A database outage or a
   saturated server fails the other calls too and never counts; a failure of this call alone, whatever its status, counts, because
   waiting it out would hand every flagged lease back until its attempt is spent. Calls are counted, not leases, and an
   `ACKNOWLEDGED` resets the count. On fallback the host closes its receive, hands back what waits, waits for the answers of its
   hand-backs (their retries of §5), logs once, and registers again at its own minor with `declines_acknowledgement = true`; after
   10 minutes (with a random factor in `[0.8, 1.2]`, doubling for each fallback in a row up to an hour, and back to 10 minutes after
   an `ACKNOWLEDGED`) it registers again without declining. It keeps every feature of its minor; only the flag is declined, and each
   fallback is counted (`acknowledgementFallbacks`).
6. **Stopping.** A stopping host waits, at most the receive bound and within its `stop(timeout)`, for the verdict of an
   acknowledgement in flight, runs what is `ACKNOWLEDGED` as it runs any task in flight, and hands back the rest.
7. **Logs.** The happy path logs nothing new; `NOT_HELD`, `CANCELLED`, a lease given up, a refusal and a fallback log one line
   each, with identifiers only.

### Scoped instances

A collection item or loop body leased to a 1.8 session is marked, flagged, acknowledged, refused capabilities and withdrawn by the
same rules. The acknowledgement addresses the step row by its full address, so it does not depend on the path. The withdrawal is the
scoped withdrawal of the build of Architecture decision 0098 (one path transaction per run and instance path, with the addressed
event), whose re-read `FOR UPDATE` is the same; the refusal is a variant of `live_path_fence`; a stopping collection or loop drains a
withdrawn instance as one never leased. Each task of the plan has a scoped row in its evidence.

### Rollback

1. Stop marking: set `SAGAT_LEASE_WITHDRAWAL=off` on every engine. Once every engine runs with it, no engine marks a
   lease; sessions at 1.8 go on, their new leases unflagged; the marks already written keep their meaning, withdrawn at their
   deadlines and on their hand-backs, whatever the switch.
2. Wait until no mark counts (`worker_leases_marked` is 0): every marked lease ends, acknowledged, withdrawn or reported, with its
   meaning unchanged; that takes the longest earlier deadline in flight plus the period of the lost-lease pass and its backlog. Then
   remove `delivery-acknowledgement-v1` (the owner function, as `docs/runbooks/instance-registry.md` section 5 describes; its
   trigger keeps `lease-withdrawal-v1` while it is recorded). Removing it first would also stop marking, within one registry
   heartbeat, but would end the protection of the marks in flight: they would lapse as today.
3. Roll the binary back. The floor is the build of Architecture decision 0098, and the registry enforces it: that build decodes the
   cause `NOT_ACKNOWLEDGED` (precondition (a) of the task phase), and every writer of the cause reads `lease-withdrawal-v1` under the
   lock its removal waits for, which cannot be removed once a journal holds `step.lease_withdrawn` (Architecture decision 0098
   decision 8). On the floor a 1.8 session's hand-back is an ordinary failure, because that build clamps the holder's minor to its
   own ceiling (Decision 006-059 (3)), and this build honours none once the requirement is removed, so a capability served by a
   floor process can never be followed by a withdrawal. A session
   at 1.8 meeting a binary below 1.7 is the case of design 041; an unflagged lease asks nothing of 1.8.
4. The `down` migration refuses while a mark counts, while the history names the capability, or while a live process lists it; marks
   that no longer count are dropped with the columns, and the dependency trigger with its function.

### Failure modes and their exact outcomes

| Case | Outcome |
|---|---|
| A frame or a `PollTasks` answer lost; the host crashes before acknowledging; the engine crashes between the claim and the withdrawal of Architecture decision 0098 | At the earlier deadline the lease is withdrawn (`NOT_ACKNOWLEDGED`): `READY`, attempt not consumed, no review; the next lease has the same attempt number and the next generation |
| The acknowledgement never leaves the host | Repeated; on a verdict the task runs once; if the host gives the lease up first, nothing runs and the hand-back withdraws it (`NOT_STARTED`), also when it lands after the expiry and before the pass |
| The acknowledgement's call outlasts its deadline at the server | Abandoned before its commit, answered `UNAVAILABLE` (`ACKNOWLEDGEMENT_DEADLINE`); the host repeats it |
| A repeated acknowledgement with less than the margin left | `CANCELLED` (`too_late`): the host hands back; for an acknowledged lease an ordinary failure, never a handler started on an expiring lease |
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
| A hand-back after a capability was served | An ordinary retryable failure; a violation. A capability call and a hand-back at once are ordered by the run or path lock: the record commits before the serve, or the hand-back withdraws first and the call finds the lease not held |
| A report, heartbeat or capability call carrying the token of a lease withdrawn as `NOT_ACKNOWLEDGED` | Refused as stale; counted, audited once per step and withdrawn token, alerted: a handler may have run on a lease the journal says never ran (Gap 006-042). A hand-back or an acknowledgement with such a token is counted apart, as evidence that nothing ran |
| A hand-back that lands after its host registered again | Decided from `lease_hand_back_generation`, so still withdrawn |
| The attempt's withdrawals reach the cap of four | The withdrawal command refuses a fifth: the pass ends a provisional lease past a deadline with `expire_attempt` (an unknown outcome) and a hand-back is an ordinary failure, counted `capped` and alerted; the claim keeps marking |
| `AcknowledgeTasks` refused by a proxy or a rolled-back replica, or failing alone while the host's other calls are answered | `UNIMPLEMENTED`, three refused calls, or three calls given up while other calls were answered: the host hands back and registers again declining acknowledgement, at its own minor, for 10 minutes and then again without declining; `SagatWorkersDecliningAcknowledgement` and `SagatAcknowledgementsFailing` fire |
| A database outage or a saturated server while acknowledgements are in flight | The other calls fail too: the calls are repeated, leases whose give-up passes are handed back (withdrawn when the hand-back lands, or withdrawn by the pass at the deadline if it cannot); no fallback |
| A flagged lease received by a host that declines | Handed back, never dispatched; it triggers nothing |
| A host clock skewed by more than the time-to-live | Its give-up is clamped on its monotonic clock; its first heartbeat follows `lease_time_left_ms`; it acknowledges as any other host |
| A host configured with a long heartbeat interval | Its first heartbeat of a flagged lease still comes at a third of `lease_time_left_ms` |
| An engine restarts between the claim and the acknowledgement | Nothing of the lease lives in the engine; any engine of the build answers |
| The requirement removed while marked leases exist | No new mark after one registry heartbeat; the marks in flight lose their protection: at a deadline the pass ends them with `expire_attempt` (`lapsed`) and a hand-back is an ordinary failure; both requirements removed in a row while a mark exists: the pass's read under the locks orders it, and nothing is withdrawn after the removal |
| An engine with `SAGAT_LEASE_WITHDRAWAL=off` | It marks nothing and withdraws no unmarked lease; a marked lease is withdrawn by it as by any engine of the build |
| A lease whose earlier deadline is under 18 s | Not marked (`short_deadline`); today's meaning (Gap 006-047) |
| A failover to an asynchronous standby or a point-in-time recovery after an acknowledgement committed | The new timeline ends every earlier mark: a repeated acknowledgement answers `ACKNOWLEDGED` (`recovered`) with the margin, or `CANCELLED` without it; at its deadline the lease lapses as today, never withdrawn; its hand-back is an ordinary failure |
| A crash and restart of the primary, which keeps the timeline | The acknowledgement committed with `synchronous_commit` at least `local`, so it survives; no lease is marked while `fsync` is off (`no_durability`) |
| Any restore (a snapshot started as it is, a base backup without `recovery.signal`, a logical dump, a managed service's restore) | `control-plane end-lease-marks` before the engines start ends every mark; skipped, a restore that keeps the timeline is Gap 006-048 |
| A switch to another cluster that has not caught up (a logical-replication cut-over) | Gap 006-048: the runbook's step applies before the engines move |
| A database that does not let the runtime role read its timeline | The function answers null: no lease is marked (`no_timeline`), every statement still works, and `SagatDeliveryAcknowledgementInactive` fires while the requirement is recorded |
| A hand-written client at 1.8 that runs a flagged lease without acknowledging and makes no call during it | Withdrawn and run again with the same attempt number and idempotency key (Gap 006-042); a later call with its token is counted and alerted |

### Observability

- `worker_task_acknowledgements_total{verdict, reason}`; `grpc_requests_total{method="AcknowledgeTasks", code}`.
- `worker_leases_undelivered_total{cause, outcome}` (the counter of Architecture decision 0098) gains the cause `NOT_ACKNOWLEDGED`;
  `worker_lease_marks_skipped_total{reason}`; `worker_leases_expired_total{mark}` (`acknowledged`, `recovered`, `unmarked`), since the
  journal does not record that a lapsed lease had been acknowledged (Gap 006-046).
- `worker_task_delivery_delay_seconds`, from the claim to the acknowledgement on the database clock; and
  `worker_task_acknowledgement_call_seconds`, the server-side duration of one call by its number of leases (`1`, `2-8`, `9-64`).
- Gauges without labels: `worker_leases_marked`, `worker_leases_provisional`, `worker_leases_provisional_overdue`; and
  `worker_sessions{protocol, declines}`, the live sessions by accepted minor and whether they decline acknowledgement, with the
  counter `worker_registrations_declining_total`.
- `worker_capability_calls_refused_total{method, reason="TASK_NOT_ACKNOWLEDGED"}`, `worker_protocol_violations_total{method}`,
  `worker_calls_after_withdrawal_total{kind}` (`report`, `capability`, `hand_back`, `acknowledgement`).
- Alerts, with `promtool` cases at the default and at an 18 s time-to-live:
  - `SagatLeasesNotAcknowledged`: withdrawals with the cause `NOT_ACKNOWLEDGED` above 1% of marked claims over 15 minutes (frames are
    being lost or hosts are not acknowledging; a threshold chosen as an operating limit, not measured).
  - `SagatAcknowledgementsFailing`: `grpc_requests_total{method="AcknowledgeTasks"}` with a code other than `OK` above 5% of the
    calls over 10 minutes, or withdrawals with the cause `NOT_STARTED` of marked leases above 1% of marked claims over 15 minutes
    (hosts are handing back what they could not acknowledge).
  - `SagatLeaseAttemptCapped`: withdrawals of marked leases refused at the cap (`worker_leases_undelivered_total{cause,
    outcome="capped"}` for `NOT_ACKNOWLEDGED`, and for `NOT_STARTED` of a marked lease) above 1% of marked claims over 15 minutes.
  - `SagatWorkersDecliningAcknowledgement`: any live session declining for 15 minutes while the requirement is recorded, or
    declining registrations above 10% of live sessions over 15 minutes (`worker_registrations_declining_total`), which catches a
    fleet-wide first fallback shorter than 15 minutes.
  - `SagatDeliveryAcknowledgementInactive`: any increase of `worker_lease_marks_skipped_total{reason=~"no_timeline|no_durability"}`
    while the requirement is recorded (the operator believes the protection is on and it is not).
  - `SagatLeaseDeliverySlow`: the 99th percentile of the delivery delay over 10 minutes above twice the baseline Task 006-056
    records, and never above the receive bound.
  - `SagatCallsAfterWithdrawal`: any increase of the `report` or `capability` kind.
- The call's span links to the trace context of each lease answered `ACKNOWLEDGED` (at most 64), never as a parent. No log line,
  label or attribute carries a token, a payload or an unbounded identifier.
- SDK metrics hooks gain `acknowledgements`, `notHeld`, `givenUp` and `acknowledgementFallbacks` counters.

### Acceptance matrix

| Area | Required evidence |
|---|---|
| Exactness | Barrier tests: the acknowledgement holds the row while the pass's withdrawal waits, the reverse, the same with the hand-back's withdrawal, a repeated acknowledgement while the first commits, an acknowledgement after a deadline and before the pass, a hand-back 1 ms after the expiry with the pass held, a capability call against a hand-back in both orders, the removal of each requirement between the pass's read and its commit, a revocation; root and scoped |
| The pass ends what it does not withdraw | With a requirement removed, and at the cap, a marked lease past its expiry is ended by one pass with `expire_attempt` and not picked again; root and scoped |
| The marks | Each of the three lease statements; a lease written with the old column list is not marked; a mark of another timeline, lower, higher or null, does not count; the static check of the assignments and its negative test; the barrier with a run transaction loaded before an acknowledgement |
| Withdrawal | A provisional `Activity` lease past its expiry, and one past its attempt deadline: `step.lease_withdrawn` cause `NOT_ACKNOWLEDGED`, no review, the attempt reused; an acknowledged lease that expires is an unknown outcome; the cap: after four withdrawals a marked lease lapses at its deadline and its hand-back spends the attempt, counted `capped` |
| Hand-back | From a lease under the reserved code, withdrawn, also after its expiry when provisional; from one claimed below 1.8, an ordinary failure; after the session registered again at another minor, decided by the lease; for an acknowledged lease or after a capability was served, an ordinary failure; on the floor build, a 1.8 session's hand-back is an ordinary failure; a handler-raised code reaches the server as `WORKER_ERROR` |
| Capabilities | The descriptor-driven test, root and scoped; a report on a provisional lease is served; a wrong token gets the ordinary stale answer |
| Gate | The capability unlisted, the activation entry absent and the minor at 7 before Task 006-057; no mark while the requirement is not recorded; the trigger and the command refuse both orders, with the race held open; the trigger's pair equals the entry's `requires` |
| Cost | `LEASE` and both completion lines unchanged; `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` pinned, the former with no run lock, no scan of the run's steps, no journal or outbox insert, fewer statements than the latter; a 64-lease call itemised with its triggers |
| Throughput (gate, before the SDKs and before 1.8 is offered) | The transitions scenario at 1.8 against 1.7, median of five interleaved runs each, network between workers and services: no more than 15% lower; the successful 64-lease calls within 3 s at the 99th percentile and fewer than 0.1% of calls ended by the deadline; no heartbeat shed while acknowledgements are admitted in a burst; the heartbeat's 99th percentile in the burst under 2 s; the rise of ready to handler entry and the share of claims left unmarked as `short_deadline` reported |
| Six SDKs | `delivery-acknowledgement` passes in six languages with a reviewed golden; under the override every host scenario but `protocol-negotiation` and those that hand back runs at 1.8 with its existing golden unblessed and acknowledges at least one flagged lease |
| Fallback | A 30 s `UNAVAILABLE` outage of the server (every call failing) with three flagged frames in flight does not fall back; two minutes of `UNAVAILABLE` for this call alone while the stream is answered falls back and spends no attempt; three refused calls fall back, and the host registers again without declining after its period; a flagged frame delivered after the declining registration is handed back |
| Durability | A row whose mark has another timeline or none is an unmarked lease for the pass and the hand-back and `recovered` for the acknowledgement (the parity test and a pass in `integration_durable_execution`); with `ALTER ROLE sagat_app SET synchronous_commit = off` the acknowledgement's transaction commits with `local`; the probe refuses `fsync = off`; a role without the right gets a null timeline and the operations snapshot, the pass and the acknowledgement still work; the failover drill of `infra/dr/drill.sh` reads `sagat_database_timeline()` before and after each disaster, finds it changed, and finds a provisional mark held at the kill not withdrawn after the promotion; `control-plane end-lease-marks` ends every mark and writes its audit record |
| Released v0.2 | `provisional-lease-cutover` forward and its rollback case, with a session stored at 8 and a hand-back on the floor, with and without a capability served; a mixed fleet before the activation and after the removal: `ResolveSecrets` served by a floor control plane, the hand-back received by this build, an ordinary failure |
