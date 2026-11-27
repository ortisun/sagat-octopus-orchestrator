# ADR-0050: Many-run operations admit runs under stable keys in one lock-ordered transaction, and never buffer a signal or reopen a run

**Status:** Accepted (2026-10-29). ~~Decisions that rely on the run permit of proposed ADR-0056 (feature 022) stay gated until that ADR is accepted.~~ (ADR-0056 was accepted on 2026-11-13, OD-2026-11-13-17, so the gate is lifted; corrected 2026-11-27, feature 022 decision D022-312.) Decision 2 is refined for a refused run input by ADR-0092 decision 7 (2026-11-16): such an action is recorded `INPUT_INVALID` and its time acted on, not owed.
**Decided by:** owner, accepting the agent proposal of 2026-10-26 (`docs/owner-decisions.md`, 2026-10-29)
**Refines / supersedes:** refines ADR-0020 (shard fence reused for schedules, batch jobs and pending resets), ADR-0039 (rerun source guard reused by reset) and ADR-0030 decision 17 (kept unchanged for Signal-with-Start); depends on ADR-0056 (022 run permits; ~~proposed~~ accepted 2026-11-13, corrected 2026-11-27, D022-312); supersedes none.
**Enforced by:** for decision 7, the lease row's leaf rule by `scripts/check-architecture.py` (CI `architecture`, with its tests in `scripts/tests/test_check_architecture.py`), and the slot order of the admission paths that exist by `slot_order` (`tests/fault/schedules-bulk-operations/slot_order.rs`, T018-012): each path runs against a reference transaction that holds one lock of the path's table at a time, for every lock but the last, and takes the rest in the order of the slots, and the path must not deadlock with it. It is a **release gate**: a pull request runs `fault_schedules_bulk_operations` only when its files change, so a change of an admission path that does not touch them is caught at the release. It proves the order of the locks the references name for the paths of its cases (the schedule pass, trigger-now, backfill, a buffered release, the payment of a `CANCEL_OTHER` debt, the release of a delayed start, the business-id claim, and the request and completion of a pending reset), and nothing about a lock the reference does not name, a path it does not run, or another interleaving; for decision 3, the run-driven barrier test (`run_driven_locks.rs`, T018-076); for decision 9 (fleet capability activation), the tests listed in [ADR-0082](ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md) (task T018-080: the activation race held by locks, the guards, the privileges, the closed gate of a control plane and of an engine); pending tests of feature 018 — the exhaustive reset-rule table test (every `RunEvent::name()` has a rule, T018-052), the slot-order barrier tests of T018-012 extended to the drafts' admission paths, `fault_schedules_bulk_operations` (crash per statement of a firing, stale fence, duplicate pass), the lock-order interleaving harness (T018-012), the run-driven barrier test against every aggregate table (T018-076), the lock monitor of T018-049, the reset replay-equals-projection corpus including a 022 permit holder and 020 search attributes (T018-055, T018-077), the no-buffer assertions of T018-040, and the contract baselines (worker protocol unchanged; IR grows only by `reapplyOnReset`). Until they exist the rules are review-only. *Added 2026-11-27 (feature 022 decision D022-309):* the feature 022 cases of `slot_order` (`tests/fault/schedules-bulk-operations/slot_order.rs`): the new creators, a publication against a draft test admission and the retained private definition (task T022-224); a binding replacement against a lease-expiry pass that releases a cacheable successor (task T022-254); a `RunRecovery` admission against a revocation of its binding, both orders, as `sagat_app` (task T022-259); the completion of a pending reset of a `RunRecovery` requester against a revocation (task T022-262).

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
   | 3 | run-creating aggregates: 018 schedule row or pending-reset row → 018 business-id row; 030 `START` operation message. A pass reads its shard fence in the same transaction; the lease row is a leaf lock, outside this order (below) | ADR-0050 order (a) |
   | 3R | existing runs the transaction acts on: chain before run (014); the parent reservation of a child admission; the ADR-0039 source identity guard of a rerun or reset; an existing run commanded for an aggregate (018 Signal-/Update-with-Start holder) | implemented order: 017 and ADR-0039 take these before the catalog lock and memberships |
   | 4 | 4a namespace fences and authority: 017 namespace secret/catalog lock → 021 variable fence (writers) → 024 data-catalog fence (writers) → memberships `FOR SHARE` (017 helpers) → *2026-11-26 (feature 022 decision D022-277):* the actor's `run_recovery_namespaces` row of the namespace `FOR SHARE`, when the actor's role is namespace-bound (*2026-11-27, D022-297, D022-314:* taken through `public.sagat_lock_run_recovery_binding`, since the runtime role holds no UPDATE on the table; after the memberships where the path takes them, and for a person's start, which takes none, the only authority lock) → *2026-11-25 (D022-269):* the namespace's draft-executions advisory key, shared, for an admission that reads the setting → 029 access fences (shared) → namespace row; 4b catalog heads and pins (`FOR SHARE` on admission): 019 dispatch-catalog fence → 019 deployment fences (id order) → 017 connection heads → 021 resource heads → 021 variable heads → 024 asset heads → 025 provider and MCP-server heads → 030 service-endpoint head → 030 allow-list entries | ascending id within each kind |
   | 5 | secret rows (ascending) → secret ACL rows | *2026-11-25 (feature 022 decisions D022-254, D022-265):* a cacheable entry step's lookup locks: the secret-access and membership locks of its secret ids (shared, ascending), then an unlocked pre-read of the entry's `producer_subject`, then that subject's membership lock and the activity's binding advisory lock (shared); written into this table 2026-11-26 (D022-281) |
   | 6 | new run rows: retained definition (017) → tree and active-run admission → payload rows → run insert with journal, steps, tasks, timers, outbox, audit (parent before child) → run pins (017 connections, 019 deployment, 021 bindings, 024 data assets) | *2026-11-25 (feature 022 decision D022-235):* the journal, step, task and outbox rows are written by `persist` after slot 7; the slot names the lock order, and those inserts lock nothing another transaction waits for |
   | 7 | 022 semaphores by (pool, scope, group, key) | ADR-0056 decision 2. *2026-11-25 (feature 022 decision D022-235):* run-scope semaphores and windows, then the cache scope, then the cache entry (`FOR SHARE`, ADR-0135, the producer re-checked on it, a change `unauthorized`, D022-265), then step-scope semaphores and windows |
   | 8 | 8a bookkeeping rows of a transaction whose subject is one existing run (018 batch job → item, obligation, pending-reset insert); 8b quota, rate and usage rows last, including 025 `ai_usage_daily` and the deferred content flush (ADR-0027) | usage rows are always last |

   **The lease row is a leaf lock, outside the slots (D2083).** A row of `scheduler_shards` is taken `FOR SHARE` by every fence (`holds_shard`, `check_fence`) and `FOR UPDATE SKIP LOCKED` by the keeper (acquire, renew and release: one autocommit statement each, which never waits for a row and takes no other row). A shared lock never conflicts with a shared lock, and the only exclusive taker never waits and holds nothing else, so no transaction waits for the lease row while holding another lock, and none holding it waits for another: it can be in no cycle, and its position relative to the run rows and the aggregate rows is immaterial. The passes of the aggregates take the fence before their aggregate row and the passes that act on one run (timer, lease, deadline, release of a delayed start) after the run's row, and both are correct for that reason. What would break it is a statement that locks the row more strongly (`FOR UPDATE`, `FOR NO KEY UPDATE`, `UPDATE`) outside the keeper, a keeper statement that waits (no `SKIP LOCKED`), or a keeper that reads a run: `scripts/check-architecture.py` refuses each of them. (The deadlock the first slot test met, against a reference that took the lease `FOR UPDATE` and then a run row, was the reference's: it modelled a keeper that does not exist.)

   Recorded deviation (aligned, T018-057): the `CANCEL_SOURCE` request takes its source row (3R), then the pending-resets quota lock (8b), then inserts the entry (8a), then writes the receipt it read in slot 2; no cycle, argued in ADR-0081 decision 5.

   Recorded deviation (D2223, 2026-11-13): a batch `RERUN` item that keeps its business identifier takes the business-id row (slot 3, order (a)) and, after the new run (6), its job and item rows (8a), in the one transaction decision 2 asks for; order (a) otherwise never locks a job or an item. No cycle: no transaction that holds a job or an item row locks a business-id row or a run, and the other transactions that take a business-id row first lock no job or item.

   *Amended 2026-11-24, delegate (OD-2026-11-24-4; feature 022 decision D022-214, ADR-0136 and ADR-0137 proposed):* the
   slots of feature 022's new creators. A namespace failure-handler dispatch takes its dispatch row (`FOR UPDATE SKIP
   LOCKED`) in slot 3, as a run-creating aggregate like a pending-reset row; the failed source run's identity guard
   (`KEY SHARE`, ADR-0039) in 3R; the handler workflow's head in 4b; the handler run in 6; and updates its dispatch row
   (already locked) in 8a. A draft test run takes the `fromRun` source runs (`KEY SHARE`, ascending id) in 3R and the
   draft's workflow row (`FOR SHARE`, the revision check) in 4b, never before slot 1 as feature 022's plan row T16 first
   said; its pin payload rows, `test_run_pins` rows and the upstream pins' `step.mocked` events are new run rows of slot
   6, written with the run. A standalone step takes 018's business-id row in slot 3. Every one of them then takes its
   gates in slot 7 through `Engine::start`.
   *Revised 2026-11-25, delegate (OD-2026-11-24-4; feature 022 decisions D022-222, D022-223, D022-240):* the draft's
   workflow row and the handler workflow's head are read without a lock (a moved draft is compared by its revision),
   because publication and activation lock the workflow row before the namespace's secrets; nothing of the two is in
   slot 4b any more. The dispatcher locks its dispatch row inside `RunStore::create` through a `NewRun` field, as a
   pending reset does. The private definition of a standalone step or a draft test is retained where 017's `create`
   retains it today, `ad_hoc::retain` with its workflows quota lock before the slot-4a locks, not in slot 6: a deviation
   inherited from 017, proved by the slot-order case of feature 022's task T022-224.

   Recorded deviation (feature 022 decision D022-223, 2026-11-25): the terminal commit of a run that qualifies for the
   namespace failure handler inserts its dispatch row, a slot-3 run-creating aggregate, on a transition of an existing
   run. No cycle: the row is new, so no transaction can wait for it before the commit makes it visible; its key is the
   failed run, whose lock serializes every writer of it; and the dispatcher locks only committed rows (`SKIP LOCKED`).
   `scripts/check-architecture.py` refuses an insert into `failure_handler_dispatches` outside the terminal path of
   `persist`, and a lock of one outside the dispatcher and the redrive.
   *Extended 2026-11-25 (feature 022 decision D022-242):* to the lineage key. The insert is `ON CONFLICT (tenant_id,
   namespace_id, lineage_run_id) WHERE lineage_first DO NOTHING`, followed when it inserted nothing by a second insert of
   a `SUPPRESSED_LINEAGE` row; both are followed in the run's own commit only by the `run.finished` row of the run's own
   journal, then usage rows (decision D022-280, 2026-11-26). The first may
   wait for another run's uncommitted first row of the same lineage; that wait is a leaf wait, because after it the
   waiting commit takes only rows of its own run and usage rows, and the other commit takes nothing of this run.

   *Accepted 2026-11-25, delegate OD-2026-11-24-4, after three review rounds recorded in
   specs/022-flow-control-step-policies/reviews/2026-11-25/ (feature 022 decision D022-271):* the feature 022 amendments
   of this decision dated 2026-11-24 and 2026-11-25 (the new creators' slots, the slot-5 lookup locks with the pre-read
   of the producer, the slot-7 cache rows and re-check, the slot-6 note, the recorded deviation of the dispatch insert
   and its lineage key). *Added 2026-11-25 (decision D022-269):* an admission that reads the draft-executions setting
   holds the namespace's advisory key for it shared in slot 4a.
   *Written 2026-11-26 after the re-reads of `1276d5b7`, under the acceptance of D022-271 (feature 022 decisions D022-277,
   D022-280, D022-281):* the slot table above carries the slot-5 pre-read and producer lock, the slot-7 re-check and the
   slot-4a draft-executions key; an admission or a command on an existing run whose actor's role is namespace-bound
   (`RunRecovery`) reads the actor's `run_recovery_namespaces` row `FOR SHARE` in slot 4a after the memberships *(how it
   is read is superseded by the paragraph of 2026-11-27 below: through a lock function, after the memberships where the
   path takes them; marked 2026-11-27, feature 022 decision D022-331)*, and the
   revocation that deletes it takes only its audit and usage rows after it, so it joins no cycle (a `slot_order.rs` case,
   task T022-259). *Added 2026-11-26 (decision D022-288):* the completion of a pending reset of such an actor reads the
   same row in its re-check of the requester, right after `sagat_lock_memberships`, in slot 4a *(through the lock
   function since 2026-11-27, decision D022-297)*.
   *Written 2026-11-27 after the third re-reads (feature 022 decisions D022-297, D022-298, D022-314); its acceptance, and
   that of the paragraph above, is recorded after the short re-read of these fixes (OD-2026-11-25-2 step 4):* the
   binding row is taken through `public.sagat_lock_run_recovery_binding` (`SECURITY DEFINER`, `FOR SHARE OF` the row,
   executable by `sagat_app` alone), since a row lock needs UPDATE privilege. The binding sits after the memberships
   where the path takes them; a person's start takes none (the reference of `slot_order.rs:696-697`), and for a
   `RunRecovery` start the binding row is then the only authority lock, which is enough: a foreign key from the binding
   to the membership with its role (`ON UPDATE RESTRICT`) refuses a role change while the row exists, and a membership
   deletion's cascade waits for the row, so the one shared lock fences an unbind, a role change and a membership removal
   alike. A membership deletion takes the membership row and then, by its cascade, the binding rows, the order of slot
   4a; no later slot of a write by the subject takes the subject's own membership.

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

~~None yet.~~ *(Corrected 2026-11-27, feature 022 decision D022-331: the ADR is accepted, and part of its evidence
exists: the slot-order references of `tests/fault/schedules-bulk-operations/slot_order.rs`, task T018-012, and the
lease row's leaf rule of `scripts/check-architecture.py`.)* ~~This is a proposal.~~ (Accepted 2026-10-29; corrected 2026-11-27, feature 022 decision D022-312.) Design: `specs/018-schedules-bulk-operations/plan.md` (transaction table,
lock order, failure matrix), `research.md` D1001, D1004, D1005, D1013, D1014, D1016, D1023, D1024, D1032, D1035, D1036, D1037, D1605, D1606.
~~Required before acceptance:~~ Still owed by the accepted ADR (reworded 2026-11-27, decision D022-331): the tests named
in **Enforced by**, run against real PostgreSQL with the runtime role and recorded here, and a benchmark of firing lag
and reset transaction time on a declared environment.
