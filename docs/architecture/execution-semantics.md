# Durable execution semantics

## Architectural choice: state machine + checkpointed steps

V1 uses a **declarative Workflow IR interpreter** and step checkpointing. It does not require deterministic replay of the application's arbitrary stack: the SDKs' DSL is compiled to IR, or SDK workers receive deterministic commands with stable IDs and memoized results. In imperative code-first mode, any code outside steps may re-execute: the SDK MUST forbid/warn about external side effects outside activities. *Note (2026-11-24, constitution 5.0.0):* this paragraph describes the engine as built; release 0.3.0 adds the procedural mode of design 044, which does replay workflow code deterministically against the journal, in the SDK worker and never in a platform process (principle IV).

To avoid promising perfect conversion, we support two modes:

1. **Graph-native:** JSON/IR definition in the Studio or declarative SDK, static and versioned, native execution in the engine.
2. **Workflow as code (release 0.3.0, design 044, being built):** a workflow function in an SDK language, run by the SDK's determinism runtime in the tenant's worker, that decides commands in *workflow tasks* and is replayed against the run's journal; every command that does work is an IR node the engine executes. It requires a worker whose binding lists the workflow type and that registered the run's code digest, and proof of determinism (a determinism study per language and the shared replay conformance scenarios, constitution principle IV). The workflow task, the activation window, the pending rule and the review causes are in [Workflow as code (design 044)](#workflow-as-code-design-044) below.

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

**Persistence:** append-only journal with a monotonic per-Run sequence; snapshots are an optimization and can be regenerated from the journal of decisions and retained data. Since constitution 5.1.0 (principle I) a journal leaves the database only on two audited paths, after a verified archive: feature 020's per-run purge of a terminal, settled run under a namespace retention policy, and design 049's retirement of a month's partitions once every run in them is purged. Neither is built yet.

## Step identity and determinism

- Each `step_key` is stable and unique within the scope of the logical path (loop: index/element key; parallel: branch key). Do not reuse a step_key for different semantics within the same version.
- `step_id = stable_hash(version_id, run_id, path, step_key)` (derived logical identity, hash collision detected by unique + full key stored).
- Pure functions may be recomputed; side effects only inside activities/steps with a confirmed checkpoint.
- Timestamp, random, UUID in orchestrator code must come from APIs registered as deterministic commands or persisted inputs. In code mode (design 044) they need no command: the time is the one recorded in the `workflow_task.started` that opens the activation, and random numbers and identifiers come from the run's seed, delivered in every workflow task (Technical Plan 1.8).
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

## Workflow as code (design 044)

Approved and being built: each paragraph names the task of [design 044](../designs/044-workflow-as-code.md) that
builds it, and nothing here exists on `develop` before that task lands. "Technical Plan" followed by a number is a
section of that design; the model (code version, commands, code definition) is in
[workflow-ir-and-sdk-modes.md](workflow-ir-and-sdk-modes.md#workflow-as-code).

### The workflow task

A code run is an ordinary run (`runs` row, journal, steps) whose start version is a code version. It has one reserved
**workflow-task step** (Technical Plan 2, Task 044-4 and Task 044-8):

- its node identifier is the reserved `~workflow`, which `NodeId::parse_stored` accepts, for which `NodeId::is_derived`
  is false and `NodeId::forward` returns nothing; its `steps.kind` is `WORKFLOW_TASK`; its node is
  `NodeKind::WorkflowTask`, built from the step row and the pin, never an activity node; its activity field holds the
  reserved name `sagat.workflow:<type>:<hex digest>` of the current code version;
- it is `READY` when a workflow task is due, `LEASED` while a worker holds it, `WAITING` with kind `DECIDER` when the
  function is blocked and nothing is pending, `WAITING` with kind `CLOSED` once a terminal command or continue-as-new is
  accepted, and under review (`WAITING` on the operator) when a person must act;
- every other step of a code run is created by a command, with the node `c-<n>`; derived steps keep their suffixes
  (`c-12~c`). After a waking event the planner makes the workflow-task step `READY` instead of following graph edges,
  and it never ends a code run by itself except for the run's deadline, the cancellation deadline, a cancellation
  without grace, an exhausted closing window or the cancellation of a run under review.

One lease of that step is a **workflow task**. It is offered only to a session that registered the run's workflow type
and current code digest and whose identity's binding lists the type (Requirement 044-002). Its lease differs from an
activity's because a workflow task has no external effect: its commands act only when the server commits them
(Technical Plan 4.1):

1. **No heartbeat.** Its time-to-live is the version's `workflowTaskTimeoutMs` (default 10,000 ms, from 1,000 to 60,000
   ms), counted from the delivery of the last history page the worker reads; reading history extends it, up to
   120,000 ms in all.
2. **Its expiry is a counted failure, never reconciliation or review.** Every lease-expiry path (`command::expire_attempt`,
   `expire_instance`, `expire_path_instance`, the scheduler's `leases` pass and the maintenance passes) treats
   `WORKFLOW_TASK` first and journals `workflow_task.timed_out`; a parity test holds the paths together
   (`apps/execution-engine/tests/workflow_task_expiry_parity.rs`, Task 044-8).
3. **Its token is refused by every activity RPC** (`Heartbeat`, `CompleteStep`, `FailStep`, `ResolveSecrets`,
   `ResolveDatabase`, `AppendStepLogs`). It carries no secret. It reads history with `ReadHistory`, reads only blobs its
   run's history references, never a blob whose carrier schema has a `secret` field, and writes blobs only for command
   values that do not fit the message.
4. **It journals only `workflow_task.started`** (no `step.leased`), with the activation's time, the window start
   `S(k − 1)` and the flags `continue_as_new_suggested` and `history_limit_reached` as they stand at the lease.
5. **The claim's workflow-task arm** checks the session's digest against the run's current one and the namespace's
   pause, and skips what applies only to activities (per-activity quotas, breakers, pool delivery, capacity windows, the
   run-document input); workflow tasks count in the namespace's quota bucket `sagat.workflow` (at most 1,024 leased at
   once). The protocol minor is decided per step, so an activity-only worker on an older minor still leases the run's
   activities.

### The activation window

Signals, timer firings and activity outcomes can commit while a workflow task is leased; they are journaled at once in
their own transactions and belong to the next activation. Let `S(k)` be the journal sequence of the
`workflow_task.started` of the workflow task that produced completion `k` (the starts of tasks that failed or timed out
are skipped). Activation `k` delivers the visible events whose sequence is in the window `(S(k − 1), S(k)]`, in journal
order, with `S(0) = 0`, so activation 1 starts with `run.started`, which carries the input. The function's clock,
`ctx.now()`, is the time recorded in the `workflow_task.started` at `S(k)`, never earlier than the previous one. A live
worker and a replaying worker apply the same rule, so replay sees what the live run saw (Requirement 044-005, Technical
Plan 4.3, Architecture decision 0104; the generator test `activation_window::live_and_replayed_activations_are_equal`
of Task 044-9). A worker whose cache holds the run at exactly `S(k − 1)` applies the new window to its cached state; any
other worker reads the visible history from sequence 1 with `ReadHistory` and replays it.

### Visible and waking events

The domain decides what the function sees and what makes a workflow task due, from the event and its origin
(`run_events.origin`: `completion`, `effect`, `engine`, `external`), by two exhaustive matches with no catch-all arm,
`RunEvent::is_visible()` and `RunEvent::is_waking(origin)` (Technical Plan 4.2, Task 044-4):

- **Visible:** `run.started`, the command records, the outcomes of steps (completed, failed, timed out, cancelled,
  resolved after review), timers fired, waits satisfied or timed out, children finished, approvals of approval commands
  decided or expired, saga outcomes, signals buffered, updates admitted, `run.cancel_requested`, markers,
  `workflow_task.started` and `workflow_task.completed`.
- **Not visible:** lease renewals, the attempts and retries of an activity, hints, review and audit events, the events of
  derived steps (`~c`, `~r`) and of scope steps (`s-<n>`), and the internals of a step's admission (a gate's grant, the
  approval that releases an activity held for approval). The function sees only a step's outcome.
- **Waking:** decided by the event and its origin. An event a completion writes (origin `completion`) as the record or
  the immediate bookkeeping of its own commands, and a workflow-task event, never wakes; a visible event of any other
  origin (`engine`, such as `run.started`, or `external`, such as a signal) wakes, and so does a command's immediate
  effect (a wait satisfied at once by its look-back, the cancellation of a step that was still `READY`), which the
  completion appends with the origin `effect`. A waking event makes the next workflow task due.

The history a worker receives is typed `HistoryEvent` messages. A command record carries its number, kind and target,
never its body, and no event carries an actor subject.

### The pending rule

Pending is derived from the journal, never set by hand (Technical Plan 4.3, Task 044-9). Every journal write of every run
goes through one typed append function of `crates/infra-postgres`, `append_events` (Task 044-31). The run row keeps two
journal sequences:

- `code_last_waking_seq`, moved by `append_events`, in the same statement, whenever it appends a waking event of a code
  run;
- `code_last_completed_start_seq`, the `S` of the last completion.

A workflow task is **pending exactly when `code_last_waking_seq` is greater than `code_last_completed_start_seq`**. The
start transaction sets the second to 0 and the first to the sequence of `run.started`, so the first workflow task is
pending by the rule. Hence:

- a transaction that appends a waking event makes the workflow-task step `READY` (with its outbox hint) only if it was
  `WAITING` with kind `DECIDER`, in that same transaction; a step under review or `CLOSED` is never woken;
- a completion decides the step's state after appending its own events: `CLOSED` after a terminal command or an accepted
  continue-as-new, `READY` if pending, `WAITING` with kind `DECIDER` otherwise;
- a failed or timed-out workflow task does not move `code_last_completed_start_seq`, so the activation it did not
  answer stays pending, and a run retried or moved off a review is made `READY`;
- the reconciler checks each `DECIDER` wait after 60 s (and every 3,600 s while it lasts) by reading the run's journal
  backwards to `code_last_completed_start_seq`, filtered by origin and by `RunEvent::waking_kinds()`, and wakes the run
  if it finds a waking event (`workflow_task_pending_repaired_total`); it never touches a step under review or `CLOSED`.
  A test holds that list equal to `is_waking` (`the_reconcilers_waking_kinds_equal_is_waking`, Task 044-9);
- the reducer has rules for the workflow-task step built from the `workflow_task.*` events and `is_waking`, so a replay
  of the journal equals the stored step, and `verify_run` compares every code column and the overlay rows with a replay
  of the journal alone.

### The completion, the command chain and deferred terminal commands

`CompleteWorkflowTask` commits, in one transaction with the chain lock, the run row and the workflow-task step locked and
the lease fenced: the command records and the other journal events (through `append_events`), the payloads of command
inputs, the overlay rows, the steps, timers and waits the commands create, the projections they change, the outbox
records and the next state of the workflow-task step (Requirement 044-004, Technical Plan 5.2, Task 044-9). A
completion whose `applied_through_seq` is below `S(k)` (the SDK did not apply the whole window) puts the run under review
with `NONDETERMINISM_RISK`. A completion that arrives while the run's stop drain runs, or whose transaction reads a time
at or after the cancellation deadline, is refused as stale.

**The command chain.** The server keeps per code run `code_command_chain`, a SHA-256 chain over the number, kind, target
and body digest of every accepted command (Technical Plan 6.2, Architecture decision 0104). The history never carries a
command body, so a worker can produce the chain only by producing the same commands, their bodies included; a
completion whose replayed chain differs puts the run under review with `CHAIN_MISMATCH` (Requirement 044-007).

**Deferral of terminal commands.** A command that ends the run (complete, fail, acknowledge cancellation) or continues
it as new is deferred, without counting a failure, while a saga scope of the run aborts, a step is under review or has
an unknown outcome, or, for continue-as-new, an activity, child or approval is open (cause `OUTSTANDING_OBLIGATIONS`), and
while a waking event committed after the activation's start is unseen (cause `UNHANDLED_EVENTS`) (Requirement
044-031, Technical Plan 4.4). Then:

- the completion's other commands commit; the deferral is recorded in its `workflow_task.completed` with the deferred
  command's kind, target and body digest (server-side fields, never returned); the command takes no command number;
- `CompleteWorkflowTask` answers OK with the outcome `ACCEPTED_DEFERRED` and its cause;
- the SDK re-emits the command last in each later activation, live or replayed; a re-emission whose kind, target or body
  digest differs from the deferral record puts the run under review with `CHAIN_MISMATCH`;
- after 3 deferrals for unseen events the run refuses new signals and updates (`409 RUN_CLOSING`) until a completion
  commits without one.

A terminal command while a saga scope is open and not aborting puts the run under review with `COMMAND_INVALID`, because
only the function can close the scope. Constitution 5.1.0 (Owner decision 2026-11-25-3) words principle IV for this: the
command that ends the run is deferred and the rest of the completion commits all or nothing (Gap 044-024). An accepted
terminal command closes the workflow-task step and puts the run into the engine's stop drain, which writes `run.finished`
with the function's outcome once no leased work is left.

### Review causes

A run goes under review by the step review of feature 003: the workflow-task step gets `step.needs_review` with the cause
as its code, no waking event and no reconciler pass releases it, the review is journaled and audited, and
`workflow_task_reviews_total{cause}` counts it. The run's other steps, timers and waits go on and their events wait for
the function. The retry is the existing review retry (`ReviewAction::Retry`, `run:operate`); skipping or resolving the
workflow-task step is refused, and a manual hold is refused on `~workflow`. The count of consecutive failures is
`runs.code_consecutive_failures`, reset by every committed completion, a retry and a move (Requirement 044-008,
Technical Plan 6.3; causes by Task 044-11, remedies by Task 044-12, moves by Task 044-13).

| Cause | Raised by | Behaviour | Remedies |
|---|---|---|---|
| `NONDETERMINISM` (a replayed command differs from its record, or a patch marker does not match) | SDK | Under review at once | Move, reset, cancel |
| `CHAIN_MISMATCH` | Server | Under review at once | Retry, move, reset, cancel |
| `NONDETERMINISM_RISK` (the C# scheduler, Java thread or Rust runtime check; a completion below `S(k)`) | SDK, or the server for a completion below `S(k)` | Under review at once | Retry, move, reset, cancel |
| `COMMAND_OUTSIDE_ENVELOPE` | Server | Under review at once | Move, reset, cancel |
| `COMMAND_INVALID` (a fragment outside its profile, a malformed command, a terminal command with an open scope, a command on a closed scope, a blob of another run) | Server | Under review at once (a refusal that depends on mutable state also offers a retry) | Move, reset, cancel |
| `VALIDATOR_COMMAND` (an update validator issued a command) | SDK | Under review at once | Move, reset, cancel |
| `WORKFLOW_TASK_FAILED` (an uncaught exception that is not the SDK's workflow failure) | SDK | The step is `READY` again after 1, 2, 4 and 8 s; the fifth consecutive failure puts the run under review | Retry, move, reset, cancel |
| `ACTIVATION_DEADLINE`, a workflow task timeout, `COMMAND_VALUE_RELEASED`, `COMMAND_VALUE_LIMIT`, `COMMAND_LIMIT` (more than 2,048 steps that can still act), `STORAGE_QUOTA_EXCEEDED` | SDK (the deadline) or server | Counted with `WORKFLOW_TASK_FAILED`, with the same backoff | Retry, move, reset, cancel |
| `OUTSTANDING_OBLIGATIONS`, `UNHANDLED_EVENTS` | Server | Not counted: the terminal command is deferred | None needed |
| `CLOSING_WINDOW_COMMAND` (a command the closing window does not admit) | Server | Under review at once | Move, cancel |
| `HISTORY_LIMIT` (a visible count of 51,200) | Server | Under review at once | Retry opens the closing window; cancel |
| `RETAINED_BYTES_LIMIT` (a completion over the run's 4 MiB of retained bytes) | Server | Under review at once | Move, reset, cancel |

A cause raised at once is not retried automatically, because the same code would raise it again; `CHAIN_MISMATCH` and
`NONDETERMINISM_RISK` can come from one faulty worker, so a person may retry them. **The rule that a stop or a deadline
keeps a pending review does not hold the workflow-task step's review:** the run's deadline, a cancellation without
grace and the cancellation of a run under review end it as they end any run, applying each open saga scope's cancel
policy. A reset of a code run (feature 018) offers points only where feature 018 does, at the end of a completion's
journal block with no attempt in flight, no unknown outcome, no saga scope open, aborting or compensating, no approval
or child open and no update pending; the new run keeps the source's seed and current code version.

### The closing window

A code run's history is bounded (Requirement 044-027, Technical Plan 6.3, Task 044-12). From a visible count of 10,240
or 2.5 MiB of retained bytes the function is told that continuing as new is suggested (when the run heads an open
chain); from a visible count of 50,176, 3 MiB of retained bytes, or while a closing window is open, external signals and
updates are refused (`409 HISTORY_HEADROOM`); at 51,200 the run goes under review with `HISTORY_LIMIT`, and the claim
refuses to lease a workflow task once that count is reached by any writer.

The retry of a `HISTORY_LIMIT` review opens the **closing window**: workflow tasks with the flag `historyLimitReached`
for 1,024 + 3 × (the steps that can still act at the retry) more visible events, the bound never above 58,368, journaled
in the `step.retry_requested` that opens it. In the window only commands that end the run, continue it as new, cancel
its outstanding work, close or abort its sagas, record markers, set the state, consume signals, or decide and complete
updates are accepted (`CLOSING_WINDOW_COMMAND` otherwise). A completion is judged against the window's bound, and one
that would cross it is refused and ends the run `FAILED` (`HISTORY_LIMIT`) in its transaction. There is one window per
run: a `HISTORY_LIMIT` review raised inside it has no retry. When another writer uses the window up, a scheduler pass
ends the run `FAILED` (`HISTORY_LIMIT`) through the stop drain.

### Cancellation and moves

A cancellation request to a code run does not stop it: it journals `run.cancel_requested` (visible and waking) and sets
the run's cancellation deadline to now plus the version's grace (default 60 s, 1 s to 3,600 s), and the function answers
with a terminal command or an acknowledgement; a scheduler pass stops the run `CANCELLED` at the deadline, judged by the
database clock (Requirement 044-022, Technical Plan 8.4, Task 044-22). A terminal command still deferred at the deadline
is not an answer. A cancellation without grace (`graceMs: 0`, `run:operate`) and the cancellation of a run under review
stop it at once. Every path that cancels a run goes through one domain function, `execution::request_cancellation`.

A **move** re-pins one open code run to another code version of the same workflow type, on a person's request, in one
transaction per run, only when a worker of the target digest replayed the run's history with the same command chain and
body digests, the workflow task is not leased, and the run has not completed an activation past the one verified;
events committed after it are delivered in the next activation, on the target code (Requirement 044-023, Technical Plan
7.3, Task 044-13; constitution 5.1.0, Owner decision 2026-11-25-3, Gap 044-025). Every step that can still act
(outstanding steps, and the succeeded members of scopes not yet settled, with their derived entries) must fit the
target's envelope, and the target's declarations must fit the run: it declares every signal name with pending signals
and every update not yet completed, accepts the committed state against its state schema, and has gated entries and a
`state.maxBytes` equal to the source's (`ENVELOPE_MISMATCH` otherwise). A run with a stop reason, a `CLOSED`
workflow-task step or under a `HISTORY_LIMIT` review is not moved (`RUN_NOT_MUTABLE`); a leased step is checked again,
then `RUN_BUSY`. The start version and every identity derived from it do not change.

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
