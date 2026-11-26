# Buffered signals

*   **Author:** Claude Code, for the owner (under Owner decision 2026-11-20-6 and Owner decision 2026-11-24-4)
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

Release 0.3.0 lets a workflow version declare signals that are kept when no step waits for them. Today a signal reaches a run
only if a `waitEvent` step of the run is already waiting for its name; otherwise `signalRun` answers `409` "the run is not waiting
for that signal" and the sender must try again later (`deliver_signal`, `crates/application/src/runs.rs`). With this design, a
version that declares a signal name keeps such a signal in a bounded inbox of the run, and the next wait of that name consumes it
when it is entered, in the same transaction. The owner put buffered signals in 0.3.0 (Owner decision 2026-11-24-1; the roadmap
assessment of 2026-11-11 names "buffered signals (an early signal is refused today)").

An author declares the names at the top of the version, beside `nodes`:

```json
"signals": [
  {"name": "approved", "maxPending": 1, "maxBytes": 16384},
  {"name": "order",    "maxPending": 50, "maxBytes": 65536}
]
```

What this gives, and how it differs from Temporal:

- **An early signal is kept.** "Send the approval before the workflow reaches the approval step" and a signal-driven work queue
  work for a version that asks for it. Every name it does not declare, and every version published before this design, behaves
  exactly as today, `409` included, so no caller and no run in flight sees a change.
- **One signal, one consumer.** A signal of a declared name satisfies exactly one wait: the one already waiting, or the next one
  that is entered. Temporal gives every signal to the workflow's code, which reads it from a channel; here a graph version's
  consumer is a `waitEvent` node of the root graph, and a code version's consumer (design 044) is the workflow function, which
  reads from the same inbox when it chooses.
- **Bounded by what the author declares.** `maxPending` per name, `maxBytes` per payload; publication refuses declarations whose
  bytes could exceed half of what a run may retain (2 MiB of its 4 MiB), so the other half is always left to step outputs and
  senders can never make a step fail for room (Requirement 054-007).
- **A lifecycle in the transitions that exist.** Continue-as-new moves pending signals to the successor; the end of a run discards
  them and journals how many; a reset rebuilds them and reapplies those that arrived after its point; Signal-with-Start buffers
  instead of refusing. Nothing expires silently and nothing is dropped without a journal record.
- **No new lock and no aggregate edge.** Every write is a row of the run, under the run's lock or in the transaction that creates
  the run.

The rules are [Architecture decision 0111](../adr/ADR-0111-a-version-may-declare-buffered-signals-kept-in-a-bounded-per-run-inbox.md),
revised with this design. It supersedes in part Architecture decision 0050 decision 4 and Architecture decision 0030 decision 17,
for versions that declare a buffered name and for nothing else. The register of decisions, gaps and questions is
`specs/054-buffered-signals/research.md` (Decision 054-001 onward). The principles cited by numeral are those of
[the constitution](../constitution.md).

### Terms

- **Signal; name; live delivery.** A signal is a named JSON payload of at most 256 KiB sent to a run (`POST
  /runs/{runId}/signals/{signalName}`), to a chain's head (`chain.signal`), by a batch item (feature 018) or with a start
  (Signal-with-Start). Its name is matched with the `eventType` of a root `waitEvent` node. *Live delivery* is today's path: a
  waiting step is satisfied in the signal's transaction, its output the payload (`wait.satisfied` with `via: signal`, or
  `via: signal_with_start` for the signal of a Signal-with-Start).
- **Declared name.** A name listed in the version's `signals`. Only declared names are buffered. The version's *level* for this
  design is 1 when it declares at least one name, 0 otherwise; publication records it (`workflow_versions.signal_inbox_level`).
- **Inbox; pending signal.** The inbox of a run is its rows in `run_signal_inbox`. A row exists exactly while its signal is
  pending: received, not yet consumed, moved or discarded. Each settlement deletes the row and is journaled.
- **The invariant.** A root wait of a declared name that is waiting and a pending signal of that name never exist in one run at
  one commit: a signal that finds the wait is delivered live, and a wait that finds the signal consumes it at its entry. Live
  delivery, consumption, the handoff's refusal of an entry wait, the reset's rebuild and the races all keep it. One test helper,
  `assert_inbox_invariant(tx)`, checks it after every commit of `integration_buffered_signals` and `fault_buffered_signals`
  (Requirement 054-020).
- **Sequence.** A pending signal is identified by the run's journal sequence of its `signal.buffered` event, so the inbox's order
  is the journal's.
- **Consumer.** For a graph version, a `waitEvent` node of the root graph whose `eventType` is the name (signals reach root waits
  only, today and here). For a code version, the workflow function (design 044, Technical Plan 10).
- **Entry of a wait.** The transition in which a root `waitEvent` step starts waiting: the planner's resolution of its
  correlation (`Evaluation::Correlation` in `crates/domain/src/execution/plan.rs`, resolved in the loop of `Engine::run` in
  `crates/application/src/execution.rs`), which journals `wait.started`.
- **The run lock; the chain lock.** Every transaction that changes a run locks its continuation chain, then the run's row, and
  holds both until it commits (Architecture decision 0030 decision 18). A *path transaction*, the transaction that advances one
  instance inside a collection or loop body, takes the same run row lock (`lock_path_revision`,
  `crates/infra-postgres/src/execution.rs`).
- **Run segment; continuation.** A continue-as-new ends a run and starts its successor in one chain (feature 014, Architecture
  decision 0030); each run of the chain is a segment. The handoff is the transaction that does it.
- **Business identifier; open holder; binding-then-run order.** Signal-with-Start names a business identifier (feature 018); the
  open run bound to it, if any, is its *holder*. Its transaction locks the identifier's binding row, then the run (slots 3 and 3R
  of Architecture decision 0050 decision 7).
- **Reset; point; prefix; reapply.** A reset (feature 018) creates a new run from a source run's journal up to a chosen sequence,
  the *point*; the events up to it are the *prefix*, re-emitted in the new run. With reapply `SIGNALS`, the signals the source
  received after the point are sent again to the new run. Each event type has a reset rule in feature 018's registry
  (`specs/018-schedules-bulk-operations/contracts/journal-and-events.md`): `COPY` (re-emitted), `REMATERIALIZE` (re-emitted, and
  its side rows re-created), `EXCLUDE` (a point after it is not offered).
- **Recorded no-op.** A transition that loses a race commits nothing of its own and is counted or audited, as today (principle I).
- **Closed gate.** Once the operator records a fleet requirement, a process that lacks the capability serves nothing
  (Architecture decision 0082).
- **Retained bytes.** What a run may keep: today the sum of its step outputs, at most 4 MiB (`MAX_RUN_OUTPUT_BYTES`,
  `crates/application/src/execution.rs`), because every lease carries the run's data. The sum is kept in a ledger,
  `run_instance_usage.logical_output_bytes`, under a check of 4,194,304 (migration `20261204000000_logical_output_byte_ledger`), and
  read through `RunTx::logical_output_bytes()`. With this design, pending payloads count too (Requirement 054-008).

## The Problem

A signal that arrives one millisecond before its wait is refused. The sender cannot tell whether "not waiting" means "too early",
"too late" or "never": it must poll, retry and guess, and a person who approves a request before the workflow reaches its approval
step sees an error. Three Temporal patterns cannot be written at all:

- **Early approval.** A run computes for ten minutes, then waits for `approved`; the approver answers after two minutes.
- **A signal-driven queue.** Orders arrive as `order` signals and are handled one at a time; orders that arrive while one is handled
  must wait their turn, and continuing as new must not lose the orders still waiting. A graph's root is acyclic (`check_acyclic`,
  `crates/domain/src/workflow.rs`), so a graph version handles one `order` per run segment and continues as new for the next (CUJ
  2); a code version (design 044) loops over its channel in one run.
- **Signal-with-Start for a workflow whose entry does some work first.** Today Signal-with-Start rolls back with
  `SIGNAL_NOT_ACCEPTED_AT_START` unless the entry waits for the signal (Architecture decision 0050, Consequences), so a "shopping
  cart" whose entry loads the cart cannot be started by its first `addItem`.

Feature 018 left buffered signals out of scope (`specs/018-schedules-bulk-operations/spec.md`, Out of scope; Requirements FR-018-024
and FR-018-039), on Architecture decision 0050 decision 4, which rejected them because they need "a lifecycle for undelivered
signals (expiry, transfer across continuation, quotas)" and change what a `409` means. This design gives that lifecycle and keeps
the `409` for everything that does not opt in. Design 044 (workflow as code) needs the same inbox: a workflow function reads signals
when it chooses (Gap 044-017 of `specs/044-workflow-as-code/research.md`).

## Requirements

### Functional

- **Requirement 054-001 — Declarations.** A workflow document MAY carry a top-level `signals` array of 1 to 32 entries, each
  `{"name", "maxPending", "maxBytes", "buffered"}`: `name` an event type (`$defs/name` of the IR schema, the grammar of
  `EventType`: ASCII letters, digits, `_`, `.`, `:` and `-`, at most 128 characters), unique in the array; `maxPending` an integer
  from 1 to 1,000; `maxBytes` an integer from 1 to 262,144, default 65,536; `buffered` optional and, when present, `true` (a listed
  name is buffered; the field is accepted so that design 044's documents, which write it, are valid; Decision 054-009).
  Publication MUST refuse, with the field's JSON pointer and a reason: a duplicated name (`SIGNAL_DUPLICATED`); a sum of
  `maxPending` over the entries above 1,000 (`SIGNAL_BOUNDS_OVER_RUN`); a sum of `maxPending × maxBytes` over the entries above
  2,097,152, half of a run's retained bytes (`SIGNAL_BYTES_OVER_RUN`); in a graph document, a name that is not the `eventType` of at least one `waitEvent` node of
  the root graph (`SIGNAL_WITHOUT_CONSUMER`); in a graph document that has a `continueAsNew` node, an entrypoint that is a
  `waitEvent` of a declared name (`SIGNAL_ENTRY_WAIT_CONTINUES`: a continuation's handoff opens such a wait without the planner,
  Technical Plan 7; an author puts an activity first; Decision 054-013). A code document (design 044) declares its signals in the same shape, with the
  additions design 044 gives them. *Acceptance:* a publication test per refusal and per boundary value, the IR schema test, the
  Studio's form test.
- **Requirement 054-002 — The level and the fleet requirement.** Publication MUST record the version's level (Terms). A version of
  level 1 MUST publish only once the operator recorded the requirement `signal-inbox-v1` for control planes and engines
  (Architecture decision 0082); until then publication answers `422` with the reason `CAPABILITY_NOT_ACTIVE` and the capability in
  the problem's detail, the one reason the shared hold gives designs 044, 045 and 054 (Task 054-18; Gap 054-007). Publication holds
  the requirement until its transaction commits (`platform_requirement_hold`, Technical Plan 15). The requirement MUST NOT be removed
  while a workflow version of level 1 exists, retired included. *Acceptance:* the registry tests and the released 0.2 scenario.
- **Requirement 054-003 — Undeclared names are unchanged.** A signal whose name the run's version does not declare, and every signal
  to a run of a version of level 0, MUST be handled exactly as today: delivered to every waiting root step of that name, a copy each,
  or refused `409` (`NOT_WAITING`); Signal-with-Start keeps `SIGNAL_NOT_ACCEPTED_AT_START`. This is a rule of graph runs: a code run
  has no live delivery, and design 044 refuses a signal its version does not declare (`409 SIGNAL_NOT_DECLARED`). *Acceptance:* the
  existing signal tests of `integration_durable_execution` and `integration_schedules_bulk_operations` pass unchanged.
- **Requirement 054-004 — One consumer per signal of a declared name.** A signal of a declared name to a graph run MUST satisfy
  exactly one wait. When one or more root `waitEvent` steps of that name wait, it MUST be delivered live to the one with the lowest
  node id, and to no other; the planner enters waits in that order too (it plans root steps in node-id order), so one rule decides
  live delivery and consumption. A root graph is acyclic, so two waits of one name wait at once only in parallel branches. When none waits, it MUST be buffered (Requirement 054-005). The name's `maxBytes` applies to every
  signal of a declared name, delivered live or buffered (`SIGNAL_TOO_LARGE`), so that acceptance never depends on timing. To a code
  run every signal of a declared name is buffered (Technical Plan 10). *Acceptance:* unit tests of the choice; an integration test with two
  waiting steps of one name.
- **Requirement 054-005 — Buffering.** A signal of a declared name that no step consumes MUST be checked, in the run's transaction
  under the run lock, in this order, after authentication, authorization and the payload's JSON and 256 KiB checks of today: the
  idempotency key, whose repeated use answers the first answer (`buffered` included) before any bound; a payload above the name's
  `maxBytes`, refused as an oversized payload is today (`VALIDATION_FAILED`) with the reason `SIGNAL_TOO_LARGE` (checked for live
  delivery too, Requirement 054-004); a run that has ended or is stopping, refused `409` with the reason `RUN_ENDED` or
  `RUN_STOPPING`; for a graph run, a name that nothing can still consume, refused `409` with the reason `NO_CONSUMER_AHEAD`, the
  honest "too late". The predicate is over the version's nodes: a root `waitEvent` node of the name is a *consumer ahead* when the
  run has no step for it yet (it has not been routed, which includes a branch the run may never take) or its step is `PENDING`;
  the refusal applies only when no node of the name is a consumer ahead and the version has no `continueAsNew` node. So an early
  signal (CUJs 1 and 3) is kept, and a signal whose only wait sits on a branch not taken is kept until the run ends and is then
  discarded and counted; the refusal is never a guess;
  a name that already holds `maxPending` pending signals, refused `409` with the reason `SIGNAL_INBOX_FULL`; retained bytes that the payload would take above 4 MiB,
  refused `429 QUOTA_EXCEEDED` as today; the tenant's content quota (Architecture decision 0027), as today. A signal that passes is
  stored, journaled `signal.buffered`, audited `run.signalled` and answered `202` with `deliveredTo: []`, `buffered: true` and its
  `sequence`, all in one transaction. A refusal commits nothing. *Acceptance:* a test per refusal and its order; the idempotency
  test; the transaction test that a failure after the inbox write leaves no row.
- **Requirement 054-006 — Consumption at a wait's entry.** In the transaction that enters a root `waitEvent` of a declared name,
  when the run has pending signals, the store MUST read only the oldest pending signal of that name, through the inbox's index, and,
  if there is one, the wait MUST be satisfied in that transaction: `wait.started` then `wait.satisfied` with `via: buffered` and the
  signal's sequence; the payload becomes the step's output by its digest; the row is deleted. Each pending signal MUST be consumed by
  at most one wait, oldest first, in the order in which the commits enter waits. A run with no pending signal MUST read nothing more.
  *Acceptance:* unit tests of the planner; an integration test per entry path (a worker's completion, a timer, a condition, the
  run's creation, a continuation's successor); the budget line of Non-functional requirement 054-001.
- **Requirement 054-007 — The bound an author can rely on.** The guide and the Studio MUST state the bound this way: a run of a
  graph version keeps up to `maxPending` pending signals of a name, each of at most `maxBytes`; publication keeps the declared bytes
  within half of what a run may retain (2 MiB), so pending signals never take room from step outputs below 2 MiB, and at run time a
  signal within the bounds is refused only when the run's outputs already hold more than the other half (`429`). With `maxBytes` of
  262,144 a version can declare at most 8 pending signals in all (2,097,152 / 262,144); with the default of 65,536, at most 32. A
  step's output that would pass the 4 MiB still fails `RUN_DATA_LIMIT`, as today; pending signals can no longer cause it. For a code
  run, design 044's headroom rules (`HISTORY_HEADROOM` from 3 MiB) also apply, and its guide says so (Gap 054-008). One exception,
  stated in the guide: a code run continued under another version keeps every moved signal of a name that version declares, even
  above its `maxPending` or `maxBytes`; new signals of that name are refused until its count is below `maxPending` (the moved bytes
  stay within the 2 MiB the predecessor's version allowed). *Acceptance:* the guide page and the Studio's message, reviewed in Tasks 054-14 and 054-15.
- **Requirement 054-008 — Pending payloads count in the retained bytes.** Every check of the 4 MiB bound MUST count the run's
  pending payloads with its step outputs, so that consuming a pending signal never takes a run over the bound. They are charged to
  the run's existing byte ledger (`run_instance_usage`, Technical Plan 6), whose check enforces the sum in the database.
  *Acceptance:* a test per check site that a run with pending payloads refuses an output that fits without them; the ledger's
  check refusing a write past 4 MiB.
- **Requirement 054-009 — Precedence over the look-back of events.** A root `waitEvent` that declares `lookbackMs` and whose
  `eventType` is a declared name MUST consume a pending signal at its entry before any event is considered. The look-back of
  feature 004 (`steps.event_since`, the `late_waits` scan of `crates/scheduler/src/events.rs`) acts only on a wait still waiting
  after its entry committed. When the inbox had none, the first commit among a live signal, a live event, the look-back scan's
  delivery and the wait's timeout wins, as today. *Acceptance:* a test with a pending signal and a matching event older than the
  wait; a test with an empty inbox where the look-back wins.
- **Requirement 054-010 — Continue-as-new.** In the continuation's handoff transaction (chain lock, then run lock), pending signals
  whose name the successor's version declares MUST move to the successor in their order, each journaled in the successor as
  `signal.buffered` with `movedFrom` (the predecessor and its sequence), and the rest MUST be discarded; the predecessor journals
  `signal.moved` with the count and the successor, and `signal.discarded` with the count and the cause `not_declared` when any was
  discarded. *Acceptance:* an integration test per case; the replay test of both runs.
- **Requirement 054-011 — The end of a run.** The transaction that ends a run, on every path that ends one (the planner's and the
  store-side paths of Technical Plan 8), MUST discard every pending signal and journal
  `signal.discarded` with the count and the cause `run_ended`, before `run.finished`, when the count is above zero. *Acceptance:* a
  test per way a run ends (completed, failed, cancelled, compensated, partial).
- **Requirement 054-012 — Reset.** A reset MUST rebuild the new run's inbox from the prefix (a signal buffered before the point and
  not consumed, moved or discarded before it is pending in the new run, with its sequence) and, with reapply `SIGNALS`, MUST treat a
  `signal.buffered` after the point that is not a move as a recorded signal, in journal order with the `wait.satisfied` events of
  `via: signal`: delivered, or buffered when the new run's version declares its name. A `wait.satisfied` with `via: buffered` is not
  a candidate, because its signal is its `signal.buffered`. A candidate that can be neither ends the reapply of signals, as today.
  *Acceptance:* the reset tests of Task 054-10; the registry test (every event type has a rule).
- **Requirement 054-013 — Signal-with-Start.** For a declared name, a request that creates a run MUST deliver or buffer the signal
  in the creation transaction and MUST NOT roll back with `SIGNAL_NOT_ACCEPTED_AT_START`; a refusal of Requirement 054-005 rolls the
  creation back. An open holder is signalled as `signalRun` is. The binding-then-run lock order of Architecture decision 0050
  decision 7 MUST stay as it is. *Acceptance:* the with-start tests and the slot-order test of Task 054-11.
- **Requirement 054-014 — Payloads.** A pending payload MUST be stored as run content in `payloads` (content-addressed, charged to
  the tenant's content quota) and referenced by its row. A read MUST NOT return a pending payload. Once consumed it is the step's
  output, classified by the `outputSchema` of the `waitEvent` that consumed it (feature 016), as a live signal's is today. The rows
  of a run MUST be deleted with the run (feature 020's purge); a discarded payload follows feature 020's rule for every payload no
  row references. *Acceptance:* the read tests of Task 054-12; the privilege audit.
- **Requirement 054-015 — Only run rows.** Every write of this design MUST be a row of the run it concerns, under its run lock, or a
  row of a run being created in its creation or handoff transaction. No transition driven by a run MUST lock or write a schedule,
  business-id, batch job, batch item, obligation or pending-reset row (Architecture decision 0050 decision 3). *Acceptance:* the
  run-driven barrier test of feature 018 (`run_driven_locks.rs`) with a pending signal.
- **Requirement 054-016 — Visibility.** `GET /runs/{runId}/signals` MUST list a run's pending signals (sequence, name, bytes,
  received time, `movedFrom`), never a payload, under `run:read`, paged by sequence; `Run` gains `pendingSignals`; `SignalResult`
  gains `buffered` and `sequence`. The Studio shows the pending signals of a run and the declarations of a version with the bound of
  Requirement 054-007; the six SDK clients return `buffered` and list pending signals. *Acceptance:*
  `contract_rest_durable_execution`, the Studio's tests, each SDK's unit tests, the conformance scenario `buffered-signals`.
- **Requirement 054-017 — Code versions.** The inbox MUST serve design 044: a code run's signal of a declared name is always
  buffered; the store consumes pending signals by their sequences in a completion's transaction, refusing a sequence that is not a
  pending signal of that run; the lifecycle of Requirements 054-010 to 054-012 applies. *Acceptance:* the store tests of Task 054-6
  here, and design 044's scenarios `code-signals` and `code-signal-at-return` there.
- **Requirement 054-018 — Races.** Each race of Technical Plan 12 MUST resolve as its row says, and be tested with a barrier, never
  a sleep (AGENTS.md rule 18, "races are tested with held locks or barriers, never with sleeps"). *Acceptance:*
  `fault_buffered_signals`.
- **Requirement 054-019 — Telemetry and audit.** The metrics, spans, logs and audit records of Technical Plan 14 MUST carry closed
  labels only: no tenant, namespace, run, signal name or payload as a label or in a log line. *Acceptance:* the metrics text test of
  `integration_buffered_signals`.
- **Requirement 054-020 — Journal and replay.** The events of Technical Plan 11 are written only for runs of a version of level 1;
  the invariant of Terms MUST hold after every commit (checked by `assert_inbox_invariant(tx)` after each commit of the two suites,
  not by the replay, whose events do not mark commits); a run's replay MUST
  equal its projection (the pending bytes derived by joining each settled sequence to its `signal.buffered`), the inbox rows, `runs.pending_signals` and the ledger's
  `pending_signal_bytes` included. *Acceptance:* the
  replay-equals-projection corpus of `integration_buffered_signals`.

### Non-functional

- **Non-functional requirement 054-001 — Nothing for a run without pending signals.** A lease, a completion and every other
  transition of a run whose `runs.pending_signals` is zero keep their statement counts (the lines of
  `tests/integration/claim-transition-efficiency/work_budget.rs`, unchanged); the count and the bytes are written by the statements that write
  `runs` today. A wait's entry that consumes costs two statements (the read-and-delete through the index, and the ledger's move of the bytes);
  one that finds nothing costs the read.
- **Non-functional requirement 054-002 — Bounded lifecycle work.** A continuation or an end of a run with pending signals adds at
  most three statements (the successor's rows, the deletion, the journal rows) and one update of the successor's byte ledger,
  whatever the count; the rows and journal events they write grow with the count, at most 1,000 each. With none pending, it adds
  nothing.
- **Non-functional requirement 054-003 — Isolation.** One new table, keyed by `tenant_id`; the runtime role `sagat_app` gets
  `SELECT`, `INSERT` and `DELETE` on it and nothing else; one `EXECUTE` grant on the shared hold function.
- **Non-functional requirement 054-004 — Compatibility.** The IR grows by one optional top-level field, read only for versions of
  level 1; the REST contract grows by one route and optional fields; the worker protocol does not change; a version of level 0 runs
  as today.

### Success criteria

- **Success criterion 054-001.** CUJ 1 runs in six languages: an approval sent before its step is reached completes the run
  without a retry by the sender.
- **Success criterion 054-002.** A chain of CUJ 2 that handles 500 `order` signals, one per segment, consumes each exactly once,
  in order, and its chain's journals account for every one (consumed, moved or discarded).
- **Success criterion 054-003.** Every race of Technical Plan 12 resolves as stated under its barrier test, also as the runtime role.
- **Success criterion 054-004.** Non-functional requirements 054-001 and 054-002 measured and reported.

### User stories

- **User story 054-01.** As a workflow author, I let a signal arrive before its step and know how many are kept (CUJs 1 and 2).
- **User story 054-02.** As a client developer, I start a workflow with its first signal even when its entry does some work first
  (CUJ 3).
- **User story 054-03.** As an operator, I see which signals a run holds and how many it discarded (CUJ 4).
- **User story 054-04.** As the author of a code workflow (design 044), I read signals from a channel when my code chooses (CUJ 5).

## Goals

- Temporal's early-signal behaviour for every version that asks for it, with one consumer per signal.
- No change for runs in flight, for versions that do not declare, or for callers who rely on today's `409`.
- One inbox for graph and code versions, with one lifecycle.
- No new lock, no aggregate edge, and no new statement for a run whose inbox is empty.

## Non-Goals

- **Buffering every signal of every run** (Temporal's default). It changes the answer of `signalRun` for runs in flight (principle
  IV); a version opts in instead (Decision 054-002).
- **An expiry per signal.** A signal lives as long as its run; a bounded count and a bounded number of bytes are the bounds
  (Decision 054-005).
- **Body waits as consumers.** Signals reach root waits only, today and here; a wait inside a collection or a loop body is woken by
  events (Gap 054-002).
- **Validating a graph signal's payload against a schema.** Design 044 declares a schema for a code version's signals; for a graph
  version the consuming wait's `outputSchema` classifies the output, as today (Gap 054-003).
- **Signals to a run inside its start delay.** Refused as today (feature 018); Signal-with-Start keeps `START_DELAY_WITH_SIGNAL`
  (Gap 054-004).
- **Reading or deleting a pending signal by a person.** The list shows metadata only; an operator who must drop a signal cancels or
  resets the run (Gap 054-005).

## Open Questions

None. The questions raised while designing are decided in `specs/054-buffered-signals/research.md` (Decisions 054-001 to 054-014; Gaps 054-001 to 054-010).

## Owner decisions needed

None open. Tasks 054-3 (the reset rules of the new event types), 054-7 (the byte checks of collections, loops and the event wake),
054-8 (the chain-addressed signal path and batch `SIGNAL` items), 054-9 (the handoffs, the store-side ends of chains, collections
and loops, the scheduler's collection passes), 054-10, 054-11 and 054-18 (the floor of Architecture decision 0098) change code that
features 004 (the event wake), 014 (continuation, collections, loops) and 018 (Signal-with-Start, reset, batch items) and
Architecture decision 0098 own, and Task 054-1 changes their contracts and feature 004's signal rule; they are gated by Owner decision
2026-11-26-540, a delegate's row under Owner decision 2026-11-24-4 whose `Covers` names them. The owner's decisions this design rests
on: Owner decision 2026-11-24-1 (the scope), Owner decisions 2026-11-20-6 and 2026-11-24-4 (the delegation of its questions, of
its architecture decision and of its approval; the coordinating session's decision that buffered signals are design 054 and that
superseding Architecture decision 0050 decision 4 in part is within the delegation), and Owner decision 2026-11-25-2 (the approval
rule of 0.3.0 designs) and Owner decision 2026-11-20-8 (documentation first). For the record, not open: each question below was
decided under that delegation, with its options and reasoning in the register:

- Feature 004's look-back as the substrate, or an inbox of the run? An inbox of the run; the inbox is read first (Decision 054-001).
- Opt in per version and name, or buffer everything? Opt in (Decision 054-002).
- Copies to every waiting step, or one consumer? One consumer, for declared names (Decision 054-003).
- Keep settled rows with a state, or keep only pending rows? Only pending rows (Decision 054-004).
- Bounds: three runtime bounds, or declared bounds checked at publication? Declared bounds (Decision 054-005).
- At continue-as-new: discard, refuse, or move? Move what the successor declares, discard the rest (Decision 054-006).

## Critical User Journeys (CUJs)

### CUJ 1: The approval arrives first

*   **Given**: a version declaring `{"name": "approved", "maxPending": 1, "maxBytes": 16384}`, whose run computes a quote for ten
    minutes, then enters `waitEvent` `approved` (timeout one day), on a fleet that recorded `signal-inbox-v1`.
*   **When**: the approver's client sends `approved` at minute two.
*   **Then**: the answer is `202` with `buffered: true` and a sequence; the run's page shows one pending signal; at minute ten the
    wait is satisfied in its entry transaction (`wait.satisfied`, `via: buffered`) and the run goes on; a second `approved` sent
    while the first is pending is refused `409 SIGNAL_INBOX_FULL`, and the client knows the first is kept.

### CUJ 2: A queue of orders across continuations

*   **Given**: a version declaring `{"name": "order", "maxPending": 50, "maxBytes": 65536}` whose root graph is the activity
    `prepare` (a continuation's entry is an activity), then `waitEvent` `order`, then `handleOrder`, then a continue-as-new; each
    run segment handles one order.
*   **When**: orders arrive faster than they are handled, and a segment continues as new with 12 pending.
*   **Then**: each order is consumed once, oldest first; the 12 move to the successor in their order (`signal.moved` in the
    predecessor, twelve `signal.buffered` with `movedFrom` in the successor), and the successor's wait consumes the oldest at its
    entry; the 51st pending order is refused `409 SIGNAL_INBOX_FULL` and the sender retries later. A code version (design 044)
    handles the same queue in one run, reading its channel in a loop.

### CUJ 3: Signal-with-Start of a shopping cart

*   **Given**: a version whose entry is the activity `loadCart`, followed by `waitEvent` `addItem`, declaring `addItem`.
*   **When**: a client calls Signal-with-Start with the business identifier `cart-42` and the signal `addItem`.
*   **Then**: the run is created and the signal buffered in one transaction (`202`, `started: true`, `buffered: true`); after
    `loadCart` completes, the wait consumes it. Today the same request rolls back with `SIGNAL_NOT_ACCEPTED_AT_START`.

### CUJ 4: An operator explains a lost message

*   **Given**: a run with three pending `order` signals.
*   **When**: an operator cancels the run.
*   **Then**: the cancellation's terminal transaction journals `signal.discarded` with count 3 and cause `run_ended`; the run's page
    says "3 pending signals were discarded when the run ended"; `GET /runs/{runId}/signals` lists none.

### CUJ 5: A code workflow reads a channel

*   **Given**: a code version of design 044 that declares `paid` and handles it with `ctx.onSignal("paid", ...)` only after an
    activity.
*   **When**: `paid` arrives while the activity runs.
*   **Then**: it is buffered here and delivered to the function at its next activation (design 044); the completion that handles it
    consumes it by its sequence (Requirement 054-017).

## Technical Plan (Architecture)

### 1. The IR and publication

Additive to `specs/001-platform-foundation/contracts/workflow.schema.json` (Architecture decision 0032). The root object is closed
(`additionalProperties: false`), so a reader built before this design refuses a document that carries `signals`; that is why
publication waits for the fleet requirement (Requirement 054-002), and why no stored document can carry the key at level 0. The new
root property:

```json
"signals": {
  "type": "array", "minItems": 1, "maxItems": 32,
  "items": {
    "type": "object", "additionalProperties": false,
    "required": ["name", "maxPending"],
    "properties": {
      "name": {"$ref": "#/$defs/name"},
      "buffered": {"const": true},
      "maxPending": {"type": "integer", "minimum": 1, "maximum": 1000},
      "maxBytes": {"type": "integer", "minimum": 1, "maximum": 262144, "default": 65536}
    }
  }
}
```

A name is parsed as an `EventType` (`crates/domain/src/workflow.rs`), the type a `waitEvent`'s `eventType` already has. The
other refusals of Requirement 054-001 are the domain's: `SignalDeclarations` in `crates/domain/src/workflow.rs` (a new type beside
`WorkflowDefinition`, without `serde_json`), built by `infra-ir`'s parser, validated with the definition, and holding for each name
its `max_pending` and `max_bytes`. `SIGNAL_WITHOUT_CONSUMER` is checked against the root graph's `waitEvent` nodes only.

Publication (`publish` in `crates/application/src/definitions.rs`, reached by `publish_through` and `prepare`, so by promotions and
approved requests) computes the level, holds `signal-inbox-v1` for a version of level 1 (Technical Plan 15), and writes the level in
the statement that inserts the version; the ad hoc insert of `crates/infra-postgres/src/execution/ad_hoc.rs` writes 0. A republication
of the same bytes reuses the stored version, whose level is already right, because no document stored before this design can carry
`signals`.

### 2. The inbox, its counters and its migration

```sql
CREATE TABLE run_signal_inbox (
    tenant_id      uuid        NOT NULL,
    run_id         uuid        NOT NULL,
    sequence       bigint      NOT NULL,
    name           text        NOT NULL,
    payload_digest bytea       NOT NULL,
    bytes          integer     NOT NULL,
    received_at    timestamptz NOT NULL,
    version_id     uuid        NOT NULL,  -- the run's version when the signal was buffered; kept by a move and a reset
    moved_from_run uuid        NULL,
    moved_from_seq bigint      NULL,
    PRIMARY KEY (tenant_id, run_id, sequence),
    CONSTRAINT run_signal_inbox_run_fk FOREIGN KEY (tenant_id, run_id) REFERENCES runs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT run_signal_inbox_payload_fk FOREIGN KEY (tenant_id, payload_digest) REFERENCES payloads (tenant_id, digest),
    CONSTRAINT run_signal_inbox_name_size CHECK (octet_length(name) BETWEEN 1 AND 128),
    CONSTRAINT run_signal_inbox_bytes_range CHECK (bytes BETWEEN 0 AND 262144),
    CONSTRAINT run_signal_inbox_sequence_positive CHECK (sequence > 0),
    CONSTRAINT run_signal_inbox_moved_consistent CHECK ((moved_from_run IS NULL) = (moved_from_seq IS NULL))
);
CREATE INDEX run_signal_inbox_name_idx ON run_signal_inbox (tenant_id, run_id, name, sequence);
```

Every row is pending (Decision 054-004): a consumption, a move and a discard delete their rows, and the journal records each. So
the index needs no predicate: it holds only pending rows, and the oldest pending signal of a name is the first entry of its range.
The table holds at most 1,000 rows per run (Requirement 054-001), so it stays as large as the live backlog, not as the history.

`runs` gains `pending_signals integer NOT NULL DEFAULT 0`, with `CONSTRAINT runs_pending_signals_range CHECK (pending_signals
BETWEEN 0 AND 1000) NOT VALID` added separately (an inline check cannot be `NOT VALID`; every check of this design is added that
way), set by the statement that
already updates the run's row in the transaction that journals a change of the inbox (its `run_version` update), so a transition
of a run with an empty inbox reads it with the run's row and writes nothing more. The run's byte ledger, `run_instance_usage`,
gains the sibling column `pending_signal_bytes bigint NOT NULL DEFAULT 0`, and its check is replaced (`NOT VALID`) by `logical_output_bytes >= 0 AND
pending_signal_bytes >= 0 AND logical_output_bytes + pending_signal_bytes <= 4194304` (Technical Plan 6). `RunState` gains `pending_signals` (a count, restored from
the row), which is all the domain needs to know: whether a wait's entry must ask the store (Technical Plan 4).

`workflow_versions` gains `signal_inbox_level smallint NOT NULL DEFAULT 0` with its check `IN (0, 1)` added `NOT VALID`, and a partial index
`ON workflow_versions (signal_inbox_level) WHERE signal_inbox_level > 0` for the floor of Technical Plan 15.

**One migration of this design and its companion that validates** in `crates/infra-postgres/migrations/` (beside the shared
prerequisite of Task 054-18, a migration of its own built only if designs 044 and 045 have not landed it), timestamped after the latest migration of `develop` and of the branches it
composes with when it merges (design 045's and design 044's), built by the checklist of `.claude/skills/migration`: the table and
its index; the new columns (metadata-only, since each has a constant default) and their checks, the check on `runs` of Technical
Plan 8 and the ledger's replaced check, all added `NOT VALID` so that no table is scanned under `ACCESS EXCLUSIVE`, and validated
by the companion migration under `SHARE UPDATE EXCLUSIVE`, which does not block writes (design 043's pair
`20270118000000`/`20270118000100` is the pattern; the lock impact is written in the migration, item 8 of the skill); the capability `signal-inbox-v1` in the registry's three closed lists (`serving_instances_capabilities_known`,
`platform_requirements_capability_known`, `platform_requirement_history_capability_known`, replaced as design 043's migration did,
restating the lists in force); the comparison guard of the hold function (Technical Plan 15); the floor's row and its function; the grants
(`SELECT, INSERT, DELETE ON run_signal_inbox TO sagat_app`); and the storage classification of each variable-size column in
`crates/infra-postgres/storage-classification.json` (`name` as `content`; `payload_digest` as `protected_system`, because the
content it names is measured in `payloads`). The down migration refuses while a row of the table exists, while a version of level
1 exists, or while the requirement's history names the capability, and restores the closed lists.

### 3. Admission of a signal

One function decides for every entry point (`signalRun`, `chain.signal`, a batch `SIGNAL` item, Signal-with-Start, reset reapply):
`deliver_signal` in `crates/application/src/runs.rs`, which today has a twin in `crates/application/src/chain_progress.rs`
(`signal`); Task 054-8 makes the chain path call the same decision. Under the run lock, after the checks that exist today
(authorization, a JSON payload of at most 256 KiB, the idempotency replay):

1. **Undeclared name, or a version of level 0:** today's path, unchanged (Requirement 054-003).
2. **Declared name, graph run, a root wait of that name waiting:** the name's `maxBytes` is checked first (`SIGNAL_TOO_LARGE`), as for
   every signal of a declared name, live or buffered, so that acceptance never depends on timing; then live delivery to the first
   waiter (Requirement 054-004); the answer names that one node. The retained-bytes check is today's, with pending bytes counted (Technical Plan 6).
3. **Declared name, no waiter, or a code run:** the checks of Requirement 054-005 in their order; then the domain's command
   `buffer_signal(def, state, name, bytes, digest)` emits `SignalBuffered`; the engine stores the payload in `payloads` (the
   existing content-addressed insert), and the store, when it writes that event, inserts the inbox row with the event's sequence,
   raises `runs.pending_signals` and charges the payload's bytes to the ledger's `pending_signal_bytes`. The per-name count is read from the index range of the name, bounded by `LIMIT maxPending`.

The idempotency receipt of a delivery is today's comma-separated list of the woken nodes; a buffering's receipt is `#` and the
sequence (`#42`), a form no node id can take (node ids use letters, digits, `_`, `.` and `-`), read back into `buffered: true` and the sequence by every reader
of a signal receipt: `signal_with_receipt` and `delivered_before` (scope `run.signal:{id}`) and the chain path's `parse_nodes`
(scope `chain.signal:{chain}`, `crates/application/src/chain_progress.rs`); a build that predates the receipt never reads one, because of the
floor (Technical Plan 15). The with-start replay test covers it. The audit record is today's (`run.signalled`, the reason "signal
'name' buffered"); the journal event is announced as every journal
event is. The idempotency record keeps the answer, so a repeated key with the same body answers `buffered: true` and the same
sequence; with another body it is `409 IDEMPOTENCY_KEY_REUSED`, as today. A refusal after the idempotency check commits nothing,
neither the payload nor the receipt. An error of `deliver_signal` that is not one of these answers drops the transaction.

A chain-addressed signal resolves the chain's head under the chain lock and then follows the same steps for that run. A batch
`SIGNAL` item settles as any item does: `SUCCEEDED` when delivered or buffered, `FAILED` with the refusal's reason otherwise.

### 4. Consumption at a wait's entry

The domain stays pure and asks once per entry, in the evaluation that already makes a wait wait. When the planner asks for the
correlation of a root `waitEvent` whose `eventType` is a declared name while `state.pending_signals > 0`, its
`Evaluation::Correlation` carries `take_buffered: true`. The engine's resolve loop (`Engine::run`) resolves the key as today and,
for such an evaluation, makes one more store call, `take_oldest_pending_signal(name)`:

```sql
WITH taken AS (
  DELETE FROM run_signal_inbox
   WHERE (tenant_id, run_id, sequence) = (
         SELECT tenant_id, run_id, sequence FROM run_signal_inbox
          WHERE tenant_id = $1 AND run_id = $2 AND name = $3
          ORDER BY sequence LIMIT 1)
  RETURNING tenant_id, sequence, payload_digest, bytes)
SELECT taken.sequence, taken.payload_digest, taken.bytes, p.content
  FROM taken JOIN payloads p ON p.tenant_id = taken.tenant_id AND p.digest = taken.payload_digest;
```

and passes it back in `Resolution::Correlation { node, key, taken }`. The engine records the consumption in `Effects.consumed:
Vec<(NodeId, Digest, u32)>`, from which `persist` writes the `wait.satisfied` row's `run_events.payload_digest` and the step's
`output_digest` (so a reset, which rebuilds step outputs from the prefix's digests, keeps the consumed output), and which it leaves
out of the output charge. With a signal, the domain emits `wait.started` and then
`WaitSatisfied { node, via: Buffered, signal: sequence }`, and lowers `pending_signals`; the store writes the step's
`output_digest` from the taken row (the payload row exists, nothing is copied), lowers `runs.pending_signals`, and moves the
row's bytes with its own statement, `UPDATE run_instance_usage SET logical_output_bytes = logical_output_bytes + b,
pending_signal_bytes = pending_signal_bytes - b`, compared with the values it read. The output charge of today
(`charge_logical_output_bytes`, which charges outputs whose `output_digest` was null before the transaction and checks
`total() + charge` first) leaves the outputs of `Effects.consumed` out, so the bytes are never counted twice and the total does not change. The one statement returns the content too, and the engine stages it as the node's output in the
transaction's `RunData` (`stage_outputs`, as a live signal's payload is staged today), so a condition, a correlation or an
activity's input evaluated later in the same transaction reads it; a test reads the consumed output in the entry transaction.
Without one (another name holds the pending signals), the wait
waits. Because the read deletes in the same statement, a second wait of the same name entered later in the same transaction reads
the next row; a rollback restores both. A correlation that fails to evaluate fails the step as today, and takes no signal.

A root `waitEvent` enters `WAITING` through that resolution, with one exception: the scheduler's continuation handoff
(`continue_evaluated` in `crates/infra-postgres/src/chain_progress.rs`) may open a successor's entry `waitEvent` store-side. A graph
version of level 1 cannot reach it with a declared name (`SIGNAL_ENTRY_WAIT_CONTINUES`, Requirement 054-001). The other store-side
paths that release a successor without the engine's loop resolve no correlation: the collection's finish (`primitives.rs`)
resolves only an `end` node's output and refuses any other pending evaluation, the loop's routing (`route_loop_successor`,
`loop_progress.rs`) writes an activity `READY` or an `end` done, and `continue_as_new` requires an activity entry. Task 054-3 adds a
test that each of these paths refuses or cannot reach a `waitEvent` of a declared name, so that no entry bypasses the inbox.

### 5. The look-back of feature 004, weighed

Feature 004 already has a mechanism for an event that arrives before its wait: an event is admitted to `inbox_events` (a row per
tenant, namespace, source and event id, its payload inline, purged when its deduplication window ends), a waiting step records
`steps.event_since` (its entry less its `lookbackMs`, at most seven days), and the `late_waits` scan of the event dispatcher finds,
for a wait not yet scanned, an undelivered event of its type and correlation, records the delivery in `event_deliveries` (once per
consumer, `wait:{run}/{node}`), and wakes the step in a later transaction. Using it for signals means admitting each signal as an
inbox event with a source of its own and the run as its correlation. It was rejected as the substrate (Decision 054-001) because on
five points its semantics are not a signal's:

| Point | Feature 004's look-back | What a buffered signal needs |
|---|---|---|
| Lifetime | The deduplication window of the source (24 hours by default, 30 days at most), and a look-back of at most seven days; after that the event is gone without a record | As long as the run, and a journal record of every settlement |
| Consumers | One delivery per consumer: every wait that looks back gets the same event | Exactly one wait per signal |
| Timing | Delivered by a scan in a later transaction, after the wait committed | Consumed in the wait's entry transaction, in the order of arrival |
| Scope | Namespace-wide rows correlated by a key the wait's expression computes | A row of the run, with the run's lock, counters and lifecycle (continuation, end, reset) |
| Accounting | The payload inline in `inbox_events`, outside the run's retained bytes and outside `payloads` | Counted in the run's retained bytes and stored as run content, classified as the step's output once consumed |

What this design reuses from feature 004 and today's signals: the `waitEvent` node as the consumer (no new node kind), the
`wait.satisfied` event and its `via` field, the idempotency scope `run.signal:{id}`, the `payloads` store and its quota trigger.

**Which source a wait consumes first** (Requirement 054-009): the inbox, at the wait's entry, in its entry transaction. The look-back
scan re-checks under the run lock that the step still waits for exactly that event (`wake` in `crates/scheduler/src/events.rs`), so a
wait the inbox satisfied is never woken again by an event, and its `event_deliveries` row is not written. When the inbox had none,
the wait waits, and the first commit among a live signal, a live event, the scan's delivery and the timeout wins (principle I). The
reasoning: a signal is addressed to this run by name and was accepted for it; an event was published to everyone who listens, and
its look-back is a best effort bounded by a window. The guide says so.

### 6. Retained bytes

The run's byte ledger already exists: `run_instance_usage.logical_output_bytes`, capped at 4,194,304 by a check, charged by the
statements that write an output and read by one port method, `RunTx::logical_output_bytes()` (`crates/application/src/execution.rs`,
`crates/infra-postgres/src/collection_outputs.rs`). This design keeps that column what it is, the output bytes, and charges pending
payloads to a sibling column of the same row, `pending_signal_bytes`, under one check of the sum (Technical Plan 2), so the
database refuses any write that would take outputs and pending payloads past 4 MiB, whatever path makes it. Every place that
compares with the bound then reads the sum:

- the ledger read (`collection_outputs::inspect`, which builds `EffectiveOutputBytes`, and the path transaction's) reads both
  columns: `EffectiveOutputBytes` gains `pending()`, `ledger()` stays the output bytes as read (the charging statements compare the
  column with it for their own concurrency check), `total()` becomes outputs, verified collections and pending together, and
  `legacy_bytes()` subtracts the pending bytes too; the port method `logical_output_bytes()` returns `total()`;
- the sites that compare `total()` with `MAX_RUN_OUTPUT_BYTES` then count pending bytes with no change of their own, and each gets a
  test: `crates/infra-postgres/src/primitives.rs` (three, the collection's finish and its admission), the loop's carry check in
  `crates/scheduler/src/passes.rs`, and the callers of `logical_output_bytes()`;
- the charging statements' predicates (`charge_logical_output_bytes` and `charge_collection_owner_output` in
  `crates/infra-postgres/src/execution.rs`, and the path charge in `execution/path.rs`), today `logical_output_bytes + $4 + $5 <= $6`, add `pending_signal_bytes`;
- the collection-finalization scan of `crates/infra-postgres/src/maintenance.rs`, which selects work that fits the room, subtracts
  `pending_signal_bytes` too;
- the three sites that sum `RunData.outputs` themselves (`wake` in `crates/scheduler/src/events.rs`, a retained-size read in
  `crates/scheduler/src/passes.rs`, `Engine::evaluate_run_result` in `crates/application/src/execution.rs`) read the method instead,
  add the run's pending bytes to their in-memory sum, so that they answer with today's typed refusal rather than the check's error.
  They read the ledger's `pending_signal_bytes` only when `runs.pending_signals > 0` (known from the run's row), so a run with no
  pending signal, level 0 included, makes no new statement; budget lines for an event's wake and an `end` output on a run of level
  0 hold it. `evaluate_run_result` is synchronous and holds no transaction, so it gains a `pending: u64` parameter, which its two
  callers (`crates/application/src/execution.rs` and `crates/scheduler/src/passes.rs`) supply that way.

Task 054-7 changes each, with a test per site. A buffering adds to `pending_signal_bytes`; a consumption moves the bytes to
`logical_output_bytes` in one update (the sum does not change, so a consumption never fails for bytes); a move debits the
predecessor and credits the successor; a discard debits.

### 7. Continue-as-new

Decision 054-006 (move what the successor declares) and Decision 054-013 (the handoffs). A continuation has two handoff paths in `crates/infra-postgres/src/chain_progress.rs`: `continue_as_new` and `continue_evaluated`
(the scheduler's, called from `crates/scheduler/src/passes.rs`). Both lock the chain, then the run. Today the predecessor's events
start at its `run_version + 1` (`run.continued` in `continue_as_new`, `continuation.selected` in `continue_evaluated`), each appended
by one `append_event` call, and the successor is inserted with its first three events (sequences 1 to 3) and `run_version` 3. In
both, and only when `runs.pending_signals > 0` (read with the run's row):

1. one `INSERT ... SELECT` writes the successor's rows for the names the successor's version declares, in the order of their
   sequences, numbered 4, 5, ... after the successor's three events, with `moved_from_run`, `moved_from_seq` and the original
   `version_id`;
2. one `INSERT INTO run_events ... SELECT` over those rows appends the successor's `signal.buffered` events with `movedFrom`, one per
   moved row, at the same sequences, with `run_events.payload_digest` set (a bulk append, not one `append_event` call per row), and
   the successor's `run_version` is written as 3 plus the count;
3. one `DELETE` removes the predecessor's rows; the predecessor appends `signal.moved {count, to}` when any moved and
   `signal.discarded {count, cause: "not_declared"}` when some were not declared, with one `append_event` call each, *before* its
   handoff event (`run.continued` or `continuation.selected`), at `run_version + 1` and `+ 2`, because the handoff event ends the
   run and `RunState::apply` refuses any event after a terminal one; the handoff event follows, and the idempotent replay looks for
   it by type after the expected version; `mark_telemetry` is raised to the predecessor's last sequence and to the successor's
   `3 + count`; both runs' counts and byte ledgers are set by the
   statements that write those rows today (the successor's ledger row, which a trigger creates, gets one update more; the budget
   lines count it).

The successor's entry is an activity, or a wait of an undeclared name (Requirement 054-001), so nothing is consumed in the handoff;
the successor's first wait of a declared name consumes the oldest moved signal at its entry.

**A buffering between the scheduler's read and the handoff.** The scheduler reads a run under its lock, evaluates the continuation's
input outside it, and hands off with `expected_run_version` (`passes.rs`, `continue_evaluated`); today a signal that finds no waiter
commits nothing, but a buffering appends to the journal. So that a steady flow of signals cannot starve a continuation, both handoffs
(and `finalize_continuation_expression`) accept a run whose version advanced past the expected one only by `signal.buffered` events:
under the locks they read the types of the events after the expected version (at most the run's pending count, through the journal's
key) and go on when every one is `signal.buffered`, which changes no step, input or state the evaluation read. Any other event
refuses the handoff as today. The same check, one store helper (`advanced_only_by_buffering(conn, tenant, run, expected_version)`),
serves every path that reads a run outside its lock and refuses at write time on a changed version: `finalize_chain` (the deadline
and the segment limit), the loop's and the collection's store-side transitions (`loop_progress.rs`, `primitives.rs`), and the
scheduler's collection passes (`crates/scheduler/src/passes/collection_controls.rs`, `collection_event_waits.rs`,
`collection_timers.rs`); a barrier test lands a signal between the deadline pass's read and `finalize_chain`. A rule of
`scripts/check-architecture.py` requires every comparison of `run_version` in `crates/infra-postgres` and `crates/scheduler` to
go through the helper or to be on an allowlist with its reason, so a missed path fails the check. The public `expectedRunVersion`
of the run operations (`crates/api-http/src/dto_exec.rs`) keeps its exact meaning, a client's compare-and-set on the journal it
read: a buffered signal changes the journal, so such a request is refused as any change refuses it, and the client reads again. It
cannot be starved without bound: a name accepts at most `maxPending` signals before refusing, so at most 1,000 buffered signals can
advance a run's version before its inbox is full. The idempotent replay of `continue_evaluated`, which today looks for `continuation.selected` at
`expected_run_version + 1`, looks for the first `continuation.selected` after the expected version instead, since buffered signals
may precede it. A barrier test lands a signal between the evaluation and the handoff, and a replay test follows it.

A graph run continues under its own version (`chain.version_id = run.version_id`), so every pending signal moves; a code run may
continue under another version (design 044), whose declarations decide. The payload rows are not copied: the successor's rows name
the same digests. At most 1,000 rows move, and the successor's retained bytes start at their sum, within its 4 MiB. The cost of a
handoff grows with the pending signals it moves (up to 1,000 rows and 1,000 events): a graph version that handles one signal per
segment (CUJ 2) pays it per consumed signal, and a chain has at most its `max_segments` segments; Risks and the guide say so, and
point a high-volume queue to a code version (design 044), which handles it in one run.

### 8. The end of a run

Decisions 054-007 and 054-013. A run ends on several paths, and each MUST discard its pending signals in its transaction and journal `signal.discarded {count,
cause: "run_ended"}` immediately before `run.finished`, when the count is above zero:

- **The planner**, for a run that completes, fails, is cancelled, compensates or ends partial (an effect that cannot be undone,
  principle III): it emits `SignalsDiscarded` before `RunFinished` when `state.pending_signals > 0`.
- **The store-side terminal paths** that append `run.finished` themselves: `finalize_chain` in `chain_progress.rs` (the chain's
  segment limit, its tree budget, a successor input that fails its schema, an expression that fails, the deadline's `TIMED_OUT`), the
  collection's finish in `primitives.rs`, and the loop's two ends in `loop_progress.rs` (a loop whose successor is `end`, and a
  drained or stopped loop, which a cancellation reaches). These work on a raw `PgConnection` transaction, not a `RunTx`, so each calls one free
  helper of `crates/infra-postgres`, `discard_pending_signals(conn, tenant, run) -> Result<u16, StoreError>` (which the `RunTx`
  method of the same name also calls), which deletes the rows, sets the
  count and the pending bytes to zero, and returns the count for the event. The exact event shapes these paths match (for example
  `[EndReached, RunFinished]` and the instance-limit shape in `primitives.rs`) admit the optional `SignalsDiscarded` before
  `RunFinished`. A rule in `scripts/check-architecture.py` lists every writer of `run.finished` outside the domain's planner (the
  `RunEvent::RunFinished` constructions and the string literals `"run.finished"` and `'run.finished'` in `crates/infra-postgres/src`,
  such as the loop's `append_run_event` at `routed_sequence + 2`) against an allowlist whose entries each call the helper, so a
  later path cannot be missed.

**A database check holds every path to it:** `runs` gets `CHECK (pending_signals = 0 OR status IN ('RUNNING', 'WAITING', 'PAUSED',
'CANCELLING', 'COMPENSATING', 'MANUAL_REVIEW'))`, so a path that ends a run without discarding fails its transaction in the tests
instead of leaving rows behind; Task 054-3 adds a test per path. A signal for a run that has ended, or that is stopping
(`CANCELLING`, `COMPENSATING`), is refused (`RUN_ENDED`, `RUN_STOPPING`; Requirement 054-005), so nothing is buffered after the
discard. The payloads stay as run content until the run is purged, as a step's output does.

### 9. Reset and reapply

Feature 018's reset re-emits the source's events `1..N` under the new run with `restored: true` and rebuilds side rows by the
registry of `specs/018-schedules-bulk-operations/contracts/journal-and-events.md`. This design declares its rows there (Technical
Plan 11): `signal.buffered` is `REMATERIALIZE`. The store's hook that inserts an inbox row when it writes `signal.buffered` skips
restored events (`restored: true`), so a signal consumed inside the prefix never comes back; instead one statement, computed from the
prefix (every `signal.buffered` whose sequence no `wait.satisfied`, `signal.consumed`, `signal.moved` or `signal.discarded` of the
prefix settles), inserts the new run's rows with the same sequences and digests and sets the count and the pending bytes, before the
settle reduction that follows `run.reset`, so a wait released by that reduction finds them. A test resets a prefix that buffered
and consumed one signal: the new run's inbox is empty and no second `wait.satisfied` appears. A pending signal at the point does not make the point ineligible. A continuation's successor is never a reset source (feature
018's registry marks the chain's event types `EXCLUDE`, Gap 1001 of feature 018), so the signals moved into it never meet a reset.

Reapply (`reapply` in `crates/application/src/runs/reset.rs`, its plan in `runs/reapply.rs`): with `SIGNALS`, the candidates after
the point are the `wait.satisfied` events of `via: signal` (today's) and the `signal.buffered` events of `via: signal` without
`movedFrom`, in journal order; a Signal-with-Start's signal is skipped as today (`via: signal_with_start`, buffered or not). A
buffered candidate's payload is found by its row's `run_events.payload_digest`, as today's candidates are; one whose payload is gone
is `NOT_REAPPLIED` with `NotRetained`, as today. Each goes through `deliver_signal` under the reset's key, so it is delivered or
buffered by the new run's version. Today only `NOT_WAITING` is mapped to `REJECTED` and any other error aborts the reset; this design
maps each new refusal to a `REJECTED` candidate with a new `ReapplyReason`, recorded and parsed in `reapply.rs`: `InboxFull`
(`INBOX_FULL`, for `SIGNAL_INBOX_FULL`), `NoConsumerAhead` (`NO_CONSUMER_AHEAD`), `TooLarge` (`TOO_LARGE`, for `SIGNAL_TOO_LARGE`) and
`RunBytes` (`RUN_BYTES`, for the run's `429`). A rejected signal ends the reapply of signals, as `NOT_WAITING` does today; the
reset itself goes on.

### 10. Code versions (design 044)

Design 044 declares a code version's signals in this shape, every one buffered, with a closed `schema` and the refusals and headroom
rules of its Requirement 044-014. What this design gives it:

- **Buffering always.** A signal of a declared name to a code run is buffered whether or not the function waits, so the next
  activation receives it as the history event `signal.buffered` (design 044, section 4.3).
- **Consumption by sequence.** The inbox's contract for design 044 is one store method of the run's transaction,
  `consume_pending_signals(sequences: &[u64]) -> Result<Result<(), NotPending>, StoreError>`: all or nothing, it deletes the listed rows, moves their
  bytes from the ledger's pending bytes to its output bytes (the payloads become the activation's inputs, retained as the run's
  content), and lowers the count; it refuses, deleting nothing, when any sequence is not a pending row of that run, and names the
  first such sequence. Design 044 journals `signal.consumed` and maps the refusal to `COMMAND_INVALID`. Its text "the inbox rows
  marked consumed" means these deletions: a row exists only while its signal is pending (Decision 054-004; Gap 054-006 asks design
  044 to cite this contract).
- **The version that accepted it.** Each row keeps `version_id`, the run's version when the signal was buffered, kept by a move
  and a reset, so that design 044 classifies a pending signal by the version that accepted it (its section 7) even after the run
  continued under another version.
- **The same lifecycle.** Moves at continue-as-new (by the successor version's declarations), the discard at the end, the
  rebuild at a reset (from the prefix's `signal.buffered` and `signal.consumed`, which design 044 declares `COPY`), and the floor.
- **A refinement of Architecture decision 0111 decision 1:** the consumer of a code version's signal is the function, not a
  `waitEvent` node; `SIGNAL_WITHOUT_CONSUMER` applies to graph documents only. This closes Gap 044-017's first condition.

Design 044 owns the activation, the command, its refusals (`SIGNAL_NOT_DECLARED`, `HISTORY_HEADROOM`, `RUN_CLOSING`) and the move
of a code run's pin, which requires the target to declare every name with pending signals.

### 11. Journal events and their reset rules

In `specs/054-buffered-signals/contracts/journal-events.md` (written by Task 054-1) and in feature 018's registry:

| Event | Detail | Written by | Reset rule |
|---|---|---|---|
| `signal.buffered` (new) | `name`, `bytes`, `via` (`signal`, `signal_with_start`), `acceptedBy` (the version), `movedFrom` (`{run, sequence}`, for a move); the payload's digest in the row's `run_events.payload_digest` column, as a step's output's is | a buffering; a move into a successor | `REMATERIALIZE` |
| `wait.satisfied` (exists) | `via: buffered` (new value) and `signal` (the sequence, new) | a consumption at a wait's entry | `COPY` (additive detail) |
| `signal.moved` (new) | `count`, `to` | the predecessor of a continuation | `EXCLUDE` (a continuation tail) |
| `signal.discarded` (new) | `count`, `cause` (`run_ended`, `not_declared`) | the end of a run; a continuation | `EXCLUDE` (a terminal or continuation tail) |

An older build decodes none of them, which is one reason for the floor of Technical Plan 15. The domain's reducer applies each to
`pending_signals`; the replay corpus compares the count, the bytes and the rows with the projection (Requirement 054-020).

### 12. The races (AGENTS.md rule 9, "the race between timeout and signal")

Every row is decided under the run lock, which every one of these transactions takes (a signal, a wait's entry, a timer, a
completion, an end, a handoff after the chain lock, a path transaction). The loser commits nothing or a recorded no-op.

| Race | Resolution | Test |
|---|---|---|
| a signal and the entry of a wait of its name | signal first: buffered, then consumed at the entry; entry first: the wait waits, then the signal is delivered live. One `wait.satisfied` either way, an empty inbox | the entering transaction (a worker's completion) held at a barrier while the signal blocks on the run lock, and the reverse |
| a signal and the wait's timeout | signal first: live delivery, the timer's later attempt is today's recorded no-op; timeout first: the signal finds no waiter and is buffered when a later wait or a continuation can still consume it, otherwise refused `NO_CONSUMER_AHEAD` | the timer pass held at a barrier, both orders |
| a signal and the run's end | signal first: buffered, then discarded and counted by the end; end first: refused `RUN_ENDED` | the ending transaction held, both orders |
| a signal and a continuation | signal first: buffered, then moved; handoff first: the old run refuses `RUN_ENDED`, a chain-addressed signal reaches the successor | the handoff held after its chain lock, both orders |
| a signal buffered between the scheduler's evaluation of a continuation and its handoff | the handoff accepts a run version advanced only by `signal.buffered` and moves the new signal with the others | a barrier between the evaluation and the handoff |
| two waits of one name and one pending signal | the first entry consumes it; the second waits. In one transaction, the lower node id, which the planner enters first, consumes it | two parallel branches entered in one transaction; two transactions serialized at a barrier |
| two signals of one name at `maxPending − 1` | the first is buffered, the second refused `SIGNAL_INBOX_FULL` | both blocked on the run lock, released in turn |
| a signal and a reset of the run | the reset's source is ended or cancelled first (feature 018); then as the end | the existing reset barrier with a pending signal |
| a crash after the commit of a buffering, before the answer | the retry with the same key answers the first answer | the answer dropped by the harness |
| a restart | nothing is lost: the inbox is rows and the journal | the engine restarted with pending signals |

Each row is built test-first by the task that makes it possible, in `tests/fault/buffered-signals/races.rs`: Task 054-8 the
signal against a wait's entry (`signal_and_wait_entry_in_both_orders`), against its timeout (`signal_and_wait_timeout_in_both_orders`),
against the run's end (`signal_and_run_end_in_both_orders`), two waits and one signal (`two_waits_take_one_signal`) and two signals
at the bound (`two_signals_at_the_bound`); Task 054-9 the continuation (`signal_and_continuation_in_both_orders`) and the signal
between the evaluation and the handoff (`signal_between_evaluation_and_handoff`); Task 054-10 the reset (`signal_and_reset`); Task
054-12 the crash and the restart (`crash_after_buffering_answers_the_first_answer`, `restart_keeps_pending_signals`).

No broker hint is involved: a wait consumes in its entry transaction, and a signal is admitted by an API call that commits before
it answers (principle VII).

### 13. REST, the Studio and the SDKs

`specs/001-platform-foundation/contracts/openapi.yaml`, additive:

- `GET /runs/{runId}/signals?after={sequence}&limit={1..200}` (`listPendingSignals`), under `run:read`, answers `{items:
  [{sequence, name, bytes, receivedAt, movedFrom}], next}`; another tenant's run is `404` as any missing run.
- `SignalResult` gains `buffered` (boolean, false when absent) and `sequence`; `Run` gains `pendingSignals`; the problem body gains
  the reasons `SIGNAL_INBOX_FULL`, `SIGNAL_TOO_LARGE`, `RUN_ENDED`, `RUN_STOPPING`, `NO_CONSUMER_AHEAD` and the publication reasons of Requirement 054-001.
- The descriptions that say "a signal is never buffered" (the signal routes, Signal-with-Start) say "unless the version declares
  the name", and `crates/api-http/src/error.rs`'s sentences follow.

The Studio (`frontend/apps/studio`): the version's inspector edits `signals` with the server's validation and shows, per name, the
bound of Requirement 054-007 and the share of the 2 MiB the declarations use; a run's page shows the pending signals from the list (no payload) and the discarded counts from the
journal, with the projection's lag (principle VI).

The six SDKs (TypeScript, Go, Python, C#, Java, Rust), one meaning each (principle IV): the client's `signal` returns `buffered` and
`sequence`; `listPendingSignals(runId)` pages the list; the graph builder takes `signals` with the validation of Requirement 054-001
from one shared fixture. The conformance scenario `buffered-signals` (`tests/contract/sdk-conformance/`) runs CUJs 1 and 3 and the
refusals in six languages. The worker protocol does not change.

### 14. Telemetry and audit

- Metrics, closed labels only: `signals_total{outcome}` with `delivered`, `buffered`, `not_waiting` (`NOT_WAITING`), `refused_full`
  (`SIGNAL_INBOX_FULL`), `refused_size` (`SIGNAL_TOO_LARGE`), `refused_bytes` (the run's `429`), `refused_quota` (the tenant's content
  quota), `refused_ended` (`RUN_ENDED`), `refused_stopping` (`RUN_STOPPING`), `refused_no_consumer` (`NO_CONSUMER_AHEAD`); `signal_inbox_settled_total{how}` with `consumed`, `moved`, `discarded`;
  `signal_inbox_wait_seconds` (a histogram from a signal's buffering to its consumption, at the consumption).
- Spans: the signal's transaction carries `sagat.signal.outcome`; a wait's entry that consumes carries `sagat.signal.buffered=true`.
- Audit: a buffered signal is audited as every signal (`run.signalled`); a consumption, a move and a discard are system
  transitions, journaled and not audited, as a wait's satisfaction is today. Publishing a version with `signals` is audited as every
  publication.
- Logs: closed values only (an outcome, a cause); the name and the payload stay in the journal and the audit record.

### 15. Upgrade, rollback and runs in flight

- A version of level 0 runs exactly as today at every step of a rolling upgrade; a run in flight never changes level, since its
  version is immutable (principle IV).
- The operator records `signal-inbox-v1` once every control plane and engine offers it (`platform_requirement_activate`,
  Architecture decision 0082); until then publication of a version of level 1 answers `422 CAPABILITY_NOT_ACTIVE` with `capability: signal-inbox-v1`. Once recorded,
  a binary without the capability serves nothing (the closed gate). Both kinds need it: a control plane admits and buffers signals
  and publishes versions; an engine enters waits, ends runs and hands off continuations.
- **The publication hold** (Architecture decision 0112, proposed with this design) is the function shared with designs 044 and
  045, built once, with the floor's scaffold below, by a
  prerequisite migration and pull request of its own (Task 054-18) that whichever of the three designs builds first lands and the
  other two depend on (Gap 054-007). It is `SECURITY DEFINER`, `SET search_path = pg_catalog, pg_temp`, uses schema-qualified
  names, and has `EXECUTE` revoked from `PUBLIC` and granted to `sagat_app`. It has no list of capabilities in its body: the closed
  check `platform_requirements_capability_known` bounds the value, so each design adds its capability to that check, not to the
  function. Its body, the one design 044 gives in its Decision 044-206:

  ```sql
  CREATE FUNCTION public.platform_requirement_hold(capability text) RETURNS boolean
  LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
  DECLARE
      v_kinds text[];
  BEGIN
      SELECT r.kinds INTO v_kinds FROM public.platform_requirements r
       WHERE r.capability = platform_requirement_hold.capability FOR SHARE;
      RETURN FOUND AND v_kinds @> ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE']::text[];
  END;
  $$;
  ```

  A design's own migration that finds the function compares `prosecdef`, `proconfig`, the owner and the body normalized as
  `regexp_replace(btrim(prosrc), '\s+', ' ', 'g')` with this text, and fails on any difference: a guard against drift, never a
  way to replace it. The store calls it inside the publication's transaction under a `lock_timeout` (store port:
  `hold_requirement(capability: Capability) -> Result<bool, StoreError>` on the publication transaction); `false` is
  `CAPABILITY_NOT_ACTIVE` with the capability named, a lock timeout `UNAVAILABLE` with the reason `REQUIREMENT_BUSY`.
- **The floor for a rollback.** The prerequisite also builds the floor's scaffold, so that no design replaces another's code: a
  table `platform_requirement_floors (capability text PRIMARY KEY, holds regprocedure NOT NULL)`, its capability under the same
  closed check as `platform_requirements`, owned by the migration role, with no grant to `sagat_app`; and a trigger function `platform_requirement_floor()`,
  `SECURITY DEFINER` with a fixed search path, `BEFORE DELETE` on `platform_requirements` (a requirement is never updated:
  `platform_requirements_guard` refuses every `UPDATE`, and a removal is a `DELETE` made only inside `platform_requirement_remove`),
  which refuses unless the transaction is `READ COMMITTED` (so that, after waiting for a publication's `FOR SHARE`, it reads the
  committed versions) and calls the `holds` function registered for that capability (`EXECUTE
  format('SELECT %s', holds)`; a `regprocedure` prints its own argument list, and only a migration can insert one) and refuses with `FLOOR_HELD` when it returns
  true. Each design adds one row and one function. This design's: `signal_inbox_floor_holds() RETURNS boolean`, `SECURITY DEFINER`,
  fixed search path, `SELECT EXISTS (SELECT 1 FROM public.workflow_versions WHERE signal_inbox_level > 0)`, one probe of the partial
  index of Technical Plan 2, retired versions included: an older build cannot parse those versions or decode their runs' journals.
  Architecture decision 0098's own rule (on the branch of pull request #260) and design 045's join the same registry.
- The released 0.2 scenario (`scripts/test-release-upgrade-profile.py`): runs started on 0.2 finish on 0.3 unchanged; a 0.2 SDK
  client signalling a declared name reads the new optional fields as unknown and ignores them.

### 16. Other features

| Feature or record | Status on 2026-11-26 | What this design asks of it, or gives it |
|---|---|---|
| Architecture decision 0030 decision 17 | accepted | superseded in part, for declared names: a signal may be pending, and pending signals move across continuation |
| Architecture decision 0050 decision 4 | accepted | superseded in part, for declared names: Signal-with-Start and reset reapply may buffer; decisions 1 to 3 and 5 to 9 unchanged |
| 004 (events, `lookbackMs`) | built | unchanged; its spec's sentence "Signals remain accepted only by a waiting step" gains "unless the version declares the name" (Task 054-1); the inbox is read before the look-back |
| 014 (continuation chains) | built | the handoff moves pending signals (Technical Plan 7) |
| 016 (classification) | built | a consumed signal is the step's output, classified as today; a pending payload is never read |
| 018 (Signal-with-Start, reset, batch) | built | Requirements FR-018-024 and FR-018-039 refined; the registry gains three rows (`signal.buffered`, `signal.moved`, `signal.discarded`) and one revised (`wait.satisfied`'s detail); a batch `SIGNAL` item may buffer |
| 020 (visibility, purge) | approved draft | purges the inbox rows with the run; may list `pendingSignals` |
| 022 (flow control) | being built | Signal-with-Start's refusal when 022 would queue the new run is unchanged |
| 044 (workflow as code) | being designed | the inbox, the consumption by sequence and the lifecycle (Technical Plan 10); shares the hold function |
| 045 (activity timeouts) | being designed | shares the hold function and the floor trigger |
| ADR-0082 (instance registry) | accepted | one capability, `signal-inbox-v1` |

### 17. Interfaces

The signatures a builder needs, in the crates that own them:

- `crates/domain`: `WorkflowDefinition::signals() -> &SignalDeclarations`; `SignalDeclarations::get(&EventType) ->
  Option<&SignalDeclaration>` with `max_pending: u16` and `max_bytes: u32`; `Evaluation::Correlation { node, expression,
  take_buffered: bool }`; `Effects.consumed: Vec<(NodeId, Digest, u32)>` (in `crates/application`); `Resolution::Correlation { node, key: Result<CorrelationKey, ()>, taken: Option<TakenSignal> }` with
  `TakenSignal { sequence: u64, bytes: u32 }`; `RunEvent::SignalBuffered { name: EventType, bytes: u32, payload: Digest,
  via: WaitVia, accepted_by: WorkflowVersionId, moved_from: Option<(RunId, u64)> }` (`via` is `Signal` or `SignalWithStart`); `RunEvent::WaitSatisfied { node, via: Option<WaitVia>,
  signal: Option<u64> }` with `WaitVia::Buffered`; `RunEvent::SignalsMoved { count: u16, to: RunId }`;
  `RunEvent::SignalsDiscarded { count: u16, cause: DiscardCause }` with `DiscardCause::{RunEnded, NotDeclared}`; four new
  `ReapplyReason` values (Technical Plan 9);
  `buffer_signal(def, state, name, bytes, payload, via, pending_of_name: u16, now) -> Result<Progress, SignalRefusal>`, in which the
  domain owns the ordered refusals of Requirement 054-005 it can decide (`SIGNAL_TOO_LARGE`, `RUN_ENDED`, `RUN_STOPPING`,
  `NO_CONSUMER_AHEAD`, `SIGNAL_INBOX_FULL` from the count the store read), the application then checking the ledger's bytes and the
  tenant's quota, which only the store can, and a unit test fixes the order; `RunState::pending_signals() -> u16`.
  `Digest` is the existing `sagat_domain::workflow::Digest`, the SHA-256 of the payload and the key `payloads` uses; the
  application computes it (it already depends on `sha2`), and the store inserts the content under it, treats a mismatch as an
  invariant error, and writes it to the journal row's existing `run_events.payload_digest` column (through `append_event`'s digest
  argument), never to the event's detail, so no journal read exposes it.
- `crates/application`: `deliver_signal(...) -> Result<SignalOutcome, AppError>` with `enum SignalOutcome { Delivered(Vec<NodeId>),
  Buffered { sequence: u64 } }`; `signal` and `signal_item` return `SignalOutcome`, `signal_with_receipt` returns
  `(SignalOutcome, bool)`; `Effects` gains `buffered_payload: Option<Vec<u8>>`, the content the store writes to `payloads` with the
  `signal.buffered` it persists; the `RunTx` port gains `take_oldest_pending_signal(&EventType) -> Result<Option<(TakenSignal,
  Vec<u8>)>, StoreError>`, `pending_signal_count(&EventType, limit: u16) -> Result<u16, StoreError>`, `consume_pending_signals(&[u64])
  -> Result<Result<(), NotPending>, StoreError>`, `discard_pending_signals() -> Result<u16, StoreError>` (calling the free helper
  of Technical Plan 8) and
  `list_pending_signals(after: Option<u64>, limit: u16) -> Result<PendingSignalPage, StoreError>`; the publication transaction gains
  `hold_requirement(Capability) -> Result<bool, StoreError>`; `Engine::evaluate_run_result(node, expression, data, pending: u64)`.
- `crates/api-http`: `SignalResult { deliveredTo, buffered, sequence }`, `PendingSignal { sequence, name, bytes, receivedAt,
  movedFrom }`, `PendingSignalPage { items, next }`, as in `openapi.yaml` after Task 054-1.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A graph queue handled one signal per segment (CUJ 2) moves every pending signal at every handoff, up to 1,000 rows and events each, and a chain has at most `max_segments` segments | Stated in Technical Plan 7 and the guide; the guide points a high-volume queue to a code version (design 044), which consumes from its channel in one run; the budget suite measures a handoff with 1,000 pending (Task 054-16) |
| A sender fills a run's inbox and the run's step outputs then lack room | The declarations' bytes are capped at 2 MiB at publication (Requirement 054-001), so outputs always keep the other 2 MiB: a full inbox refuses the sender; a step fails `RUN_DATA_LIMIT` only when its own outputs pass what they always had |
| A Temporal user expects every signal to be kept | The `409` names the reason; the guide's comparison table says how to declare a name; Signal-with-Start's refusal names the missing declaration |
| A buffered signal is discarded when its run ends | Journaled with its count and shown on the run's page; the `202` says "kept for a later wait while the run lives", and the guide says so |
| A store-side path enters a wait without the planner and misses a pending signal | The test of Task 054-3 over the three store-side release paths, and the statement in Technical Plan 4 |
| A check of the 4 MiB bound forgets the pending bytes | The bytes are in the existing ledger, read by the one method the checks use, under a database check of the sum; the three sites that sum outputs themselves move to the ledger (Task 054-7) |
| Two waits of one name in parallel branches: an author expected a copy each | One consumer is the rule for declared names, stated in the guide; an author who wants a copy each does not declare the name |
| A rollback below this release after a version declared signals | Refused by the floor; stated in the release notes and the runbook; the operator records the requirement only when a rollback is no longer planned |
| Security: another tenant's run is signalled or listed | Every statement is scoped by `tenant_id`; a missing or foreign run is `404` and the attempt is audited, as today |
| Security: a pending payload is read by someone the step's classification would refuse | No read returns a pending payload; once consumed it is read as a step output under feature 016 |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | A buffering, a consumption, a move and a discard are journaled and written in the transaction that decides them, under the run lock; the inbox and its counters are a projection that the replay corpus checks. The principle's race of a timeout and a signal holds as written: the first commit wins; when the timeout wins, the signal is a no-op for the wait that timed out (nothing of that wait changes) and is audited (`run.signalled`), and when the signal wins, the timer's attempt is today's recorded no-op. What the signal then does is a separate transition, its admission for a consumer still ahead (another root wait of its name not yet entered, or a continuation); with none ahead it is refused `NO_CONSUMER_AHEAD`. This reading adds no exception to the principle; design 044 relies on principle IV's own sentence for code runs |
| II. Modular domain, ports and adapters | Pass | `SignalDeclarations`, the events and the evaluation in `crates/domain` without `serde_json`; parsing in `infra-ir`; SQL in `infra-postgres`; the engine's resolve in `application`; no new crate edge (`scripts/check-architecture.py`) |
| III. External effects are at-least-once; sagas compensate | Pass | A signal is not an external effect; a sender's retry is deduplicated by its key within the idempotency retention, and after it is a new signal, which the guide states (at least once, never exactly once); effects a consumption leads to are activities under the existing rules |
| IV. Compatibility is versioned and pinned | Pass | Opt-in per version and name; level 0 unchanged, `409` included; one optional IR field behind a fleet requirement with a floor; optional REST fields; no protocol change; code versions consume through design 044's verified command |
| V. Code, tenant boundary and secrets are isolated | Pass | No code runs on the server; tenant-scoped rows; `SELECT`, `INSERT`, `DELETE` for the runtime role; one definer function with a fixed search path, revoked from `PUBLIC`; no pending payload is readable |
| VI. Observability and audit are part of the feature | Pass | Closed-label metrics, spans, the audit of every signal, discarded counts in the journal and on the run's page with the projection's lag |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | A repeated signal is answered from the idempotency record; journal events are announced through the outbox; no decision depends on a broker hint |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Barrier tests per race, crash and restart, replay equals projection, contract tests, conformance in six languages; gaps registered |
| Delivery Constraints | Pass | No new technology; one migration of this design with its validating companion and, unless design 044 or 045 landed it first, the shared prerequisite's; the registry of Architecture decision 0082 |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A tolerance of a run version advanced only by `signal.buffered`, on the paths that read a run outside its lock (one helper and a source rule) | A steady flow of buffered signals must not starve a continuation, a deadline's end or a collection's step | Retrying: bounded only by the inbox's fill, and every retry repeats the evaluation |
| The shared hold and registry of floors (Architecture decision 0112) | Four requirements need a publication hold and a removal floor without one design replacing another's code | A function and a trigger rule per design, compared or replaced in landing order |
| `SIGNAL_ENTRY_WAIT_CONTINUES` | The scheduler's handoff opens a successor's entry wait without the planner (Gap 054-009) | Consuming in the handoff: a satisfied step the handoff cannot plan on |
| A table `run_signal_inbox` | Pending signals must be durable rows of the run with the run's lifecycle | Feature 004's `inbox_events` (Technical Plan 5) |
| A count on `runs` and a column on the byte ledger | A transition of a run with an empty inbox must read and write nothing more, and pending bytes must count wherever outputs do | A count query in every transition; a second byte counter beside the ledger |
| `workflow_versions.signal_inbox_level` and its partial index | The floor for a rollback in one indexed predicate | Scanning stored documents |
| `take_buffered` on the correlation's evaluation | The domain stays pure and the store reads one row per entry, in the evaluation that already makes a wait wait | Loading the pending rows into the run's state on every transition |
| One consumer for declared names while undeclared names keep a copy each | The number of waits a signal satisfies must not depend on timing | Copies when waiting, one when buffered |
| Routing every 4 MiB check through one method | Pending bytes must count everywhere, or a consumption could break the bound | A separate budget for pending payloads, which could fail a consumption |
| A list route and six client methods | An operator and a client must see what a run holds | The journal alone: it shows arrivals, not what is still pending |

## Alternatives Considered

- **Feature 004's look-back as the substrate.** Rejected on lifetime, consumers, timing, scope and accounting (Technical Plan 5;
  Decision 054-001). Its pieces that fit are reused.
- **Buffer every signal of every run.** Rejected: it changes `signalRun`'s answer for runs in flight and callers that rely on the
  `409` (principle IV; Decision 054-002).
- **Keep refusing (Architecture decision 0050 decision 4).** Rejected: leaves the parity gap the owner put in 0.3.0 and leaves design
  044 without a substrate.
- **Keep settled rows with a state column and a partial index of pending rows** (the first draft). Rejected: the rows grow with the
  history of a long run, the journal already records each settlement, and a table of pending rows needs no predicate (Decision
  054-004).
- **Load the pending rows into the run's state.** Rejected: up to 1,000 rows read on every transition of such a run; the counter and
  one indexed read at a wait's entry suffice.
- **Copies to every waiting step for declared names too.** Rejected: a signal would satisfy one wait or several depending on whether
  it arrived before or after them (Decision 054-003).
- **Three runtime bounds (per name, per run, bytes).** Rejected: an author could not predict which one refuses; the per-run count
  and the bytes are checked at publication instead, leaving one runtime bound per name and the shared bytes (Decision 054-005).
- **An expiry per signal.** Rejected for 0.3.0: a signal that expires silently is a lost business message; the run's life bounds it.
- **Refuse a continuation while signals are pending, or discard them.** Rejected: the first lets a sender block a run's progress,
  the second loses messages (Decision 054-006).
- **A new node kind for consuming a signal.** Rejected: `waitEvent` already is the consumer, with its timeout and route.

## Non-Functional Requirements

Non-functional requirements 054-001 to 054-004 are stated under Requirements. Their tests: the budget lines of
`integration_claim_transition_efficiency` (unchanged, plus one line for a wait's entry that consumes and one for a completion of a
run with pending signals) for 054-001; a line per lifecycle transition in the same suite for 054-002; the privilege audit of
`integration_platform_foundation` and every suite as the runtime role for 054-003; the released 0.2 scenario and the contract
baselines for 054-004.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Test first: each task that changes behaviour adds its tests and sees them fail for the right reason before the code (AGENTS.md rule
18). Documentation first: Task 054-1 changes the contracts and documents before any code (Owner decision 2026-11-20-8). The tasks
are ordered into slices for two builders whose lanes never touch the same crate at once: the server lane owns `crates/` and `apps/`
one slice at a time; the edges lane owns `sdks/`, `frontend/` and `docs/`. Each server slice is one pull request that changes every
caller of a signature it changes, so the workspace builds after each.

| Slice | Lane | Tasks | Crates and folders | Depends on |
|---|---|---|---|---|
| 0 | either | 054-1 | `specs/001-platform-foundation/contracts/`, `specs/054-buffered-signals/contracts/`, `specs/018-schedules-bulk-operations/contracts/`, `docs/` | this design approved |
| 1 | server | 054-2, 054-3 | `crates/domain`, its callers in `crates/application`, `crates/infra-postgres` (the event codec, exhaustive matches) and `crates/infra-ir` | slice 0 |
| 2a | server | 054-18, only if neither design 044 nor 045 landed it; its own pull request | `crates/infra-postgres` (one migration), `tests/integration/platform-foundation/` | slice 1 |
| 2 | server | 054-4, 054-5, 054-6 | `crates/infra-postgres`, `crates/infra-ir`, `crates/application` (the store port, the publication gate), the two new suites' skeletons | slice 2a, or design 044's or 045's equivalent landed |
| 3 | server | 054-7, 054-8 | `crates/application`, `crates/infra-postgres`, `crates/scheduler`, `crates/api-http`, `apps/control-plane`, `apps/execution-engine`, `scripts/` | slice 2 |
| 4 | server | 054-9, 054-10, 054-11 | `crates/infra-postgres` (`chain_progress.rs`, `primitives.rs`), `crates/application` (`runs/reset.rs`, `runs/reapply.rs`, `runs.rs`), `tests/fault/` | slice 3 |
| 5 | server | 054-12, 054-16 | `tests/integration/buffered-signals/`, `tests/fault/buffered-signals/`, `tests/integration/claim-transition-efficiency/`, `tests/performance/` | slice 4 |
| 6 | edges | 054-13 | `sdks/<language>` (six), `tests/contract/sdk-conformance/` | slice 0; its conformance run needs slice 3 |
| 7 | edges | 054-14 | `frontend/apps/studio` | slice 0 (types from `openapi.yaml`); its journeys need slice 3 |
| 8 | either | 054-15, 054-17 | `docs/`, `scripts/test-release-upgrade-profile.py` | slices 5, 6 and 7 |

- [ ] T054-1 Write the contracts and documents first: the `signals` property of `workflow.schema.json`; `openapi.yaml`
  (`listPendingSignals`, `SignalResult`, `Run.pendingSignals`, the reasons, the descriptions that said "never buffered");
  `specs/054-buffered-signals/contracts/journal-events.md` and `errors.md`; the four rows of feature 018's reset registry and the
  refinement of Requirements FR-018-024 and FR-018-039 in its spec; feature 004's signal sentence; "Superseded in part by
  Architecture decision 0111" in Architecture decisions 0030 (decision 17) and 0050 (decision 4); the "Refined by" pointer in
  Architecture decision 0082; `docs/architecture/execution-semantics.md` (the inbox and its lifecycle). The contract suites fail on
  the new baselines until slices 2 and 3. The rows of `docs/engineering/architecture-principles.md` (AGENTS.md rule 21) for the
  new rules and their checks: pending bytes in every 4 MiB check (the ledger's check of the sum), every writer of `run.finished`
  discards (the source rule of Technical Plan 8), no store-side entry of a declared wait (the test of Task 054-9), the version
  tolerance only through its helper (the source rule of Technical Plan 7); Architecture decisions 0111 and 0112 cited.
  Refs: Requirement 054-001, Requirement 054-016, Requirement 054-020, Non-functional requirement 054-004
  Owner-gate: OD-2026-11-26-540
- [ ] T054-2 Domain declarations: `SignalDeclarations` with `max_pending` and `max_bytes`, the validation of Requirement 054-001
  (duplicates, the two sums, a consumer among the root `waitEvent` nodes of a graph definition), the level. Unit tests first, one
  per refusal and per boundary value (32 and 33 entries; names of 128 and 129 characters and one with a character outside the `EventType` grammar; `maxPending` 1,000 and 1,001;
  `maxBytes` 262,144 and 262,145; sums of 1,000 and 1,001; byte sums of 2,097,152 and 2,097,153; a name used only by a body wait; an entry wait of a declared name
  with and without a `continueAsNew` node).
  Refs: Requirement 054-001, Requirement 054-002
- [ ] T054-3 Domain events and planning: `SignalBuffered`, `SignalsMoved`, `SignalsDiscarded`, `WaitVia::Buffered` and the
  `signal` field of `WaitSatisfied`; `RunState.pending_signals` and `restore`; `buffer_signal`; the choice of the first waiter
  (Requirement 054-004); `take_buffered` on `Evaluation::Correlation` and `taken` on `Resolution::Correlation`; the discard before `run.finished` for every
  way the planner ends a run; the `NO_CONSUMER_AHEAD` predicate (no root wait of the name still `PENDING`, no `continueAsNew`); the reset rules of the new types in `crates/domain/src/execution/reset.rs`; the event codec of the new
  events and every exhaustive match, so the workspace builds. Unit tests first; replay equals projection over a corpus that buffers,
  consumes, moves and discards; journals written before this design decode as before.
  Refs: Requirement 054-004, Requirement 054-006, Requirement 054-011, Requirement 054-020
  Owner-gate: OD-2026-11-26-540
- [ ] T054-18 The shared prerequisite of Architecture decision 0112 (Gap 054-007; listed here, in the order of its slice 2a, though
  numbered last), its own
  migration and pull request, built only if neither design 044 nor design 045 has
  landed it: `platform_requirement_hold` with the body of Technical Plan 15, its `REVOKE` and grant, the one refusal reason
  `CAPABILITY_NOT_ACTIVE` naming the capability, and the floor's scaffold of Technical Plan 15 (the table
  `platform_requirement_floors`, the trigger function `platform_requirement_floor()` with its `READ COMMITTED` check) that each
  design extends with one row and one function.
  `integration_platform_foundation` tests first: the function's privileges and configuration, a hold against a concurrent removal,
  the trigger's refusal through a test capability, two capabilities registered at once.
  Refs: Requirement 054-002
  Owner-gate: OD-2026-11-26-540
- [ ] T054-4 The migration of Technical Plan 2 and its validating companion (the table with its `version_id` and index, the new columns and their checks, the
  check on `runs` that a terminal status has no pending signal, the ledger's replaced check, the capability in the three closed
  lists, the comparison guard of the hold function, the floor's row and its function `signal_inbox_floor_holds`, the grants, the classification, the down migration's
  refusals). `integration_platform_foundation` first: migrations as the runtime role, the privilege audit, the registry's privilege
  test, the down migration's refusals.
  Refs: Requirement 054-002, Requirement 054-014, Non-functional requirement 054-003
- [ ] T054-5 Publication: `infra-ir` parses `signals` and maps the domain's refusals to pointers; `publish` computes the level,
  holds `signal-inbox-v1` through the store's hold and writes the level; the ad hoc insert writes 0. `integration_gitops_versioning`
  tests first: an IR fixture per reason, the requirement's refusal, the hold against a concurrent removal, a republication.
  Refs: Requirement 054-001, Requirement 054-002
- [ ] T054-6 The store (the event codec of the new events is slice 1's, Task 054-3): the skeletons of the suites
  `integration_buffered_signals` and `fault_buffered_signals` (their `[[test]]` entries in `tests/Cargo.toml`, their `main.rs`,
  their entries in `scripts/integration-suite-durations.toml`), so that later tasks add their tests there first; the inbox written
  from `signal.buffered`; `take_oldest_pending_signal`,
  `Effects.consumed` written to the journal's and the step's digest and left out of the charge, `consume_pending_signals` (for
  design 044), `discard_pending_signals`, `list_pending_signals`, the per-name count; the count in the
  statements that update `runs`; the ledger's `pending_signal_bytes` and `logical_output_bytes()` returning the sum. Store tests per
  method first, with two tenants, and a test that the ledger's check refuses a write past 4 MiB.
  Refs: Requirement 054-006, Requirement 054-008, Requirement 054-014, Requirement 054-017
- [ ] T054-7 Retained bytes everywhere, the sites of Technical Plan 6: the ledger read returning both values, the charging
  statements' predicates (`charge_logical_output_bytes`, `charge_collection_owner_output`, the path charge), the consumption's move
  of bytes and its exclusion from the output charge, the three `total()` comparisons of `primitives.rs`, the collection-finalization scan of `maintenance.rs`, and
  the three sites that sum `RunData.outputs` (`wake` in `crates/scheduler/src/events.rs`, the retained-size read of
  `crates/scheduler/src/passes.rs`, `Engine::evaluate_run_result` with its `retained` parameter and its two callers). A test per check site of `MAX_RUN_OUTPUT_BYTES` and per
  predicate first (the worker's and scoped outputs, the event wake, the `end` output, collections, loops,
  children, the signal paths): a run with pending payloads refuses, with its typed answer, an output that fits without them.
  Refs: Requirement 054-008
  Owner-gate: OD-2026-11-26-540
- [ ] T054-8 Admission: `deliver_signal` and the chain path through one decision (Technical Plan 3), the engine's resolve of a
  correlation with `take_buffered`, batch items; `crates/api-http`'s list route, the answers and reasons; the capability
  `signal-inbox-v1` known to both process kinds: `Capability::SignalInboxV1` in `crates/application/src/instances.rs` (the variant,
  `ALL`, `as_str`), its row in `specs/018-schedules-bulk-operations/data-model.md`, the test that keeps them equal to the schema's
  lists (`tests/integration/platform-foundation/migrations.rs`) and the registry tests (`apps/control-plane/src/registry.rs`,
  `apps/execution-engine/src/settings.rs`), but not yet listed as offered (Task 054-9 lists it); the readers of a `#` receipt in
  both scopes, `parse_nodes` of the chain path included. Contract tests first (`contract_rest_durable_execution`), then a test per refusal and
  its order (`NO_CONSUMER_AHEAD` included), the idempotency answer and its `#` receipt (with-start replay included), and a wait's
  entry per entry path.
  Refs: Requirement 054-002, Requirement 054-003, Requirement 054-004, Requirement 054-005, Requirement 054-006, Requirement 054-009, Requirement 054-013, Requirement 054-016
  Owner-gate: OD-2026-11-26-540
- [ ] T054-9 Continue-as-new and the store-side ends, and the capability: a build lists `signal-inbox-v1` as offered only from
  this slice on, since before it a run that buffers could not continue or end correctly, so no version of level 1 publishes
  earlier; the move and the discard in both handoffs (`continue_as_new`,
  `continue_evaluated`) in the order of Technical Plan 7, their tolerance of a run version advanced only by `signal.buffered` (and
  `finalize_continuation_expression`'s) and of every path Technical Plan 7 names through `advanced_only_by_buffering`, the predecessor's
  events before its handoff event, the discard in `finalize_chain`, the collection's finish and the loop's two ends with their
  shape matchers, and the rule of `scripts/check-architecture.py` over the writers of `run.finished`.
  Integration tests first: a signal buffered between the scheduler's evaluation and its handoff (barrier), each store-side end
  with pending signals and the check on `runs` refusing a terminal status with pending signals
  (`integration_buffered_signals::ends::<path>_discards_pending_signals`, one per path), the store-side release paths never
  entering a `waitEvent` of a declared name (`integration_buffered_signals::entries::store_side_paths_never_enter_a_declared_wait`), all names declared, some undeclared (a code run's
  continuation under another version, built with design 044's fixtures), an empty inbox (no statement added), the
  successor's first wait consuming the oldest moved signal, replay of both runs.
  Refs: Requirement 054-010, Requirement 054-011, Requirement 054-015
  Owner-gate: OD-2026-11-26-540
- [ ] T054-10 Reset: the inbox hook skipping restored events and the one rebuild statement before the settle reduction (a test
  of a prefix that buffered and consumed a signal, and of a reset after a consumption that reads the step's output), the
  rematerialization of pending rows, the reapply candidates (Signal-with-Start's skipped) and their
  payloads by the journal's digest column, the four new `ReapplyReason` values with their names and parse. Tests first: a
  rejected buffered candidate per new reason (the reset goes on), a with-start buffered signal not reapplied, a
  point with pending signals, a reapplied buffered signal delivered and one buffered, a consumed one not reapplied twice, a purged
  payload, the registry test.
  Refs: Requirement 054-012
  Owner-gate: OD-2026-11-26-540
- [ ] T054-11 Signal-with-Start: deliver or buffer in the creation transaction for a declared name; an open holder as `signalRun`;
  the refusals roll back. The slot-order test with a buffered signal in `tests/fault/schedules-bulk-operations/slot_order.rs` first,
  then the with-start tests.
  Refs: Requirement 054-013, Requirement 054-015
  Owner-gate: OD-2026-11-26-540
- [ ] T054-12 Complete the suites `integration_buffered_signals` and `fault_buffered_signals` (created by Task 054-6, their race
  rows written by Tasks 054-8 to 054-10 as Technical Plan 12 assigns them): the crash and restart, the look-back precedence, the run-driven barrier test with a pending signal, the metrics
  text test, the read tests (no payload, another tenant's run), the replay corpus, and the chain of CUJ 2 handling 500 `order`
  signals one per segment, each consumed once and in order, with every signal accounted for in the chain's journals; every suite also with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
  Refs: Requirement 054-009, Requirement 054-014, Requirement 054-015, Requirement 054-018, Requirement 054-019, Requirement 054-020, Success criterion 054-002, Success criterion 054-003
- [ ] T054-13 The six SDKs: the client's `buffered` and `sequence`, `listPendingSignals`, the builder's `signals` with the shared
  fixture; the conformance scenario `buffered-signals`; each SDK's checks of `sdks/AGENTS.md`.
  Refs: Requirement 054-016, Success criterion 054-001
- [ ] T054-14 The Studio: the declarations in the inspector with the bound and the warning, the run's pending signals and discarded
  counts; component tests first, then the journeys.
  Refs: Requirement 054-001, Requirement 054-007, Requirement 054-016
- [ ] T054-15 Documents for people: a guide page on signals (declared and undeclared names, one consumer, the bound, the look-back
  precedence, the comparison with Temporal, at least once, why a queue segment that continues as new starts with an activity, and a
  high-volume queue in a code version); the runbook `docs/runbooks/buffered-signals.md` (the requirement and its
  floor, the metrics, a full inbox); `docs/sdk-compatibility.md`; the release notes; `contract_developer_experience` regenerated only
  after reading its diff.
  Refs: Requirement 054-007, Requirement 054-019
- [ ] T054-16 Measure Non-functional requirements 054-001 and 054-002: the budget lines of `integration_claim_transition_efficiency`
  (the existing lines recorded as a baseline before slice 1, and run again in every server slice, which must keep them; new lines for a consuming entry, a completion with pending signals, a
  continuation and an end with 1,000 pending), and the time of a continuation moving 1,000 signals, recorded with its environment in
  `tests/performance/reports/`.
  Refs: Non-functional requirement 054-001, Non-functional requirement 054-002, Success criterion 054-004
- [ ] T054-17 The released 0.2 and mixed-fleet scenarios: runs from 0.2 finish unchanged, publication refused before the requirement,
  the floor, rollback before the requirement, a 0.2 client against a declared name.
  Refs: Requirement 054-002, Non-functional requirement 054-004

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`python3 scripts/check-architecture.py` with its fixtures, `python3 scripts/check-agent-config.py`,
`python3 scripts/check-design-docs.py`, `python3 scripts/check-plain-words.py --base origin/develop`, the unit tests of the crates it
changes and of those that depend on them, and the suites below that it changes or that read what it changes
(`scripts/select-test-targets.py`):

- `integration_buffered_signals` and `fault_buffered_signals` (new), also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
- `integration_durable_execution` (undeclared names unchanged), `integration_events_webhooks_messaging` (the look-back unchanged and
  its precedence), `integration_schedules_bulk_operations` and `fault_schedules_bulk_operations` (with-start, reset, batch, slot order,
  run-driven locks), `integration_workflow_primitives` (continuation);
- `integration_claim_transition_efficiency` (the budget lines);
- `integration_platform_foundation` (the migration as the runtime role, the privilege audit, the registry);
- `integration_gitops_versioning` (publication and the hold);
- `contract_rest_durable_execution`, the IR schema test;
- `sdk_conformance` for `buffered-signals` in six languages, and the per-SDK checks of `sdks/AGENTS.md`;
- `(cd frontend/apps/studio && npm run api && npm run check:api && npm test)`;
- the released 0.2 scenario of Task 054-17.

Failure cases (principle VIII): each race of Technical Plan 12 under its barrier; a crash between a buffering's commit and its
answer; a restart with pending signals; a duplicate signal; a refusal after the inbox write (nothing committed); a continuation and
an end with 1,000 pending signals; a reset over pending signals; a removal of the requirement while a version of level 1 exists.
