# ChildRoot capacity coordination

Refs: T014-014–017/040/042–044, FR-014-004/005/014/015, G715/G717/G719/G721.
Base: `develop@888074e6`, after native Acquire PR65. Own branch/worktree:
`feature/014-capacity-coordinator` / `sagat-octopus-capacity-coordinator`.

## Owner scope

O705: scheduler-tick sweeper, a separate transaction per native operation.
O706: possession gates only granting worker leases. Internal scheduler progress and
existing-lease reports/heartbeats remain governed by their current semantics.
These explicit answers are recorded in `docs/owner-decisions.md`; they are not execution evidence.

## Ordered increments

- [x] Record the actual O705/O706 answers and identify cross-session ownership.
- [x] Inventory current capacity readers/writers and claim/discovery gates; define an ADR
  for the group lock position and caller boundaries before adding a production caller (G721).
  ADR-0075 is proposed; root/scoped `WorkerService` are the two `LeaseGrant` producers.
  The activation/source inventory fitness check passes, with five negative/positive tests.
  This text scan is not a general Rust call-graph proof and does not close runtime G721.
- [x] Add the pure worker-possession prerequisite, independently of runnable-work eligibility.
  `capacity_worker_possession`: RED 0 passed / 6 failed (wrongly accepted facts); after the
  implementation, `cargo test -p sagat-domain --locked --quiet`: 614 passed / 0 failed /
  0 ignored across 46 target summaries. Private build directory; no PostgreSQL claim yet.
- [ ] Guard all worker-lease grants under the existing chain/run and group protocol;
  prove both claim/transfer orders, continuation, expiry, tenant and nonmember compatibility.
- [ ] Establish a shared worker eligibility predicate and differential evidence (G715).
  Candidate selection currently includes sampled refusal rows and treats activity quotas as
  a hint; its result must not be misrepresented as an exact lease eligibility verdict.
- [ ] Add fenced, bounded ChildRoot sweep discovery and independent native-command progress,
  with held-lock/restart/lost-hint tests and closed outcomes/OTel/audit.
- [ ] Wire the scheduler-tick caller only after the preceding proofs and source inventory,
  then run integrated gates and update actual acceptance evidence.

## Coordination and implementation constraints

The other bounded-execution session owns dirty changes in domain `command.rs`, `mod.rs`,
`path_state.rs`, `scoped_compensation.rs`, PostgreSQL `execution.rs` and `byte_ceilings.rs`.
The schedules session owns feature018 in the misleadingly named `capacity-protocol` worktree.
Iceberg, elastic scaling, classification and parity worktrees are also read-only inputs.
Prefer new feature-specific files; before any necessary shared call site, inspect the other
session's actual diff and keep the hunk separate. Never stash/reset/clean its code.

Current registry uniqueness is group-scoped: it does not establish that a child chain has one
active membership globally. The claim boundary must not silently choose an arbitrary group.
Resolve exact membership/ambiguity and unsupported scoped paths explicitly before activation;
do not infer that owner approval supplied a missing database invariant.

No new tenant capacity counter or pausing after policy reduction (D704/D705). No receipt replay
as present authority, external exactly-once promise, fabricated eligibility or premature nesting
activation. Canonical tasks stay unchecked until their complete original acceptance is met.
Heavy builds/real PostgreSQL validation use CI per owner preference; local checks remain light
while other sessions build. All temporary build targets are private and owned.

## Evidence

Owner answers were received in this conversation on2026-10-29. Initial read-only inventory:
capacity state access is currently confined to native metadata/domain modules; no production
caller exists. This is an observation to be enforced and reviewed, not a runtime safety proof.
`workers.rs` documents that candidate discovery samples refused secret-pool steps and only
hints at activity admission. Native transfer/Finish already lock chain → own run → group NOWAIT
→ current scheduler fence, with immutable receipt reconciliation. Their previously passing
native tests are regression prerequisites, not new coordinator acceptance.

The PostgreSQL RED test ran in CI40988195080/job123844042134 on1079a1a0:
`capacity_finish::worker_leases::vacant_child_stays_ready_without_consuming_an_attempt_or_quota`
failed at `vacancy grants no external lease`: 0 passed / 1 failed / 0 ignored, 10.29s.
The build used the cache and took 1m17s. The superseded preliminary workflow was cancelled
after collecting this expected failure, to release shared CI runners; it is not a full gate pass.
The adapter scaffold was then replaced with the shared transactional guard. Nine focused
PostgreSQL tests and the integrated gates await CI; production activation remains off.

`python3 -m unittest discover -s scripts/tests -q`: 70 tests, OK, one pre-existing skip.
The new fitness check separately observed RED (five failing negative subcases), then GREEN
(five tests pass). Formatting, architecture (24 crates), agent configuration and actionlint pass.
