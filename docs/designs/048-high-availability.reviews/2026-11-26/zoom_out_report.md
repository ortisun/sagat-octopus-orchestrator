# egm-zoom-out on design 048, full round of 2026-11-26

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 1e28351031ae51b7b6625a86f4c4350790ffd2c1
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing.

## Major

1. The stale-tolerant read is not needed for 0.3.0 (Requirement 048-021; Task 048-10): an optimisation with no measured
   bottleneck that gains no availability and contradicts feature 032's D1580 ("No stale reads"). Fix: a follow-up, with
   `StaleReadPool`, the header, the architecture rule and the stale-pool branch of the `25006` rule.
2. Nothing runs the Kubernetes reference in 0.3.0 (Task 048-3 has static checks only; Task 048-12 is unscheduled), yet
   Success criterion 048-002 and User story 048-01 depend on it. Fix: a free-runner smoke (3 kind nodes, one pod
   deletion), or label the reference unverified; drop the Barman backups (disaster recovery, not high availability).
3. Copying the former primary's data directory is redundant: the driver's ledger checked against the new primary already
   catches any lost acknowledgment. Fix: drop the copy; keep the negative control on the ledger check.
4. The release-gating Compose drill depends on design 051's driver; `tests/performance/scalability-ha/dr.rs` already has
   `dr_acked_workload` and `dr_verify`. Fix: reuse them.
5. Detaching connections is over-built: it serves only "both standbys gone", a deliberate write stop, and doubles every
   process's connection budget. Fix: waiting commits keep their slots and the acquire timeout answers 503.
6. Commit routing is enforced by a fragile lint over about 236 `.commit()` sites. Fix: `begin_safely` returns a newtype
   whose only commit is the deadline commit; keep the lint only for pool-level writing calls.
7. The outcome rule treats all of class 40 as "not applied", including `40003` (statement completion unknown);
   `confirmed_abort` (`capacity_finish.rs`) already has the right set: 23, `40001`, `40P01`. Fix: generalise it.
8. `durability_beacons` duplicates the instance registry, whose heartbeat already has its own connection, a row per
   instance and a write that waits for a standby. Fix: confirm with an immediate registry heartbeat.

## Nit

1. The 310 s report window is sized to the release gate, but leases last 30 s and a late report gets `STALE_ATTEMPT`.
2. The confirmation for one-run commands looks speculative; they are idempotent and run by an operator.
3. Some derivations (probe timing, budget arithmetic, the jitter clamp) belong in tests or the runbook.
4. A three-member etcd on Compose costs runner resources, and no drill injects an etcd fault.
5. Feature 032's `plan.md` describes a beacon on every new connection, and its `tasks.md` sets `synchronous_commit` in
   replay paths; correct both in Task 048-1.

Counts: Blocker 0, Major 8, Nit 5
