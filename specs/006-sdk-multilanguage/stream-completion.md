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
   under invalid credentials. A committed lease whose frame was never handed to the transport is
   withdrawn ("Leases that never reach a host", rule 1); a lease handed to the transport recovers
   by expiry and is never requeued merely because its response was suppressed.

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

Decisions D006-001 to D006-018 and D006-030 to D006-039 of [`research.md`](research.md); ADR-0098. A committed lease that no host received used to be
left to lapse; since feature 037 a lapse of an `Activity` opens a review for work that never ran. The rules:

1. **The server withdraws what it knows it did not send.** A lease is *handed to the transport* when `TaskBody::poll_next`
   returns the frame that carries it (for a unary `PollTasks`, when the handler returns its answer). Every lease committed by a
   claim and not handed to the transport is withdrawn: the leases of a claim that ends after the producer was told to stop, of
   a frame queued when the window deadline passes, of a frame drained at token expiry or when the body is dropped, of a frame
   sent into a closed queue, and of a unary poll whose handler is dropped before it returns.
2. **The producer is told to stop, never aborted in a claim.** The run transaction it is in finishes (it is bounded by the
   store's timeouts); the claim leases no further run once its consumer is gone; nothing more is queued; what it holds goes
   to the withdrawal. The producer and the unary claim run on the tracker a graceful shutdown waits for (D006-035, D006-036).
3. **A withdrawal is fenced.** One transaction per run, under the run lock; it acts only on a step that is still `LEASED` with
   the generation and lease-token hash the server issued. It appends `step.lease_withdrawn` (`node`, `attempt`, `generation`,
   `cause`: `NOT_SENT` or `NOT_STARTED`), clears the lease, returns the step to `READY` and sets its attempt back by one, so
   the next lease reuses that attempt number with the next generation. A fence that does not hold changes nothing and is
   counted (`worker_leases_undelivered_total{cause, outcome="fence_missed"}`). One attempt is withdrawn at most four times
   (D006-031); the fifth is counted `capped`. At most a quarter of the engine's pool withdraws at once (D006-038).
4. **Only when every live process honours it.** The engine withdraws only while the requirement `lease-withdrawal-v1` is
   recorded for control planes and engines (instance-capability registry, ADR-0082; `control-plane capability-activate
   lease-withdrawal-v1`). Until then the lease lapses as before and `worker_leases_undelivered_total{cause, outcome="lapsed"}`
   counts it. The requirement cannot be removed once a journal holds the event; `SAGAT_LEASE_WITHDRAWAL=off` stops an engine
   writing new withdrawals (D006-032).
5. **A host hands back what it drops** (`sdks/SPEC.md` §6.2): `FailStep` with `TASK_NOT_STARTED`, retryable, before any
   handler of the task runs; a handler that raises the code has it rewritten by its SDK. The server withdraws it (cause
   `NOT_STARTED`) only when the session that holds the lease negotiated 1.8 or later, the requirement is recorded, the step
   is still `LEASED` and the cap is not reached (D006-030); otherwise it is an ordinary failure with the host's `retryable`
   flag, never an unknown outcome.
   At 1.8 ("Delivery acknowledgement (protocol 1.8)" below) the reservation is read from the lease, not from the session; it is
   honoured only while `delivery-acknowledgement-v1` is recorded, for a lease claimed after that recording on the current database
   server; and a lease that has not run (still `LEASED`, no capability served) is withdrawn on its hand-back whether or not it was
   acknowledged, also after its expiry.
6. **What is not covered.** A frame lost on the wire after it was handed to the transport, and a lost unary answer, still
   lapse (Gap 006-001) when unflagged; for a lease flagged at 1.8 delivery acknowledgement closes it (next section). A process that stops between a claim and its withdrawal leaves
   the lease to lapse (Gap 006-002); a flagged lease is provisional and is withdrawn at its own earlier deadline instead.





## Delivery acknowledgement (protocol 1.8)

Decision 006-040 to Decision 006-066 of [`research.md`](research.md); Architecture decision 0099; Owner decision 2026-11-20-12.
Minor 1.8 adds a flag on the lease (`acknowledge_before_start`), a call (`AcknowledgeTasks`) and a registration field
(`declines_acknowledgement`), and is the minor at which the hand-back code `TASK_NOT_STARTED` is reserved for the host
(Architecture decision 0098 decision 5). A flagged lease is *provisional* until its host acknowledges it; the host runs nothing and
calls nothing for it but the acknowledgement and the hand-back before the server has committed that acknowledgement; a provisional
lease whose earlier deadline passes is withdrawn (the cause `NOT_ACKNOWLEDGED`), so a frame lost on the wire and a lost `PollTasks`
answer (Gap 006-001) no longer become a review. An acknowledged lease is an attempt once it runs; while it is still `LEASED` and
its host hands it back, it is withdrawn, as a provisional lease's hand-back is. Otherwise it has today's meaning in every respect.
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
  optional int64 attempt_time_left_ms = 4;    // when ACKNOWLEDGED and the attempt has a deadline: the time left before it, likewise
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
  to 64, not the formula. `AppendStepLogs` of 1.7 is classified explicitly in the same change. A shed acknowledgement answers
  `RESOURCE_EXHAUSTED` with the reason `ACKNOWLEDGEMENT_SHED` and costs at worst a withdrawal; a shed heartbeat can cost a review.

### The hand-back at 1.8

Architecture decision 0098, as amended by its build review (its decisions 5, 7 and 8; Decision 006-030 to Decision 006-039 on its
branch), holds the server's rule: a `TASK_NOT_STARTED` withdraws the lease (cause `NOT_STARTED`, no attempt consumed, no review)
only from a lease under the reserved code, while `lease-withdrawal-v1` is recorded, the step is `LEASED` with the issued fence, and
the attempt's withdrawals are under four; otherwise it is an ordinary failure, the attempt spent, the flag the host sent honoured,
never a review. This design adds:

- **The reservation is read from the lease.** The claim writes `lease_hand_back_generation` when the claiming session was accepted
  at 1.8 or later; the hand-back decides from it, under the run lock, never from the session row, which registering again under the
  same worker name overwrites. This build's code does it (Task 006-050).
- **Only while `delivery-acknowledgement-v1` is recorded, for a lease claimed after that recording on the current server.** The
  hand-back reads the requirement and its `required_since` under the lock its removal waits for, and honours the reservation only
  when `lease_claimed_at` is later and `lease_claim_incarnation` is the current server's start time. While it is recorded no live
  engine is of a build that serves a capability without recording it (and an engine silent at the census serves no capability call until its
  gate is decided again, "Negotiation and activation"), so `lease_capability_generation` is complete for every lease claimed since. Before the
  activation, after a removal, for a lease claimed before a re-activation, or on another server incarnation, a 1.8 hand-back is an
  ordinary failure that spends the attempt, as below 1.8.
- **The build of Architecture decision 0098 never honours a 1.8 hand-back.** It decides the holder's minor as the smaller of the
  stored minor and its own compiled ceiling, so with its ceiling at 7 or below it reads no session as 1.8, and its tests raise both
  (Decision 006-059 (3)). A hand-back that reaches an engine of the older build in a mixed fleet, or after a rollback, is an ordinary
  failure, the safe direction.
- **A lease its host did not start is withdrawn on its hand-back, provisional or acknowledged.** For a lease whose mark counts that
  is still `LEASED` (no heartbeat has made it `RUNNING`) and to which no capability was served, the hand-back is withdrawn, also
  after the expiry and with a stop asked, until the pass has ended it: nothing ran, the host says so as it does for a provisional
  lease (the trust assumption), and the pass would withdraw a provisional one anyway. So a hand-back that lands after the expiry,
  which the host's give-up makes common when no attempt is answered, costs no attempt, and neither does an acknowledgement whose
  answer was lost, which the host gives up and hands back without knowing it committed. The fences of the report path, the secret
  report fence of a step with declared secrets included, are relaxed for it as Decision 006-059 (1) says; under a stop, the
  withdrawal's settle cancels a `FORWARD` step. An unmarked lease keeps the rule of Architecture decision 0098.
- **Once per attempt after a hand-back.** The withdrawal sets `provisional_withdrawn_attempt`, so the next lease of the attempt is
  not marked: a failing acknowledgement path costs a lease at most one hand-back per attempt, and a planned hand-back (a host's stop,
  a decline, `CANCELLED`) leaves that attempt's next delivery unprotected (Gap 006-047).
- **The engine's switch does not reach a marked lease.** `SAGAT_LEASE_WITHDRAWAL=off` stops new marks and the withdrawals of
  Architecture decision 0098 for unmarked leases; the hand-back and the pass's withdrawal of a lease whose mark counts need both
  requirements and not the switch, so a mark keeps its meaning whichever engine handles it.
- **Not after it ran, or after a capability was served**: a hand-back of a `RUNNING` lease is an ordinary failure, and of a lease
  whose `lease_capability_generation` is its generation an ordinary failure counted and audited as a protocol violation (Decision
  006-053).
- **The order of locks**: the binding checks of the report path come before the requirement read and the re-read of the step row
  `FOR UPDATE`, in every withdrawal path.

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
- From the same task every engine of the build lists `delivery-acknowledgement-v1`, meaning it serves `AcknowledgeTasks`, replays
  the cause `NOT_ACKNOWLEDGED`, refuses capabilities to provisional leases, and withdraws a marked provisional lease at its deadline
  and on its hand-back (Architecture decision 0082 decision 7: nothing is listed that the process will not do). The requirement is
  recorded for engines only: a control plane serves no worker call and runs no pass, and the floor build's control planes will
  decode the cause once request (2) of Decision 006-059 is built, which a released build must have before Task 006-057, so requiring
  them would only add a step to every rollback. An engine with
  `SAGAT_LEASE_WITHDRAWAL=off` still does all of it and lists it; the switch only stops it marking new leases and withdrawing
  unmarked ones (the hand-back above).
- `control-plane capability-activate delivery-acknowledgement-v1` (the generic command of Decision 006-034 of the server build)
  gains its entry in Task 006-057, with `empty_fleet: false`, so the command refuses it before the build lists the capability, and
  the entry refuses while `lease-withdrawal-v1` is not recorded for both kinds, with a message that says so. That refusal is a
  convenience, not the guarantee: an engine marks only while it knows both requirements are recorded, and every withdrawal of a
  marked lease reads both under the locks their removals wait for, so no order of activations and removals, by the command or by the
  hand SQL of `docs/runbooks/instance-registry.md` section 5, can write an event without its replay floor; the wrong order costs only
  marks that are never honoured (a trigger of its own was considered and rejected, Decision 006-047).
- Every engine of the build serves `AcknowledgeTasks` whatever it knows of the requirements. It learns of an activation or a removal
  within one registry heartbeat (Gap 006-044). A mark already written keeps its meaning until its lease ends, whatever the switch;
  removing the requirement ends the protection of the provisional leases in flight, which is why a rollback drains them first.
- **The read of both requirements** is one statement, `delivery_acknowledgement_required()` (`SECURITY DEFINER`, `search_path =
  pg_catalog, pg_temp`, `public.` names, revoked from `PUBLIC` and granted to `sagat_app`, as `lease_withdrawal_required()` of
  Architecture decision 0098 decision 8 is), which takes both rows `FOR SHARE` in the order `delivery-acknowledgement-v1`, then
  `lease-withdrawal-v1`, the order in which removals must happen, and returns whether `lease-withdrawal-v1` is recorded for both
  process kinds (control planes replay the event too), whether `delivery-acknowledgement-v1` is recorded for engines, and the
  latter's `required_since`. The port returns it as a witness: the run and path transaction traits gain `type Requirements:
  RequirementsHeld` (a trait of `crates/application` with read-only accessors) and `delivery_acknowledgement_required(&mut self) ->
  Result<Self::Requirements, StoreError>`; the PostgreSQL adapter's concrete type has a private constructor; the decision function
  and the withdrawal command are generic and take `&Tx::Requirements`, not booleans, and their unit and parity tests implement the
  trait with a test type. These generic functions are the application's withdrawal services (the root and path withdrawals and the
  hand-back) and its decision function; the domain's `withdraw_lease` takes plain domain values and returns, besides its events,
  whether this withdrawal is the last the cap allows. So a withdrawal path that holds a transaction cannot decide without calling the
  read; that the witness is of the same transaction is the barrier tests' to prove. Every withdrawal of a lease whose mark counts, the pass's included, calls it in its transaction
  before it writes; when a requirement is not recorded so it writes no withdrawal, and the pass ends the attempt with
  `expire_attempt` instead, counted `lapsed`. A `NOT_SENT` withdrawal, whose token never left the server, needs only
  `lease-withdrawal-v1`, as Architecture decision 0098 has it. A hand recording of `delivery-acknowledgement-v1` without
  `lease-withdrawal-v1` for both kinds turns marking off with its own skip reason, `requirement_missing`, which the inactive alert
  watches.
- **An engine silent at an activation's census** serves no capability call (the calls of decision 8 of Architecture decision 0099:
  every lease-token call but `Heartbeat`, `CompleteStep`, `FailStep` and `AcknowledgeTasks`) once its registry has been silent for
  the liveness window less 5 s (25 s), measured from the start of its last successful beat, until a beat that returns an epoch and
  the gate's decision under that epoch have both succeeded; the registry's own connection carries a statement timeout of at most 5 s,
  so the margin holds by construction and no configuration is refused. The beat is a new overload, `instance_heartbeat(uuid, bytea,
  boolean)` (`SECURITY DEFINER`, `search_path = pg_catalog, pg_temp`, `public.` names, revoked from `PUBLIC` and granted to
  `sagat_app`, with a privilege test of `prosecdef` and `proconfig` run as the runtime role); the two-argument function keeps its
  contract for older binaries. It reads the epoch first, `FOR SHARE SKIP LOCKED`: an owner transaction that holds the row (an
  activation, or a removal by hand SQL left open) never blocks it; it then still commits `last_seen` and returns no epoch, and it
  raises when the row is missing rather than skipped. A beat with no epoch is a successful beat for liveness, readiness and the rule
  that closes the gate; it keeps an engine's current capability decision, and extends its 25 s window, only while that window has
  not lapsed; it never opens capability calls on an engine whose window lapsed, which waits for a beat with an epoch. So an owner
  transaction held open keeps serving engines serving and refusing engines refusing, and the census, which counts every engine that
  beats, keeps the activation safe. The gate is checked again before the first byte of the answer of a call that hands data out
  (`ResolveSecrets`, `ResolveDatabase`, `ReadBlob`); a call that writes (`WriteBlob`, `AppendStepLogs`) is checked before its write.
  The refusal is `UNAVAILABLE` with the envelope reason `REGISTRY_SILENT` and a retry hint, counted in
  `worker_capability_calls_refused_total{method, reason="REGISTRY_SILENT"}`; the SDKs at 1.8 repeat a capability call on it within
  the handler's deadline (`sdks/SPEC.md` §5); a host below 1.8 does not, and its handler fails. Heartbeats, reports, hand-backs and
  claims keep flowing: none hands anything over, a floor engine's claim writes no reservation, and its hand-back is an ordinary
  failure. This amends Architecture decision 0082 decision 5 for capability calls only, in the build of Architecture decision 0098
  (Decision 006-059 (7)), and this build inherits it for its engines too.

### The marks

Columns of `steps`, all written for a lease's generation. A mark counts only while its generation is the step's `generation`, the
step is `LEASED` or `RUNNING`, `lease_claim_incarnation` is not null and equals the current server's `pg_postmaster_start_time()`,
and `lease_claimed_at` is later than the `required_since` of `delivery-acknowledgement-v1` where that is read:

| Column | Written by | Meaning |
|---|---|---|
| `lease_hand_back_generation bigint`, `lease_claimed_at timestamptz` and `lease_claim_incarnation timestamptz` | the claim, for a session accepted at 1.8 or later: the generation, the claim's time (`clock_timestamp()` after the lock) and `pg_postmaster_start_time()` | the lease is under the reserved hand-back; when it was claimed; the database server it was claimed on |
| `lease_offered_generation bigint` | the claim, when it marks the lease provisional (and sets the flag) | provisional until acknowledged |
| `lease_acknowledged_at timestamptz` | cleared by a claim that marks; set by the acknowledgement | acknowledged |
| `lease_capability_generation bigint` | the first capability call served to the lease, before it is served | a capability was served to this lease |
| `withdrawn_lease_token_hash bytea` | a withdrawal of a lease whose mark counts, `NOT_ACKNOWLEDGED` or on its hand-back; replaced by the next | evidence, read only on the stale path of a call |

and `worker_sessions.declines_acknowledgement boolean NOT NULL DEFAULT false`, written by the host's own registration (the
session upsert sets it from the registration, so one that does not decline clears it).

- **The attempt's flag: `steps.provisional_withdrawn_attempt integer`.** This is attempt data, not lease data: no generation binds
  it, no lease write clears it, and nothing ever clears it; a later attempt number simply never equals it, and losing it (a projection
  rebuilt from the journal, which does not record it) only means more flags, which the cap bounds; at the cap itself a refused fifth
  withdrawal writes nothing, so after a rebuild an attempt at the cap stays flagged until it ends, with today's meaning. The claim,
  the withdrawal and every other transition write a step row through the same three projection statements (`upsert_step`,
  `upsert_scoped_instance`, the path statement), so the flag has one form in each of them, for every caller:
  `provisional_withdrawn_attempt = CASE WHEN $flag AND status = 'LEASED' AND generation = $fence THEN attempt ELSE
  provisional_withdrawn_attempt END`. `$flag` is bound true only in the projection call of the withdrawn node: the application's
  withdrawal service, after its `FOR UPDATE` re-read under the run or path lock, calls a port method keyed by node and generation
  (`mark_withdrawn_attempt(&node, generation)`), which the adapter consumes in that node's call alone, for a `NOT_STARTED` withdrawal
  of a lease flagged at its claim whose mark counts, or for the withdrawal that the domain's `withdraw_lease` reports as the last the
  cap allows; every other call, the claim and every sibling touched by the same append included, binds false. `$fence` is the
  withdrawn lease's generation. The old-row
  fence (`status`, `generation`) keeps a later append of the same transaction from writing a row already withdrawn, and `attempt` on
  the right is the stored value before the same statement sets it back by one (PostgreSQL evaluates `SET` against the old row), so
  it is the withdrawn lease's attempt, the one the next claim writes again. This is the one bound boolean the static check allows, in
  the flag's own allowed set, because a stale caller can only bind false, the safe direction. No statement other than the three names the column. The claim reads it from
  the row it writes and marks only while it differs from the attempt being claimed. A step whose attempt is reset can only make it
  match a later attempt, which means fewer flags.
- **The incarnation.** `pg_postmaster_start_time()` is executable by every role in stock PostgreSQL (Task 006-044 checks the pinned
  image), never fails on a standby, and changes whenever another database server takes over: a promotion (the standby is another
  server), a point-in-time recovery, a snapshot or a base backup started as a server with or without `recovery.signal`, a managed
  service's restore or failover to a new instance, a cut-over to another cluster, the return of an old primary (PostgreSQL demotes
  only through a restart), and the loss of the operating system. Compared by equality, it involves no comparison of clocks. A
  statement evaluates it once, as a scalar subquery. So an acknowledgement committed before any of these and lost by it can never
  become a false "nothing ran". It assumes one writable server, as the row lock of the exactness argument does; the gauges read it
  on the primary. The database's timeline was considered and rejected: it does not change for a snapshot, a base backup without
  archive recovery, a managed restore or a cut-over, and reading it needs a guarded function. The cost is that a planned restart of
  PostgreSQL ends the protection of the leases in flight (they keep today's meaning).
- **A mark that does not count** (another incarnation, or none) is, for the acknowledgement, an attempt (it answers `ACKNOWLEDGED`,
  counted `recovered`, under the same margin and stop rule as any other), and for every withdrawal an unmarked lease under
  Architecture decision 0098's rules, except that its hand-back is an ordinary failure, because a capability record of another
  incarnation may have been lost. The withdrawals and the hand-back also read `required_since` and treat a lease claimed before the
  requirement's current recording the same way; the acknowledgement does not read it, which is safe, because the withdrawals decide
  alone. `lease_claimed_at` and `required_since` are both `clock_timestamp()` on the same server; an equal value does not count, and
  only a database clock stepped backwards across a removal and a re-activation could make an older lease count (Gap 006-044).
- **Durable commit.** The acknowledgement's settings statement, and the capability record's, raise `synchronous_commit` to `local` for
  their transaction when it is `off` (never lowering a stronger value), so a crash of PostgreSQL that the postmaster recovers without
  restarting, which keeps the start time and loses the write-ahead log not yet written, cannot lose a committed acknowledgement or
  capability record; a lost claim or withdrawal is safe. `fsync` off is outside the guarantee (Gap 006-048): a commit then reaches
  only the kernel, which the loss of the operating system loses (that starts another server, which the incarnation sees) but which a
  device's write-back error can also lose with the server still running. Task 006-044 records whether a crash restart changes the
  start time; if it does, the raise can go too.
- **A restore that does not start the server** (a logical restore into the same running server, a virtual machine reverted with its
  memory, a storage snapshot rolled back under a running server) is done with every engine stopped, and PostgreSQL is restarted
  before the engines start, and
  `SELECT pg_postmaster_start_time()` is checked to have changed (section 7 of `docs/runbooks/disaster-recovery.md`). That gives the
  leases in flight today's meaning, as a promotion does by itself; no command, function or table is needed.
- The claim writes them in the statement that already writes the lease, in each of the three: `upsert_step` (root),
  `upsert_scoped_instance` (a scoped instance claimed in a full-run transaction) and the path statement of `execution/path.rs`, and
  returns whether it marked and the first reason it did not.
- It marks the lease provisional when the session was accepted at 1.8 or later and does not decline acknowledgement, the engine knows
  both requirements are recorded as above, its `SAGAT_LEASE_WITHDRAWAL` is on, `provisional_withdrawn_attempt` is not the attempt
  being claimed, and the lease's earlier deadline (the earlier of the claim's time plus the engine's time-to-live and the claim's time plus `timeoutMs`) is at
  least three receive bounds (18 s) after the claim. The claim does not read the attempt's withdrawals: the cap of four is applied by
  the withdrawal command, in its own transaction, from the journal (decision 5 of Architecture decision 0099). Otherwise the lease is
  not marked, the flag is false, and `worker_lease_marks_skipped_total{reason}` counts it with the first reason that holds, in this
  order: `not_activated`, `requirement_missing`, `switch_off`, `declined`, `flagged_before`, `short_deadline`.
- No statement assigns a column of the marks from a value it read earlier. The allowed forms are: the claim's, in SQL conditioned
  on a new lease (the stored `generation` differs from the one written), never on a bound flag alone; the engine's other statements
  keep them (`ELSE` the column) or clear them with the lease; the acknowledgement sets `lease_acknowledged_at = clock_timestamp()`
  only; the capability paths set `lease_capability_generation = generation` only; the withdrawal sets `withdrawn_lease_token_hash`
  from the row it holds. `provisional_withdrawn_attempt` has its own allowed set: the one fenced form above, in the three
  statements, and nothing else. A static check in `scripts/check-architecture.py`
  fails on any other form in `crates/infra-postgres` and the migrations, with negative tests for a bound-flag form and a read-back
  form, and, for the attempt's flag, for an assignment outside the three named functions, `THEN $m` (a bound attempt), a branch that
  yields `NULL`, a form without the old-row fence, and a second assignment of the column in one statement (the old-row columns
  accepted unqualified or with the statement's alias, and the three `INSERT`s allowed to leave the column out);
  a barrier test holds a run transaction that loaded the run before an acknowledgement and makes it write the row through each
  statement that can touch a `LEASED` row.
- No `CHECK` ties the columns to the status: an older binary must still be able to write the row. A mark left by a binary that does
  not know the columns never counts, because the next lease has the next generation.
- The condition is one SQL macro of the adapter and one pure function of the application, which extends `LeaseDeadlines`
  (`crates/application/src/execution.rs`) and receives "the mark is current" as a boolean the adapter computes, not a server start
  time; a parity test in `integration_durable_execution` writes every combination of a row (the incarnation equal, other and null;
  claimed before and after the recording, among them) and compares them; the claim-time condition on the attempt flag is the
  matrix's of Task 006-047, not the macro's. The macro has two
  named forms, with and without the recording, and the static check knows which statement uses which.
- The gauges are `FILTER` clauses of the scan the operations statistics query already makes of leased steps
  (`crates/infra-postgres/src/operations.rs`), through the same macro; no index is added. `worker_leases_marked`: steps whose mark
  counts. `worker_leases_provisional`: of those, `LEASED` and not acknowledged. `worker_leases_provisional_overdue`: of those, past a
  deadline. The rollback's drain and the `down` migration's refusal use `worker_leases_provisional`'s condition.
- The marks are lease data: not journaled, not derived by the replay, and no test of replay against projection asserts them.

### The server's acknowledgement

One transaction per call, through a store port of its own (`AcknowledgementTx`: `matched`, `bound_patterns`,
`current_worker_authorized`, `lock_matched`, `acknowledge`); no run lock:

1. The call has a deadline of half the receive bound (3 s) from its arrival, which covers waiting for a pooled connection (the
   acquisition is bounded by the time left). The service checks it before each statement and, once it has passed, rolls the
   transaction back and returns the connection to the pool (it discards it only when the rollback does not return within 100 ms),
   so a call ends at most one statement timeout (1 s) past its deadline, 4 s in all. The deadline starts in the platform's own
   outermost layer of the gRPC server: tonic's `concurrency_limit_per_connection` is replaced by a concurrency layer of the platform
   under the stamping layer, keyed by connection, with a reserve for the class `Acknowledge`, so a call queued behind long transfers
   is still answered within its bound. The configuration key `concurrency_per_connection` keeps its name, its default (64) and its
   meaning; each connection's semaphore is held in its `CappedConnectInfo` and dropped with it (no map keyed by the peer, which an
   unauthenticated peer could grow); the reserve is 4 of those places for the class `Acknowledge`, and applies only from Task 006-057,
   so the dormant build changes no class's concurrency. Stream credit beyond `max_concurrent_streams` waits in the client, so an
   acknowledgement attempt that the host's transport never started on a stream does not count toward a decline (§6.2). It answers `UNAVAILABLE` with the reason
   `ACKNOWLEDGEMENT_DEADLINE` in its details, never `DEADLINE_EXCEEDED`, which the host keeps for its own bound. The
   `statement_timeout` also bounds the `COMMIT`; a commit whose wait for a synchronous standby is cut short has committed locally
   (PostgreSQL warns, the client sees success), and the incarnation covers the failover that would lose it. `worker:execute`, once
   (`PERMISSION_DENIED`, audited as today). One statement sets, for the transaction (`set_config(..., true)`), `lock_timeout` and
   `statement_timeout` to 1 s, and `synchronous_commit` to `local` when it is `off`.
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
   attempt, kind (`FORWARD`, compensation, reconciliation), token hash, marks, deadlines, `cancel_requested`, the run's stop reason,
   `pg_postmaster_start_time()` and
   `clock_timestamp()`; with `SKIP LOCKED` the statement never waits, so the clock is sampled when the locks are held. A row it
   could not lock answers `RETRY` and costs nothing. The `runs` row is read, not locked: a stop committed after that read is ordered
   after the acknowledgement and reaches the host through its first heartbeat, as any stop of a running task does.
5. The pure function decides each locked row: provisional with the request's token, attempt and generation, no stop asked, at least
   one receive bound left before both `lease_expires_at` and `attempt_deadline_at` (when set): it wins. With the same token, an
   acknowledged lease (counted `duplicate`) or a lease whose mark does not count (counted `recovered`) answers `ACKNOWLEDGED` under the
   same margin and stop rule and writes nothing. A stop is `cancel_requested` or a run stop reason, and counts only for a `FORWARD`
   task: a compensation or reconciliation, which the server leases on purpose while a run is stopping, is acknowledged whatever the
   run's stop reason, as its heartbeat is never told to stop (`sdks/SPEC.md` §6.4). A `FORWARD` lease with a stop, and an acknowledged
   or recovered lease with less than the margin left, answer `CANCELLED` (reasons `stop_asked`, `too_late`): run nothing, hand back.
   Anything else answers `NOT_HELD`, with its reason. The token is compared here, not in the statement's `WHERE`, so a row skipped is
   always a row held by another transaction.
6. `acknowledge`: one update of the winning rows, which the transaction already holds: `lease_acknowledged_at = clock_timestamp()`.
   It returns the delay since `lease_claimed_at` and the time left before `lease_expires_at` and before `attempt_deadline_at`, which
   the answer reports, clamped at 0.
7. A deadlock, a serialization failure, a lock or statement timeout, the call's deadline, or an unavailable store ends the call with
   `UNAVAILABLE` (retryable); the host repeats it, and the rows already written answer `ACKNOWLEDGED` again. No transient error is ever
   answered `NOT_HELD`.

Nothing else: the acknowledgement does not restore the run, renew the lease, apply a transition, write the journal or the outbox.
About eight statements per call whatever its size (the settings, `matched`, two for the binding, two for the membership,
`lock_matched`, `acknowledge`), plus the triggers of `steps` the update fires, which the budget's top-level count does not show and
Task 006-055 itemises. Counted: `worker_task_acknowledgements_total{verdict, reason}` with the verdicts `acknowledged`, `duplicate`,
`recovered`, `not_held` (reasons `no_match`, `withdrawn` (the token is the step's withdrawn one, as a probe's is), `too_late`,
`not_provisional`, `unbound`), `cancelled` (reasons `stop_asked`,
`too_late`), `retry` and `deadline`.

### The withdrawals

Every withdrawal of a lease whose mark counts, under the run lock (the whole-run or the path transaction), after the report path's
binding checks (the lost-lease pass, a system actor, has none) and after reading both requirements with
`delivery_acknowledgement_required()` (one statement; "Negotiation and activation"), re-reads the step row `FOR UPDATE` and decides with the pure function on what it read. The withdrawal command of Architecture decision 0098
then applies the cap of four per attempt from the journal count it already reads (`lease_withdrawals`, one statement), as for every
other cause; a withdrawal that brings the attempt's count to four, whatever its cause, also sets `provisional_withdrawn_attempt`,
so an attempt at the cap is not flagged again. The lost-lease pass is the only pass that ends expired leases, so in it every outcome other than a withdrawal ends the
attempt with `expire_attempt` in the same transaction, as today; it never leaves an expired lease for a later pass.

| Under the row lock | Outcome |
|---|---|
| Provisional, and its expiry or its attempt deadline passed by `clock_timestamp()`, both requirements recorded, under the cap | The lost-lease pass withdraws it (Architecture decision 0098 decision 3; root in the whole-run transaction, scoped in the path transaction, as its build does): `step.lease_withdrawn` with `cause = NOT_ACKNOWLEDGED`, the lease and its marks cleared, `withdrawn_lease_token_hash` kept, the step `READY`, the attempt set back by one, the generation kept; the settle runs, so a step the planner stops is cancelled at once (a compensation or reconciliation is kept and leased again) and a stopping collection or loop drains the instance as one never leased |
| The same, at the cap | `expire_attempt` in the same transaction: an unknown outcome, consultation or review; `worker_leases_undelivered_total{cause="NOT_ACKNOWLEDGED", outcome="capped"}`. The pass has only silence, four times over; a hand-back at the cap is the host's word and is an ordinary failure |
| The same, a requirement not recorded | `expire_attempt` in the same transaction, counted `lapsed` |
| `LEASED`, provisional or acknowledged, no capability served, a hand-back under the reserved code, whether or not a deadline passed, both requirements recorded, under the cap | The withdrawal of Architecture decision 0098, cause `NOT_STARTED`, with `provisional_withdrawn_attempt` set to the withdrawn lease's attempt when it was flagged at its claim, and `withdrawn_lease_token_hash` kept; under a stop the settle cancels a `FORWARD` step |
| The same, at the cap or a requirement not recorded | An ordinary retryable failure, the attempt spent (Architecture decision 0098 decision 7), counted `capped` or `lapsed` |
| Acknowledged, and expired or timed out | `expire_attempt`, as today: an unknown outcome, consultation or review; `worker_leases_expired_total{mark="acknowledged"}` |
| A mark that does not count (another incarnation, none, or claimed before the requirement's current recording) | An unmarked lease under Architecture decision 0098's rules, its switch included, except that its hand-back is an ordinary failure; at a deadline `expire_attempt`, counted `worker_leases_expired_total{mark="recovered"}` |
| `RUNNING` with an unacknowledged mark (a host that heartbeated first) | `expire_attempt`, as today; the heartbeat was counted as a violation |
| `RUNNING` (heartbeated), or a capability served, handed back | An ordinary retryable failure, the attempt spent; after a capability, a violation |
| A 1.8 lease that is not marked (`lease_hand_back_generation` only), handed back | Withdrawn as Architecture decision 0098 decides, only while `delivery-acknowledgement-v1` is recorded and for a lease claimed after its recording on the current incarnation; otherwise an ordinary failure |
| A marked lease the server never handed to the transport (`NOT_SENT`) | Withdrawn as Architecture decision 0098 decides, needing only `lease-withdrawal-v1`: its token never left the server |
| Not marked, below 1.8 | As today, and as Architecture decision 0098 decides |
| Anything else (acknowledged since the candidate was read, replaced) | Nothing; `fence_missed` (the lease is no longer this one) |

The candidate query of the pass returns the marks only as a hint; the decision is the re-read's. The engine's
`SAGAT_LEASE_WITHDRAWAL` is not consulted for a lease whose mark counts.

**Exactly one of the acknowledgement and the pass's withdrawal commits for a lease.** The acknowledgement writes a row only while it
holds the row's lock and found it provisional with the request's fence and at least one receive bound before its deadlines; the pass
takes the same row lock and withdraws only a lease still provisional past a deadline. If the acknowledgement holds the row first, the
pass reads `lease_acknowledged_at` set and does nothing; if the pass holds it first, the acknowledgement skips the row (`RETRY`) and,
asked again, finds the token cleared (`NOT_HELD`). An acknowledgement repeated while its first attempt is still committing skips the
row too and is answered `ACKNOWLEDGED` on the next ask. The host runs a handler only on `ACKNOWLEDGED`. So a handler of a flagged
lease runs only after the server committed that its host holds it, and a lease the pass withdraws was not run by a host that keeps
the protocol (the trust assumption, Decision 006-053), on a database that keeps what it committed (the incarnation and the durable
commit, Decision 006-062). A hand-back is different: it can follow an acknowledgement and still withdraw a lease that is `LEASED`
with no capability served, and its "nothing ran" rests on the host's promise and the capability record, not on the row lock; a later
call with its token is counted as evidence against it. The deadline decides when the pass looks, never whether both can happen; the
losing side of the race between the deadline (a timeout) and the acknowledgement (the host's signal) is the journal's withdrawal
event or the counted `NOT_HELD` (constitution principle I).

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
   the update, the commit and the answer's transit (together `p`, up to about 4 s since the call ends by 4 s); the host sends the first
   beat at the earlier of its interval and a third of (`lease_time_left_ms` less the time its attempt took on its monotonic clock,
   which covers `p`), whatever interval it is configured with, and before the handler starts when that time is under 3 s. With `T`
   left at the sample (at least 6 s) and `T − a` at least 3 s at receipt, the beat leaves at most (`T − a`) / 3 after the answer and
   has at least two thirds of `T − a`, 2 s, to commit, which the gate's heartbeat bound (2 s at the 99th percentile) fits; under 3 s
   the heartbeat is sent before anything runs and the lease is handed back if it fails (item 3 of "The host at 1.8"). Before the attempt deadline: a heartbeat does not move it, so the margin is what
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
   which bounds claim to handler entry from above. The gate requires the 64-lease calls within half the receive bound (3 s) at the
   99th percentile over every call, measured to its end, with fewer than 0.1% of calls answered other than `OK` (the deadline, a lock
   or statement timeout and the store unavailable all count; `RETRY` is `OK`), half of the receive bound being left for the network and the host, and the heartbeat's 99th percentile in a burst under
   a third of it (2 s), which is what the margin of item 3 assumes (the 3 s threshold of item 3 makes them fit: two thirds of 3 s is
   2 s); the rise of ready to
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
   the interval and a third of `lease_time_left_ms` less the attempt's duration, on the monotonic clock. When that time left before
   the expiry is under 3 s, the host sends the first heartbeat before it starts the handler, with the retries of §5 bounded by the
   time left, starts it on that heartbeat's success, hands the lease back if it fails, and drops it on `STALE_ATTEMPT` (§6.4). A
   heartbeat that reaches the server after such a hand-back has withdrawn the lease is counted as a call after a withdrawal of its
   own kind, `heartbeat`, audited but not alerted: a handler that really ran also reports or calls a capability, which are alerted. When `attempt_time_left_ms` less the attempt's duration is under 2 s,
   the host treats the answer as `CANCELLED` (`too_late`) and hands the lease back, because a handler started then would most likely
   time out into an unknown outcome and a heartbeat does not move that deadline. `RETRY`: the leases of a frame answered `RETRY` are
   asked for again together, as part of the same call, after a backoff of their own (200 ms doubling to 1 s, never past the earliest
   of their give-ups), which the user's retry options do not change; `RETRY` does not advance the backoff of §5 and is not a failed
   call. `CANCELLED`: run nothing, hand the lease back. `NOT_HELD`: drop it, run nothing, report nothing, free its slot, emit
   `dropped` with the code `NOT_HELD`.
4. **A call that fails** (a transport failure, `UNAVAILABLE`, `DEADLINE_EXCEEDED`, `CANCELLED`, `UNKNOWN`, `ABORTED`,
   `RESOURCE_EXHAUSTED` after its hint, a retryable `INTERNAL`, an unusable answer): repeat it, without the leases already decided or
   handed back, with the backoff of §5 and no limit on attempts, each attempt bounded to the receive bound; after a failure that
   carries the server's envelope, an attempt with more than eight leases is split in two calls; `UNAUTHENTICATED`: once more with a
   fresh token. A status in neither this list nor item 5's refusals (a second `UNAUTHENTICATED`, a non-retryable `INTERNAL`,
   `OUT_OF_RANGE`, `DATA_LOSS`) is not repeated: its leases wait for their give-up. The host gives a lease up when its time left at
   receipt (its expiry minus the host's clock at receipt) minus the receive bound has passed on its monotonic clock, clamped to
   between one receive bound and 120 s, and never while an attempt is in flight: it waits for that attempt's outcome, at most the
   receive bound, and runs a late `ACKNOWLEDGED`. The give-up counts from the earlier of the expiry and, when the lease carries its
   timeout, the attempt deadline, both read at receipt. The host gives a lease up early when it has waited at least one receive bound
   since its first attempt was sent, at least one attempt ended in that time, every attempt that ended ended with an answer other than
   a shed (a shed is `RESOURCE_EXHAUSTED` with the envelope reason `ACKNOWLEDGEMENT_SHED`; any other status, with the envelope or not,
   counts: a deadline, a lock or statement timeout, a mesh's cut or rate limit), and, after the most recent of those attempts ended,
   another call of the host that reaches the database (a heartbeat, a report, a hand-back) was sent and answered `OK`, or a frame with
   leases was received, with no failed call of the host to the database since; it then starts no further attempt, waits for one in
   flight as the ordinary give-up does (and runs a late `ACKNOWLEDGED`), and otherwise gives the lease up. A `RETRY` restarts the
   window, and the halves of a split call share it. A failover, which fails every call that reaches the database until the promotion,
   therefore waits for the ordinary give-up, and the repeat after the promotion is answered; so does a busy engine's shedding; and so
   does a host whose only work is flagged leases, which has no other call. A lease given up is not run and is handed back; a hand-back
   that lands after its expiry, before the pass, is still a withdrawal, and so is one of a lease that was acknowledged without the
   host learning it, while it is `LEASED`. After a hand-back the attempt is not flagged again (decision 5 of Architecture decision
   0099), so a failing path costs a lease at most one hand-back per attempt and no withdrawal cycling.
5. **Fallback, bounded, on evidence of the host's own path.** A transient answer (a status of item 4) that carries the server's
   error envelope (the `x-sagat-error` metadata that the worker service and its admission put on every status they make) is the
   server's own trouble and never counts toward a decline. The host declines acknowledgement only on: `UNIMPLEMENTED` (a proxy or
   an older server); three calls in a row refused with `PERMISSION_DENIED`, `INVALID_ARGUMENT`, `FAILED_PRECONDITION` or `NOT_FOUND`
   (a closed list, the same in `sdks/SPEC.md` §6.2); or three counted calls in the run since the last `ACKNOWLEDGED`. A call counts
   when it was given up (every lease given up with no verdict), every attempt of it ended with no answer or with a status without
   the envelope (a gateway's 503, a mesh's timeout), not counting an attempt that the transport never started on a stream, and its
   probe failed the same way: once the hand-back of one of its leases has been answered `OK`, or refused stale with the server's
   envelope because the pass won, which shows the server reachable, the host sends one `AcknowledgeTasks` for that lease, one attempt,
   evidence only, which a working path answers `NOT_HELD` (reason `withdrawn`, decided in `matched` by the same primary key and tenant
   when the token is the step's withdrawn one; after a recovered lease's hand-back, an ordinary failure, it is `no_match`); a verdict
   other than `NOT_HELD` or `RETRY` means the hand-back did not land, and the host hands back once more, sends no second probe, and
   does not count the call. A path that swallows this call swallows the probe too; an outage that has healed answers it. Calls are ordered
   by the give-up of the last of their leases; the split halves of one call are one call; a call that does not qualify neither counts nor breaks the run; an
   `ACKNOWLEDGED` that a 2 s rule turned into `CANCELLED` still resets it; a run with no counted call for 10 minutes starts again. On
   fallback the
   host waits a random delay of up to one receive bound, closes its receive, hands back what waits, waits for the answers of its
   hand-backs (their retries of §5), logs once, and registers again at its own minor with `declines_acknowledgement = true`; after 10
   minutes (with a random factor in `[0.8, 1.2]`, doubling for each fallback in a row up to an hour, and back to 10 minutes after an
   `ACKNOWLEDGED`) it registers again without declining. It keeps every feature of its minor; only the flag is declined, and each
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

1. Stop marking: set `SAGAT_LEASE_WITHDRAWAL=off` on every engine and restart them (the switch is read at start). Confirm it: every
   live engine in the registry has started since, `worker_lease_marks_skipped_total{reason="switch_off"}` rises on every engine that
   claims, and `worker_leases_provisional` falls and does not rise again. An engine created later from the old values (an
   autoscaler's template) would keep marking, so change the template too. The marks already written keep their meaning, withdrawn at
   their deadlines and on their hand-backs, whatever the switch. While the switch is off, the `NOT_SENT` and unmarked `NOT_STARTED`
   withdrawals of Architecture decision 0098 stop too, so those leases lapse as before 0098 for the length of the drain.
2. Wait until `worker_leases_provisional` is 0. Provisional leases are the ones whose withdrawal at a deadline would stop when the
   requirement goes; an acknowledged `LEASED` lease's hand-back, like an unmarked 1.8 lease's, becomes an ordinary failure after the
   removal, the safe direction and the documented cost of a removal. Nothing renews a
   provisional lease, so this takes at most the longest earlier deadline among them (a time-to-live or a `timeoutMs`) plus the period
   of the lost-lease pass and its backlog, whatever 1.8 workers keep doing. Then remove `delivery-acknowledgement-v1` (the owner
   function, as `docs/runbooks/instance-registry.md` section 5 describes; remove it before `lease-withdrawal-v1`, if that is removed
   at all). Removing it first would also stop marking, within one registry heartbeat, but would end the protection of the
   provisional leases in flight: they would lapse as today.
3. Roll the binary back, with `SAGAT_LEASE_WITHDRAWAL` set back to its value before the rollback in the same configuration (one
   restart), so the floor build keeps the withdrawals of Architecture decision 0098. The floor is the first released build of
   Architecture decision 0098 that decodes the cause `NOT_ACKNOWLEDGED` (request (2) of Decision 006-059, a hard precondition of Task
   006-057, so no mark and no such withdrawal exists before it), and the registry enforces it: every writer of the cause reads `lease-withdrawal-v1` under the
   lock its removal waits for, which cannot be removed once a journal holds `step.lease_withdrawn` (Architecture decision 0098
   decision 8). On the floor a 1.8 session's hand-back is an ordinary failure, because that build clamps the holder's minor to its
   own ceiling (Decision 006-059 (3)), and this build honours none once the requirement is removed, so a capability served by an
   engine of the floor build can never be followed by a withdrawal. A session at 1.8 meeting a binary below 1.7 is the case of
   design 041; an unflagged lease asks nothing of 1.8.
4. The `down` migration refuses while a provisional mark counts, while the history names the capability, or while a live process
   lists it; acknowledged marks and marks that no longer count are dropped with the columns, which loses nothing, because the floor
   build treats those leases as attempts.

### Failure modes and their exact outcomes

| Case | Outcome |
|---|---|
| A frame or a `PollTasks` answer lost; the host crashes before acknowledging; the engine crashes between the claim and the withdrawal of Architecture decision 0098 | At the earlier deadline the lease is withdrawn (`NOT_ACKNOWLEDGED`): `READY`, attempt not consumed, no review; the next lease has the same attempt number and the next generation |
| The acknowledgement never leaves the host | Repeated; on a verdict the task runs once; if the host gives the lease up first, nothing runs and the hand-back withdraws it (`NOT_STARTED`), also when it lands after the expiry and before the pass |
| The acknowledgement's call outlasts its deadline at the server | Abandoned before its commit, answered `UNAVAILABLE` (`ACKNOWLEDGEMENT_DEADLINE`); the host repeats it |
| A repeated acknowledgement with less than the margin left | `CANCELLED` (`too_late`): the host hands back; while the lease is `LEASED` with a mark that counts, the hand-back is withdrawn, otherwise an ordinary failure; never a handler started on an expiring lease |
| The acknowledgement's answer is lost after its commit | The host repeats; the server answers `ACKNOWLEDGED` again (`duplicate`) and the handler runs once, or `CANCELLED` (`too_late`) if the margin is gone, or the host gives the lease up: its hand-back of the acknowledged, still `LEASED` lease is withdrawn, no attempt spent. Past the expiry the hand-back races the pass, which ends an acknowledged lease with `expire_attempt` (a review for an `Activity`); if the hand-back cannot reach the server either, the lease lapses into a review (Gap 006-041) |
| The first heartbeat, sent before the handler, commits and its answer is lost | The lease is `RUNNING`; the host hands it back, an ordinary failure that spends the attempt (Gap 006-041) |
| A database failover while flagged leases wait for their acknowledgement | Every call that reaches the database fails after the failure, the heartbeats too, so no host gives up early; with a 30 s time-to-live the repeats after a 15 s promotion are answered `ACKNOWLEDGED` (`recovered`) with the margin and the tasks run; a lease whose give-up passes before the promotion (an 18 s time-to-live, a long failover) follows Gap 006-047: its hand-back is an ordinary failure, and so is the hand-back after a `CANCELLED` without the margin, the attempt spent |
| A repeat while the first attempt is still committing | The row is skipped (`RETRY`); the next ask answers `ACKNOWLEDGED` |
| An acknowledgement with less than one receive bound left, or after a deadline, before the pass | `NOT_HELD` (`too_late`), nothing written; the pass withdraws the lease |
| An acknowledgement after a withdrawal, or after the lease was replaced | `NOT_HELD`; the host drops the task; counted apart, never as a call after a withdrawal |
| Acknowledgement and withdrawal at the same moment | Exactly one holds the row; the other finds it changed |
| The host stops, or crashes, after `ACKNOWLEDGED` and before the handler starts | Stopping: it runs the task, as any task in flight. Crashing: the lease lapses into a review (Gap 006-040; the window is in-process and holds no call) |
| A `FORWARD` step is asked to stop, or its run has a stop reason, before the acknowledgement | `CANCELLED`, nothing written; the host hands back; the withdrawal's settle cancels the step; if the hand-back is lost, the withdrawal at the deadline does |
| A compensation or reconciliation of a stopping run | Acknowledged as any other lease and run; never answered `CANCELLED` for the stop |
| A binding revoked after the claim | Ordered with the acknowledgement by the binding's lock: `NOT_HELD` (`unbound`), one audit record; the lease is withdrawn at its deadline and leased to a bound worker |
| A token that does not match (forged, old, another tenant's run) | `NOT_HELD` (`no_match`), no lock taken; at most one audit record per call |
| A row held by another transaction (a busy run, a cancellation in progress) | `RETRY` for that lease alone; the call never waits for a row |
| A capability call on a provisional lease | `FAILED_PRECONDITION`, `TASK_NOT_ACKNOWLEDGED`; counted and audited as a violation |
| A `Heartbeat`, `CompleteStep` or a `FailStep` other than the hand-back on a provisional lease | Served as today; counted and audited once per step and generation as a violation |
| A hand-back after a capability was served | An ordinary retryable failure; a violation. A capability call and a hand-back at once are ordered by the run or path lock: the record commits before the serve, or the hand-back withdraws first and the call finds the lease not held |
| A report, heartbeat or capability call carrying the token of a marked lease withdrawn as `NOT_ACKNOWLEDGED` or on its hand-back | Refused as stale; counted (a heartbeat under its own kind, not alerted), audited once per step and generation (the identical-record rule, a reason from stored values only), alerted: a handler may have run on a lease the journal says never ran (Gap 006-042). A hand-back or an acknowledgement with such a token is counted apart, as evidence that nothing ran |
| A hand-back that lands after its host registered again | Decided from `lease_hand_back_generation`, so still withdrawn |
| The attempt's withdrawals reach the cap of four | The withdrawal that reaches it sets `provisional_withdrawn_attempt`, so the attempt is not flagged again; a fifth withdrawal is refused. A marked lease at the cap exists only when earlier withdrawals were written without the flag (by the floor build): the pass ends it past a deadline with `expire_attempt` (an unknown outcome) and its hand-back is an ordinary failure, counted `capped` |
| `AcknowledgeTasks` refused by a proxy or a rolled-back replica, or swallowed on the host's path | `UNIMPLEMENTED`, three refused calls, or three counted calls (no answer or no server envelope, while another call or a hand-back was answered): the host hands back and registers again declining acknowledgement, at its own minor, for 10 minutes and then again without declining; `SagatWorkersDecliningAcknowledgement` and `SagatAcknowledgementsFailing` fire |
| The acknowledgement failing alone, at the server or on the host's path (a deadline, a lock or statement timeout, a defect, a mesh's cut), while the host's other calls are answered | The host gives each flagged lease up after one receive bound and hands it back (withdrawn, `provisional_withdrawn_attempt` set); its next lease of the attempt runs unflagged: about 6 s of delay, no attempt spent, no host declines on the server's envelope; the second claim and withdrawal fall on the server in trouble; `SagatAcknowledgementsFailing` fires. Sheds are waited out to the ordinary give-up |
| A database outage or a saturated server while acknowledgements are in flight | The other calls fail too: the calls are repeated, leases whose give-up passes are handed back (withdrawn when the hand-back lands, or withdrawn by the pass at the deadline if it cannot); no fallback |
| A flagged lease received by a host that declines | Handed back, never dispatched; it triggers nothing |
| A host clock skewed by more than the time-to-live | Its give-up is clamped on its monotonic clock; its first heartbeat follows `lease_time_left_ms`; it acknowledges as any other host |
| A host configured with a long heartbeat interval | Its first heartbeat of a flagged lease still comes at a third of `lease_time_left_ms` |
| An engine restarts between the claim and the acknowledgement | Nothing of the lease lives in the engine; any engine of the build answers |
| The requirement removed while marked leases exist | No new mark after one registry heartbeat; the provisional leases in flight lose their protection: at a deadline the pass ends them with `expire_attempt` (`lapsed`) and a hand-back is an ordinary failure; both requirements removed in a row while a mark exists: the pass's read under the locks orders it, and nothing is withdrawn after the removal; recorded again later: leases claimed before the new recording do not count |
| An engine of the floor build silent at an activation's census reconnects | It serves no capability call until a beat and its gate's decision succeed; heartbeats and reports flow; its gate then closes |
| `delivery-acknowledgement-v1` recorded by hand without `lease-withdrawal-v1` for both kinds | Nothing is marked (`requirement_missing`); `SagatDeliveryAcknowledgementInactive` fires |
| An engine with `SAGAT_LEASE_WITHDRAWAL=off` | It marks nothing and withdraws no unmarked lease; a marked lease is withdrawn by it as by any engine of the build |
| A lease whose earlier deadline is under 18 s | Not marked (`short_deadline`); today's meaning (Gap 006-047) |
| A failover, a point-in-time recovery, a snapshot or base backup started as a server, a managed restore, or a cut-over to another cluster, after an acknowledgement committed | Another server: every earlier mark stops counting; a repeated acknowledgement answers `ACKNOWLEDGED` (`recovered`) with the margin, or `CANCELLED` without it; at its deadline the lease lapses as today, never withdrawn; its hand-back is an ordinary failure |
| A crash of PostgreSQL that the postmaster recovers without restarting | The acknowledgement and the capability record committed with `synchronous_commit` at least `local`, so they survive; with `fsync` off only the loss of the operating system loses a commit, and that starts another server |
| A logical restore into the same running server, a virtual machine reverted with its memory, a storage snapshot rolled back under a running server | The runbook restarts PostgreSQL before the engines start, which ends every mark; skipped, it is Gap 006-048 |
| A planned restart of PostgreSQL | Another start time: the marks in flight stop counting and keep today's meaning |
| A hand-written client at 1.8 that runs a flagged lease without acknowledging and makes no call during it | Withdrawn and run again with the same attempt number and idempotency key (Gap 006-042); a later call with its token is counted and alerted |

### Observability

- `worker_task_acknowledgements_total{verdict, reason}`; `grpc_requests_total{method="AcknowledgeTasks", code}`;
  `worker_task_acknowledgement_failures_total{reason}` (`shed`, `deadline`, `lock_timeout`, `statement_timeout`, `conflict`, `store`), every
  `UNAVAILABLE` and `RESOURCE_EXHAUSTED` the handler or its admission returns counted under one of them.
- `worker_leases_undelivered_total{cause, outcome}` (the counter of Architecture decision 0098) gains the cause `NOT_ACKNOWLEDGED`
  and two bounded labels added in this build, `mark` (`provisional`, `acknowledged`, `recovered`, `unmarked`) and `stop` (`true`,
  `false`), which leave its sums by cause and outcome unchanged; `worker_leases_expired_total{mark}` uses the same `mark` values;
  `worker_lease_marks_total`, incremented by the claim when it marks, the denominator of the ratios below;
  `worker_lease_marks_skipped_total{reason}`; `worker_leases_expired_total{mark}` (`acknowledged`, `recovered`, `unmarked`), since the
  journal does not record that a lapsed lease had been acknowledged (Gap 006-046).
- `worker_task_delivery_delay_seconds`, from the claim to the acknowledgement on the database clock; and
  `worker_task_acknowledgement_call_seconds`, the server-side duration of one call by its number of leases (`1`, `2-8`, `9-64`).
- Gauges without labels: `worker_leases_marked`, `worker_leases_provisional`, `worker_leases_provisional_overdue`; and
  `worker_sessions{protocol, declines}`, the live sessions by accepted minor and whether they decline acknowledgement, with the
  counter `worker_registrations_declining_total`.
- `worker_capability_calls_refused_total{method, reason="TASK_NOT_ACKNOWLEDGED"}`, `worker_protocol_violations_total{method}`,
  `worker_calls_after_withdrawal_total{kind}` (`report`, `heartbeat`, `capability`, `hand_back`, `acknowledgement`);
  `worker_capability_calls_refused_total{method, reason}` gains `REGISTRY_SILENT`.
- Alerts, with `promtool` cases at the default and at an 18 s time-to-live:
  - `SagatLeasesNotAcknowledged`: withdrawals with the cause `NOT_ACKNOWLEDGED` above 1% of `worker_lease_marks_total` over 15 minutes (frames are
    being lost or hosts are not acknowledging; a threshold chosen as an operating limit, not measured).
  - `SagatAcknowledgementsFailing`: `grpc_requests_total{method="AcknowledgeTasks"}` with a code other than `OK` above 5% of the
    calls over 10 minutes, or withdrawals with the cause `NOT_STARTED`, `mark` `provisional` or `acknowledged` and `stop="false"`, above
    1% of `worker_lease_marks_total` over 15 minutes (hosts are handing back what they could not acknowledge). Its
    runbook symptom names the cost: each flagged lease waits one receive bound (or, during sheds, a failover or on a host with no
    other call, until its ordinary give-up) and is then claimed again unflagged.
  - `SagatLeaseAttemptCapped`: owned here, since Architecture decision 0098 defines none: withdrawals refused at the cap
    (`outcome="capped"`, every cause) above 1% of withdrawals over 15 minutes.
  - `SagatWorkersDecliningAcknowledgement`: any live session declining for 15 minutes while the requirement is recorded, or
    declining registrations above 10% of live sessions over 15 minutes (`worker_registrations_declining_total`), which catches a
    fleet-wide first fallback shorter than 15 minutes.
  - `SagatDeliveryAcknowledgementInactive`: any increase of `worker_lease_marks_skipped_total{reason="requirement_missing"}` (the
    operator believes the protection is on and it is not). The skip reasons are counted in order with `not_activated` first, so this
    one is counted only while `delivery-acknowledgement-v1` is known to be recorded and the rule needs no registry series.
  - `SagatLeaseDeliverySlow`: the 99th percentile of the delivery delay over 10 minutes above twice the baseline Task 006-056
    records, and never above the receive bound.
  - `SagatCallsAfterWithdrawal`: any increase of the `report` or `capability` kind.
  - `SagatCapabilitiesRefusedRegistrySilent`: `REGISTRY_SILENT` refusals for 2 minutes (a registry connection failing, or an owner
    transaction held open on the epoch, which the runbook says never to leave open).
- The call's span links to the trace context of each lease answered `ACKNOWLEDGED` (at most 64), never as a parent. No log line,
  label or attribute carries a token, a payload or an unbounded identifier.
- SDK metrics hooks gain `acknowledgements`, `notHeld`, `givenUp` and `acknowledgementFallbacks` counters.

### Acceptance matrix

| Area | Required evidence |
|---|---|
| Exactness | Barrier tests: the acknowledgement holds the row while the pass's withdrawal waits, the reverse, the same with the hand-back's withdrawal, a repeated acknowledgement while the first commits, an acknowledgement after a deadline and before the pass, a hand-back 1 ms after the expiry with the pass held, an acknowledged `LEASED` lease 1 ms past its expiry with its hand-back and the pass in both orders (exactly one of `step.lease_withdrawn` and `expire_attempt`), a capability call against a hand-back in both orders, the removal of each requirement between the pass's read and its commit, a revocation; root and scoped |
| The pass ends what it does not withdraw | With a requirement removed, and at the cap reached by withdrawals written without the flag, a marked lease past its expiry is ended by one pass with `expire_attempt` and not picked again; root and scoped |
| The bound | With every acknowledgement attempt failing at the server (`ACKNOWLEDGEMENT_DEADLINE`, a statement timeout, a namespace lock held), one lease at an 18 s and at a 30 s time-to-live: its first lease is given up after one receive bound and handed back, its second runs unflagged, no attempt spent; a re-hint of the `READY` step and a run transaction touching it between the withdrawal and the next claim, and a `NOT_SENT` withdrawal of the second lease before a third claim, leave the attempt unflagged; the column holds the attempt `step.lease_withdrawn` carries; four `NOT_SENT` withdrawals leave the fifth lease unflagged; two `LEASED` siblings of one run with equal generations, one withdrawn on its hand-back and the other touched by the same append, the other's flag unchanged; root and scoped |
| The marks | Each of the three lease statements; a lease written with the old column list is not marked; a mark of another incarnation, or none, does not count, and for the withdrawals one claimed before the requirement's current recording; an attempt with `provisional_withdrawn_attempt` set is not marked; the static check of the assignments and its negative tests; the barrier with a run transaction loaded before an acknowledgement |
| Withdrawal | A provisional `Activity` lease past its expiry, and one past its attempt deadline: `step.lease_withdrawn` cause `NOT_ACKNOWLEDGED`, no review, the attempt reused; an acknowledged lease that expires is an unknown outcome; the cap: after four withdrawals written without the flag (through the floor build's statement, or by SQL leaving the column unwritten) a marked lease is ended by the pass with `expire_attempt` at its deadline and its hand-back spends the attempt, counted `capped` |
| Hand-back | From a lease under the reserved code, `LEASED`, with no capability served, claimed after the requirement's recording on the current incarnation, withdrawn, provisional or acknowledged, also after its expiry or with a stop asked, with declared secrets too; a committed acknowledgement whose answer is dropped and whose repeat comes under the margin, with `maxAttempts = 1`: handed back, withdrawn, run on the next lease; from one claimed below 1.8, before the activation, before a re-activation or on another incarnation, from a `RUNNING` lease, or after a capability was served, an ordinary failure; on the floor build, a 1.8 session's hand-back is an ordinary failure; a handler-raised code reaches the server as `WORKER_ERROR`; a `CompleteStep` or `Heartbeat` after the `NOT_STARTED` withdrawal of an acknowledged lease counted and audited as a call after a withdrawal; a recovered lease answered `CANCELLED` and handed back, an ordinary failure |
| Compensations | A cancelled run's flagged compensation, and a reconciliation under a stop, acknowledged, run once and never withdrawn; root and scoped; in the conformance scenario too |
| Capabilities | The descriptor-driven test, root and scoped; a report on a provisional lease is served; a wrong token gets the ordinary stale answer |
| Gate | The capability unlisted, the activation entry absent and the minor at 7 before Task 006-057; no mark while either requirement is not recorded as required (`lease-withdrawal-v1` for both kinds, recorded by hand for engines only among the cases); the command refuses `delivery-acknowledgement-v1` while `lease-withdrawal-v1` is not recorded for both kinds; the removal of either requirement held between a withdrawal's read and its commit, in both orders; a withdrawal path cannot be compiled without the requirements' witness |
| Cost | `LEASE` and both completion lines unchanged; `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` pinned, the former with no run lock, no scan of the run's steps, no journal or outbox insert, fewer statements than the latter; a 64-lease call itemised with its triggers |
| Throughput (gate, before the SDKs and before 1.8 is offered) | The transitions scenario at 1.8 against 1.7, median of five interleaved runs each, network between workers and services: no more than 15% lower; the 64-lease calls within 3 s at the 99th percentile over every call and fewer than 0.1% answered other than `OK`; the pass's withdrawal rate in a burst of provisional expiries (a worker fleet cut off) reported; no heartbeat shed while acknowledgements are admitted in a burst; the heartbeat's 99th percentile in the burst under 2 s; the rise of ready to handler entry and the share of claims left unmarked as `short_deadline` reported |
| Six SDKs | `delivery-acknowledgement` passes in six languages with a reviewed golden; under the override every host scenario but `protocol-negotiation` and those that hand back runs at 1.8 with its existing golden unblessed and acknowledges at least one flagged lease |
| Fallback | An outage of every call, the hand-backs too, that ends 3 s after the give-ups, with three flagged frames in flight, at an 18 s and a 30 s time-to-live, does not fall back (the probes are answered); a 15 s database failover at a 30 s time-to-live with three flagged frames in flight and a running task that heartbeats 20 ms before the failure: no early give-up, no fallback, no review, no attempt spent, the leases run as `recovered`; the same at an 18 s time-to-live: the outcome of Gap 006-047 (an ordinary failure, no review); with a running task heartbeating throughout, two minutes of answers with the server's envelope (a statement timeout) give each lease up after one receive bound, run it on its next lease, and do not fall back; on a host whose only work is flagged leases, the same answers wait for the ordinary give-up; a 10 s burst of sheds is waited out; a proxy that cuts `AcknowledgeTasks` of held leases at 1 s while the server takes 2 s (the probe not delayed), with a running task heartbeating: each lease given up after one receive bound, no fallback, no attempt spent; two minutes of a 503 without the envelope for this call alone falls back at the third counted call (the probes swallowed too), also with `max_inflight` equal to the frame size, at an 18 s time-to-live and with the pass running every second, with at least three flagged step-attempts in the window; a host receiving frames faster than one a second falls back too; a connection with more than 128 calls, and one with 64 transfers in flight, does not fall back; three refused calls fall back, and the host registers again without declining after its period; a flagged frame delivered after the declining registration is handed back |
| Durability | A row whose mark does not count is an unmarked lease for the pass and the hand-back and `recovered` for the acknowledgement (the parity test and a pass in `integration_durable_execution`); with `ALTER ROLE sagat_app SET synchronous_commit = off` the acknowledgement's and the capability record's transactions commit with `local`; the failover drill of `infra/dr/drill.sh` reads `pg_postmaster_start_time()` before and after each disaster, after a snapshot-style restart (the data directory copied and started) and after the restart the runbook asks for, finds it changed, and finds a provisional mark held at the kill not withdrawn after the promotion |
| Released v0.2 | `provisional-lease-cutover` forward and its rollback case (the switch confirmed off, the drain on provisional leases ending while a long acknowledged task runs and a 1.8 worker keeps working, the binary rolled back with the switch set back, the floor then withdrawing an unmarked lease), with a session stored at 8 and a hand-back on the floor, with and without a capability served; a mixed fleet before the activation and after the removal, the stream on an engine of this build and `ResolveSecrets` on an engine of the floor build, the hand-back received by this build: an ordinary failure; an engine of the floor build silent across an activation refusing capability calls, and serving heartbeats, after it reconnects until its gate is decided; the beat's answer held so that the engine's last answer lags the database's `last_seen`, with the activation at the database's 30 s mark: the engine already refuses |
