# Security review of `efabe42d..d70a60940` (task T022-116; decisions D022-137 to D022-139), 2026-11-26

The repository's read-only `security-reviewer` read the three commits of `feature/022-slice-activation`. No Blocker and no
Major; two minors and two nits. The coordinator ruled that every security finding is fixed, none accepted. The responses
are in `activation-review-responses.md`.

## Minor 1. The give-back lost `AND leased_at IS NULL`

- **Where:** `crates/infra-postgres/src/execution/readiness_gate.rs:1278-1282` and `:1342-1345`.
- **Scenario:** the same as correctness minor 1: a defence against a mistake of the domain is gone, so a wrong
  `rate_limit.returned` would delete a grant that still counts and let the key grant one more slot in its window.
- **Asked:** one fix serves both findings.

## Minor 2 (documentation). Gap G022-041 is easier to reach than its text says

- **Where:** `crates/application/src/worker/withdrawal.rs:242-265` (the hand-back verdict); gap G022-041; the runbook,
  section 6.
- **Scenario:** a tenant's own host on protocol 1.8 can keep a lease past its window and then send `FailStep`
  `TASK_NOT_STARTED`. The withdrawal has no time bound, so the step is leased again in the same window as another grant:
  at any period, at will, within one tenant. The gap's text speaks of a rare lease that waited, most likely with
  `per: second`.
- **Asked:** correct the gap and the runbook; better, if there is a sound way, refuse (or count again) a withdrawal of a
  rate-gated lease whose recorded slot has already stopped counting, so that the gap closes instead of being documented;
  weigh it and decide.

## Nit 3. The guard lets the runtime role rewrite a lease instant

- **Where:** `20270217000000_flow_control_withdrawn_lease.up.sql`, line 28.
- **Scenario:** `sagat_app` may clear a recorded instant and then record an earlier one. Not an escalation: the role
  already holds `DELETE` on `rate_limit_grants`.
- **Asked:** say in the migration's comment that the guard is a defence against bugs, not against the role.

## Nit 4. The key version read again under the epoch lock does not serialize with key-version commands

- **Where:** `crates/infra-postgres/src/instances.rs:494-503` (`ActivationPrerequisite::holds`) and `:528-532`
  (`census_step`).
- **Scenario:** the read takes no lock on the key version row, so a future command that deactivates a version would not
  wait for the activation that relies on it.
- **Asked:** read the row `FOR SHARE`; the test proves serialization, not only the second read.
