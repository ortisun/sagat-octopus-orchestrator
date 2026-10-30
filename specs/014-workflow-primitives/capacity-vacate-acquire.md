# Vacate and Acquire for ChildRoot capacity members (B1d.2 / B2) — proposal D706–D715, revision 2

Status: **Native Vacate and Acquire implemented and validated, still dormant. Owner approved a scheduler-tick sweeper and worker-lease-only possession gates on 2026-10-29; production coordination remains to be implemented.**
Revision 2 followed correctness and architecture reviews; the native Vacate increment has separate
transaction and storage/contract reviews (2026-10-28). Refs: T014-014..017/040/042, ADR-0030 §§13–19, ADR-0037, A0/A1 (`capacity.rs`), B0 lock inventory, B1a
entry decision, `capacity-eligibility.md` D703–D705, `capacity-discovery.md` (B1d.1, read-only, another branch).
Ids: D706–D715, G714–G723, O705–O706 (range D701–D799 / G701–G799 / O701–O799).

The pure transitions `CapacityGroup::vacate` and `acquire` exist and are tested. The transactional
`PgCapacityVacateStore` now uses Vacate for **ChildRoot members only**, dormant, with no production caller.
The native Acquire command is implemented; [its evidence](./capacity-acquire.md) is bound to merged PR65 and CI40868186605 (all12 jobs passed, workspace4028/0/82 existing ignored; native Acquire17/0/0 and Vacate18/0/0). Possession is not a permit: neither command leases, releases, cancels or changes a run, a
step or a chain. Items marked *(review)* come from a reviewer's reading of the code; re-verify them when the
test is written, they are not yet evidence.

## What changed from revision 1

1. **Replay is looked up before the head check** (D707, D711). Revision 1 copied Finish's order, which checks the
   head first. Finish can do that because a FINAL head never moves; these commands target a head that can
   continue, so a lost acknowledgement followed by a continuation would answer `StaleHead` instead of the receipt.
2. **Acquire no longer claims to prove there is work to run** (D709). Revision 1's list of "schedulable work"
   was neither the scheduler's definition nor sufficient *(review)*: a `DELAYING` loop scope whose timer is due
   (`loop_progress.rs:1180-1209`), an unrouted terminal collection step (`20261205000000_bounded_execution_discovery`)
   and `run_sagas`/approval/child-link obligations (`maintenance.rs:105-145`) are work that has no `READY` step,
   while a `READY` step held back by activity admission or an open breaker is not runnable. Safety (one holder)
   does not need the claim; liveness is restored by a sweeper (D714).
3. **Vacate on idle is a sweeper, not an inline edge** (D714, O705 restated). An inline Vacate inside the
   report or step transaction would take the group row `NOWAIT`, so another member's Acquire would make an
   unrelated worker report fail with `Busy`; it also loses the edge on a crash and touches about eight writer files.
4. **Two slices, Vacate first** (D715).
5. The soundness argument for the Vacate gate is rewritten (D708) and the evidence list is corrected (below).

## Decisions

- **D706 — Scope.** `member_kind='CHILD_ROOT'` only, matching B1d.1 discovery. `SCOPED_PATH` members have no
  chain and a different authority; they get their own commands **and their own receipt table** (G716), so the
  transfer table never needs the nullable-binding rewrite that `20261212000000` needed for Finish. A
  `SCOPED_PATH` member given to these commands is `BindingMismatch`, never `Missing` (tested, mixed-kind group
  included). As in Finish, the member's chain must have a `child_chain_links` parent, else `BindingMismatch`.
- **D707 — Entry and order.** Read the member (discovery only) → exact chain `FOR UPDATE` → exact head run
  `FOR UPDATE` (on a replay this is the historical run the command names) → group header `FOR UPDATE NOWAIT` →
  re-read the member under the group → scheduler fence and database clock (`locked_scheduler_time`, sampled after
  the shard guard; the shard `FOR SHARE` is a blocking wait bounded by lease renewal, as in Finish) →
  **receipt lookup** (same key: fingerprint conflict, or return the original result) → current-head check →
  CAS on group revision, epoch, chain revision and run revision → gate → A1 transition → writes. One group, one
  run, never a second existing run; `55P03` is a distinct `Busy` that consumes the whole transaction and a retry
  starts fresh. A replay still needs a valid binding and a current scheduler fence; a lost fence fails closed.
- **D708 — Vacate gate (quiescence), structural.** The selected member is the holder (`NotHolder` otherwise) and
  ACTIVE. The chain's current head equals the observed head, and the head is **not final** (a FINAL head belongs
  to Finish, which already vacates; `finality()` becomes a three-way verdict final / not final / contradictory and
  a current `CONTINUED` head is contradictory; a historical continued head can replay its receipt). **No step of the head run is `LEASED` or `RUNNING`**, expired or not; the gate
  reads no clock. `READY`, `PENDING`, `WAITING` and `RETRY_WAIT` steps may remain. *Soundness:* the only statements
  that set a step to `LEASED` or `RUNNING` are in `execution.rs` (`persist`, about line 5749) and
  `execution/path.rs` (about line 1122) *(review)*, and both run under chain `U` then run `U`; the probe is a plain
  read under that run lock. This is the invariant, not "every writer enters through chain then run" (`bindings.rs`
  and a parent `FOR NO KEY UPDATE` path do not, but cannot produce those statuses). Known limits, recorded not
  hidden (G723): a worker whose lease was reaped may still be running as a zombie, which the gate cannot see; and an
  activity that blocks inside a worker waiting on a descendant keeps Vacate refused indefinitely.
- **D709 — Acquire gate, minimal and honest.** Group `VACANT` at the expected epoch; member ACTIVE; same head and
  not-final conditions; head run status movable (`Running`, `Waiting`, `Cancelling`, `Compensating`; `Paused` and
  `ManualReview` cannot move, terminal goes to Finish). The match over `RunStatus` is exhaustive, no wildcard.
  **It does not assert that the member has work.** A member that acquires with nothing to run is quiescent, so D714
  vacates it; the cost is bounded latency, not a stuck group. Proving work needs one shared runnable predicate
  (G715, O706) and is not part of this slice.
- **D710 — Writes.** The header (one UPDATE of revision, epoch, possession, holder under the held row and CAS), the
  audit, the outbox and the receipt, and nothing else: no run, step, chain, journal, member or quota write
  (asserted by row counts and `xmin`, which also catches a no-op UPDATE but not a lock-only change). Locks are
  not writes: Acquire's deferred holder FK takes `KEY SHARE` on the member row at COMMIT and the receipt FKs
  take it on member, chain and group. No cycle is known, because the only update-strength writer of a member row
  is Finish and it needs the group first; Vacate sets the holder to NULL and skips that check.
- **D711 — Receipt.** One append-only `capacity_group_transfer_receipts`, key `(tenant_id, group_id,
  operation_key)`, `kind IN ('VACATE','ACQUIRE')`, the 32-byte fingerprint, `chain_id NOT NULL`, the four expected
  counters, and the result: revision, epoch, **`resulting_active_members`** (a replay cannot rebuild it once other
  members join or finish), possession, holder, time, audit id and outbox id. Foreign keys copied from the Finish
  table: member binding `(tenant, group, member, namespace, chain)`, the chain, and the result holder.
  `UNIQUE(tenant, group, resulting_revision)` per table, CHECKs `resulting_revision = expected_group_revision + 1`,
  `resulting_epoch = expected_group_epoch + 1`, and possession tied to kind (VACATE → `VACANT` with no holder;
  ACQUIRE → `HELD` by the member). **The table is evidence, not authority**: `sagat_app` has INSERT, so no
  constraint can stop a row whose counters disagree with the header; the header is the authority and a revision
  is not unique across this table and the Finish table. (Revision 1's "a receipt cannot be forged" is withdrawn.)
  The fingerprint is domain-separated (`sagat.capacity.transfer.child-root.v1`, kind byte, fixed-width ids,
  big-endian counters, renewable scheduler authority excluded) with an independently generated test vector.
  A replay returns the original result, `replayed=true`, and says nothing about current possession: a caller
  compares epochs.
- **D712 — Typed errors, telemetry, audit, events.** Errors: `Busy`, `Missing`, `BindingMismatch`, `StaleHead`,
  `StaleGroupRevision`, `StaleGroupEpoch`, `StaleChainRevision`, `StaleRunRevision`, `SchedulerFenceLost`,
  `NotHolder`, `Occupied`, `NotQuiescent`, `NotMovable`, `InactiveMember`, `ChainFinal`, `ContradictoryProjection`,
  `FingerprintConflict`, `CounterExhausted`, `Storage`, `CommitOutcomeUnknown { operation_key }`. The domain error
  mapping is an **exhaustive match with one test per variant**; Finish's `_ =>` arm would turn `NotHolder`,
  `Occupied` and `StaleEpoch` into `ContradictoryProjection`. Telemetry: one span with the label `operation`
  (`vacate`|`acquire`, not `kind`, which clashes with `member_kind`), the Finish outcome vocabulary plus a closed
  refusal class of about six values so that `NotQuiescent`, `NotMovable` and `Occupied` are not folded into one
  `refused`; an attempts counter and a duration histogram labelled by operation and outcome, and one transition
  counter per operation, as Finish has. Audit actions `CapacityMemberVacated` and `CapacityMemberAcquired`: the
  `repeat_window` and `distinct_cap` tables end in `_ => None`, so "every table decided" is not compiler-enforced;
  add the assertion `domain/tests/audit.rs` has for Finish that neither action collapses repeats. Outbox hints
  `capacity.member.vacated` and `capacity.member.acquired`: channel, operation and message in
  `events.asyncapi.yaml`, `events.baseline.json`, `hint_contract.rs`, the contracts README. A hint is a hint: a
  lost one must not strand a group (D714).
- **D713 — Commit uncertainty is the Finish contract, with one classifier.** Only server-confirmed constraint,
  serialization and deadlock aborts imply rollback; anything after COMMIT dispatch is `CommitOutcomeUnknown`, reconciled
  by re-sending the retained command. `confirmed_abort` exists three times already *(review)*; make one `pub(crate)`
  and use it (a one-line visibility change in `capacity_finish.rs`, not a fourth copy). Reuse
  `CapacityFinishIdentity` and `CapacityFinishExpectations` as they are (same shape and meaning); do not rename
  them here and do not mirror them. The new module holds only what differs: the command with its operation, the
  fingerprint prefix, the result, the error enum.
- **D714 — Convergence is a level-triggered sweeper (proposed, O705).** Possession cannot be taken (A0), so a
  holder that cannot progress must be vacated by the system: an idle `Waiting` holder, a `Paused` or
  `ManualReview` holder, a holder that just acquired with nothing to run, and a holder whose head is FINAL but not
  yet Finished (that one is released by Finish, not Vacate). A scheduler-tick sweeper does this: it pages `HELD`
  groups whose holder looks quiescent (bounded pages, an index on `possession='HELD'`, like B1d.1 discovery),
  and for each calls the command in its own transaction, one run lock at a time. It is idempotent and crash-safe,
  adds at most one tick of latency, never runs inside a worker or step transaction, and leaves the scheduler's
  run transactions and the other lane's files untouched. A lost hint is harmless because waiting members find a
  vacancy by the same discovery. The sweeper and its discovery query are the caller of G719, not this slice.
- **D715 — Two slices.** (V) Vacate: the shared migration, receipt table, vocabulary, ports and the structural gate;
  no clock, no dependency on discovery. (A) Acquire: after V, once G715/O706 say what the gate may claim. A
  `RunStatus` variant added by the other lane breaks the exhaustive match on purpose; tell that lane.

## Same-delivery footprint

`crates/domain` (new `execution/capacity_transfer.rs` with the pure gates; `execution/mod.rs`: one `mod` line next
to `mod capacity;` and a change to the `pub use capacity::{…}` block; `audit.rs` and its test),
`crates/application/src/capacity_vacate.rs` and `lib.rs`, `crates/infra-postgres` (adapter, `lib.rs`, migration
with column-list INSERT grant and a down that refuses retained receipts, `storage-classification.json` whose
existing "no retained content/log delta" wording must be revisited), `crates/telemetry/src/capacity.rs` and
`lib.rs`, the AsyncAPI contract and baseline, `specs/014-workflow-primitives/contracts/README.md`,
`runtime_privileges.rs`, `docs/traceability.md`, `validation-report.md`, `docs/owner-decisions.md`,
`docs/adr/README.md`, a runbook note ("group stuck HELD by an idle holder, or VACANT with an eligible member") and
`tests/integration/workflow-primitives/capacity_transfer*.rs`. **It does touch two files the other lane edits**
(`crates/domain/src/execution/mod.rs` and `tests/integration/workflow-primitives/main.rs`, one `mod` line each, plus
`hint_contract.rs`): do it as one small additive commit so the collision stays a rebase of a re-wrapped `pub use`
block and an alphabetical list.

## Required evidence (each seen failing for the right reason first, runtime role and owner role)

1. Pure gates: table over every `RunStatus` and lease fact, plus a property test against an independent reference.
2. Both commands, happy path: exact result, header and receipt, one audit, one outbox; row counts and `xmin` of
   runs, steps, chains, members and journal unchanged.
3. Vacate: refused with a `LEASED` and with a `RUNNING` step, expired lease included; allowed with `READY`,
   `PENDING`, `WAITING`, `RETRY_WAIT`; refused for a non-holder, for a final head, for a `SCOPED_PATH` member
   (`BindingMismatch`) and for a member without a parent link.
4. **Races with barriers, never sleeps:** a claim transaction holds chain and run behind a barrier, Vacate waits,
   the claim commits `LEASED`, the retained command returns `StaleRunRevision`, and a refreshed command
   returns `NotQuiescent`; also test the reverse order; Vacate against Finish of the
   same member; two members racing to Acquire in both orders (one winner, the loser is `Busy`; its stale command on
   retry is `StaleGroupRevision`, and only a refreshed command sees `Occupied`); a held group row gives `Busy` and
   releases the chain and run guards (independent probes).
5. **Replay:** after the state moved on; after the member re-acquired and leased a step; **after the chain
   continued to a new head**; after the scheduler lease was lost (fails closed); the same key with another
   command is `FingerprintConflict`; `replayed=true` after a later vacate carries the old epoch.
6. Commit: a confirmed abort versus a lost acknowledgement, reconciled through the receipt, using the Finish
   commit-loss harness. The deferred-edge test uses the test-only deferred constraint trigger Finish uses
   (`capacity_finish_observations.rs`) and holds the member row at update strength; holding
   `tenant_retained_usage` would block nothing here and pass vacuously.
7. Cross-tenant, cross-namespace and member/chain mismatch are inert; an absent group is never "vacant".
8. Runtime-privilege audit passes for every statement; the down migration refuses with retained receipts; the
   `confirmed_abort` classifier has its own unit test.
9. Fitness (rule 21): the reviewed `contracts/step-status-writers.json` records every extracted literal SQL
   statement that writes `steps.status` (31 statements in six files, including non-lease writers). The
   platform-foundation inventory test rejects additions or changes. This conservative source inventory does
   not prove bind-expression semantics or locks; native claim barriers and review remain necessary. C1 must
   add possession checks before activation. Regenerate explicitly with
   `SAGAT_UPDATE_STEP_WRITER_INVENTORY=1 cargo test -p sagat-tests --test integration_platform_foundation -- step_status_writer_inventory_is_explicit_before_capacity_activation`,
   read the diff, then rerun without the update flag.
10. A lost hint (rule 9) is covered when the sweeper exists; until then the gap is recorded, not tested.

## Open obligations

- **G714 — Index for the lease probe (implemented; CI validation pending).** `steps_run_open_idx (tenant_id, run_id)` covers
  five open statuses *(review)*, so the success case walks every open step of the run (thousands on a 10k-item
  collection). Migration `20261226000000` adds the partial index on `(tenant_id, run_id) WHERE status IN ('LEASED','RUNNING')` with an `EXPLAIN` availability probe. That forced-index probe is not evidence of natural planner choice
  or large-workload performance.
- **G715 — Runnable predicate.** The scheduler has no single definition (`maintenance.rs` `due_*`, `workers.rs`
  `CLAIMABLE`, the loop and collection discovery). Before Acquire may claim work, one shared `macro_rules!` SQL
  fragment under the scheduler and Acquire plus a differential test over every wake and step status, boundary
  times, the scoped flag on and off, a loop `DELAYING`, an unrouted terminal step and a gated `READY` step.
- **G716 — `SCOPED_PATH` members:** same two commands, own authority, own receipt table.
- **G717 — Possession is not read by anyone** (rule 11): nothing gates a claim, so this slice cannot change
  behaviour. C1 gates worker claims and, per O706, possibly scheduler transitions.
- **G718 — Fairness and starvation:** not claimed.
- **G719 — The caller is the D714 sweeper,** its discovery query and its `HELD` index; never two run locks.
- **G720 — Receipt retention:** two receipts, two audit rows and two outbox rows per vacate/acquire cycle, append-only
  and unbounded except by the group's `bigint` revision. Decide a per-group ceiling or a compaction path.
- **G721 — ADR before any production caller:** the possession protocol, the lock position of the group header
  (chain, run, group `NOWAIT`) and the D714 sweeper. B0/ADR-0037 say the lock graph is not accepted and the order lives
  only in handoff notes. Take the ADR number when it is written, after grepping every worktree.
- **G722 — Which runs are members.** The schema admits `CHILD_ROOT` and `SCOPED_PATH` only. A top-level run parked on
  a child is not a member today; who the holder is, and so whether the O705 scenario can occur, depends on creation (B1e).
- **G723 — Zombie workers and blocking activities** (D708).
- **O705 (owner) — Approve D714?** The holder that cannot progress is vacated (or Finished) by a scheduler-tick
  sweeper in its own transaction, instead of inline when it becomes idle. It costs up to one tick of latency.
- **O706 (owner) — What does possession gate?** Only worker leases, or also scheduler-owned transitions (timer fire,
  retry release, control nodes, child outcomes)? If only leases, a due-but-unfired wake is not Acquire work and the
  clock leaves the gate; if both, the scheduler's own transactions are gated, which is the larger change.

## Progress (slice V)

- **2026-10-27 — pure gates (evidence item 1).** `capacity_transfer.rs` in the domain with `vacate_gate`,
  `acquire_gate` and `CapacityTransferRefusal`; exhaustive `match` over `RunStatus`, no wildcard. Seen failing
  first against a stub that accepted everything (5 failed, 1 passed, the passing one being the list-coverage
  check). After the implementation:
  `cargo test -p sagat-domain --test capacity_transfer --locked` 6 passed, 0 failed;
  `cargo test -p sagat-domain --locked` 40 targets, 550 passed, 0 failed, 0 ignored; strict `cargo clippy -p
  sagat-domain --all-targets --locked -D warnings`, `cargo fmt --check`, `check-architecture.py` (24 crates, no
  cycles) and `check-agent-config.py` pass. Private `CARGO_TARGET_DIR`, `CARGO_PROFILE_DEV_DEBUG=0`, 2 jobs. The
  truth table in the test is hand-written and is the specification; no mutation run (paused by the owner).
- **2026-10-28 — native Vacate increment, CI pending.** Application command/fingerprint, immutable transfer
  receipts (migration 26), PostgreSQL transaction, typed errors, audit, closed telemetry and the additive
  `capacity.member.vacated` AsyncAPI hint are implemented. Eighteen native cases cover ordinary/replayed
  transitions, leased and expired leases, CAS/fence/binding refusal, continuation replay, NOWAIT release,
  atomic rollback, concurrent duplicates, real worker-claim serialization, scheduler expiry, lost COMMIT
  acknowledgements, migration/grants/index availability, mixed member kinds and OTLP/metrics.
  Independent read-only transaction review found no defect; storage review corrected the classification
  bound for the reserved `ACQUIRE` discriminator from six to seven bytes.
- **Validation record:** fingerprint stub RED: three failed; native adapter stub RED: three failed. The first
  native implementation run returned eight passed and one fixture error (`lease_until` was not a column);
  the fixture now uses the existing `age_lease` helper. AsyncAPI checks returned three passed/one missing
  baseline failure, followed by one explicit baseline-regeneration pass; the additive diff was read.
  An expanded native run was interrupted for disk exhaustion and is **not acceptance evidence**. Its
  diagnostic output exposed two test-harness issues, corrected with recursive blocker discovery and the
  existing COMMIT proxy's required constraint name. The complete corrected suite, fingerprint tests,
  non-regeneration baseline, runtime grants and released-source profiles must pass CI.
- The standalone existing-source SQL scanner rejected an empty writer inventory (31 missing statements);
  this negative control is not a Cargo test result. The checked-in inventory is validated by the CI test.
- No production caller, Acquire, sweeper, claim guard, creator or quota/backfill is enabled. The migration's
  down path is an operator rollback with services quiesced, and refuses retained receipts. G720 receipt
  retention, O705/O706 and other activation obligations remain open. Nothing is ticked in `tasks.md`; the
  eight canonical tasks stay open.
