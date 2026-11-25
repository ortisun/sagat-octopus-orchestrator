# Workflow as code

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

From release 0.3.0 a workflow can be a function written in any of the six SDK languages (TypeScript, Go, Python, C#,
Java, Rust) instead of a graph. The function runs in the tenant's own worker. It never touches the outside world
itself: it asks the platform to do things (run an activity, sleep, wait for an event, start a child, ask a person for
an approval, open a saga) and the platform records each request and each result in the run's journal. When the worker
that holds the function's state in memory is gone, another worker rebuilds that state by running the function again
from the start against the journal ("replay"); because the function is deterministic, it makes the same requests in
the same order, and the worker skips the ones already recorded. This is the programming model of Temporal, Cadence and
Azure Durable Functions. The owner decided that it is built in 0.3.0 in all six SDKs, starting with a determinism study
per language before any code, and that the declarative graph stays a supported model with the same guarantees beside
it (owner decision 2026-11-24-3, "Sim, entra na 0.3.0", "Yes, it goes into 0.3.0").

The design adds no second execution engine. The function is a *decider*: in each *workflow task* it receives the events
since its last decision and answers with a list of *commands*. Every command that does work is a node of the existing
workflow intermediate representation (IR): an activity, a timer, a `waitEvent`, a `child`, an `approval` or a `script`
node, in a restricted *code profile* of that node kind, checked by the publication parser of that kind and executed by
the same engine code that runs graph workflows. So a code run gets the leases and fencing, the retries, compensation as
derived steps, the uncertain-effect rules of feature 037, the gates of feature 022, the step logs of feature 041, payload
classification and blobs that the graph engine already has. The workflow task itself is leased like a step, and its
completion commits the journal events, the scheduled steps, the timers, the subscriptions and the outbox in one
transaction.

What the design does that Temporal, as of its Worker Deployment versioning and its SDK replayers, does not
(Technical Plan 17 has the comparison):

- **The code identity is computed, not chosen.** A code version names the SHA-256 digest of the code and its runtime;
  only a worker that runs that digest receives the run's workflow tasks. Temporal pins by a build identifier the
  deployer chooses.
- **Code changes under a running run only after a proof for that run.** A *move* re-pins one run to other code only
  after a worker of that code has replayed the run's own history and produced the same commands, inputs included.
  Temporal's automatic upgrade replays no history before it applies.
- **The server checks determinism too.** It keeps a keyed hash chain of the commands it accepted, inputs included, and
  refuses a completion whose replayed commands do not produce it.
- **A code bug never loses a run.** A nondeterminism or a repeated failure puts the run under review with its cause and
  its state intact, until an operator moves it to fixed code, resets it or terminates it.
- **Compensation and unknown outcomes are the platform's.** A saga opened from code is planned and run by the engine
  with the rules of features 003 and 037, not by `try` and `catch` blocks in each workflow.
- **Queries work without a worker**, from declared state, also after the run has ended.

The decisions, gaps and owner questions are in the register
[specs/044-workflow-as-code/research.md](../../specs/044-workflow-as-code/research.md). The architecture decisions are
[Architecture decision 0103](../adr/ADR-0103-a-code-workflow-is-a-decider-whose-commands-are-ir-nodes-committed-in-the-run-transaction.md)
(the decider and the one transaction),
[Architecture decision 0104](../adr/ADR-0104-replay-follows-the-activation-boundary-and-the-server-holds-the-command-chain.md)
(the activation boundary and the command chain),
[Architecture decision 0105](../adr/ADR-0105-a-code-run-pins-a-code-digest-and-moves-only-after-a-verified-replay.md)
(pinning and verified moves),
[Architecture decision 0106](../adr/ADR-0106-each-sdk-runs-workflow-code-under-a-named-determinism-runtime.md) (the
runtime per language and the deterministic primitives),
[Architecture decision 0107](../adr/ADR-0107-effects-from-code-are-leased-steps-and-sagas-are-planned-by-the-engine.md)
(effects, the code profile of commands and sagas),
[Architecture decision 0108](../adr/ADR-0108-code-queries-are-answered-from-declared-state-and-worker-queries-are-ephemeral.md)
(queries and what they disclose) and
[Architecture decision 0109](../adr/ADR-0109-workflow-tasks-travel-on-the-next-free-worker-protocol-minor.md) (the
protocol).

### Terms

- **Code workflow, code version, code run.** A workflow whose published version is a *code version* (Technical Plan 3),
  and a run of such a version. A *graph* workflow, version and run are what exists today.
- **Start version, current code version.** The version a code run was created with (`runs.workflow_version_id`, never
  changed, and the source of every identity derived from the version), and the code version whose code runs its
  workflow tasks now (`runs.code_version_id`, equal to the start version until a move).
- **Code digest.** The SHA-256 that identifies the code, its dependencies and its runtime for one workflow type
  (Technical Plan 3.2).
- **Capability envelope.** The part of a code version that lists what its commands may use: activities with the secrets
  and output schema of each, scripts, child workflows, named approval policies (Technical Plan 3.1).
- **Binding.** The record of feature 009 that lets one worker identity serve named activities of a namespace
  (`crates/domain/src/binding.rs`); for code workflows it also lists the workflow types the identity may serve, as exact
  entries `workflow:<type>`.
- **Decider, workflow task.** The function is a decider. A workflow task is one lease of the run's reserved step
  `~workflow`, during which a worker runs the function until it is blocked and answers with commands (a *completion*) or
  with a cause (a *failure*). As for every step, the *attempt* counts the leases of the step and the *generation* fences
  each lease; for `~workflow` both increase with every workflow task.
- **Command, command number.** One request of the function. Commands are numbered 1, 2, 3... in the order the function
  makes them over the whole run; the node of a command that creates a step is `c-<n>` where `n` is its command number.
- **Journal sequence.** The per-run sequence of `run_events` (`seq`). Every event has one.
- **Visible event, visible count.** An event the function can observe (Technical Plan 4.2); the visible count is the
  number of visible events of the run.
- **Activation, activation run.** The visible events one workflow task delivers to the function, applied one at a time in
  journal order; the activation run is the period during which the worker runs the function on them. The function
  *yields* when it is blocked waiting for the platform.
- **Replay.** Running the function from the start against the history of a run without issuing the commands that are
  already recorded.
- **Command chain.** The keyed hash over the accepted commands of a run (Technical Plan 6.2), not to be confused with the
  *run chain* of continue-as-new (feature 014).
- **Sticky.** The worker that completed the last workflow task keeps the run's state in memory and is offered the next
  one first.
- **Eager dispatch.** Returning a new lease in the answer to a completion, instead of announcing it for a poll.
- **Under review.** The state of a code run whose `~workflow` step a person must act on (Technical Plan 6.3), using the
  step review of feature 003.
- **Move.** An authorized person's request to change the current code version of one open code run, applied only after
  a verified replay.

### What this design builds on

| Item | Where | What this design uses |
|---|---|---|
| Feature 002, durable execution | [specs/002-durable-execution/spec.md](../../specs/002-durable-execution/spec.md) | Runs, steps, leases, the 4 MiB a run retains, signals, children |
| Feature 003, sagas and compensation | [specs/003-sagas-compensations/spec.md](../../specs/003-sagas-compensations/spec.md) | Derived compensation and reconciliation steps, step review, `PARTIAL` |
| Feature 009, secrets and bindings | [specs/009-security-multitenancy/spec.md](../../specs/009-security-multitenancy/spec.md) and `crates/domain/src/binding.rs` | Bindings, `ResolveSecrets`, the publisher's right to a secret |
| Feature 014, workflow primitives | [specs/014-workflow-primitives/spec.md](../../specs/014-workflow-primitives/spec.md) | Declared state, queries, updates, continue-as-new and the run chain, the tree budget |
| Feature 016, payload classification | [specs/016-payload-classification/spec.md](../../specs/016-payload-classification/spec.md) | Classified fields, redaction and reveal grants on every read |
| Feature 018, schedules, batch, business identifiers, reset | [specs/018-schedules-bulk-operations/spec.md](../../specs/018-schedules-bulk-operations/spec.md) | Batch jobs and items, business identifiers, the with-start calls, reset |
| Feature 019, worker deployments | [specs/019-worker-deployments-dispatch/spec.md](../../specs/019-worker-deployments-dispatch/spec.md) | Routing of activities; the single-run repin shape |
| Feature 020, visibility | [specs/020-visibility-analytics-archival/spec.md](../../specs/020-visibility-analytics-archival/spec.md) | Search attributes, added when it lands |
| Feature 022, flow control | [specs/022-flow-control-step-policies/spec.md](../../specs/022-flow-control-step-policies/spec.md) | Run-level and step-level gates |
| Feature 031, codecs | [specs/031-client-side-encryption/spec.md](../../specs/031-client-side-encryption/spec.md) | Payloads encoded by the client |
| Feature 037, uncertain effects | [specs/037-compensation-safety/spec.md](../../specs/037-compensation-safety/spec.md) | Reconciliation and review of unknown outcomes |
| Feature 041, step logs | [docs/designs/041-step-logs.md](041-step-logs.md) | Step logs of activities; minor 1.7 |
| Design 043, run input validation | [docs/designs/043-run-input-validation.md](043-run-input-validation.md) | The input gate and the platform's schema validator |
| Architecture decisions 0003, 0010, 0018, 0020, 0028, 0031, 0050, 0076, 0082, 0089 | [docs/adr/](../adr/README.md) | The IR, derived steps, publication, shards, payload inspection, result disclosure, many-run operations, uncertain effects, the instance registry, gated steps |
| The worker protocol | [worker.proto](../../specs/001-platform-foundation/contracts/worker.proto), [worker-runtime-protocol.md](../architecture/worker-runtime-protocol.md) | Leases, fences, the minor register |
| The rules for agents | [AGENTS.md](../../AGENTS.md) rule 3, the constitution [docs/constitution.md](../constitution.md) | One transaction per decision; the principles |

## The Problem

The platform offers two ways to write a workflow today, a graph in the Studio and a graph built by an SDK builder, and
both produce the declarative IR that the Rust engine interprets (`docs/architecture/workflow-ir-and-sdk-modes.md`,
table "Three experiences, a single engine"). The third row of that table, "Procedural code-first (later phase)", was
never specified: `docs/architecture/execution-semantics.md` calls it "Code-orchestrated (P2)" and asks for "a registered
worker and proof of determinism"; Architecture decision 0003 says "procedural worker with determinism rules later";
`sdks/SPEC.md` says an SDK "is **not** a workflow runtime. It does not execute workflow code, replay anything, keep a
journal, or decide a transition".

What a graph cannot express, a person writing code expects to write directly: a loop whose body depends on the result
of the last activity, a retry policy that changes with business rules, a dynamic number of parallel branches, a state
machine with many signals, ordinary functions and types shared with the rest of the application, and unit tests in the
language's own test framework. The owner's goal for release 0.3.0 is to surpass Temporal and Windmill (owner decision
2026-11-24-1), which a platform without workflow as code does not do.

Temporal's model leaves these problems to the workflow author or the operator:

- **Nondeterminism found when a run reaches it.** A changed function that replays an open run makes different commands;
  the worker finds out only when it replays that run, and the run's workflow task then fails and retries until a fix is
  deployed. Worker Deployment versioning can pin a run to its build, but a run on automatic upgrade replays the new code
  without any check, and a pinned run's build must be kept running by hand.
- **Patches checked by nothing before they apply.** `patched()` and `GetVersion()` branches are only as correct as the
  author's tests; replayers exist, but nothing on the server runs one before a run takes new code.
- **Sagas by hand.** Compensation is `try` and `catch` code per workflow; an unknown outcome of a payment is the author's
  problem.
- **Queries need a live worker** and fail when none runs.

This repository has the pieces a better answer needs: an append-only journal with a per-run sequence (`run_events`,
`crates/infra-postgres/migrations/20261009000100_execution.up.sql`), one transaction that commits a step completion with
the next work and the outbox (`PgRunTx::persist`, `crates/infra-postgres/src/execution.rs`), a worker protocol with
leases and fences, immutable published versions with a canonical digest (`workflow_versions`), compensation as derived
steps (Architecture decision 0010) with the uncertain-effect rules of feature 037, the step review of feature 003,
declared state with engine-answered queries (feature 014), batch jobs and resets (feature 018) and a conformance suite
common to the SDKs (`tests/contract/sdk-conformance/`). What is missing is a determinism runtime in each SDK, a workflow
task the engine can lease and commit, and the versioning rules that keep code and history together.

## Requirements

### Functional

- **Requirement 044-001** A published workflow version MUST be either a graph version (today's document) or a code
  version (Technical Plan 3), never both. A code version MUST name its workflow type (1 to 48 characters,
  `^[A-Za-z][A-Za-z0-9_-]{0,47}$`), its language, its code digest and its capability envelope, and MUST be immutable like
  every version. *Acceptance:* the publication tests of Task 044-3.
- **Requirement 044-002** A code run's workflow tasks MUST be offered only to a worker session that registered the
  workflow type and the code digest of the run's current code version and whose identity has the exact binding
  `workflow:<type>`; every completion MUST be refused as stale when the session's digest is not the run's current
  digest. *Acceptance:* `workflow_tasks_follow_the_current_digest`, `a_star_binding_does_not_cover_a_workflow_type`,
  `a_completion_from_a_session_of_another_digest_is_stale`.
- **Requirement 044-003** The function MUST be driven by workflow tasks: leases of the reserved step `~workflow`, fenced
  by attempt, generation and lease token, delivering the visible events of the activation window (Technical Plan 4.3),
  and answered by a completion or a failure. A workflow task MUST carry no secret and MUST be refused by every activity
  RPC (`Heartbeat`, `CompleteStep`, `FailStep`, `ResolveSecrets`, `ResolveDatabase`, `AppendStepLogs`).
  *Acceptance:* the contract tests of Task 044-8.
- **Requirement 044-004** A completion MUST commit, in one transaction with the run row and the `~workflow` step locked
  and the lease fenced: the journal events of the completion and of every command, the steps, timers and waits the
  commands create, the projections they change, the outbox records that announce new work, and the scheduling of the
  next workflow task when visible events are pending (`AGENTS.md` rule 3). *Acceptance:* the fault test of Task 044-9
  kills the engine before each statement of the completion and observes either nothing or all of it.
- **Requirement 044-005** Activation k MUST deliver exactly the visible events whose journal sequence is after the
  `workflow_task.started` of the workflow task that produced completion k − 1 and up to the `workflow_task.started` of
  the workflow task that produces completion k (Technical Plan 4.3), to a live worker and to a replaying worker alike.
  *Acceptance:* the property test `activation_boundary_replays_every_interleaving_to_the_same_commands`.
- **Requirement 044-006** Every SDK MUST detect, during replay, a command whose kind or target differs from the recorded
  one at the same command number, a recorded command the function no longer makes, and a patch marker that does not
  match, and MUST report it as a failure with cause `NONDETERMINISM`, the command number, the actual kind and target and
  the code digest, and no payload. *Acceptance:* the conformance scenario `code-nondeterminism`.
- **Requirement 044-007** The server MUST keep, per code run, the command chain of Technical Plan 6.2 over the command
  number, kind, target and keyed digest of the canonical fragment of every accepted command, and MUST refuse a
  completion whose replayed chain differs, with cause `CHAIN_MISMATCH`. *Acceptance:*
  `a_completion_with_a_foreign_command_chain_is_refused_and_puts_the_run_under_review`,
  `a_changed_activity_input_on_replay_is_a_chain_mismatch`.
- **Requirement 044-008** The causes of Technical Plan 6.3 MUST behave as its table says: some put the run under review
  at once, some are retried with backoff and put it under review at the fifth consecutive failure, some are retried
  without being counted. Under review, no workflow task is offered; the run's other steps, timers and waits go on; the
  review is journaled with its cause and audited; and it ends only by one of the remedies of that table or by the run's
  deadline or forced cancellation. *Acceptance:* one integration test per cause and per remedy (Task 044-12).
- **Requirement 044-009** An uncaught exception that is not the SDK's workflow failure type MUST fail the workflow task,
  not the run. A workflow failure MUST end the run `FAILED` with the code and safe message the function gave.
  *Acceptance:* the conformance scenario `code-failures`.
- **Requirement 044-010** The current time MUST be the database time recorded in the `workflow_task.started` that opens
  the activation; random numbers MUST come from xoshiro256** seeded with the run's 32-byte seed; identifiers MUST be
  version 4 UUIDs drawn from that generator (Technical Plan 1.8). The six SDKs MUST produce the same sequence for the
  same seed. A reset MUST keep the seed of its source run. *Acceptance:* the conformance scenario
  `code-deterministic-primitives`, and `a_reset_of_a_code_run_keeps_its_seed`.
- **Requirement 044-011** Each SDK MUST run workflow code under the determinism runtime of Technical Plan 1 for its
  language, MUST refuse to register workflow types when that runtime's precondition fails (Technical Plan 1.11), and MUST
  stop an activation run that does not yield within 2 s with cause `ACTIVATION_DEADLINE`. *Acceptance:* each runtime's
  tests in Task 044-10 and Task 044-15 to Task 044-19, and the scenario `code-activation-deadline`.
- **Requirement 044-012** A code run MUST execute only these commands in 0.3.0: schedule an activity, run a script,
  start and cancel a timer, wait for an event, start a child, request an approval under a named policy, open, close and
  abort a saga scope, request the cancellation of one of its own steps or children, record a marker (patch, side
  effect), set the declared state, accept, reject and complete an update, complete, fail or continue as new, and
  acknowledge a cancellation. Each work command MUST carry a fragment of the code profile of its node kind (Technical
  Plan 5.1); a fragment with a field outside that profile MUST be refused. *Acceptance:*
  `every_command_kind_round_trips_through_its_code_profile` and one refusal test per excluded kind and per excluded
  field.
- **Requirement 044-013** A command MUST stay within the current code version's capability envelope: an activity listed
  by name or prefix, with only the secrets listed for that activity entry (whose use the publisher was allowed at
  publication, feature 009) and the output schema of that entry; a script listed by digest and ready for the tenant and
  namespace; a listed child workflow; a named approval policy. No activity name of a command or of the envelope may start
  with `script.` or `sagat.`. A command outside the envelope MUST put the run under review with cause
  `COMMAND_OUTSIDE_ENVELOPE`. *Acceptance:* `an_activity_command_cannot_carry_a_secret_of_another_activity_entry` and
  one test per envelope member.
- **Requirement 044-014** A code version MAY declare signals. A signal to an open code run MUST be accepted for a
  declared name whether or not the function is waiting, journaled once under the existing idempotency key, charged to the
  run's 4 MiB retained bytes (`429 QUOTA_EXCEEDED` beyond, as for a graph run), and delivered in the next activation. It
  MUST be refused `409 SIGNAL_NOT_DECLARED` for an undeclared name, `429 SIGNAL_BACKLOG` beyond 1,024 signals journaled
  since the last `workflow_task.started`, and `409 HISTORY_HEADROOM` once the visible count reaches 50,176. Graph runs
  keep today's rule. *Acceptance:* the conformance scenario `code-signals`.
- **Requirement 044-015** A code version MAY declare updates, each with its `disclosure` and a closed `resultSchema`
  (Architecture decision 0031). An update MUST be admitted durably with its `updateKey`, validated by the function's
  validator when its admission event is applied, journaled as accepted or rejected, handled by the function (which may
  issue commands), and completed with a result validated against its `resultSchema` or with a failure.
  `POST /runs/{runId}/updates/{name}` MUST answer `200` with the result when the update completes within `waitMs` (at
  most 60,000 ms) and `202` with the status otherwise; `GET /runs/{runId}/updates/{updateKey}` MUST return the status
  to the principal that admitted it or to a principal with `run:operate`. At most 64 admitted updates per run may await
  acceptance (`429 UPDATE_BACKLOG`). Update-with-Start (feature 018) MUST work for a code version. *Acceptance:* the
  conformance scenarios `code-updates` and `code-update-with-start`.
- **Requirement 044-016** A code version MAY declare state and queries in the IR's `state` form (feature 014). The
  function MUST be able to set the declared state by a command; the engine MUST answer the declared queries from the
  committed state with no worker, also after the run ended while its data is retained. *Acceptance:* the conformance
  scenario `code-declared-state`.
- **Requirement 044-017** A code version MAY declare worker queries, each with its `disclosure` and a closed
  `resultSchema`. A worker query MUST be answered by a worker of the run's current digest within the caller's timeout (at
  most 30,000 ms), without a journal event and without changing the run; the answer (at most 256 KiB) MUST be validated
  against its `resultSchema` by the server before it is returned; a handler that issues a command MUST fail the query,
  not the run. *Acceptance:* the conformance scenario `code-worker-queries`.
- **Requirement 044-018** A child started from code MUST be a run of any published version, graph or code, created by
  the existing child machinery with its cancel policy, depth and tree budget; a graph `child` node MUST be able to start
  a code version. *Acceptance:* the conformance scenario `code-graph-mixed`.
- **Requirement 044-019** Continue-as-new from code MUST close the run as continued and start the successor in the same
  transaction, under the run chain rules of feature 014 (quiescence, deadline, segment limit), with a new seed. The
  successor MUST pin the same version unless the command asks for the workflow's active version. *Acceptance:* the
  conformance scenario `code-continue-as-new`.
- **Requirement 044-020** A saga opened from code MUST be a compensation domain of the engine: activities scheduled in it
  with `compensateWith` or `reconcileWith` get the derived steps of Architecture decision 0010; an abort plans their
  compensations in reverse order of completion in run-scoped plan rows; unknown outcomes follow feature 037; the
  function learns only the saga's outcome. *Acceptance:* the conformance scenarios `code-saga` and `code-saga-uncertain`.
- **Requirement 044-021** The function MUST NOT run an effect itself through the platform: there are no local
  activities. A completion MAY ask for the leases of the activities it just scheduled (eager activities) and a sticky
  session's activity completion MAY ask for the run's next workflow task (eager workflow task); an idempotent replay of
  either answer MUST NOT return a lease. *Acceptance:* the eager tests of Task 044-14.
- **Requirement 044-022** A cancellation request MUST reach the function in its next activation; the function MUST
  acknowledge it by completing, failing or acknowledging the cancellation. If it has not done so within the version's
  cancellation grace (default 60 s, from 1 s to 3,600 s), the engine MUST end the run `CANCELLED` itself, applying the
  cancel policies of its outstanding work. The run deadline MUST be enforced by the engine without the function.
  *Acceptance:* the conformance scenario `code-cancellation`.
- **Requirement 044-023** A person with `run:operate` and `version:activate` on the namespace MUST be able to move one
  open code run, and with `run:batch` as well every open run of a version or a list, to another published code version of
  the same workflow in the same namespace that is approved for activation where the namespace requires approval. The
  server MUST re-pin a run only when a worker of the target digest replayed its history with the same command chain,
  its visible count has not changed since, its `~workflow` step is not leased, and its outstanding steps fit the target's
  envelope; otherwise the run stays as it was and the move item records the reason. Every applied move MUST be journaled
  and audited. *Acceptance:* the conformance scenario `code-patch-and-move` and the tests of Task 044-13.
- **Requirement 044-024** Each SDK MUST offer `patched(id)` and `deprecatePatch(id)` with the meaning of Technical Plan
  7.2, recorded as markers. *Acceptance:* `code-patch-and-move`.
- **Requirement 044-025** Each SDK MUST ship a replayer that replays exported histories against the local code and
  reports the first mismatch, usable in a test of the language's own framework. A person with `run:read` and the new
  `history:export` MUST be able to export a code run's history (`GET /runs/{runId}/history`), with classified fields
  redacted by the rules of feature 016 unless the person also holds the reveal rights of feature 016, and never a field
  classified `secret`; every export MUST be audited. *Acceptance:* the replayer test in each SDK and
  `a_history_export_redacts_classified_fields_without_a_reveal_grant`.
- **Requirement 044-026** A move MUST be available as a dry run that records, per run, the verdict of the replay
  without moving anything. *Acceptance:* `a_dry_run_move_records_verdicts_and_moves_nothing`.
- **Requirement 044-027** A code run's history MUST be bounded: from a visible count of 10,240 or 3 MiB of retained
  bytes the function is told that continuing as new is suggested; at a visible count of 51,200 the run goes under review
  with cause `HISTORY_LIMIT`, whose retry opens a closing window of 1,024 visible events in which only commands that end,
  continue or wind down the run are accepted (Technical Plan 6.3). *Acceptance:* `the_history_limit_suggests_then_reviews_then_closes`.
- **Requirement 044-028** A code run MUST be listed, read and drawn like a graph run: the run view, the events, the
  observed graph of its commands, its review and its moves. *Acceptance:* the Studio and REST tests of Task 044-23.
- **Requirement 044-029** The interactions of Technical Plan 12 MUST hold, each tested when its feature is built.
  *Acceptance:* Task 044-27.
- **Requirement 044-030** The first publication of a code version MUST activate the requirement `code-workflows` of the
  instance registry (Architecture decision 0082), so that every instance that does not report the capability closes its
  gate, including one that registers later; an instance without it MUST never open a code run. *Acceptance:*
  `a_late_instance_without_the_code_capability_closes_its_gate`.
- **Requirement 044-031** A terminal command MUST be refused with cause `OUTSTANDING_OBLIGATIONS` while a saga scope of
  the run is open or aborting or a step of the run is under review or has an unknown outcome; for other outstanding work
  it MUST apply the cancel policies of that work in its own transaction. *Acceptance:*
  `a_terminal_command_with_an_open_saga_is_refused`, `a_terminal_command_cancels_outstanding_timers_and_activities`.
- **Requirement 044-032** A code run MUST have at most 2,048 outstanding steps, timers, waits and children at a time
  (cause `COMMAND_LIMIT`, counted), and a namespace at most 64 worker queries, 256 update waits and 16 replay-check tasks
  in flight (`429` with `retry-after-ms`). *Acceptance:* one test per bound.
- **Requirement 044-033** An operator MUST be able to pause the workflow tasks of every code run of a namespace (no
  workflow task offered, runs keep their state) and resume them, audited. *Acceptance:*
  `pausing_code_workflows_stops_workflow_tasks_and_keeps_runs`.

### Non-functional

- **Non-functional requirement 044-001 (replay cost)** Each SDK MUST replay a history of 10,240 visible events in at most
  1 s and one of 51,200 in at most 5 s on the reference worker host, that is at most 97.65625 µs per event
  (1,000,000 µs / 10,240 = 97.65625 µs; 5,000,000 µs / 51,200 = 97.65625 µs). Benchmark criteria on a stated host, not
  promises (constitution, Delivery Constraints).
- **Non-functional requirement 044-002 (commits)** A code workflow that runs one activity and ends MUST cost at most 7
  committed transactions without eager dispatch (the start, the first workflow task lease, its completion, the activity
  lease, the activity completion, the second workflow task lease, the final completion) and at most 5 with both eager
  paths and a sticky worker (the activity lease and the second workflow task lease travel in answers). A graph workflow
  with one activity costs 3, so the ratios are 7 / 3 ≈ 2.33 and 5 / 3 ≈ 1.67.
- **Non-functional requirement 044-003 (statements)** An ordinary workflow task completion MUST run at most 10 SQL
  statements whatever the number of its commands, up to 256: 7 always (Technical Plan 5.2) and at most one each for saga
  rows, declared state and update receipts. A terminal or continue-as-new completion MUST add exactly the statements of
  the existing graph terminal or handoff path.
- **Non-functional requirement 044-004 (latency)** With a sticky worker and a history of at most 1,024 visible events,
  the time from the commit of a signal to the commit of the workflow task that handles it MUST be at most 50 ms at the
  99th percentile on the reference environment; at most 250 ms when no sticky session is alive; at most 1,250 ms when a
  live sticky session is busy (its 1,000 ms preference, then 250 ms). Benchmark criteria.
- **Non-functional requirement 044-005 (memory)** The sticky cache of a worker MUST hold at most 256 runs by default. An
  idle cached run MUST cost at most 1 MiB in TypeScript, Python, C# and Java and at most 256 KiB in Go and Rust, so a full
  default cache costs at most 256 MiB (256 × 1 MiB) or 64 MiB (256 × 256 KiB). Benchmark criteria; Gap 044-001 (memory
  not measured).
- **Non-functional requirement 044-006 (bounds)** A completion carries at most 256 commands and at most 4 MiB of inline
  command payload; a history page at most 1,024 events or 4 MiB, so a history at the hard limit is at most
  51,200 / 1,024 = 50 pages; a replay-check task at most 16 runs, each read by pages under its own capability; a
  registration at most 64 workflow types; a worker query answer at most 256 KiB.
- **Non-functional requirement 044-007 (security)** No workflow code runs in a platform process. A workflow task holds no
  secret. Another tenant's run, history, update or query is indistinguishable from a missing one. No payload, secret or
  worker-supplied free text appears in a review cause, a nondeterminism report, a metric or a log.

### Success criteria

- **Success criterion 044-001** The conformance scenarios of the Verification Plan pass in all six SDKs with reviewed
  goldens and no selection filter.
- **Success criterion 044-002** No code run is lost or silently changed by a deploy: the fault suite deploys a changed
  function while 100 runs are open and observes no review caused by nondeterminism without a move; with a move, the runs
  whose replay does not match stay on their version.
- **Success criterion 044-003** The benchmark of owner decision 2026-11-24-1 reports the replay cost, the commit count,
  the statement count and the latency of Non-functional requirements 044-001 to 044-004 against Temporal on the same
  host, with its environment and failures stated.

### User stories

- **User story 044-01** As a developer, I write a workflow as a function in my language, test it with my test framework,
  publish it and run it (journey 1).
- **User story 044-02** As a developer, I change my function while runs are open, and the platform proves which runs can
  take the change before it applies it (journeys 2 and 3).
- **User story 044-03** As an operator, I see a run under review, why, and I fix it without losing it (journey 4).
- **User story 044-04** As a developer, I write a saga in code and the platform compensates it, including unknown
  outcomes (journey 5).
- **User story 044-05** As a client, I signal, update and query a code run as I do a graph run (journey 6).
- **User story 044-06** As a developer, I ask a long loop how far it has got without making it publish its progress into
  the journal at every iteration (journey 7).

## Goals

- Workflow as code in the six SDKs with one semantic contract and one conformance suite.
- No second execution engine: every effect is a node the engine already executes.
- Versioning that cannot change the code under a running run without a proof for that run.
- The guarantees of the graph mode (one transaction per decision, leases and fences, compensation, uncertain effects,
  tenant boundary, audit) for the code mode, unchanged.
- Bounded and measured costs, compared with Temporal by the benchmark of owner decision 2026-11-24-1.

## Non-Goals

- Running workflow code in a platform process, or sandboxing it as a security boundary. The function runs in the
  tenant's worker, like an activity; the determinism runtimes of Technical Plan 1 isolate for determinism, not for
  security.
- Converting code to a graph or a graph to code. The Studio draws the commands a code run made, not a blueprint
  (constitution principle IV: "imperative code that cannot be converted is not promised a round trip").
- Local activities (effects run inside a workflow task). Technical Plan 10.3 says why.
- In 0.3.0: the `database`, `icebergWrite`, `forEach`, `while`, `repeatUntil` and `parallel` nodes as commands (Gap
  044-003, a graph child covers them), signalling or cancelling another run from code (Gap 044-004), an automatic move
  policy (Gap 044-006), search attributes before feature 020 lands, and inline graph fragments (Gap 044-011).
- Exactly-once external effects. Activities from code are at-least-once, as every activity.

## Open Questions

None open. Every product and design question of this design was decided by the coordinating session under owner
decision 2026-11-20-6, each recorded in the register as delegated, with its reasoning (Owner question 044-001 to Owner
question 044-014). The gaps that stay open are registered (Gap 044-001 to Gap 044-016).

## Owner decisions needed

None. The design rests on four owner decisions of 2026-11-24, recorded in `docs/owner-decisions.md` by the pull requests
that carry them, which reach `develop` before this design's pull request: owner decision 2026-11-24-1 (the full-parity
scope of release 0.3.0), 2026-11-24-2 (constitution 5.0.0 admits a procedural code-first mode), 2026-11-24-3 (workflow
as code in 0.3.0) and 2026-11-24-4 (the coordinating session accepts this design's architecture decisions, settles its
review findings and approves it). The questions of the design are delegated under owner decision 2026-11-20-6.

## Critical User Journeys (CUJs)

### Journey 1: The first code workflow (User story 044-01)

*   **Given**: a TypeScript developer with a worker that serves the activities `inventory.reserve` and `payments.charge`.
*   **When**: she writes `async function order(ctx, input)` that reserves, charges, sleeps one day and sends a reminder
    unless a `paid` signal arrived; runs `sagat-workflows bundle` (which writes the bundle and its manifest with the
    digest); publishes the code version with `client.definitions.publishCode(manifest, envelope)`; starts her worker with
    the bundle; and starts a run.
*   **Then**: the run view shows the commands as nodes (two activities, a timer, a wait), the journal shows each workflow
    task, and her unit test, which runs the function in the SDK's test environment and replays the run's exported
    history, passes.

### Journey 2: A deploy while runs are open, in a compiled language (User story 044-02)

*   **Given**: a Go team whose workflow code lives in its own worker binary (the recommended topology for compiled
    languages, Technical Plan 3.2), with 3,000 open runs of code version 7 (digest A).
*   **When**: the pipeline builds the new binary (digest B), runs `myworker --sagat-manifest` to print its manifest,
    publishes it as version 8 and promotes it, deploys the digest B workers beside the digest A ones, and requests
    `sagat workflow move --from 7 --to 8 --all-open` as an explicit request of the pipeline's identity, which holds
    `run:operate`, `version:activate` and `run:batch`.
*   **Then**: new runs pin version 8; each open run whose replay on digest B matches is moved; the version list shows the
    runs still on version 7 and why, so the digest A workers are stopped only when that number is 0. If the change was
    only in an activity outside the workflow binary, digest B equals digest A and nothing needs to move.

### Journey 3: Moving long runs to fixed code (User story 044-02)

*   **Given**: the runs of journey 2 last months, and version 8 wraps its change in `patched('reminder-v2')`.
*   **When**: the operator first requests a dry-run move, then the move.
*   **Then**: the dry run lists, per run, the verdict; the move re-pins the 2,990 runs that match, each with a journal
    event and an audit record; the 10 that took a branch the new code changed without a patch stay on version 7, and the
    move job lists them with the command number of the first difference.

### Journey 4: A run under review (User story 044-03)

*   **Given**: a Go worker whose function divides by zero for one input.
*   **When**: the workflow task panics five times in a row.
*   **Then**: the run is under review with cause `WORKFLOW_TASK_FAILED`, the Studio shows the cause, the panic's type name
    and the command number, `workflow_task_reviews_total{cause}` increases and an alert fires; the developer publishes a
    fix as a new code version and moves the run, which resumes where it stopped. Nothing it had done is lost or repeated.

### Journey 5: A payment saga with an unknown outcome (User story 044-04)

*   **Given**: a code saga that reserves stock and charges a card, each with `compensateWith`, and the charge declares
    `reconcileWith: 'payments.lookup'`.
*   **When**: the charge times out after the request was sent (`OUTCOME_UNKNOWN`).
*   **Then**: the engine runs the lookup; the provider says `applied`; the charge completes with the lookup's output; the
    function continues. Had the lookup answered `unknown`, the step would wait for review and the function would be
    waiting on the charge, with nothing in its code to handle the case.

### Journey 6: Signals, updates and declared queries (User story 044-05)

*   **Given**: a code run of an approval process that declares the signal `cancelRequest`, the update `addLine` and the
    declared state `{lines, total}` with the query `total`.
*   **When**: a client signals before the function waits, sends `addLine` with `waitMs: 5000`, and queries `total` while
    every worker is stopped.
*   **Then**: the signal is delivered at the function's next activation; the update answers `200` with its result when a
    worker is up, `202 ACCEPTED` when the handler waits on an activity longer than 5 s; the query answers from the
    committed state without a worker.

### Journey 7: How far has the loop got (User story 044-06)

*   **Given**: a code run that reconciles 50,000 ledger lines in batches, keeping its counters in memory.
*   **When**: an operator sends the worker query `progress`.
*   **Then**: the sticky worker answers from memory within the timeout, the answer is checked against the declared
    `resultSchema`, nothing is journaled, and the loop has not had to publish its counters 50,000 times.

## Technical Plan (Architecture)

### 1. The determinism study

The function must produce the same commands in the same order every time it is replayed against the same history.
Everything it can observe besides the history must therefore be either supplied by the history (results, signals,
time) or deterministic by construction (its own code, the SDK's primitives). For each language the study says how the
SDK intercepts each source of nondeterminism (runtime), which hazards remain, what isolation exists, how a breach is
detected, the precondition, and the verdict. The claims marked *spike* were settled by throw-away programs run on
2026-11-24 on the development machine; their exact results and versions are recorded in Decision 044-041 of the
register, and each is reproduced as a red test of the runtime's task. They choose between designs; they are not
benchmarks.

#### 1.1 The sources of nondeterminism, for every language

| Source | What the function sees | How every SDK supplies it |
|---|---|---|
| Results of effects | Activity, child, approval and wait outcomes | From the history, applied one event at a time in journal order |
| Time | "Now" | The database time of the activation's `workflow_task.started`, constant during the activation run |
| Time zone and locale | Formatting of dates, numbers, text | UTC and the root (invariant) locale, by the runtime (each section below) |
| Sleep | A delay | A durable timer command |
| Randomness, identifiers | Random numbers, UUIDs | A generator seeded by the run (section 1.8) |
| Concurrency | Several branches of the function | A deterministic scheduler that runs one branch at a time in a fixed order |
| Unordered collections | Iteration order of hash sets and maps | Made deterministic (Python with a fixed hash seed), forbidden where the order differs between instances (Go maps, Rust `HashMap` with the default hasher), or documented where it depends on identity hashes (Java, C#, Python objects without their own hash) |
| Input and output | Files, network, environment, processes | Refused where the runtime can see it, flagged by an analyzer where it cannot |
| Global state | Module or static variables mutated across runs | A fresh context per run (TypeScript) or per-run objects with a documented rule (others) |
| The runtime and the dependencies | A different interpreter, library or SDK | Part of the code digest (section 3.2) |

#### 1.2 TypeScript

- **Runtime.** Each run gets its own `node:vm` context created with `vm.constants.DONT_CONTEXTIFY` (an ordinary global
  object created inside the context, not a host object), `microtaskMode: 'afterEvaluate'` and
  `codeGeneration: { strings: false, wasm: false }`. The workflow code is one bundle produced by the SDK's
  `sagat-workflows bundle` command (esbuild, pinned and hash-locked, Decision 044-012); the worker compiles the bundle
  once per digest as a `vm.Script` and runs it in each run's context. Inside the context the SDK's own code, evaluated
  there, replaces `Date` (the constructor without arguments and `Date.now`) with the workflow time, `Math.random` with
  the run's generator, `setTimeout`, `setInterval` and their `clear` functions with durable timers, and the default time
  zone and locale of `Intl` with UTC and the root locale; it removes `WeakRef`, `FinalizationRegistry`,
  `SharedArrayBuffer`, `Atomics`, `crypto`, `performance`, `process` and `queueMicrotask`'s host form.
- **Bridge.** No host function or object is placed in the context. The host calls into the context only with
  `vm.runInContext` and string arguments (the events as JSON text), and reads the commands back as a JSON string
  returned by a function defined inside the context.
- **Hazards.** Imports of `node:` built-ins and of their bare names (`fs`, `crypto`, `net`...), and of the activity
  modules, are refused by the bundler, so the function cannot reach the file system or the network through an import.
- **Concurrency.** Promises are the only concurrency. The SDK resolves the promise of one history event, then evaluates
  an empty script in the context, which drains the context's own microtask queue completely, before it applies the next
  event; so `Promise.race` and `Promise.all` resolve in journal order (*spike*).
- **Isolation.** With `DONT_CONTEXTIFY`, `this.constructor.constructor('return typeof process')()` returned `undefined`
  in the context, while an ordinary contextified object returned `"object"` (the host's `process`), even with code
  generation from strings refused (*spike*). The context is still not a security boundary (Gap 044-008): workflow code is
  the tenant's own, in the tenant's worker.
- **Detection.** The bundler's import check, the replaced globals, the activation deadline, the command comparison of
  section 6.1 and the server's command chain.
- **Precondition.** Node 22.8 or later for `DONT_CONTEXTIFY` (the SDK's activity worker keeps Node 20).
- **Verdict:** build. The only runtime of the six that removes the nondeterministic globals and refuses imports before
  the code runs.

#### 1.3 Python

- **Runtime.** A deterministic event loop, a subclass of `asyncio.AbstractEventLoop` whose `time()` is the workflow
  time, whose `call_soon` queue is first-in first-out on one thread, whose `call_at` and `call_later` become durable
  timer commands, and whose `run_in_executor`, sockets, subprocesses and signal handlers raise. So `asyncio.sleep`,
  `asyncio.gather`, `asyncio.wait_for` and `asyncio.timeout` work unchanged and durably (*spike*: `asyncio.sleep(30)`
  and `asyncio.wait_for(..., timeout=5)` became timers of 30 s and 5 s, and two runs gave the same commands).
- **Hazards.** String hashes depend on the process's hash seed, so the iteration order of a set of strings differs
  between processes (*spike*: four orders under four seeds, one order under a fixed seed). Objects that hash by identity
  have no stable order in a set in any process.
- **Isolation.** Workflow modules are imported once per digest through an import hook (a `sys.meta_path` finder) that is
  active both while the workflow modules load and during every activation run, and refuses the modules that reach outside
  (`socket`, `threading`, `subprocess`, `secrets`, `random`, `uuid`, `os`, `locale` and the others listed in the SDK),
  so a module-level `import uuid` fails at load. The `time` and `datetime` views the finder gives workflow modules return
  the workflow time in UTC. Behind the finder, a PEP 578 audit hook refuses the events `open`, `socket.connect`,
  `socket.getaddrinfo`, `subprocess.Popen`, `os.system` and `time.sleep` during an activation run; an audit hook cannot be
  removed once added (*spike*: `open()` refused by the hook, `import socket` and `import secrets` by the finder).
- **Detection.** The finder, the audit hook, the activation deadline, the command comparison and the command chain.
- **Precondition.** Python 3.11 or later for workflow code (`asyncio.timeout`; the activity worker keeps 3.10), and a
  fixed `PYTHONHASHSEED` chosen by the operator (any value but `random`; Decision 044-013). The content hash of the digest
  includes the `hash()` of a fixed string under that seed, so workers with different seeds have different digests and
  never share a run. A seed known outside the tenant would let a sender craft colliding keys; the operator keeps it with
  the deployment's configuration, and the activation deadline bounds the cost of a slow activation.
- **Verdict:** build.

#### 1.4 Go

- **Runtime.** A coroutine dispatcher on `iter.Pull`. The function and each branch started with `workflow.Go` run as
  coroutines; each SDK call yields to the dispatcher, which runs the ready coroutines one at a time in the order they
  became ready. `iter.Pull` switches between coroutines without the Go scheduler choosing (*spike*: 45 ns per round trip,
  against 241 ns for a goroutine and two channels, which is how a dispatcher built on goroutines hands control over).
- **Hazards.** Map iteration and `select` among ready channels are randomized (*spike*: eight orders in fifty ranges of
  one map; 59 of 100 choices of the first channel). The SDK provides `workflow.Channel`, `workflow.Selector`,
  `workflow.Go`, `workflow.Now` (UTC), `workflow.Sleep` and `workflow.SortedKeys`.
- **Isolation.** None at run time: Go cannot forbid a call. A native channel operation that blocks would stop the
  coroutine; the activation deadline turns it into `ACTIVATION_DEADLINE`.
- **Detection.** `sagatvet`, an analyzer of the `go/analysis` framework the SDK ships and runs in its test helper, refuses
  in workflow functions native `go` statements, channels, `select`, `time.Now`, `time.Local`, `time.Sleep`, `math/rand`,
  `crypto/rand` and `range` over a map; the deadline; the command comparison; the command chain.
- **Precondition.** Go 1.25 (the SDK's baseline).
- **Verdict:** build; the guarantee rests on the analyzer (Gap 044-007).

#### 1.5 C#

- **Runtime.** A single-threaded `TaskScheduler` per run with a maximum concurrency of 1, which never runs a task inline,
  and which the SDK drives until it is idle after each event. The SDK's `TaskCompletionSource` objects are created with
  `RunContinuationsAsynchronously`, so continuations are queued on the scheduler. During an activation run the SDK sets
  `CultureInfo.CurrentCulture` and `CurrentUICulture` to the invariant culture (they flow with the execution context).
- **Hazards.** `ConfigureAwait(false)` and `Task.Run` leave the scheduler (*spike*), as do `Task.Delay`, threads and
  `Parallel`.
- **Isolation.** Partial: the SDK's own awaits stay on the scheduler; other code can leave it.
- **Detection.** Every SDK call checks that `TaskScheduler.Current` is the run's scheduler and fails the workflow task
  with cause `NONDETERMINISM_RISK` otherwise. A Roslyn analyzer package flags `DateTime.Now`, `DateTime.UtcNow`,
  `Guid.NewGuid`, `new Random`, `Task.Run`, `Task.Delay`, `Thread`, `ConfigureAwait(false)`, `Parallel` and input and
  output types in workflow classes.
- **Precondition.** .NET 8 or later (the SDK's baseline).
- **Verdict:** build.

#### 1.6 Java

- **Runtime.** A dispatcher that gives a single permit to one workflow coroutine at a time; each coroutine is a virtual
  thread. The JDK chooses the carrier thread, but only the coroutine holding the permit can run, so the order is the
  dispatcher's (*spike*: 4,460 ns per handoff between virtual threads, 8,869 ns between platform threads, the model of
  Temporal's Java SDK; about 1,845 bytes of heap per parked virtual thread). A custom scheduler for virtual threads is not
  public API (*spike*: refused with `InaccessibleObjectException`), so the permit is the mechanism.
- **Hazards.** `System.currentTimeMillis`, `Random`, `UUID.randomUUID`, threads and executors, `CompletableFuture`'s
  default pool, iteration over a `HashSet` of objects whose hash is their identity (enums included), the default locale
  and time zone. The SDK provides `Workflow.currentTimeMillis`, `Workflow.newRandom`, `Workflow.randomUUID`,
  `Async.function`, `Promise` and `WorkflowQueue`.
- **Isolation.** Partial: the SDK's calls are checked; other code is not.
- **Detection.** Every SDK call checks that the calling thread is the coroutine that holds the permit and fails the
  workflow task with `NONDETERMINISM_RISK` otherwise; the command comparison; the command chain. No static analyzer in
  0.3.0 (Gap 044-002).
- **Precondition.** Java 21 for the new artifact `sagat-workflows` (the core SDK stays on Java release 17; Decision
  044-014), and a default time zone of UTC and a default locale of `Locale.ROOT` (the artifact refuses to register
  workflow types otherwise, because both are process-wide in Java).
- **Verdict:** build; the weakest static detection of the six, closed in part by the conformance and replay tests and the
  server's command chain.

#### 1.7 Rust

- **Runtime.** A single-threaded executor that polls the function's future and its spawned branches (`ctx.spawn`, not
  `Send`) in first-in first-out order. Activation runs execute on a pool of workflow threads (one per core by default,
  at most 8), on none of which a Tokio runtime is entered, so a call to `tokio::time::sleep` or `tokio::spawn` from the
  function panics; the executor catches the panic and fails the workflow task with cause `NONDETERMINISM_RISK`
  (*spike*: an executor of 70 lines gave the same commands and result in two runs).
- **Hazards.** `HashMap` and `HashSet` with the default hasher iterate differently per instance (*spike*: 49 orders in
  fifty maps of the same keys). `SystemTime::now`, `Instant::now`, `rand`, `uuid::Uuid::new_v4`, threads.
- **Isolation.** Strong for spawned work and timers: the type system keeps branches on one thread, and the missing
  runtime turns the common escapes into a caught panic.
- **Detection.** A `clippy.toml` fragment for workflow crates with `disallowed-types` (the default-hasher `HashMap` and
  `HashSet`) and `disallowed-methods` (`SystemTime::now`, `Instant::now`, `rand::random`, `uuid::Uuid::new_v4`,
  `std::thread::spawn`); the caught panic; the command comparison; the command chain. The SDK offers `ctx.now()` (UTC),
  `ctx.random()` and `ctx.uuid()`.
- **Precondition.** Rust 1.98 (the SDK's baseline).
- **Verdict:** build.

#### 1.8 The deterministic primitives, the same in six languages

- **Time.** `ctx.now()` is the database time, in microseconds, recorded in the `workflow_task.started` that opens the
  activation; it does not advance during the activation run. A language whose clock type has milliseconds (TypeScript
  `Date`) truncates.
- **Seed.** Each run has a 32-byte seed drawn from the operating system's random source at the run's creation, stored
  with the run and delivered in every workflow task. A successor of continue-as-new and a child get new seeds; a reset
  keeps its source run's seed, because it replays its source's prefix. The seed is returned by no REST read except the
  history export, and `ctx.random` and `ctx.uuid` are not for secrets (every SDK says so).
- **Generator.** xoshiro256** whose four 64-bit state words are the seed read as four little-endian unsigned integers
  (an all-zero seed is replaced by the four words of SplitMix64 started at 0). `random()` is `(next() >> 11) × 2^-53`.
  `randomInt(n)` for 0 < n ≤ 2^32 draws `x = next()` until `x ≥ (2^64 − n) mod n` and returns `x mod n` (no bias).
  `uuid()` takes 16 bytes from two `next()` values, little-endian, sets the version nibble to 4 and the variant bits to
  `10`.
- **Why one algorithm for six languages.** Replay happens in one language, so this is not needed for determinism; it
  makes one conformance golden hold all six (Requirement 044-010) and a history readable by any SDK's replayer.

#### 1.9 Behaviour flags of the SDK

An SDK release that changes how it turns calls into commands changes the code digest (section 3.2), so it never applies
to a running run without a move. For that case the protocol reserves a list of behaviour flags in the completion; 0.3.0
records none, and the mechanism is built by the first SDK release that needs it (Gap 044-015).

#### 1.10 The activation deadline and abandoned activation runs

Every SDK stops an activation run that has not yielded within 2 s of wall time (configurable from 100 ms to the workflow
task timeout) and fails the workflow task with cause `ACTIVATION_DEADLINE`. TypeScript's `vm` script timeout stops the
code. In Python, Go, C#, Java and Rust a stuck coroutine or thread cannot be killed: the dispatcher stops waiting,
abandons the coroutine and the run's cache entry, and counts it. When a worker holds 4 abandoned activation runs, it stops
taking workflow tasks, withdraws its workflow types and reports itself unhealthy, so that its process is restarted; in
Rust the stuck pool thread is replaced first.

#### 1.11 Verdicts

| Language | Runtime | Isolation for determinism | Detection beyond the command comparison and the chain | Precondition | Verdict |
|---|---|---|---|---|---|
| TypeScript | `vm` context per run without a host global, bundled code, drained microtasks | Strong: globals replaced, imports refused at bundling | Bundler import check | Node 22.8 | Build |
| Python | Deterministic asyncio loop, import finder at load and during activations, audit hook | Strong | Finder and audit hook | Python 3.11; a fixed hash seed | Build |
| Go | `iter.Pull` coroutine dispatcher | None at run time | `sagatvet` analyzer, deadline | Go 1.25 | Build |
| C# | Single-threaded `TaskScheduler`, invariant culture | Partial: the SDK's awaits | Scheduler check per call, Roslyn analyzer | .NET 8 | Build |
| Java | Virtual threads under one permit | Partial: the SDK's calls | Thread check per call | Java 21; UTC and the root locale | Build |
| Rust | Single-threaded executor on threads without a Tokio runtime | Strong for spawned work and timers | Caught panics, clippy configuration | Rust 1.98 | Build |

### 2. The model

A code run is an ordinary run (`runs` row, journal, steps) whose start version is a code version. It has one reserved
step, `~workflow` (the `~` keeps it outside the author's identifier alphabet, as the derived steps `<id>~c` of
Architecture decision 0010 do). Its activity field holds the reserved name `sagat.workflow:<type>:<digest>` of its
current code version (at most 15 + 48 + 1 + 64 = 128 characters, the limit of an activity name), which is how the
existing claim routes it (section 4.1). That step is `READY` when a workflow task is due, `LEASED` while a worker holds
it, `WAITING` with kind `DECIDER` when the function is blocked and nothing is pending, and under review when a person
must act. Every other step of a code run is created by a command, with the node `c-<n>`; derived steps keep their
suffixes (`c-12~c`).

The engine never runs the function. For a code run it changes three behaviours: after any visible event the planner
schedules the `~workflow` step instead of following graph edges; it never ends a code run by itself except for a
deadline, a forced cancellation (Requirement 044-022) or a termination; and it applies the commands of a completion
through the same command functions it uses for graph nodes (`crates/domain/src/execution/command.rs`), persisted by a
batched path of `PgRunTx` that is new code (section 5.2).

```mermaid
sequenceDiagram
  participant C as Client
  participant DB as PostgreSQL
  participant W as Worker (code digest A)
  C->>DB: start run of code version 7: run, seed, journal, ~workflow READY, outbox (one transaction)
  W->>DB: poll: lease ~workflow (generation 1, workflow_task.started with the database time)
  DB-->>W: workflow task: history from journal sequence 1, seed, time
  W->>W: run the function until it yields: commands 1 and 2
  W->>DB: complete: workflow_task.completed, step.released for c-1 and c-2, steps, outbox (one transaction)
  Note over DB: activity c-1 completes: step.completed and ~workflow READY (one transaction)
  W->>DB: poll (sticky): lease ~workflow (generation 2), the events of the new activation
  W->>W: apply the completion of c-1, run until it yields: command 3 (timer)
  W->>DB: complete: workflow_task.completed, timer.started for c-3 (one transaction)
```

### 3. The code version

#### 3.1 The document

A code version is a workflow document of the additive v1 schema in which the top-level `code` object replaces
`entrypoint` and `nodes`:

```json
{
  "apiVersion": "sagat-octopus-orchestrator.io/v1",
  "kind": "Workflow",
  "metadata": { "name": "order" },
  "code": {
    "workflowType": "order",
    "language": "typescript",
    "digest": "sha256:<64 hexadecimal characters>",
    "workflowTaskTimeoutMs": 10000,
    "cancellationGraceMs": 60000,
    "envelope": {
      "activities": [
        { "name": "inventory.*" },
        { "name": "payments.charge", "secrets": ["payments-api"],
          "outputSchema": { "type": "object", "properties": { "card": { "x-sagat-classification": "pii" } } } },
        { "name": "payments.lookup", "secrets": ["payments-api"] },
        { "name": "payments.refund", "secrets": ["payments-api"] }
      ],
      "scripts": [],
      "children": ["shipping"],
      "approvalPolicies": {}
    },
    "signals": { "paid": { "schema": { "type": "object" } } },
    "workerQueries": {
      "progress": { "disclosure": "declared-business-result-v1",
        "resultSchema": { "type": "object", "properties": { "done": { "type": "integer" } },
          "additionalProperties": false } }
    }
  },
  "inputSchema": { "type": "object" },
  "outputSchema": { "type": "object" },
  "state": { "schema": { "type": "object" }, "initial": {}, "queries": {}, "updates": {} }
}
```

- The schema change is a `oneOf` at the top level: a graph document requires `entrypoint` and `nodes`, a code document
  requires `code` and forbids them; every other top-level field keeps its meaning. A reader built before this design
  refuses a code document with `SCHEMA`, which is why the first publication activates a registry requirement
  (Requirement 044-030).
- `envelope.activities` entries are an exact name or a prefix ending in `.*`, at most 64; each lists the secrets an
  activity it covers may receive and the output schema (with its classification annotations, feature 016) of its
  output; a secret is checked at publication as an activity node's `secrets` are (feature 009). No entry may start with
  `script.` or `sagat.`.
- `envelope.scripts` lists artifact digests, each a ready artifact of the tenant and namespace, and fills
  `workflow_version_scripts` at publication as the script nodes of a graph do.
- `envelope.approvalPolicies` maps a name to an approval policy of the IR (`approvalPolicy`); a command names one.
- `state.updates` of a code version name the updates the function handles, each with `disclosure` and a closed
  `resultSchema`; their `validate` and `state` expressions are absent, because the function validates and handles them
  (section 8.2). `state.queries` keep their expression form: they are the engine-answered queries.
- `triggers`, `limits` and `chainPolicy` apply as for a graph version; a run-level `concurrency` or `rateLimit` (feature
  022) gates admission.
- Publication approval (feature 011, where the namespace requires it) reviews this document: the envelope, the
  declarations and the digest. It does not review the code, which the platform never receives; the Studio says so on the
  approval screen (Gap 044-014).

#### 3.2 The code digest and the manifest

The SDK computes the digest and writes it, with the workflow type, the language and the runtime, into a *manifest*
(JSON); the publisher sends the manifest and the envelope to `publishCode`. The worker computes the same digest when it
starts and registers it. The digest is the SHA-256 of the RFC 8785 canonical JSON of
`{"language", "runtime", "sdkName", "sdkVersion", "workflowType", "content"}`, where `runtime` is the runtime's name and
its major and minor version (for example `"node 22.11"`, `"cpython 3.12"`, `"go 1.25"`, `".net 8.0"`, `"java 21"`,
`"rustc 1.98"`), and `content` is the lowercase hexadecimal SHA-256 of:

| Language | What is hashed | How the manifest is produced |
|---|---|---|
| TypeScript | The bundle file the worker loads (it holds the workflow code and every dependency it imports) | `sagat-workflows bundle` writes the bundle and the manifest |
| Python | The sorted list of `(module name, SHA-256 of the source bytes)` of every module the import finder loaded for the workflow type (the workflow code and every dependency it imports), and the `hash()` of a fixed string under the process's hash seed | `python -m sagat_octopus_orchestrator.workflows manifest <module>` imports the workflow modules under the finder, with the deployment's `PYTHONHASHSEED`, without running a worker |
| Go | The executable file | `<worker binary> --sagat-manifest` prints it |
| C# | The workflow assembly and every assembly in its dependency closure from `deps.json`, sorted by name | `dotnet <worker>.dll --sagat-manifest` prints it |
| Java | Every jar and class directory of the worker's class path, sorted by name | `java -jar <worker>.jar --sagat-manifest` prints it |
| Rust | The executable file | `<worker binary> --sagat-manifest` prints it |

For the compiled languages this hashes more than the workflow code: a change of an activity in the same binary makes a
new digest. That is safe (a new digest never applies to a running run without a move) and its cost is that the deploy
needs a move for long runs, which matches whenever the workflow code did not change. The recommended topology for
compiled languages is therefore a worker binary for workflow code and another for activities, so that an activity
deploy does not change the workflow digest; the quickstarts of Task 044-1 show it, and journey 2 shows the pipeline.

#### 3.3 Publication

`client.definitions.publishCode(manifest, envelope)` in each SDK and `sagat workflow publish-code` publish a code version
through the existing publication transaction (Architecture decision 0018), under `definition:publish`. The
parser of `crates/infra-ir` validates the document; nothing runs code. Promotion makes the version active as today,
under `version:activate`. Before promotion a team can export recent histories and replay them in its continuous
integration, or request a dry-run move of the open runs (Requirement 044-026).

### 4. The workflow task

#### 4.1 Lease

The `~workflow` step is claimed through `PollTasks` and `StreamTasks` by the existing claim, which matches a step's
activity against the session's activities (`steps.activity = ANY(...)`, `crates/infra-postgres/src/execution.rs`) with
the index `steps_ready_idx`, the outbox hints and the reconciler unchanged. A session of the new protocol minor (section
13) registers workflow types as `(workflow type, code digest)` pairs; the server turns each pair into the reserved
activity name `sagat.workflow:<type>:<digest>` of the session, after checking the identity's exact binding
`workflow:<type>` (a `*` or prefix binding never covers it). An activity registration or a binding pattern that names
`sagat.workflow` is refused. The claim fences the lease as any step's. The lease differs from an activity's in four ways,
all because a workflow task has no external effect by construction (its commands act only when the server commits them):

1. It has no heartbeat; its time-to-live is the version's `workflowTaskTimeoutMs` (default 10,000 ms, from 1,000 to
   60,000 ms).
2. Its expiry does not enter reconciliation or review: feature 037 treats every leased *activity* as external-capable,
   and the reserved activity name is what tells it that this step is not one. An expired workflow task is journaled
   `workflow_task.timed_out` and made `READY` again with the next generation, not sticky.
3. Its lease token is refused by every activity RPC (Requirement 044-003); it reads history with `ReadHistory` and blobs
   with `ReadBlob` only for blobs referenced by that run's history.
4. The lease journals `workflow_task.started` with the database time sampled in the lease transaction and clears the
   run's `workflow_task_pending` flag: the events up to that start belong to this activation.

The binding and the tenant membership are re-checked, under the step lock, on every call made with the lease
(`ReadHistory`, `ReadBlob`, the completion, the failure), and on the calls of query and replay-check tasks.

#### 4.2 History

The lease carries the history the worker needs as typed `HistoryEvent` messages (section 13), not the journal's internal
JSON. The worker-visible history is a filtered projection of the journal: the events that change what the function can
observe (the command events, outcomes of steps, timers, waits, children, approvals and sagas, signals, updates,
cancellation requests, markers, `workflow_task.started` and `workflow_task.completed`). Lease renewals, attempts of an
activity, retries, hints and audit are not part of it. Command events carry the command number, kind and target, never
the fragment: the worker does not need the inputs of its past commands, and it must not be able to compute the command
chain without running the function (section 6.2). Each `HistoryEvent` carries its journal sequence.

- A sticky lease carries the visible events of the new activation window (section 4.3).
- Any other lease carries the first page of the visible history from journal sequence 1; the worker reads the rest with
  `ReadHistory` under the lease token, at most 1,024 events or 4 MiB per page.
- A sticky worker whose cache lost the run calls `ReadHistory` from journal sequence 1 and replays.

#### 4.3 The activation window

Signals, timer firings and activity completions can commit while a workflow task is started. They are journaled at once,
in their own transactions, and belong to the next activation. Let `S(k)` be the journal sequence of the
`workflow_task.started` of the workflow task that produced completion k (the last start before that completion; the
starts of tasks that failed or timed out are skipped). Activation k delivers the visible events with a journal sequence
in the interval `(S(k − 1), S(k)]`, in journal order, and `ctx.now()` is the time of `S(k)`. A live worker and a
replaying worker apply the same rule, so replay sees what the live run saw (Architecture decision 0104).

Every visible event sets the run's `workflow_task_pending` flag in its own transaction: if the `~workflow` step is
`WAITING` it becomes `READY` (with its outbox hint) in that transaction; if it is `LEASED`, the completion that ends the
lease sees the flag and makes the step `READY` again in its own transaction.

#### 4.4 Completion and failure

`CompleteWorkflowTask` carries the lease fence, the replayed command chain (section 6.2), the commands, and the number of
eager activity slots it offers. `FailWorkflowTask` carries a cause from the closed list of section 6.3 and, for a
nondeterminism, the command number and the actual kind and target; for a workflow task failure, the exception's type
name, accepted only if it matches `^[A-Za-z_][A-Za-z0-9_.$]{0,127}$` and dropped otherwise; never a message. Both are
idempotent by the lease's idempotency key; an idempotent replay of a completion answers its outcome without any eager
lease, and the idempotency record stores no lease token.

### 5. Commands and the one transaction

#### 5.1 The commands and their code profile

A work command carries a *fragment*: the JSON of an IR node of its kind, without `id` and `next`, in the *code profile*
of that kind, canonicalized by the SDK (RFC 8785). The code profile is the node's schema with every field that is an
expression or a template replaced by a literal, and every field only a publisher may set removed (Architecture decision
0107):

| Kind | Fields allowed in the code profile | Fields refused, and where their value comes from instead |
|---|---|---|
| Activity | `activity`, literal `input`, `retry`, `timeoutMs`, `compensateWith`, `reconcileWith`, `secrets`, `compensationRetry`, `compensationTimeoutMs`, `irreversible`, `highRisk`, the step-level `concurrency` and `rateLimit` keys of feature 022 as literals, the saga scope | `idempotencyTemplate` (the platform's step key, stable across attempts and replays), `outputSchema` (from the envelope entry), `destinations` (none from code), `onError` (the function handles failures) |
| Script | `artifact` digest, literal `input`, `retry`, `timeoutMs` | as for an activity |
| Timer | `delayMs` or `until` | none |
| Wait for event | `eventType`, literal `correlationKey`, `timeoutMs`, `lookbackMs` | `correlationExpr` (the key is a literal) |
| Child | `workflowId`, `version`, literal `input`, `cancelPolicy`, `businessId` | expression `input` |
| Approval | `policy` naming an entry of `envelope.approvalPolicies`, literal `subject` | an inline policy (eligible subjects, initiator exclusion) |

A fragment is parsed with the publication parser of its kind in its code-profile mode (`crates/infra-ir`), so every
limit of a graph node applies to the command. The other commands are typed messages:

| Command | Journal events | Created or changed |
|---|---|---|
| Schedule activity, run script | `step.released` for `c-<n>`, carrying the fragment's digest | `steps` row `READY` (or `WAITING` behind a gate of feature 022) with the fragment digest; the fragment in `payloads`; outbox hint |
| Start timer, cancel timer | `timer.started`; `step.cancelled` | `steps` row `WAITING` kind `TIMER` |
| Wait for event | `wait.started` | `steps` row `WAITING` kind `EVENT` |
| Start child | `step.released` | `steps` row with `child_run_id` reserved, started by the children pass as today |
| Request approval | `approval.opened` | as an approval node |
| Open, close, abort saga | `saga.entered`, `saga.completed`, `saga.aborting` | saga rows; an abort plans compensations (section 10) |
| Request cancellation | `step.cancel_requested` | as today |
| Record marker | `marker.recorded` (kind `patch` or `side_effect`; a side effect's value at most 64 KiB) | none |
| Set state | `state.updated` | `workflow_operation_states` |
| Accept, reject, complete update | `update.accepted`, `update.rejected`, `update.completed` | `operation_update_receipts` |
| Complete, fail run | `run.finished` | run terminal (Requirement 044-031) |
| Continue as new | `run.continued` | the successor, through feature 014's handoff |
| Acknowledge cancellation | `run.finished` (`CANCELLED`) | run terminal |

The fragment of every accepted work command is stored in `payloads` (content-addressed, append-only) and its digest in
the `step.released` event and the step row, so the step's declared secrets, output schema and input are read from an
immutable record, exactly as a graph step's are read from its published node: `ResolveSecrets` reads the secrets of a
`c-<n>` step from its stored fragment (`declared_secrets_of`, `crates/application/src/worker.rs`). A command may
reference a blob only if the blob was written under the same lease or is referenced by the run's history.

The new event kinds are `workflow_task.started`, `workflow_task.completed`, `workflow_task.failed`,
`workflow_task.timed_out`, `signal.received`, `update.admitted`, `update.accepted`, `update.rejected`,
`update.completed`, `marker.recorded` and `run.code_version_moved`; a review uses the existing `step.needs_review` and
`step.retry_requested`; every other row reuses an existing kind (`RunEvent`, `crates/domain/src/execution/event.rs`).
Feature 020 adds its search-attribute command and event when it lands (Task 044-27).

#### 5.2 The transaction of a completion

Before the transaction, outside any lock, the server parses every command and checks it against the code profile and the
IR rules. Then, in one transaction, the persistence path issues these statements, batched per table, so their number
does not grow with the number of commands:

1. Lock the run row and the `~workflow` step (`SELECT ... FOR UPDATE`); in memory, check tenant, namespace, session,
   the session's digest against the current code version, lease token hash, attempt and generation (stale:
   `ABORTED STALE_ATTEMPT`, nothing written), the replayed chain against `runs.code_command_chain`, that the first new
   command number is `runs.code_command_count + 1`, and the envelope of the current code version (a mismatch writes the
   review instead of the commands, section 6.3).
2. Insert the payloads (the fragments, the inline inputs), one multi-row statement.
3. Insert the journal events, `workflow_task.completed` first, then one per command, one multi-row statement.
4. Upsert the steps the commands create or change (timers, waits, the eager activity leases as `LEASED` rows) and the
   `~workflow` step itself (`READY` again if `workflow_task_pending` is set or an admitted update awaits acceptance,
   `WAITING` otherwise), one multi-row statement.
5. Insert the outbox rows for the new `READY` steps, one multi-row statement.
6. Compare-and-swap the run row (`run_version`, `code_command_chain`, `code_command_count`, the visible count,
   `workflow_task_pending`).
7. Write the idempotency record.
8. Only when a command needs them: the saga rows, the declared state, the update receipts (at most one statement each).

A terminal command adds the statements of the existing terminal path (the run's finished notice, the parent's
notification, the cancellation of children, the audit), and continue-as-new those of feature 014's handoff. The batched
persistence path is new code in `PgRunTx`, with its own crash tests (Task 044-9). A terminal command is refused with
`UNHANDLED_EVENTS` when `workflow_task_pending` is set, and is retried at once with the new events; after 3 consecutive
`UNHANDLED_EVENTS` refusals the run refuses new signals and updates (`409 RUN_CLOSING`) until its next completion
commits, so a stream of signals cannot keep a run from ending.

#### 5.3 The transactions that wake the function

An activity completion, a timer firing, a wait satisfied or timed out, a child finished, an approval decided, a saga
settled, a signal, an update admission and a cancellation request each already commit in one transaction (their own code
paths). For a code run each of them, in that same transaction, journals its event, increments the visible count and sets
`workflow_task_pending`, making the `~workflow` step `READY` with its outbox hint when it was `WAITING`.

### 6. Nondeterminism

#### 6.1 In the SDK

During replay the SDK compares each command the function makes with the recorded command at the same command number:
the kind and the target (activity name, script digest, child workflow, timer, event type, approval policy name, saga
scope identifier, marker identifier). A recorded command the function did not make by the time it yields with no event
left to apply, a command where the history has none, and a `patched(id)` that disagrees with the markers are mismatches.
The report (`FailWorkflowTask`, cause `NONDETERMINISM`) names the command number, the actual kind and target and the code
digest; the server takes the expected kind and target from its own record, never from the worker, and checks the actual
target against the grammar of its kind.

#### 6.2 On the server: the command chain

For each code run the server keeps `code_command_chain`, 32 bytes: `H(0)` is 32 zero bytes and
`H(k) = SHA-256(H(k − 1) ‖ uint32 big-endian length of d(k) ‖ d(k))`, where `d(k)` is the RFC 8785 JSON of
`{"seq": k, "kind": ..., "target": ..., "fragment": HMAC-SHA-256(seed, canonical fragment)}` for command k (the
`fragment` member is absent for a command without a fragment). A completion carries the chain the function produced up to
its first new command; the server compares it with the stored one and extends it with the new commands. A sticky worker
sends the chain it kept.

Because the history never carries a fragment, a worker can produce the chain only by producing the same commands, inputs
included: a worker that skipped replay, a corrupted cache, an input that changed between two runs of the same code (an
escaped `uuid4()`, a clock read) or an SDK bug all end as `CHAIN_MISMATCH`. The digests are keyed by the run's seed so
that a reader of the server's records cannot confirm a guess of an input. The check costs one comparison and one HMAC
and one SHA-256 per new command.

#### 6.3 Causes, review and remedies

A run goes under review by the step review of feature 003: the `~workflow` step gets `step.needs_review` with the cause
as its code, the run is listed with the steps under review, the audit records it, and `workflow_task_reviews_total{cause}`
counts it. The run's status, steps, timers and waits go on, and their events are journaled and wait for the function. The
retry is the existing review retry of a step (`ReviewAction::Retry`, `run:operate`); skipping or resolving the
`~workflow` step is refused.

| Cause | Raised by | Behaviour | Remedies |
|---|---|---|---|
| `NONDETERMINISM` | SDK | Under review at once | Move, reset, terminate |
| `CHAIN_MISMATCH` | Server | Under review at once | Move, reset, terminate |
| `COMMAND_OUTSIDE_ENVELOPE` | Server | Under review at once | Move, reset, terminate |
| `COMMAND_INVALID` (a fragment outside its code profile, a malformed command) | Server | Under review at once | Move, reset, terminate |
| `NONDETERMINISM_RISK` (C# scheduler, Java thread, Rust runtime check) | SDK | Under review at once | Move, reset, terminate |
| `VALIDATOR_COMMAND` (an update validator issued a command) | SDK | Under review at once | Move, reset, terminate |
| `WORKFLOW_TASK_FAILED` (an uncaught exception) | SDK | Retried after 1, 2, 4 and 8 s; the fifth consecutive failure, at least 1 + 2 + 4 + 8 = 15 s after the first, puts the run under review | Retry, move, reset, terminate |
| `ACTIVATION_DEADLINE` | SDK | Counted with `WORKFLOW_TASK_FAILED` | Retry, move, reset, terminate |
| `COMMAND_LIMIT` (more than 2,048 outstanding) | Server | Counted with `WORKFLOW_TASK_FAILED` | Retry, move, reset, terminate |
| `OUTSTANDING_OBLIGATIONS` (a terminal command while a saga or a review is open) | Server | Counted with `WORKFLOW_TASK_FAILED` | Retry, move, reset, terminate |
| Workflow task timeout | Server | Counted with `WORKFLOW_TASK_FAILED` | Retry, move, reset, terminate |
| `UNHANDLED_EVENTS` | Server | Retried at once, not counted (section 5.2) | none needed |
| `HISTORY_LIMIT` (visible count 51,200) | Server | Under review at once | Retry opens the closing window; terminate |

A cause raised at once is not retried because the same code would produce it again. The run's deadline and a forced
cancellation (Requirement 044-022) end a run under review as they end any run. The **closing window** of a
`HISTORY_LIMIT` retry offers workflow tasks with the flag `historyLimitReached` for 1,024 more visible events; in them
only commands that end the run, continue it as new, cancel its outstanding work, or close or abort its sagas are
accepted, and the SDK exposes the flag so that the function can wind down and continue as new. A **reset** of a code run
(feature 018) copies the journal prefix up to a `workflow_task.completed` into a new run with the source's seed; to the
same code version it starts at once; to another code version it is answered `202`, the prefix is replay-checked as a
move is, and the reset record holds the result.

### 7. Versioning

#### 7.1 Pinning

A run executes the code of its current code version (Requirement 044-002), equal to its start version until a move. A
new version never applies to an open run by itself. The version list shows, per code version, the open runs, the sessions
that serve its digest and the age of its oldest run. A run whose digest no session serves waits, counted by
`workflow_tasks_unserved{reason="no_digest"}` and alerted when a run has waited more than 60 s (the tenant in the span,
not the metric).

#### 7.2 Patches

`patched(id)` returns true and records a `patch` marker the first time a run reaches it; on replay it returns true if the
marker is at that point of the history and false if the history has another command there, which is where the old code
went. `deprecatePatch(id)` records the marker and always takes the new path, for code from which the old path was
removed after every run that could take it ended. These are Temporal's semantics; the difference is that the move of
section 7.3 proves, per run, that the patched code replays before it applies.

#### 7.3 Verified moves

- **Requests.** A single run: `POST /runs/{runId}/code-version` with `{toVersion, dryRun}`, under `run:operate` and
  `version:activate`. Many runs: a batch job of feature 018 with the new action `MOVE_CODE_VERSION`
  (`batch_jobs`, `batch_items`, `crates/domain/src/batch.rs`), under `run:batch` as well, with `{fromVersion, toVersion,
  dryRun}` and the batch selection (all open runs of a version, or a list). The target must be a published code version
  of the same workflow and namespace, approved for activation where the namespace requires approval.
- **The check.** For each run the server sends a `REPLAY_CHECK` task to a session of the target digest, at most 16 runs
  per task, each with its own read capability (a token whose hash is stored with the item, fenced and expiring with the
  task) under which the worker reads the run's history with `ReadHistory`. The worker replays without issuing commands
  and answers, per run, the visible count it replayed, the chain it computed, and the keyed fragment digest of each
  command, so that the server can name the first command that differs.
- **Applying.** In one transaction per run, the server locks the run and applies the move only when the chain equals
  `code_command_chain`, the run's visible count still equals the replayed count, the `~workflow` step is not leased, and
  every outstanding step fits the target's envelope. If the step is leased or the visible count moved, the item is
  checked again later, at most three times, then ends with reason `RUN_BUSY`. Applying means: a call of the database
  function that alone may change `runs.code_version_id` (Technical Plan 14), which journals `run.code_version_moved` with
  both versions, the actor and the replayed count, sets the `~workflow` step's activity to the target's reserved name,
  ends a review if any, and makes the step `READY` if events are pending; and an audit record.
- **What stays.** The start version and every identity derived from it (step identifiers, journal instance
  identifiers, compensation identifiers) do not change.
- **Dry run.** Records the verdict per item and moves nothing.

Principle IV says there is no *automatic* migration of a running workflow: a move is an explicit request by an authorized
person, per run, verified, journaled and audited, and the run references exactly one start version and one current code
version at any moment (Architecture decision 0105; Constitution Check). No move policy applies by itself in 0.3.0 (Gap
044-006).

#### 7.4 Replay in continuous integration

`GET /runs/{runId}/history` returns a code run's visible history, with its seed and the fragments of its commands, as
JSON lines of `HistoryEvent`, under `run:read` and `history:export` (granted to the admin and developer roles). Every
payload goes through the read path of feature 016: a classified field is replaced by a typed placeholder unless the
reader may see it there (`payload:read-internal` for an internal field; `payload:reveal-pii` and a reveal grant for a
personal one), and a field classified `secret` is never exported; a
payload encoded by a client codec (feature 031) stays encoded. The SDK replayers read that format; a replay that reads a
placeholder stops with the verdict `INCONCLUSIVE_REDACTED` at that command, never with a false mismatch.
`sagat workflow history export --workflow order --last 64 --dir histories/` writes a corpus for a team's tests.

### 8. Signals, updates and queries

#### 8.1 Signals

`POST /runs/{runId}/signals/{name}` keeps its route, permission, idempotency key and 256 KiB limit. For a code run the
signal is accepted under the bounds of Requirement 044-014, journaled `signal.received` with its payload digest, and
delivered at the next activation. The SDK gives the function a handler (`ctx.onSignal(name, fn)`) and a wait
(`await ctx.signal(name)`); a signal without a handler is kept in an in-memory queue per name, rebuilt by replay.
Signal-with-Start creates the run and journals the signal in the creating transaction. A signal to a run inside its
start delay is refused as today (feature 018).

#### 8.2 Updates

`POST /runs/{runId}/updates/{name}` with `{updateKey, args, waitMs}`, under the permission of today's update route,
admits the update: in one transaction, a receipt `ADMITTED` (`operation_update_receipts`), `update.admitted` and
`workflow_task_pending`. When the activation applies the `update.admitted` event, in journal order, the function's
validator runs at once, before the next event is applied (a validator that issues a command fails the workflow task with
`VALIDATOR_COMMAND`), and the completion journals `update.accepted` or `update.rejected` with a reason from a closed list;
the handler then runs as a branch of the function and may issue commands; a later completion journals
`update.completed` with the result, validated against the declared `resultSchema` by the platform's validator of design
043, or with a failure. The REST call waits up to `waitMs` (default 0, at most 60,000) by re-reading the receipt outside
any transaction after 50, 100, 200, 400 and 800 ms and then every 1,000 ms, at most 64 reads in 60 s
(5 reads by 1,550 ms, 58 more by 59,550 ms, one at the deadline), counted against the namespace's 256 waits; it answers
`200` with the result or `202` with `ADMITTED` or `ACCEPTED`, with `Cache-Control: no-store`. A repeated `updateKey` with
the same arguments returns the same receipt, a different one `409 IDEMPOTENCY_KEY_REUSED`. Update-with-Start admits the
update in the creating transaction. A graph run's update keeps its synchronous expression form.

#### 8.3 Queries

- **Declared queries (default).** The function sets the declared state with a command; the engine answers the
  expression queries of `state.queries` from the committed state exactly as for a graph run (feature 014,
  `crates/application/src/declared_operations.rs`): no worker, no journal event, audited, on closed runs while their data
  is retained (Architecture decision 0108).
- **Worker queries.** `POST /runs/{runId}/worker-queries/{name}` with `{args, timeoutMs}` (at most 30,000 ms), under
  `run:read` and `run:query`, inserts a row of `code_query_requests` (not journaled, deleted by the maintenance pass after
  the timeout plus 5 s), announced by an outbox hint, leased by a session of the run's current digest (the sticky one
  first) as a `QUERY` task, answered with `RespondWorkerQuery`, validated against the query's `resultSchema` by the
  platform's validator, and returned to the caller with `Cache-Control: no-store`. A handler that issues a command fails
  the query with `QUERY_COMMAND`. Audited with the caller and the outcome, not the arguments or the answer. At most 64 in
  flight per namespace.

### 9. Children, continue-as-new and mixing

- **Children.** A child command creates the `child` node instance, and the existing children pass starts the child run,
  graph or code, with its cancel policy, depth and the tree budget of 1,024 runs and depth 8 (feature 014). The child's
  outcome is a `child.finished` event the function sees. A graph `child` node may name a code workflow.
- **Continue-as-new.** The command hands off through feature 014's transaction: quiescence is required (no outstanding
  step, timer, wait, child, approval, unknown effect or compensation), the run chain's deadline and segment limit apply,
  the transferred input is checked against the successor's `inputSchema` (design 043), and the successor gets a new seed
  and an empty history. `version: active` pins the workflow's active version at the handoff, the usual way a long-lived
  workflow takes new code; the default `same` keeps the start version.
- **Mixing.** A code workflow uses a graph child for a large fan-out: a `forEach` over 10,000 items keeps 10,000
  activities out of the code run's history, which sees one `child.finished`. Because every work command is an IR node,
  the Studio draws a code run's observed graph from its steps and its journal with the same node pictures, marked
  `Code-orchestrated`.

### 10. Sagas and uncertain effects from code

#### 10.1 The saga scope

```typescript
await ctx.saga({ onCompensationFailure: 'manual_review' }, async (saga) => {
  await saga.activity('inventory.reserve', input, { compensateWith: 'inventory.release' });
  await saga.activity('payments.charge', input, {
    compensateWith: 'payments.refund', reconcileWith: 'payments.lookup' });
});
```

`ctx.saga` issues `open saga` with a scope identifier (`s-<n>`, the command number) and the policy
(`onCompensationFailure` is `manual_review` or `fail`, the values of the saga tables). Activities scheduled through `saga`
carry the scope. When the body returns, the SDK issues `close saga` and the compensations are dropped, as for a graph saga
that completes. When the body throws (or calls `saga.abort()`), the SDK issues `abort saga`; the engine then plans the
compensations of the scope's completed steps in reverse order of their completion, one at a time (code declares no
dependencies, so reverse completion order is the safe order), and stores that plan in run-scoped plan rows, because a
graph saga's plan is computed at publication and stored per version, which a code saga cannot be. Each compensation's
identifier is derived as a graph compensation's is, from the start version and the forward node (`c-<n>`), so it is the
same across retries and moves; its key includes the run, so a reset's run has its own keys, as for a graph reset. The
compensations run as derived steps with the retry, lease, fence and audit machinery of feature 003. The function waits;
when the scope settles the engine journals its outcome and `ctx.saga` throws `SagaCompensated`, `SagaPartial` or
`SagaCompensationFailed`. A run cancelled while a scope is open follows the scope's cancel policy. Task 044-22 chooses
between the existing run saga rows and a new run-scoped plan table and records the reason (Decision 044-019).

#### 10.2 Unknown outcomes

An activity from code that reports `OUTCOME_UNKNOWN`, times out after the attempt began or loses its lease follows
feature 037 exactly as a graph activity: its `reconcileWith` lookup runs; `applied` completes it with the lookup's output;
anything else waits for an authorized review. The function sees only the final outcome or keeps waiting. It cannot repeat
an unknown effect itself, because it has no handle on the attempt; a failure it sees is a known failure.

#### 10.3 Why no local activities

A local activity runs its effect inside the workflow task and records the result with the completion. If the worker dies
after the effect and before the completion, the effect runs again with no lease, no attempt record and no reconciliation:
an unknown outcome repeated blindly, which principle III forbids. The latency it saves is recovered by eager dispatch
(section 11.2) with every activity a leased, fenced, reconcilable step (Architecture decision 0107).

#### 10.4 Outputs and the retained bytes

The output of a `c-<n>` step is retained and charged to the run's 4 MiB as a graph step's is; a completion that would go
over fails its step with `RUN_DATA_LIMIT`, not retried, which the function sees as a known failure. Large values travel as
blobs (claim checks). The continue-as-new suggestion fires at 3 MiB of retained bytes (75 % of 4 MiB) as well as at a
visible count of 10,240.

### 11. Sticky cache and eager dispatch

#### 11.1 Sticky

When the `~workflow` step becomes `READY`, it records `sticky_session_id` (the session that completed the last workflow
task) and `sticky_until` (that moment plus 1,000 ms by default, at most 5,000 ms). The claim offers it to its session
first; another session can lease it after `sticky_until` or when the sticky session has ended. A worker evicts runs from
its cache by least recent use beyond its cache size (default 256) and tells the server with `ReleaseSticky(runIds)`,
which the server applies only to steps whose `sticky_session_id` is the caller's session in the caller's tenant.

#### 11.2 Eager dispatch

Built after the non-eager path and measured by Task 044-25 (both are in Non-functional requirement 044-002):

- **Eager activities.** A completion may offer `eager_activity_slots`. The server leases to this session, in the
  completion's transaction (statement 4 of section 5.2), up to that many of the new activity steps whose activity the
  session serves and is bound to, and returns their leases in the answer.
- **Eager workflow task.** A sticky session's `CompleteStep` may set `accept_workflow_task`; when the completion makes the
  run's `~workflow` step `READY` and that session is its sticky session, the answer carries the workflow task lease.

### 12. Interactions with other features

| Feature | Interaction |
|---|---|
| 014, declared state and run chains | Reused as described in sections 8 and 9 |
| 016, payload classification | Outputs of `c-<n>` steps are classified by the output schema of their envelope entry; every read of a code run's payloads (REST, Studio, history export) applies the redaction and reveal rules of feature 016 |
| 018, business identifiers, batch, reset | Business identifiers, the with-start calls and batch operations apply to code runs unchanged; moves are a batch action; reset as in section 6.3 |
| 019, worker deployments | A deployment routes activities only; workflow tasks are routed by the code digest of the run's current code version |
| 020, visibility | Its search-attribute command and event are added by Task 044-27 when feature 020 lands |
| 022, concurrency and rate limits | Run-level gates gate a code run's admission in `PgRunStore::create` as any run's; a run-level gate with continue-as-new is refused as for a graph (Requirement 022-017). Step-level keys on an activity command hold the step `WAITING` until the grant (Architecture decision 0089). The `~workflow` step, query tasks and replay-check tasks are never gated |
| 031, codecs | The server evaluates no expression on a code run's payloads, so inputs, outputs and signal payloads may be encoded end to end; declared state, update arguments and results the engine validates, and literal correlation keys stay plaintext |
| 041, step logs | Activities from code send step logs as any activity. A workflow task sends none; the function's log output goes to the worker's own logger, suppressed during replay |
| 043, input validation | A code run's input, a child's and a successor's are checked by the gate of design 043; update and worker-query results are validated by its validator |

### 13. Protocol

The worker protocol gains, in the next free minor of the register in `sdks/SPEC.md` at the time the contract of Task
044-2 merges (Architecture decision 0109; 1.7 is taken by feature 041, and the branch of feature 006's delivery
acknowledgement reserves 1.8; Gap 044-013):

- `WorkerCapabilities.workflow_types`: up to 64 `(workflow_type, code_digest)` pairs.
- `TaskLease.task_kind` values `WORKFLOW`, `QUERY` and `REPLAY_CHECK`, and a `WorkflowTaskLease` message: run, attempt,
  generation, lease token, deadline, seed, `started_at_us`, sticky flag, the first history page, the visible count, the
  continue-as-new suggestion and `historyLimitReached`.
- RPCs `CompleteWorkflowTask`, `FailWorkflowTask`, `ReadHistory` (server stream), `RespondWorkerQuery`,
  `RespondReplayCheck`, `ReleaseSticky`.
- `HistoryEvent`: journal sequence, kind (an enumeration of the visible kinds), the command number it answers, time, and
  a `oneof` of typed bodies with payloads as inline bytes (at most 256 KiB) or blob references; command events carry no
  fragment.
- `Command`: command number, kind, `fragment_json` for work commands, typed fields for the others.
- `CompleteStepRequest.accept_workflow_task` and an optional `WorkflowTaskLease` in its answer;
  `CompleteWorkflowTaskRequest.eager_activity_slots` and the eager activity leases in its answer; a reserved repeated
  field for behaviour flags.

A session below the new minor is never offered a `~workflow`, query or replay-check task (the claim filter of
`required_worker_protocol_minor`, `crates/application/src/worker.rs`). Everything is additive: `worker.baseline.json`
grows, nothing changes meaning.

### 14. Storage

One additive migration in `crates/infra-postgres/migrations`, following the repository's migration checklist
(`.claude/skills/migration`: grants to the runtime role `sagat_app`, a down that refuses data loss, storage
classification):

- `runs`: `code_version_id uuid` (the current code version; null for graph runs), `code_seed bytea` (32 bytes),
  `code_command_chain bytea` (32 bytes), `code_command_count integer`, `code_visible_events integer`,
  `workflow_task_pending boolean`, `code_unhandled_refusals smallint`; a check that they are all set or all null.
  `sagat_app` has no `UPDATE` privilege on `code_version_id`; it changes only through
  `sagat_move_code_run(tenant, run, from_version, to_version, ...)`, a `SECURITY DEFINER` function with
  `SET search_path = pg_catalog, pg_temp` that checks tenant, namespace, workflow and code kind of the target and writes
  the journal row itself. `runs.workflow_version_id` is unchanged and still never updated.
- `steps`: `sticky_session_id`, `sticky_until`, `fragment_digest`; `waiting_kind` gains `DECIDER`.
- `code_query_requests (tenant_id, namespace_id, run_id, id, name, args_digest, expires_at, ...)`, with `INSERT`,
  `SELECT`, `UPDATE` and `DELETE` for `sagat_app`.
- `batch_jobs.action` gains `MOVE_CODE_VERSION`; `batch_items` holds the move items.
- A namespace setting for pausing code workflows (Requirement 044-033).

### 15. Telemetry and audit

- Metrics (closed labels only): `workflow_tasks_total{outcome}`, `workflow_task_latency_seconds{sticky}`,
  `workflow_task_reviews_total{cause}`, `workflow_tasks_unserved{reason}`, `code_moves_total{verdict}`,
  `worker_queries_total{outcome}`, and in each SDK `sagat_workflow_cache_runs`, `sagat_workflow_replay_seconds`,
  `sagat_workflow_nondeterminism_total`, `sagat_workflow_abandoned_activations`.
- Spans: `workflow_task.lease`, `workflow_task.complete` with the run, the command count and the command number; the SDK's
  `workflow.activation` span, live or replay.
- Audit: publication and promotion of a code version, a review and its retry, a move request and each applied move, a
  history export, a worker query, a pause and a resume, and every refused or cross-tenant attempt on the new routes,
  tasks and RPCs.

### 16. Rollout and compatibility

1. The first publication of a code version activates the instance registry's requirement `code-workflows`
   (Architecture decision 0082); every instance that does not report it closes its gate, including one that registers
   later, so an instance of 0.2.x never opens a code run.
2. SDKs offer the new minor only when the application registers a workflow type; an activity-only worker keeps its minor.
3. The migration's down refuses while code runs are open. To roll back, the operator first lets them end or terminates
   them (no graph version is generated from code), or stays on 0.3.0 (Gap 044-009). The namespace pause of Requirement
   044-033 stops every code run's workflow tasks while a problem is found.

### 17. Against Temporal

The comparison is with Temporal's current model: Worker Deployment versioning with the Pinned and Auto-Upgrade
behaviours, `patched()` and `GetVersion`, and the SDK replayers for continuous integration.

| Concern | Temporal | This design |
|---|---|---|
| Code identity | A build identifier the deployer chooses | A digest computed from the code, its dependencies and its runtime |
| Running runs and new code | Pinned runs keep their build; Auto-Upgrade runs replay the new code with no check | Runs keep their digest; a move applies to one run only after its own history replayed on the new code with the same commands and inputs |
| Determinism check | In the SDK | In the SDK, and on the server by a keyed command chain that includes inputs |
| A workflow bug | The workflow task retries until a fix is deployed | The run goes under review with its cause and is resumed by a verified move |
| Sagas and unknown outcomes | Hand-written compensation | Engine-planned compensation with reconciliation and review of unknown outcomes |
| Queries | Need a running worker | Declared queries answered by the engine, also after the run ends; worker queries as well |
| Fan-out | Every activity in the workflow's history | A graph child with `forEach` keeps the items out of the code run's history |
| Authority | Its own persistence and matching services | One PostgreSQL transaction per decision with the outbox; one engine for graph and code |

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A determinism runtime has a hole a test did not find (Go, C#, Java above all) | The keyed command chain refuses a divergent completion, inputs included; analyzers; conformance scenarios with deliberate breaches; the review keeps the run |
| Compiled-language deploys change the digest | The recommended topology keeps workflow code in its own binary; moves of unchanged workflow code always match; the pipeline step of journey 2; the unserved alert |
| The sticky cache's memory exceeds the budget | Measured per SDK in Task 044-25 before the budget is published; the cache size is configurable |
| Workers of old digests must stay up | The version list shows what still runs on each digest; `version: active` continue-as-new and moves give a way off an old digest |
| A worker forges a `match` for a move | Without the fragments it cannot produce the keyed chain without producing the same commands; a worker of the tenant that does is harming only the tenant's own runs (Gap 044-005) |
| The new minor collides with another feature's | Architecture decision 0109 takes the next free minor at merge, as the register says |
| An instance of 0.2.x meets a code run during a rolling upgrade | The registry requirement closes its gate (Requirement 044-030) |
| A code workflow, or an outside sender, floods the journal | History limits with a closing window, the headroom for the run's own events, the per-run and per-namespace bounds of Requirement 044-032, the 4 MiB retained bytes, run-level rate limits of feature 022 |
| An outside sender makes a Python activation slow with colliding keys | The hash seed is the operator's, not a known value; the activation deadline bounds a slow activation |
| Abandoned activation runs leak threads | The cap of 4 per worker, then the worker withdraws and reports itself unhealthy |
| The constitution amendment is late | Task 044-1 depends on its merge; no code starts before it |

## Constitution Check

The rows below are checked against two texts: version 4.7.0, the constitution on `develop` when this design was written,
and version 5.0.0 as owner decision 2026-11-24-2 states it ("the declarative IR is joined by a procedural code-first
mode", and "reimplementing Temporal" leaves the out-of-scope list). Against 4.7.0 two clauses block the design:
Delivery Constraints lists "reimplementing Temporal or Restate" as out of scope and sets a "graph-native declarative IR"
baseline. Version 5.0.0 removes both, and Task 044-1 depends on its merge, so no code of this design starts before it.
Gap 044-010 tracks that its text says what a verified move is.

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | A completion commits the journal, the steps, timers, waits, projections, the outbox and the next workflow task in one transaction with the run row locked and the lease fenced (Technical Plan 5.2); every visible event sets the pending flag in its own transaction (5.3); a stale workflow task or one of another digest is refused; the activation window makes the first committed of a timer and a signal the one the function sees first; a terminal run never reopens (a run under review is not terminal) |
| II. Modular domain, ports and adapters | Pass | The workflow task lifecycle, the command chain, the code profiles and the envelope checks are pure domain code; the history projection and the wire types are in `api-grpc`; the storage and the batched persistence path in `infra-postgres`; no new dependency of the domain |
| III. External effects are at-least-once; sagas compensate | Pass | Every effect from code is a leased activity with the platform's step key and the uncertain-effect rules of feature 037; no local activities (Technical Plan 10.3); a workflow task has no effect, so its expiry is re-leased without review; sagas from code are planned by the engine; a terminal command is refused while a saga or a review is open (Requirement 044-031) |
| IV. Compatibility is versioned and pinned | Pass with conditions | A code version is immutable; a code run references one start version, never changed, and one current code version at any moment; the IR change is additive (`oneOf`) behind the registry requirement; the protocol change is an additive minor; a verified move changes the current code version only on an explicit, authorized, verified, audited request per run (Complexity Tracking); the Studio draws code runs without promising a round trip, which 4.7.0 already allows ("imperative code that cannot be converted is not promised a round trip") |
| V. Code, tenant boundary and secrets are isolated | Pass | No workflow code runs in a platform process: replays, checks and queries run on the tenant's workers; a workflow task carries no secret; secrets are declared per activity entry of the immutable version and read from the immutable fragment; publisher-only fields are refused in fragments; tenant scope and audit of refused attempts on every new table, route, task and RPC; another tenant's run is indistinguishable from a missing one; bounds on every new load path (Requirement 044-032) |
| VI. Observability and audit are part of the feature | Pass | Closed-label metrics, spans, audit of publication, promotion, review, retry, move, history export, worker query, pause; the run graph is drawn from the journal and the steps |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Signals keep their idempotency key; outbox hints for workflow tasks, queries and checks through the existing hint path; the reconciler finds a `READY` `~workflow` step whose hint was lost like any step |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Crash tests at each statement of the completion, duplicate completions, the timer and signal race at the activation window, lost hints, a failed compensation from code, restart with cold caches, a worker killed during a move; conformance in six SDKs; gaps registered |
| Delivery Constraints | Pass with conditions | Fails 4.7.0 (out of scope: "reimplementing Temporal or Restate"; baseline: "graph-native declarative IR"); passes 5.0.0 as owner decision 2026-11-24-2 states it; condition: Task 044-1 depends on the merge of 5.0.0. No new server technology; esbuild enters the TypeScript SDK's build-time dependencies (Decision 044-012) and a Java 21 artifact is added (Decision 044-014) |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A current code version that a verified move can change (principle IV) | Long-lived runs must be able to take fixed code; Temporal's automatic upgrade applies new code with no check | Never move: runs of months would hold their old workers for months, and a run under review could only be reset or terminated |
| A current code version beside the start version | Step, journal and compensation identities are derived from the start version and must not change | Re-pinning `workflow_version_id`: every derived identity of a moved run would stop matching |
| A reserved step `~workflow` with its own lease rules | The workflow task must be leased, fenced, hinted and reconciled like a step, without the review rule of activities | A separate table and claim path for workflow tasks: a second copy of the lease machinery |
| A keyed command chain on the server | Defense against an SDK, cache or input nondeterminism that would otherwise commit a divergent command | Trust the SDK's comparison alone, as Temporal does |
| Signals journaled for code runs while graph runs refuse them | A signal sent before the function waits would otherwise be lost | Refuse as for graphs; an inbox outside the journal, which replay could not see |
| Asynchronous updates for code runs | A handler may need an activity | Only synchronous updates: no handler could wait on an activity |
| Worker queries over ephemeral rows | Some answers exist only in the function's memory (journey 7) and must not grow the journal | Journal every query: unbounded history; declared state only: the loop of journey 7 would publish its counters at every iteration |
| A code profile per node kind | Expressions and publisher-only fields must not come from run-time code | The full node schema: the worker would choose classifications, egress and approval policies |
| A determinism runtime per language, analyzers for Go and C#, a Java 21 artifact, an esbuild dependency | Each language's sources of nondeterminism differ (Technical Plan 1) | One mechanism for all: none exists |

## Alternatives Considered

- **Replay in the engine (run the workflow code on the server, in a sandbox).** One place for replay, and queries
  without workers. Rejected: principle V forbids tenant code in the control plane, and it would need a server sandbox per
  language while the tenant already runs workers.
- **A second engine for code workflows (history and matching services as in Temporal).** Rejected: it would duplicate
  leases, retries, compensation and the uncertain-effect rules, and the one transaction of principle I would span two
  engines.
- **Commands as their own vocabulary instead of IR node fragments.** Smaller messages; rejected because every check,
  limit and picture of a node would exist twice and drift.
- **Temporal's automatic upgrade (runs replay on whatever code the workers run).** Rejected: it is the cause of
  nondeterminism in production the design exists to avoid.
- **A user-chosen build identifier instead of a computed digest.** Simpler for compiled languages; rejected because a
  reused identifier with different code is exactly the case that corrupts runs. The computed digest changes whenever the
  code, a dependency or the runtime changes.
- **Local activities.** Rejected for principle III (Technical Plan 10.3).
- **Graph-only, with a richer IR (loops, dynamic fan-out).** Rejected by owner decision 2026-11-24-3, and it would still
  not give code's types, functions and tests.
- **Deterministic execution through WebAssembly for all six languages.** One runtime; rejected because WebAssembly is
  not a mandatory runtime (constitution, Delivery Constraints), and Go, Java and C# workflow code would lose its ordinary
  tooling.
- **Hand-written compensation in code (Temporal's model).** Possible with the design as an author's choice; rejected as
  the platform's answer, because unknown outcomes would again be each author's problem.
- **A promotion replay check as its own mechanism.** Rejected: under pinning a promotion changes no open run; a dry-run
  move and the replay of exported histories answer the same question.

## Non-Functional Requirements

Non-functional requirements 044-001 to 044-007 are stated under Requirements. Their tests: Task 044-25 measures
Non-functional requirements 044-001, 044-002, 044-004 and 044-005 on the reference hosts (the hosts of the benchmark of
owner decision 2026-11-24-1, described in its report); the budget suite of Task 044-9 holds Non-functional requirement
044-003; the bounds of Non-functional requirement 044-006 are contract tests of Task 044-8; Non-functional requirement
044-007 is held by the security tests of Task 044-24.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Every task is test-first: its red tests are written and seen failing for the reason named before the code that makes
them pass, and the result is recorded in the register. Integration suites run on real PostgreSQL through Testcontainers.
The new targets are `contract_workflow_as_code`, `integration_workflow_as_code`, `fault_workflow_as_code` and
`security_workflow_as_code` in `tests/`; the conformance scenarios are listed in the Verification Plan. Documents change
first (constitution, Development Workflow, "Documentation first"): Task 044-1 lands before any code.

**Core slice and deferrable tasks.** The release needs the core slice: Tasks 044-1 to 044-13, 044-15 to 044-19, 044-21
(signals, updates, declared queries), 044-22, 044-24, 044-26, 044-28. The deferrable tasks can slip without reshaping a
contract, because their protocol fields and routes are reserved by Task 044-2: Task 044-14 (eager dispatch), the worker
queries of Task 044-29, Task 044-20 (patches in the SDKs after the first), Task 044-23 (Studio and CLI beyond the run
view), Task 044-25 (measurements, required before the budgets are published), Task 044-27 (each interaction when its
feature lands).

- [ ] T044-1 Write the documents the code will follow: the code mode in `docs/architecture/workflow-ir-and-sdk-modes.md`
  (the third row becomes "Workflow as code"), `docs/architecture/execution-semantics.md` (the workflow task, the
  activation window, the review causes), `docs/architecture/worker-runtime-protocol.md` (the new task kinds),
  `sdks/SPEC.md` (a section "Workflow as code"; the opening sentence that an SDK is not a workflow runtime becomes the
  three modes), `docs/sdk-compatibility.md`, and a quickstart per language under `docs/quickstarts/`, with the
  recommended topology for compiled languages and the deploy pipeline of journey 2.
  Depends on: the constitution 5.0.0 pull request (owner decision 2026-11-24-2) merged on `develop`.
  Verification: `python3 scripts/check-docs-index.py`, `python3 scripts/check-plain-words.py --base origin/develop`; a
  reviewer reads each page against this design.
  Refs: Requirement 044-001, Requirement 044-003, Requirement 044-011, Requirement 044-012
- [ ] T044-2 Contracts: `workflow.schema.json` (`oneOf` graph or code, the `code` object, the envelope, the code profiles
  of the node kinds), `worker.proto` and `worker.baseline.json` (Technical Plan 13, with the reserved fields of the
  deferrable tasks), `openapi.yaml` (the single-run move, the batch action, worker queries, update wait and status,
  history export, the namespace pause, the new errors), `events.asyncapi.yaml` (the workflow task hint), the error
  catalog, the permission `history:export` and its roles, and the register row of the new minor.
  Red tests: `contract_workflow_as_code::a_document_with_code_and_nodes_is_refused`,
  `contract_workflow_as_code::the_worker_protocol_change_is_additive_to_the_baseline`,
  `contract_workflow_as_code::the_openapi_routes_of_design_044_exist_with_their_errors_and_permissions`.
  Refs: Requirement 044-001, Requirement 044-003, Requirement 044-015, Requirement 044-017, Requirement 044-023, Requirement 044-025, Requirement 044-033, Non-functional requirement 044-006
- [ ] T044-3 Publication of a code version in `crates/infra-ir` and `crates/application/src/definitions.rs`: the `code`
  object, the digest form, the envelope (secrets per activity entry under feature 009's publisher check, the `script.` and
  `sagat.` prefixes refused, scripts filling `workflow_version_scripts`, approval policies), the declared signals,
  updates and queries with their disclosure; the registry requirement of Requirement 044-030.
  Red tests: `publication_refuses_a_malformed_digest`, `publication_refuses_an_envelope_secret_the_publisher_cannot_use`,
  `publication_refuses_an_envelope_activity_with_a_reserved_prefix`, `a_late_instance_without_the_code_capability_closes_its_gate`,
  `a_stored_code_version_reads_back_byte_identical`, `a_worker_query_without_a_closed_result_schema_is_refused`.
  Refs: Requirement 044-001, Requirement 044-013, Requirement 044-030
- [ ] T044-4 Domain: the code run state (the `~workflow` step's lifecycle, the pending flag, the command count, the visible
  count, the review causes), the commands, the command chain, the envelope check, the code profiles, and the planner arm
  that schedules the workflow task instead of following edges; pure, in `crates/domain/src/execution/code/`.
  Red tests (unit, `crates/domain`): `the_chain_of_the_golden_commands_is_the_golden_hash`,
  `a_command_outside_the_envelope_is_refused_with_its_member`, `an_activity_command_cannot_carry_a_secret_of_another_activity_entry`,
  `a_visible_event_of_a_code_run_sets_the_pending_flag`, `a_code_run_never_finishes_by_routing`,
  `activation_boundary_replays_every_interleaving_to_the_same_commands` (property test).
  Refs: Requirement 044-004, Requirement 044-005, Requirement 044-007, Requirement 044-008, Requirement 044-013
- [ ] T044-5 Command parsing in the code profile of each kind; the refusal of the kinds and fields outside it.
  Red tests: `every_command_kind_round_trips_through_its_code_profile`, `a_database_command_is_refused_as_not_supported_in_code`,
  one per excluded kind, and `a_fragment_with_an_expression_field_is_refused`, `a_fragment_with_destinations_is_refused`,
  `a_fragment_with_an_output_schema_is_refused`, `an_approval_with_an_inline_policy_is_refused`.
  Refs: Requirement 044-012, Requirement 044-013
- [ ] T044-6 Migration of Technical Plan 14 with the migration checklist.
  Red tests: `the_code_columns_are_all_set_or_all_null`, `sagat_app_cannot_update_code_version_id`,
  `the_move_function_refuses_a_version_of_another_workflow`, `the_down_migration_refuses_while_a_code_run_exists`, the
  privilege audit of `integration_platform_foundation`.
  Refs: Requirement 044-002, Requirement 044-004, Requirement 044-023
- [ ] T044-7 Run creation of a code run (seed, `~workflow` `READY` with its reserved activity name, hint) in every creator
  of design 043's table of creators, with the input gate; Signal-with-Start and Update-with-Start for code versions.
  Red tests: `starting_a_code_run_schedules_its_first_workflow_task_in_the_creating_transaction`,
  `a_code_run_gets_a_fresh_32_byte_seed`, `signal_with_start_journals_the_signal_of_a_code_run`.
  Refs: Requirement 044-002, Requirement 044-010, Requirement 044-014, Requirement 044-015
- [ ] T044-8 The workflow task lease in `crates/application/src/worker.rs` and `crates/api-grpc`: registration of workflow
  types under exact bindings, the reserved activity name, the claim through the existing path, sticky preference, the
  history projection and `ReadHistory`, the lease rules of Technical Plan 4.1, the bounds of Non-functional requirement
  044-006.
  Red tests: `workflow_tasks_follow_the_current_digest`, `a_star_binding_does_not_cover_a_workflow_type`,
  `an_activity_registration_naming_sagat_workflow_is_refused`, `a_workflow_task_lease_is_refused_by_every_activity_rpc`,
  `an_expired_workflow_task_is_released_with_the_next_generation_without_review`,
  `a_session_below_the_minor_is_never_offered_a_workflow_task`, `the_history_is_paged_at_1024_events_or_4_mib`,
  `a_revoked_binding_stops_read_history`.
  Refs: Requirement 044-002, Requirement 044-003, Non-functional requirement 044-006, Non-functional requirement 044-007
- [ ] T044-9 The batched completion transaction of Technical Plan 5.2 and the waking transactions of 5.3.
  Red tests: `a_completion_commits_all_or_nothing_at_each_statement` (fault target: the engine killed before each
  statement), `a_duplicate_completion_replays_its_outcome_without_eager_leases`, `a_stale_completion_is_refused_and_writes_nothing`,
  `a_completion_from_a_session_of_another_digest_is_stale`, `a_terminal_command_with_pending_events_is_refused_unhandled_events`,
  `three_unhandled_refusals_close_the_run_to_new_signals`, `the_completion_runs_at_most_10_statements_for_256_commands`
  (budget suite), `an_activity_completion_of_a_code_run_makes_the_workflow_task_ready_in_its_transaction`.
  Refs: Requirement 044-004, Requirement 044-005, Requirement 044-031, Non-functional requirement 044-003
- [ ] T044-10 The TypeScript runtime and API (`sdks/typescript`): the bundler and manifest command, the context without a
  host global, the string bridge, the replaced globals, the activation loop, the command comparison, the primitives, the
  replayer, the test environment.
  Red tests: `replay_of_the_golden_histories_makes_the_golden_commands`, `an_import_of_a_builtin_with_or_without_the_node_prefix_is_refused_at_bundling`,
  `the_context_global_is_not_a_host_object`, `date_now_is_the_workflow_time`, `promise_race_resolves_in_journal_order`,
  `an_infinite_loop_stops_at_the_activation_deadline`.
  Refs: Requirement 044-006, Requirement 044-010, Requirement 044-011, Requirement 044-025
- [ ] T044-11 Detection and report of nondeterminism and the server's review path for every cause of Technical Plan 6.3.
  Red tests: `a_completion_with_a_foreign_command_chain_is_refused_and_puts_the_run_under_review`,
  `a_changed_activity_input_on_replay_is_a_chain_mismatch`, `five_consecutive_failures_put_the_run_under_review_after_the_backoff`,
  `a_nondeterminism_puts_the_run_under_review_at_once_with_the_servers_expected_command`,
  `a_worker_supplied_detail_with_a_payload_is_dropped`.
  Refs: Requirement 044-006, Requirement 044-007, Requirement 044-008, Requirement 044-009, Non-functional requirement 044-007
- [ ] T044-12 The remedies: the review retry, reset of a code run to a workflow task boundary (same version at once,
  another version after a replay check, the seed kept), termination, the closing window of `HISTORY_LIMIT`; their audit.
  Red tests: `retry_ends_a_workflow_task_failed_review`, `retry_is_refused_for_a_nondeterminism_review`,
  `a_reset_of_a_code_run_keeps_its_seed`, `a_reset_to_another_version_is_replay_checked`,
  `terminating_a_run_under_review_applies_the_cancel_policies`, `the_history_limit_suggests_then_reviews_then_closes`.
  Refs: Requirement 044-008, Requirement 044-010, Requirement 044-027
- [ ] T044-13 Verified moves: the single-run route, the batch action, `REPLAY_CHECK` tasks with per-run read capabilities,
  the per-run transaction through the move function, the envelope refusal, the dry run, the permissions.
  Red tests: `a_move_applies_only_after_a_matching_replay`, `a_move_of_a_run_whose_visible_count_grew_is_rechecked_then_run_busy`,
  `a_move_waits_while_the_workflow_task_is_leased`, `a_mismatch_leaves_the_run_on_its_version_and_names_the_first_command`,
  `a_move_is_refused_when_an_outstanding_step_is_outside_the_target_envelope`, `a_moved_run_keeps_its_step_and_compensation_identities`,
  `a_move_without_version_activate_is_refused`, `an_all_open_move_without_run_batch_is_refused`,
  `a_dry_run_move_records_verdicts_and_moves_nothing`.
  Refs: Requirement 044-023, Requirement 044-026
- [ ] T044-14 Eager dispatch on the server and in the six SDKs (deferrable).
  Red tests: `eager_activity_leases_are_granted_in_the_completion_transaction`,
  `an_eager_workflow_task_is_returned_with_the_activity_completion_of_the_sticky_session`,
  `an_idempotent_replay_returns_no_lease`.
  Refs: Requirement 044-021, Non-functional requirement 044-002
- [ ] T044-15 The Python runtime (`sdks/python`): the deterministic loop, the import finder at load and during
  activations, the audit hook, the hash-seed precondition and fingerprint, the manifest command, the API, the replayer.
  Red tests: `registration_refuses_an_unset_or_random_hash_seed`, `a_module_level_import_of_uuid_is_refused_at_load`,
  `asyncio_sleep_is_a_durable_timer`, `open_is_refused_in_an_activation`, `replay_of_the_golden_histories_makes_the_golden_commands`.
  Refs: Requirement 044-006, Requirement 044-010, Requirement 044-011, Requirement 044-025
- [ ] T044-16 The Go runtime (`sdks/go`): the `iter.Pull` dispatcher, `workflow.Go`, `Channel`, `Selector`, `SortedKeys`,
  the `sagatvet` analyzer and its test helper, the manifest flag, the replayer.
  Red tests: `TestSagatvetRefusesGoStatementsSelectAndMapRange`, `TestSelectorResolvesInJournalOrder`,
  `TestABlockingNativeChannelStopsAtTheActivationDeadline`, `TestReplayOfTheGoldenHistories`.
  Refs: Requirement 044-006, Requirement 044-010, Requirement 044-011, Requirement 044-025
- [ ] T044-17 The C# runtime (`sdks/dotnet`): the scheduler, the invariant culture, the per-call scheduler check, the
  Roslyn analyzer package, the manifest flag, the replayer.
  Red tests: `ConfigureAwaitFalseBeforeAnSdkCallFailsTheWorkflowTask`, `TheAnalyzerFlagsDateTimeNowAndTaskRun`,
  `ReplayOfTheGoldenHistories`.
  Refs: Requirement 044-006, Requirement 044-010, Requirement 044-011, Requirement 044-025
- [ ] T044-18 The Java runtime (`sdks/java`, new artifact `sagat-workflows` for release 21): the permit dispatcher on
  virtual threads, the per-call thread check, the time zone and locale precondition, the manifest flag, the API, the
  replayer.
  Red tests: `aCallFromAForeignThreadFailsTheWorkflowTask`, `theArtifactRefusesToStartBelowJava21`,
  `registrationRefusesADefaultTimeZoneOtherThanUtc`, `replayOfTheGoldenHistories`.
  Refs: Requirement 044-006, Requirement 044-010, Requirement 044-011, Requirement 044-025
- [ ] T044-19 The Rust runtime (`sdks/rust`): the executor on threads without a Tokio runtime, the caught panic, the
  replacement of a stuck thread, the clippy fragment, the manifest flag, the API, the replayer; the root, `sdks/rust`,
  conformance and consumer lock files updated in one commit.
  Red tests: `a_tokio_sleep_in_a_workflow_fails_the_workflow_task`, `a_stuck_activation_thread_is_replaced_and_counted`,
  `replay_of_the_golden_histories`.
  Refs: Requirement 044-006, Requirement 044-010, Requirement 044-011, Requirement 044-025
- [ ] T044-20 Patches in the six SDKs, and the abandoned-activation cap.
  Red tests: in each SDK `patched_is_false_on_a_history_without_the_marker` and
  `four_abandoned_activations_withdraw_the_workflow_types`.
  Refs: Requirement 044-011, Requirement 044-024
- [ ] T044-21 Signals, updates and declared queries for code runs on the server: the journaled signal and its bounds, the
  update receipt lifecycle, its wait and its read rule, the validation of results, the declared state command.
  Red tests: `a_signal_before_the_function_waits_is_delivered_at_the_next_activation`, `an_undeclared_signal_is_refused_409`,
  `a_signal_beyond_the_backlog_is_refused_429`, `a_signal_at_the_headroom_is_refused_409`,
  `an_update_answers_202_when_the_handler_waits_longer_than_wait_ms`, `a_validator_that_issues_a_command_puts_the_run_under_review`,
  `an_update_result_outside_its_schema_is_refused`, `an_update_receipt_of_another_principal_is_not_found`,
  `a_declared_query_is_answered_with_no_worker`.
  Refs: Requirement 044-014, Requirement 044-015, Requirement 044-016
- [ ] T044-22 Children, continue-as-new and sagas from code on the server; the run-scoped saga plan (choosing between the
  existing run saga rows and a new table, recorded in the register); terminal commands with outstanding work;
  cancellation grace; outputs and the retained bytes.
  Red tests: `a_code_parent_starts_a_graph_child_and_sees_its_outcome`, `continue_as_new_with_version_active_pins_the_active_version`,
  `an_aborted_code_saga_compensates_in_reverse_completion_order`, `an_unknown_outcome_in_a_code_saga_waits_for_review`,
  `a_terminal_command_with_an_open_saga_is_refused`, `a_terminal_command_cancels_outstanding_timers_and_activities`,
  `a_cancelled_run_that_does_not_answer_ends_after_its_grace`, `a_code_step_output_beyond_the_retained_bytes_fails_the_step`.
  Refs: Requirement 044-018, Requirement 044-019, Requirement 044-020, Requirement 044-022, Requirement 044-031
- [ ] T044-23 Studio and command line: the run view of a code run (observed graph from its steps and journal,
  `Code-orchestrated`), the review banner and its remedies, the version list with open runs per digest, the moves, the
  namespace pause; `sagat workflow publish-code`, `history export`, `move`.
  Red tests: the Studio journey `a_code_run_under_review_shows_its_cause_and_its_remedies`, the command-line test
  `history_export_writes_json_lines_the_replayers_read`, `pausing_code_workflows_stops_workflow_tasks_and_keeps_runs`.
  Refs: Requirement 044-028, Requirement 044-033
- [ ] T044-24 Security tests and the server side of the history export: the export route with the redaction of feature
  016; tenant boundary on every new route, task, RPC and table; no secret in a workflow task; no payload or free text in
  a review, report, metric or log; the bounds of Requirement 044-032; the runtime role.
  Red tests: `security_workflow_as_code::another_tenants_run_history_is_not_found`,
  `security_workflow_as_code::a_history_export_redacts_classified_fields_without_a_reveal_grant`,
  `security_workflow_as_code::a_secret_field_is_never_exported`, `security_workflow_as_code::a_worker_query_of_another_tenant_is_not_found`,
  `security_workflow_as_code::release_sticky_of_another_session_changes_nothing`, `security_workflow_as_code::a_code_run_with_2049_outstanding_steps_is_refused`,
  the suites with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
  Refs: Requirement 044-003, Requirement 044-013, Requirement 044-025, Requirement 044-032, Non-functional requirement 044-007
- [ ] T044-25 Measurements on the reference worker host and the database host: replay cost per SDK, cache memory per SDK,
  latency, commits and statements; the comparison with Temporal for the benchmark of owner decision 2026-11-24-1.
  Verification: the report under `specs/044-workflow-as-code/evidence/` with environment, commands and failures; the
  budgets of Non-functional requirements 044-001 to 044-005 confirmed or revised in this design before release.
  Refs: Non-functional requirement 044-001, Non-functional requirement 044-002, Non-functional requirement 044-004, Non-functional requirement 044-005, Success criterion 044-003
- [ ] T044-26 The six-SDK conformance scenarios of the Verification Plan, with reviewed goldens.
  Verification: `cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored` with no selection filter, all
  scenarios passing in all six SDKs.
  Refs: Requirement 044-006, Requirement 044-009, Requirement 044-010, Requirement 044-014, Requirement 044-015, Requirement 044-016, Requirement 044-018, Requirement 044-019, Requirement 044-020, Requirement 044-022, Requirement 044-023, Requirement 044-024, Success criterion 044-001
- [ ] T044-27 The interactions of Technical Plan 12, each when its feature is built: the gates of feature 022, the
  routing of feature 019, the search attributes of feature 020 (with their command and event), the encoded payloads of
  feature 031.
  Red tests: `a_concurrency_key_on_a_code_activity_waits_for_its_grant`, `a_workflow_task_is_never_gated`,
  `a_deployment_does_not_route_workflow_tasks`, `an_encoded_signal_payload_reaches_the_function_unread_by_the_server`,
  `a_search_attribute_command_is_journaled_and_indexed`.
  Refs: Requirement 044-029
- [ ] T044-28 The fault suite of Success criterion 044-002: a changed function deployed while 100 runs are open, with and
  without a move; every worker restarted with cold caches; a worker killed during a move's replay.
  Red tests: `fault_workflow_as_code::a_deploy_without_a_move_changes_no_run`, `fault_workflow_as_code::a_move_moves_only_matching_runs`,
  `fault_workflow_as_code::cold_caches_replay_every_run_to_the_same_commands`.
  Refs: Requirement 044-023, Success criterion 044-002
- [ ] T044-29 Worker queries (deferrable): the route, the `QUERY` task, `RespondWorkerQuery`, the validation of the answer,
  the bound per namespace, in the server and the six SDKs.
  Red tests: `a_worker_query_is_answered_from_the_sticky_cache`, `a_worker_query_expires_without_a_journal_event`,
  `a_worker_query_answer_outside_its_schema_is_refused`, `a_query_handler_that_issues_a_command_fails_the_query`.
  Refs: Requirement 044-017, Requirement 044-032

#### Follow-up tasks

None yet.

### Verification Plan

**Build order.** Task 044-1 first, after the merge of constitution 5.0.0, then Task 044-2. Then two lanes that share no
file: the server lane (Task 044-3, then Tasks 044-4 and 044-5 together, Task 044-6, Task 044-7, Task 044-8, Task 044-9,
then Tasks 044-11 to 044-13, Task 044-21, Task 044-22, Task 044-24) and the TypeScript lane (Task 044-10 against a fake
server built from the contracts and the golden histories). When Task 044-9 and Task 044-10 are both done, the other five
SDK runtimes (Task 044-15 to Task 044-19) can be built in parallel, at most two heavy builds at a time on the shared
development machine (`AGENTS.md` rule 19), then Task 044-20. Task 044-14 and Task 044-29 after the core slice; Task 044-23
after the server lane; Task 044-26 when the six runtimes exist; Task 044-28 after Task 044-13; Task 044-25 last, on a
quiet machine and a separate database host; Task 044-27 as features 019, 020, 022 and 031 land.

**Quiet machine.** The budget suite and fault target of Task 044-9, the fault suite of Task 044-28, the measurements of
Task 044-25 and the full conformance run of Task 044-26 must run alone on the machine: they measure time or they start
six language toolchains and a PostgreSQL per language.

**Conformance scenarios (all six SDKs, one golden each):**

| Scenario | What it proves |
|---|---|
| `code-run-lifecycle` | Start, two activities with `Promise.all` or its equivalent, a timer, completion; the journal and the observed graph |
| `code-deterministic-primitives` | Time, 64 numbers, 8 bounded integers and 8 UUIDs of a fixed seed equal the golden |
| `code-signals` | A signal before the function waits, a repeated key, an undeclared name refused |
| `code-updates` | Validator, accept, reject, an asynchronous handler with an activity, `200` and `202` |
| `code-update-with-start` | Update-with-Start on a code version |
| `code-declared-state` | Declared queries with every worker stopped, and on the ended run |
| `code-worker-queries` | A worker query from the sticky cache and after a replay (with Task 044-29) |
| `code-graph-mixed` | A code parent, a graph child, a code grandchild |
| `code-continue-as-new` | Quiescence, transferred state, `version: active` |
| `code-saga` | A saga that completes, one that aborts and compensates in reverse order |
| `code-saga-uncertain` | An unknown outcome reconciled `applied`, and one left for review |
| `code-cancellation` | Cooperative cancellation, and the grace that ends the run |
| `code-failures` | A workflow failure ends the run; an uncaught exception puts it under review after five consecutive failed workflow tasks |
| `code-nondeterminism` | A changed function on replay: the report and the review |
| `code-patch-and-move` | A patched version moved; a run that does not match stays |
| `code-sticky-eviction` | A cache eviction and a replay from the start on another worker |
| `code-activation-deadline` | An infinite loop stopped with its cause |
| `code-history-paging` | A history of more than 1,024 events read in pages |

**Failure tests (constitution principle VIII).** A crash before each statement of a completion (Task 044-9); duplicate
completions and duplicate signals; the race of a timer and a signal at the activation window (the property test of Task
044-4 and an integration test with the completion paused at a barrier, never a sleep); a lost workflow task hint found by
the reconciler; a failed compensation from code (Task 044-22); every worker restarted with cold caches while 100 runs are
open, and a worker killed during a move's replay (Task 044-28).

**Gates.** `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`python3 scripts/check-architecture.py`, `python3 scripts/check-agent-config.py`, `python3 scripts/check-design-docs.py`,
the per-SDK checks of `sdks/AGENTS.md`, the four new test targets, the budget suite and the conformance suite.
