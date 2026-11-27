# Security review of T022-096 and T022-076 (2026-11-19)

Reviewer: `security-reviewer` subagent, read-only, given the diff `16b0aae8..3366aeb3` with the spec and ADRs (not the
author's conclusions). Folder named for the slice's review date of the brief (2026-11-18).

## Findings

- **Major (latent): the readiness gate ignores the live-keys and waiters-per-key ceilings** (`readiness_gate.rs`,
  `decide_gates`). FR-022-054's `FLOW_CONTROL_LIMIT` and FR-022-047's per-key bound are not applied; with distinct keys a
  caller with `run:start` creates key rows without bound in one namespace. Unreachable in production (D022-037, G1198).
- **minor: return locks taken after the decision locks, unsorted** (`persist_gate_returns`): crossed groups between two
  runs deadlock (40P01).
- **minor: the architecture rule does not follow feature closure**: `default = ["step-gates-for-tests"]` or an alias
  feature in the owning package passes unflagged.
- **nit: the widened `step.failed` from `READY` (code `FLOW_CONTROL_KEY_INVALID`) holds for any node.**
- Checked with no finding: tenant boundary of every new statement, no key value or digest in logs, `Debug`, journal, API
  or projection, least privilege of `sagat_app` (no new grants needed), static SQL, bounded work per transaction, fail
  closed without keys, races under the run lock.
- Suspicions: the holder PK has no attempt (a release and a decision of the same node in one transaction would collide);
  a paused run's release would leave a step undecided.

## Resolution (author, 2026-11-19)

| Finding | Resolution | Evidence |
|---|---|---|
| Major, ceilings | Not built in this part; recorded in G1199 (extended) as a precondition of listing the capabilities; T022-101 and T022-025 own it | `research.md` G1199 |
| minor, lock order | Fixed: the gate round locks what it gives back and what it decides in one sorted pass and gives back first; `persist` locks its own returns sorted before deleting | `readiness::a_release_and_a_decision_lock_their_keys_in_one_sorted_order` (held lock, `pg_stat_activity` barrier), red with the returns left out of the round |
| minor, feature closure | Fixed: the owning package may not enable the feature from another of its features; a production call of `admitting_step_gates_for_tests` is refused | `scripts/check-architecture.py` (`TEST_ONLY_FEATURES`, `step_gates_door_errors`); a call added to `apps/control-plane/src/lib.rs` by hand was reported |
| nit | Fixed: when the state knows its gated nodes, only a gated node may fail that way | `state.rs` (`StepFailed`) |
| Suspicion 1 | Fixed by the lock-order fix: returns are applied before decisions | `readiness::a_permit_released_and_a_successor_decided_in_one_commit_is_counted_once` |
| Suspicion 2 | A paused run releases nothing; a paused run's retry is decided in its commit | `readiness_gate::a_retry_due_in_a_paused_run_is_decided_in_its_commit` |
