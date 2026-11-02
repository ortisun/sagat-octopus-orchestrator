# ADR-0050: Many-run operations admit runs under stable keys in one lock-ordered transaction, and never buffer a signal or reopen a run

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26)
**Refines / supersedes:** refines ADR-0020 (shard fence reused for schedules, batch jobs and pending resets), ADR-0039 (rerun source guard reused by reset) and ADR-0030 decision 17 (kept unchanged for Signal-with-Start); depends on proposed ADR-0056 (022 run permits); supersedes none.
**Enforced by:** pending tests of feature 018 — the exhaustive reset-rule table test (every `RunEvent::name()` has a rule, T018-052), the slot-order barrier tests of T018-012 extended to the drafts' admission paths, `fault_schedules_bulk_operations` (crash per statement of a firing, stale fence, duplicate pass), the lock-order interleaving harness (T018-012), the run-driven barrier test against every aggregate table (T018-076), the lock monitor of T018-049, the reset replay-equals-projection corpus including a 022 permit holder and 020 search attributes (T018-055, T018-077), the no-buffer assertions of T018-040, and the contract baselines (worker protocol unchanged; IR grows only by `reapplyOnReset`). Until they exist the rules are review-only.

## Context

Feature 018 adds schedules, business identifiers with Signal-/Update-with-Start, batch operations
over many runs, and reset to a point. Each creates or changes runs on behalf of something that is not
one run: a schedule, a business identifier, a batch job, a reset request. Four forces shape them:

- **Constitution I** — state, next tasks, timers and outbox commit together; a terminal run never
  returns to running; the first commit wins a race.
- **ADR-0030 decision 17** — a signal is accepted only by a step already waiting and consumed in that
  transaction; there is no pending signal. Temporal's Signal-With-Start relies on a buffered signal;
  Temporal's reset and `TerminateOther` overlap policy rely on terminate, which this platform does not
  have (cancellation drains attempts and compensates, constitution III).
- **ADR-0020** — scheduler work is fenced by a shard lease checked in the action's transaction; the
  fence bounds redundant work, a key makes the result correct.
- **Deadlock freedom** — a schedule, a binding and a job each relate to many runs; runs reach terminal
  states in transactions that hold the run's lock. If those terminal transactions had to update the
  aggregate (to free a business id, to wake a buffered schedule action, to count a batch item), the
  aggregate-then-run paths (Signal-with-Start, `CANCEL_OTHER`) would form cycles.

Existing evidence: `crates/scheduler/src/events.rs::cron_pass` starts a cron run under key
`cron-{epoch}` and moves the trigger in a second statement, which is safe only because of the key;
`crates/application/src/runs.rs` refuses a signal no step waits for; a released `waitEvent` becomes
`WAITING` inside the creation transaction (`crates/domain/src/execution/plan.rs`), so a signal can
be delivered at creation without a buffer; `RunState::replay` rebuilds state for any run id.

## Decision

1. **Stable keys.** A schedule action MUST be unique per (tenant, schedule, identity origin, nominal
   time), where scheduled and backfill actions share origin `SPEC` and trigger-now has origin `MANUAL`;
   its run MUST be admitted under idempotency scope `schedule:{id}` and key `nominal-{epoch ms}` or
   `manual-{epoch µs}` (trigger-owned schedules keep `trigger:{id}`/`cron-{epoch}`). A batch item MUST be unique per (job, run) and act
   under key `batch:{job}:{run}`. A business identifier MUST be bound to at most one open execution
   per namespace at any commit. A backfill or a catch-up MUST NOT create a second action for a nominal
   time that has one.
2. **One transaction.** A schedule action that starts a run MUST commit the fence check, the schedule
   compare-and-set, the action row, the run admission and initial reduction (journal, tasks, timers,
   outbox, audit) and the new schedule state together. A refused admission MUST commit nothing of the
   action and leave the nominal time owed. Each batch item MUST commit its run change, item outcome
   and job counters together. A reset MUST commit the new run, its re-emitted prefix, the reset marker,
   reapplied effects and the reset record together.
3. **Lock order.** Every admission follows the merged slot order of decision 7. Within it, only two
   aggregate orders exist: (a) **aggregate row → run** for a transaction that creates runs or commands
   an existing run on behalf of an aggregate (schedule action, business-id paths, reset completion):
   slot 3 → slot 3R; (b) **run → aggregate bookkeeping row** for a transaction whose subject is one run
   (batch item → job → item, `CANCEL_OTHER` obligation, `CANCEL_SOURCE` request → pending-reset
   insert): slot 3R → slot 8a. A batch item transaction MUST NOT hold a second run lock; order (b) MUST
   NOT lock a slot-3 row after its run; order (a) MUST NOT lock a job, item or obligation row. **No
   transition driven by the run itself (step completion or failure, timer, wait timeout, deadline,
   terminal transition, continuation handoff) MAY lock or write a schedule, business-id, batch job,
   batch item, obligation or pending-reset row.** Aggregates learn that a run ended by reading its
   status, which is safe because terminal status is monotonic.
4. **No buffered signal.** Signal-with-Start MUST signal an open holder exactly as `signalRun`, or
   create a run and deliver the signal in the same transaction to steps waiting after the initial
   reduction, or roll back completely. Reset reapply MUST follow the same rule. No table MAY hold a
   signal for later delivery.
5. **No reopen, no terminate.** Reset and rerun MUST create a new run with provenance (`rerun_of`,
   `reset_sequence`). A reset run's journal MUST be self-contained: the source events `1..N` are
   re-emitted under the new run identity, followed by `run.reset`, and its replay MUST equal its
   projection without reading the source. Every re-emitted row MUST carry `restored: true` so any
   journal-derived projection (telemetry, feature 020 analytics, ADR-0056's holder count) skips it. The
   reset transaction MUST pass feature 022's run-permit admission and MUST NOT queue: without a free
   permit it is refused, and `run.reset` clears any restored permit state before the fresh
   acquisition. Only reset points with no attempt in flight, no waiting step, no active scope, no
   compensating saga, no unknown outcome and no later compensation of a restored step are eligible;
   022 test-tree runs and purged sources are not sources. A source that is not terminal MUST either be refused or be cancelled
   through ordinary cancellation before the new run is created. No overlap, conflict or source policy
   MAY terminate a run.
6. **Fence for passes only.** Schedule, batch and pending-reset passes MUST check the shard fence of
   their aggregate id (`run_shard(id)`) `FOR SHARE` in each transaction. HTTP commands (trigger-now,
   with-start, reset) MUST NOT invent a scheduler lease; they serialize on rows.

7. **Merged admission-transaction lock order (coordinator decision C4).** Every transaction that admits
   a run (start, trigger, schedule action, child, rerun, reset, with-start, batch rerun, 022 standalone
   and test runs, 026 public submissions, 030 handler admission) MUST take its locks in the slots below,
   in increasing order, never returning to a lower slot; inside a slot, in the order listed. Drafts cite
   these slot numbers.

   | Slot | Rows, in order | Notes |
   | --- | --- | --- |
   | 1 | 026 public-link row or 027 app-session row (`FOR UPDATE`); the 029 access-capability check (read, no lock) | entry of the request; no run-holding transaction ever waits for these rows |
   | 2 | request receipt: idempotency lookup, then insert-or-lock of the receipt row | duplicates of one request serialize here, before any aggregate |
   | 3 | run-creating aggregates: 018 schedule row or pending-reset row → 018 business-id row; 030 `START` operation message. Pass-driven work reads its shard fence (`FOR SHARE`, ADR-0020) at the start of this slot | ADR-0050 order (a) |
   | 3R | existing runs the transaction acts on: chain before run (014); the parent reservation of a child admission; the ADR-0039 source identity guard of a rerun or reset; an existing run commanded for an aggregate (018 Signal-/Update-with-Start holder) | implemented order: 017 and ADR-0039 take these before the catalog lock and memberships |
   | 4 | 4a namespace fences and authority: 017 namespace secret/catalog lock → 021 variable fence (writers) → 024 data-catalog fence (writers) → memberships `FOR SHARE` (017 helpers) → 029 access fences (shared) → namespace row; 4b catalog heads and pins (`FOR SHARE` on admission): 019 dispatch-catalog fence → 019 deployment fences (id order) → 017 connection heads → 021 resource heads → 021 variable heads → 024 asset heads → 025 provider and MCP-server heads → 030 service-endpoint head → 030 allow-list entries | ascending id within each kind |
   | 5 | secret rows (ascending) → secret ACL rows | |
   | 6 | new run rows: retained definition (017) → tree and active-run admission → payload rows → run insert with journal, steps, tasks, timers, outbox, audit (parent before child) → run pins (017 connections, 019 deployment, 021 bindings, 024 data assets) | |
   | 7 | 022 semaphores by (pool, scope, group, key) | ADR-0056 decision 2 |
   | 8 | 8a bookkeeping rows of a transaction whose subject is one existing run (018 batch job → item, obligation, pending-reset insert); 8b quota, rate and usage rows last, including 025 `ai_usage_daily` and the deferred content flush (ADR-0027) | usage rows are always last |

   Transitions on an existing run (claims, completions, approvals, stream appends, operation messages,
   agent rows) start at slot 3R with the run (chain before run), then take the rows their own ADR lists
   (019 claim fences, 025 agent rows, 026 stream heads, 030 operation record and message), never a
   slot-3 row, and end with slot 8b. One exception is listed: a 030 caller transaction (deadline or
   cancellation) may lock its own existing `START` operation row (slot 3) after its run (3R), because
   the only aggregate-first transaction on that row is handler admission, which creates its run and
   never locks an existing run or a caller-side row (030 D1660). Any other deviation is recorded and
   aligned before implementation (`specs/018-schedules-bulk-operations/research.md`, G1605).
8. **Reset rules.** Every journal event type MUST have exactly one rule (`COPY`, `REMATERIALIZE`,
   `EXCLUDE`, `REFUSE`) in the registry of `specs/018-schedules-bulk-operations/contracts/journal-and-events.md`;
   a reset point whose prefix contains a type without a rule MUST be refused. `REMATERIALIZE` work runs
   in the reset transaction in the slots of decision 7.

9. **Fleet capability activation.** A feature that needs every control-plane or engine process to honour
   it MUST be activated through the instance-capability registry
   (`specs/018-schedules-bulk-operations/data-model.md`): processes register before serving under the
   requirement epoch, heartbeat on the database clock and refuse to serve while a requirement for their
   kind names a capability they lack; a requirement MUST be recorded only by an audited operator
   activation that locks the epoch, verifies every live instance of the required kinds and records it in
   one transaction (never inside a tenant or user transaction); until then a feature refuses its own use. No feature MAY define its own
   member table for this purpose (029 and 031 use this one).

## Considered options

1. **Keyed one-transaction admissions with the lock order above (chosen)** — reuses the run creation
   transaction, the idempotency table and the shard fence; every aggregate is consistent with its
   runs at each commit; no new lock mode or service.
2. **Buffered signals for Signal-with-Start and reset reapply** — matches Temporal more closely, but
   contradicts ADR-0030 decision 17 and feature 004's accepted rule, needs a lifecycle for undelivered
   signals (expiry, transfer across continuation, quotas) and changes what a 409 from `signalRun`
   means. Rejected.
3. **Terminal transitions update aggregates** (free the business id, wake the schedule, count the
   item) — fewer reads, but creates run → aggregate edges opposite to aggregate → run paths and
   couples every terminal transition to three new tables. Rejected for deadlock risk and write
   amplification on the hottest path.
4. **Reset by reference** (the new run's journal points at the source prefix instead of re-emitting
   it) — cheaper storage, but a run's replay would depend on another run's retention and deletion,
   and telemetry and disclosure would have to resolve two journals. Rejected.
5. **A separate scheduling service with its own leader election** — ADR-0020 already rejected an
   authority outside the database; the fence must be in the action's transaction.

## Consequences

- Signal-with-Start works only for workflows whose entry waits for the signal; documentation and the
  reason `SIGNAL_NOT_ACCEPTED_AT_START` must say so. Developers used to Temporal will meet this.
- Buffered schedule actions and pause-on-failure notice run completion with the latency of the
  schedule pass's re-check, not instantly.
- Reset copies up to the prefix ceiling (default 5,000, hard 20,000 events) in one transaction; its
  cost must be measured (G1005). Parallel workflows have fewer eligible reset points (G1001).
- Rollback: disabling admission of schedules, with-start, batches and resets by configuration keeps
  existing runs valid; the tables stay readable; the down migration refuses to drop rows that still
  hold authority or evidence.
- Runs in flight are unaffected: no existing transition gains a lock or a write.
- Coupling with drafts 020 and 022: every run creator of this feature goes through 022's admission
  (T10); a reset re-creates 020's search-attribute rows from the re-emitted upserts and refuses a purged
  source (`RUN_PURGED`). Reapplying Updates needs their arguments, which are retained only for Updates
  declared `reapplyOnReset` (one additive IR field) to keep the hot path unchanged for the rest.

## Evidence

None yet: this is a proposal. Design: `specs/018-schedules-bulk-operations/plan.md` (transaction table,
lock order, failure matrix), `research.md` D1001, D1004, D1005, D1013, D1014, D1016, D1023, D1024, D1032, D1035, D1036, D1037, D1605, D1606.
Required before acceptance: the tests named in **Enforced by**, run against real PostgreSQL with the
runtime role, and a benchmark of firing lag and reset transaction time on a declared environment.
