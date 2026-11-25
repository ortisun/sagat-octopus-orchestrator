# Durable execution semantics

## Architectural choice: state machine + checkpointed steps

V1 uses a **declarative Workflow IR interpreter** and step checkpointing. It does not require deterministic replay of the application's arbitrary stack: the SDKs' DSL is compiled to IR, or SDK workers receive deterministic commands with stable IDs and memoized results. In imperative code-first mode, any code outside steps may re-execute: the SDK MUST forbid/warn about external side effects outside activities. *Note (2026-11-24, constitution 5.0.0):* this paragraph describes the engine as built; release 0.3.0 adds the procedural mode of design 044, which does replay workflow code deterministically against the journal, in the SDK worker and never in a platform process (principle IV).

To avoid promising perfect conversion, we support two modes:

1. **Graph-native:** JSON/IR definition in the Studio or declarative SDK, static and versioned, native execution in the engine.
2. **Code-orchestrated (procedural, release 0.3.0, design 044, not built):** orchestrator function in an SDK with controlled replay and step commands; requires a registered worker and proof of determinism (a determinism study per language and the shared replay conformance scenarios, constitution principle IV, version 5.0.0). Do not assume immediate support for arbitrary control structures.

## Run states

The original conceptual diagram was `CREATED → READY → RUNNING`. `CREATED` and `READY`
are not persisted Run statuses: admission and initial scheduling commit atomically. The complete
persisted status list is in the implementation section below and in
[`RunStatus`](../../crates/domain/src/execution/state.rs); `READY` remains a step status.

Substates: `WAITING_TIMER`, `WAITING_EVENT`, `WAITING_CHILD`, `WAITING_APPROVAL`; `COMPENSATING → COMPENSATED | COMPENSATION_FAILED`.

- SUCCEEDED, FAILED, COMPENSATED, COMPENSATION_FAILED, CANCELLED and TIMED_OUT are terminal **according to the policy**; manual reprocessing opens a new Run or a new compensation segment, never erases history.
- Run timeout may start compensation; the final state depends on the policy, do not assume TIMED_OUT immediately if compensations are active.
- `PAUSED` keeps state/when valid; it is not terminal.

## Step states

`PENDING → READY → LEASED → RUNNING → {SUCCEEDED, WAITING, RETRY_WAIT, FAILED, SKIPPED, CANCELLED}`; `RETRY_WAIT → READY`, `WAITING → READY`.

### Transactional invariants

**Claim:** the Worker API grants READY work through `WorkerService::poll` and the root/scoped claim paths under the owning execution transaction and session/capacity fences. It persists the lease, generation and attempt before returning authority to the worker. The engine writes `task.ready` outbox hints when work becomes ready; a hint does not grant a lease. There is no separate scheduler `TaskAssigned` grant preceding the worker claim. See [worker admission](../../crates/application/src/worker.rs) and [scoped claims](../../crates/application/src/worker/scoped_claims.rs).

**Commit:** `CompleteStepRequest` carries task/run/step identity, attempt, generation, lease token, idempotency key, output fields and the protocol 1.5 execution `instance`. There is no `expected_version` request field. The transaction validates the attempt fence and current state, persists output/digest, journal, step update and following tasks/timers/subscriptions/outbox atomically. An identical repeated report returns its original result. gRPC rejects a stale attempt with `ABORTED` and the `STALE_ATTEMPT` trailer; HTTP status numbers are not the worker RPC contract. See [worker.proto](../../specs/001-platform-foundation/contracts/worker.proto).

**Recovery:** pending outbox is republished. An expired leased Activity records `LEASE_EXPIRED`;
its old token is stale and the uncertain outcome enters reconciliation/review rather than returning
automatically to `READY`. Only known-failed, retry-eligible work receives another attempt.

**Persistence:** append-only journal with a monotonic per-Run sequence; snapshots are an optimization and can be regenerated from the journal of decisions and retained data.

## Step identity and determinism

- Each `step_key` is stable and unique within the scope of the logical path (loop: index/element key; parallel: branch key). Do not reuse a step_key for different semantics within the same version.
- `step_id = stable_hash(version_id, run_id, path, step_key)` (derived logical identity, hash collision detected by unique + full key stored).
- Pure functions may be recomputed; side effects only inside activities/steps with a confirmed checkpoint.
- Timestamp, random, UUID in orchestrator code must come from APIs registered as deterministic commands or persisted inputs.
- Large inputs/outputs go to the object store, the journal keeps `blob_ref`, digest and metadata.

## Fork/join, loops, children, race

- Parallel: `READY` branch across multiple steps, join `all/any/quorum` with policy for losers; bounded concurrency.
- Loop: bounded `foreach` with explicit iteration/fanout limit; path includes `iteration_key` and dedupe.
- Child workflow: configurable propagation of cancellation, deadline and trace; `child_run_id` recorded before the task is emitted.
- Race between wait and timeout: row lock on the subscription; first commit wins, the other becomes a duplicate/audited. Define event_time vs receive_time: by default the **commit order** wins.

## Timers, cron and timezone

- Scheduler uses the database clock and the local monotonic clock only for internal scheduling; TTLs persisted in UTC.
- Cron with IANA timezone, explicit DST policy (`skip`, `once`, `twice`), jitter and misfire (`skip`, `catch_up_one`, `catch_up_all_bounded`). **Built (feature 004):** a trigger's cron expression has five fields and an IANA time zone (UTC by default); the DST policy is that of the `croner` library; a platform that was down fires a missed schedule once, late (`catch_up_one`), and continues from the first time after now; a tick starts its run under a key derived from the tick, so it cannot start twice, and a tick that cannot start is tried again about 30 seconds later under the same key (G84).
- A wait does not keep a process active; a signal performs atomic `subscription WAITING → FIRED` and creates a READY task.

## Payload, semantics and limits

- Data contract JSON Schema, schema per version and `content_type`. Sensitive values with redaction.
- **Proposed** V1 defaults: inline input 256 KiB, inline output 256 KiB, events 256 KiB, history per Run configurable, 10k physical steps per run as an initial guardrail subject to benchmark. Beyond that: blob storage and child workflows.
- TTL and retention configurable; a run waiting for months is viable as long as retention of the supporting data preserves the resume condition.

## Critical transaction pseudocode

```text
BEGIN;
  LOCK owning run/session and resolve the reported execution instance;
  IF already_committed(idempotency_key): return prior_result;
  ASSERT current state permits completion and task/instance/attempt/token/generation match;
  UPSERT output_blob_ref_and_hash;
  UPDATE step -> SUCCEEDED;
  INSERT journal(step.completed, ...) WITH run_seq++;
  REDUCE graph + INSERT next READY steps / subscriptions / timers;
  INSERT outbox(task_ready / run_completed / notify);
COMMIT;
-- async outbox publisher publishes task references to NATS;
-- replay/repair reconciler heals missing notification.
```

## Failure matrix and outcome

| Failure | Expected state | Handling |
|---|---|---|
| Worker dies before a lease commits | READY / unexposed | A later claim is safe because no task authority was returned |
| Worker dies after lease exposure | Unknown result | Fence the stale report; reconcile or review, and retry only after the outcome is known failed |
| Database commits, broker goes down | Transition persists | Outbox republishes |
| Broker delivers duplicate | One valid claim | Deduplicate and fencing |
| Concurrent completes | Only 1 CAS | Other responds 409 or idempotent return |
| Event arrives before the wait | May be lost without correlated inbox | Subscription-first or specific buffer/reconciliation |
| Timeout races with signal | First commit wins | Auditable events |
| S3 storage unavailable | Step does not complete with lost payload | retry upload, hash and referenced metadata |

## As implemented (feature 003): sagas, compensations and reconciliation

Run statuses are `RUNNING`, `WAITING`, `PAUSED`, `CANCELLING`, `COMPENSATING`, `MANUAL_REVIEW` (live) and `SUCCEEDED`, `FAILED`, `CANCELLED`, `TIMED_OUT`, `COMPENSATED`, `PARTIAL`, `COMPENSATION_FAILED` (terminal). The substates `COMPENSATING → COMPENSATED | COMPENSATION_FAILED` of the diagram above are statuses of their own; `PARTIAL` is the third way a saga can end. A step is additionally `SKIPPED` when an operator gave up on a compensation, and can wait on `saga`, `reconciliation` or `operator`.

- **Where decisions are made.** One function, `advance(definition, state, stop)`, says what a saga needs next; the reducer applies the events it returns in the same transaction as the report, timer or command that made them necessary. A worker's permanent failure, a lease that ran out, a cancellation and a restart therefore take the same path, and a run replayed from its journal arrives at the same place.
- **Order.** A saga that stops going forward first winds the body down (steps not yet started are cancelled; attempts in flight are asked to stop or left to finish, by policy; a step waiting for its lookup is left alone, because whether it happened decides what is undone), then records the plan (`compensation.planned`), then compensates, then ends (`saga.compensated`), and only then does the run finish.
- **Crash points.** Each arrow above is a committed transaction. A process that dies before the commit has written nothing (`tests/fault/sagas-compensations/crashes.rs`), one that dies after leaves a state from which `advance` says the next thing, and a duplicate report, a second scheduler or a lease that runs out while the answer arrives is decided once by the run's row lock and the fence (`races.rs`).
- **Timeouts racing signals.** A `waitEvent` inside a saga whose deadline and signal arrive together is decided by the first commit; if the timeout wins and nothing handles it, the saga compensates; if the signal wins it goes on (12 rounds in `races::a_signal_and_a_timeout_arriving_together...`).
- **Not promised.** Exactly-once effects (ADR-0004): the platform asks at least once, with stable keys. The default compensation-failure policy waits for a person's decision; an explicit saga's terminal `fail` policy applies only to known permanent/non-retryable failure after independent work quiesces. See the delivery-1 [saga policies](sagas.md), which are implementation in verification.



## Flow-control gates (feature 022, release 0.3.0)

A definition may put two gates on a run's start (top-level `concurrency`, `rateLimit`) and on a root `activity` or
`script` node (the same two fields on the node): at most `limit` holders of a key at once, and at most `limit` grants of
a key in any interval of `per` (`second`, `minute`, `hour`) on the database clock. Work beyond a gate waits durably in
admission order; it never fails for waiting, except at its queue deadline. This section says where the gates sit in the
execution model; the decisions are in the records it links, and the operator's side is in the
[flow-control runbook](../runbooks/flow-control.md).

Decisions: [ADR-0056](../adr/ADR-0056-concurrency-keys-are-durable-permits-granted-by-the-waiters-shard-owner.md)
(permits are rows; decisions 3 and 5 superseded),
[ADR-0088](../adr/ADR-0088-rate-limits-are-a-durable-window-of-grants-on-the-database-clock-and-the-scheduler-grants-never-the-claim.md)
(the window is rows on the database clock),
[ADR-0089](../adr/ADR-0089-a-gated-step-waits-outside-ready-and-one-fenced-grant-routine-serves-every-waiter-by-key.md)
(a gated step waits outside `READY`; one grant routine and one timeout routine),
[ADR-0094](../adr/ADR-0094-the-readiness-gate-decides-in-an-evaluation-round-before-persist.md) (where the readiness
decision is taken), [ADR-0096](../adr/ADR-0096-a-paused-run-gives-back-its-granted-unleased-steps-and-asks-again-on-resume.md)
(give-back on pause) and [ADR-0093](../adr/ADR-0093-the-flow-control-digest-key-is-rotated-per-key-with-live-rows.md)
(the digest key). Requirements: FR-022-017 to FR-022-022 and FR-022-039 to FR-022-060 in
[`specs/022-flow-control-step-policies/spec.md`](../../specs/022-flow-control-step-policies/spec.md); the transactions
T10 to T30 in its [plan](../../specs/022-flow-control-step-policies/plan.md).

### What a gate is made of

- **A key row** per tenant, namespace, pool (`ORDINARY` or `DRAFT_TEST`), scope (`RUN` or `STEP`), group and keyed
  digest of the key's value: `concurrency_semaphores` for a permit, `rate_limit_windows` for a slot. A key's value is
  never stored; the digest is an HMAC under the deployment's key ring, whose versions in force are a database row read in
  every deciding transaction (ADR-0093).
- **Holder and grant rows** (`concurrency_holders`, `rate_limit_grants`) and **waiter rows** (`concurrency_waiters`,
  `rate_limit_waiters`), changed only under the key row's lock. A run-scope grant counts from its grant instant; a
  step-scope grant counts while it is granted and not leased, then for `per` from its lease instant, so at most `limit`
  attempts of a key are leased in any window (ADR-0088 decision 2, FR-022-040).
- **Journal events** committed with the transition they cause: `concurrency.queued`, `.acquired`, `.released`,
  `.wait_timed_out`, `.dequeued`; `rate_limit.queued`, `.granted`, `.returned`, `.wait_timed_out`, `.dequeued`. A replay
  and `verify_run` reach the persisted state from the journal alone and never read the clock, a window or a queue
  (FR-022-045).

Run and step gates never share a row, and a rate limit never shares a row with a semaphore, whatever the group names
(FR-022-017, FR-022-039).

### Admission: a run's start

Every path that creates a run goes through `PgRunStore::create`, the one function that inserts a gated run
(FR-022-059; `scripts/check-architecture.py` refuses an `INSERT INTO runs` elsewhere, except the continue-as-new
successors, whose definitions may declare no gate). In the admission transaction, after the run's own row (slot 6 of
ADR-0050's admission order), it locks the semaphore (slot 7), then the window (a rate row of slot 8b), then the usage
rows, and either:

- grants every gate the start declares: holder and grant rows, `concurrency.acquired` and `rate_limit.granted`, and the
  run's first tasks; or
- queues it for every gate: waiter rows, `*.queued`, one timer at the earlier of the two deadlines, the run `WAITING`
  with no task and no step `READY`. It counts as an active run and once in the namespace's `queued_runs`.

Both or neither (FR-022-044): a run never holds a permit while it waits for a slot, nor a slot while it waits for a
permit. A synchronous start beyond a full per-key queue, a full `queued_runs` or the live-keys ceiling is refused before
any run exists (`429 QUOTA_EXCEEDED`, reasons `CONCURRENCY_QUEUE_FULL`, `RATE_LIMIT_QUEUE_FULL`, `FLOW_CONTROL_LIMIT`;
rate first when both gates refuse). A start delay decides its gates when the delay is released; a reset takes a fresh
permit and slot or is refused, and never queues (`CONCURRENCY_PERMIT_UNAVAILABLE`, `RATE_LIMIT_SLOT_UNAVAILABLE`). A
key that does not evaluate ends the new run `FAILED`/`EXPRESSION_ERROR` in the same transaction.

### The readiness gate: a gated step waits outside `READY`

A step of a node that declares `concurrency` or `rateLimit` is never `READY` without its grants (ADR-0089 decision 1).
The reducer maps every event that would make it ready (`step.released`, `step.retry_due`, `step.retry_requested`, from a
worker completion, the retry pass, a timer, a signal, an approval, a finished child, an operator's retry, the run's
start or a resume) to `WAITING` with wait kind `CONCURRENCY` or `RATE_LIMIT`. Only the grant event that completes the
step's last declared gate makes it `READY`.

The decision is taken in an evaluation round the planner asks for once it has nothing else to emit
(`Evaluation::Gates`, ADR-0094): the application evaluates the keys over the run's data, and the adapter files and locks
every key the transaction decides or gives back under, once (semaphores, then windows, each sorted by pool, scope, group,
digest), gives back first what the transaction's events release, counts, and returns for each step its grants or its
waits. `persist` writes no task-ready hint for a step left waiting, and refuses a gated step left waiting without a
decision (invariant 11 of `verify_run`). The claim reads no semaphore and no window: a gated step that a worker can see
is already granted, so steps waiting for a full key never fill a poll's window (FR-022-019, SC-022-024). A key that does
not evaluate, or is longer than 256 bytes, fails the step `FLOW_CONTROL_KEY_INVALID` through its failure route without
spending an attempt (FR-022-054).

### One grant routine and one timeout routine

Every waiter, run or step, whatever its gates, is granted by one routine and timed out by one routine (FR-022-052):

- **The grant routine** is a scheduler pass that selects keys, not waiters: keys whose waiters exist and whose
  next-eligible hint has come (a hint kept under the key's lock, which may be early and is never late), on the shards
  the replica holds, shared between tenants by the scheduler's fair selection, a bounded number of keys a pass (the
  plan's ceiling is 256; the build visits the scheduler's batch, 200 by default, decision D022-045) and at most 256
  waiters a key. The pass runs in each tick after the timeout routine and lease recording. Each grant is its own
  transaction under the fence of the key's home shard: the waiter's run (`SKIP LOCKED`), its
  `runs` row, its semaphore, its windows, the usage rows; it checks every gate the waiter declares and grants all or
  none. A grant to a run creates its first tasks; a grant to a step makes it `READY`. A grant does not re-check the
  initiator's permissions: a buffered run is an admitted run (FR-022-060).
- **The timeout routine** runs at the waiter's one deadline (the earlier of its gates') and journals, in one
  transaction, the `wait_timed_out` of the gate that refuses it then (`rate_limit` first when both do) and the
  `dequeued` of the other. A run ends `FAILED` with `CONCURRENCY_WAIT_TIMEOUT` or `RATE_LIMIT_WAIT_TIMEOUT`, without its
  workflow failure handler (it never held what it waited for); a step fails with the same code through its failure
  route, no attempt spent. A grant skips a waiter whose deadline has passed, and a grant racing the timeout or a
  cancellation is decided by the first commit.

A rate slot is freed by time, not by a commit, so the window row's hint is moved to the instant its oldest counted
grant stops counting; a permit is freed by its holder's terminal transaction, which sets the hint to now. Neither the
release nor the timeout ever locks a second run.

### Strict admission order and the reservation rule

Within one key, grants follow enqueue order (the database clock of the enqueuing commit, ties by run and node); there
is no order between keys (FR-022-042).

- **The reservation rule** (ADR-0089 decision 4): on each gate, a waiter at position `p` (the eligible waiters of the key
  ahead of it) is granted only when the key's counted holders or grants plus `p` are below its own limit. Eligible means
  not paused, not past its deadline, of a live run, and admitted by its own limit. So an older waiter refused only by
  its other gate keeps its room, and an older waiter refused by its own lower limit does not block a younger one.
- **Strict admission order** (decision D022-073, research.md): a new arrival, at admission or at a step's readiness, is
  granted at once only when no eligible waiter of the key is ahead of it and the counted holders or grants are below its
  limit; otherwise it queues and the pass grants in admission order. The extra delay this costs a start that the
  reservation rule alone would have granted at once is at most `1.2 I + 2 D` (with `I` the scheduler's idle pause,
  `SAGAT_SCHEDULER_INTERVAL_MS`, 500 ms by default, and `D` one tick), twice that plus one tick when more keys are due
  than one pass visits, and never a window ([plan](../../specs/022-flow-control-step-policies/plan.md), "The grant pass,
  by key").
- Every limit is the lower of the one the waiter row was filed with and the namespace's current ceiling
  (`concurrency_limit`; `rate_limit_per_second`, `_per_minute`, `_per_hour` for its window): a lowered ceiling applies
  to waiters at their next evaluation, a lowered declaration to new runs, and nothing granted is revoked (decisions
  D022-071, D022-074).

### Give-back on pause and on leaving `READY`

A permit is held from its grant until the attempt ends; a granted, unleased step counts as a holder and its slot counts
without end. When such a step leaves `READY` other than by its lease, it gives back what it holds in that commit
(`concurrency.released`, `rate_limit.returned`; transaction T29): a pause of its run, a hold for review, a cancellation,
the run's end (FR-022-046). On a pause the step waits again without a decision and is not asked while its run is paused;
the resume asks again, for the same attempt, at the back of its key's queue (ADR-0096). A leased attempt never gives
back its slot, whatever its outcome. A run's own permit is kept through a pause, and a paused waiter is skipped by
grants until resumed.

### Transactional boundaries

| Transaction | Locks, in order | Commits together |
|---|---|---|
| Admission (T10, T19) | ADR-0050 slots 1 to 6 (the new run), semaphore (slot 7), window (slot 8b), usage | the run, its grants or its waiter rows, the events, the queue timer or the first tasks, the counters |
| Readiness gate (T28) | the path's own order, then semaphores, then windows, sorted, then counts | for each gated step its grants (then `READY`) or its `*.queued` and timer; no task-ready hint for a waiting step |
| Grant (T26) | waiter run (`SKIP LOCKED`), key home-shard fence (share), `runs`, semaphores, windows, usage (run before fence as every pass takes them, decision D022-045; the plan's row names the fence first) | the grants of every declared gate, waiter rows removed, timer cancelled, the first tasks or the step `READY` |
| Timeout (T27) | timer path, run, `runs`, semaphores, windows, counts | one `wait_timed_out` and the other gate's `dequeued`; the run `FAILED` or the step's failure route |
| Release (T8, T14) and give-back (T29) | the path's own order, then the semaphore, then the window | `concurrency.released`, `rate_limit.returned`, rows removed, the key's hint moved |
| Lease recording (T23) | the grant row only, no run | the lease instant copied from the attempt's `step.leased` |

No transaction locks a run after a semaphore or a window, and none locks a window before a semaphore (ADR-0088
decision 4). A lease recorded late only keeps a slot longer: the pass may be late, never early.

## Worker-session claim admission

Unary polling and protocol 1.4 task streams use the same durable claim transaction. Its lock order
is the run row, the shared worker-drain fence, the worker-session row, then the sorted activity
policy/quota locks (per activity: the shared policy fences, the activity's shared fence, and its
exclusive count lock when the effective limit is between 1 and the maximum). The authority check
of an ordinary worker's report shares the namespace's secrets lock, which catalog mutations hold
exclusively, before it locks its membership row; a transaction that shares it never asks for it
exclusively afterwards (`crates/infra-postgres/src/quotas.rs`, `secrets_lock_rule`). The session row is held through commit while the transaction rechecks its
current token hash, tenant, namespace, subject, expiry and `max_inflight`, counts durable active
leases and checks the current activity bindings. Earlier claims in the same transaction consume
capacity too. A prior standalone in-flight count is only an optimization.

Registration replacement and binding revocation coordinate with that same session row. A claim
that read an old session before waiting on a fence cannot lease using that stale snapshot after
replacement commits. The test `session_replacement_revalidates_the_token_and_lowered_limit_at_the_claim_fence`
exercises that ordering; mixed unary/stream and concurrent service-instance tests exercise the cap.
No transaction or ordinary request permit spans an idle receive wait. Response-body cancellation
never immediately requeues a committed lease: expiry/reaping and fencing remain authoritative.
