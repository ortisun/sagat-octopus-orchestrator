# Correctness review of `efabe42d..d70a60940` (task T022-116; decisions D022-137 to D022-139), 2026-11-26

The repository's read-only `correctness-reviewer` read the three commits of `feature/022-slice-activation`: the documents,
the fold of `flow-control-activate` into `capability-activate`, and the withdrawn gated lease. One Major, four minors and
two nits. The coordinator relayed them; the responses are in `activation-review-responses.md`.

## Major 1. The purge can delete the grant of a step that is still leased

- **Where:** the purge predicate `purgeable_grant!` (`crates/infra-postgres/src/lib.rs:13-25`), used by the purge
  (`crates/infra-postgres/src/maintenance.rs:1438-1450`) and by a decision's reclaim of its namespace's windows (decision
  D022-066, `readiness_gate.rs` `reclaim_windows`); `require_lease` (`crates/domain/src/execution/command.rs:254`) checks
  no lease expiry.
- **Scenario:** a step's lease lives longer than its window plus a minute (a lease time to live longer than `per`). The
  lease pass records the lease instant; once the window and a minute have passed, the purge deletes the grant though its
  step is still `LEASED` at the grant's attempt. A later withdrawal clears no row, silently. Every later pause,
  cancellation or end of the run emits `rate_limit.returned`, whose row is gone, and fails
  `corrupt("the row a flow-control event gives back")` (`readiness_gate.rs:1308-1310`, `:1362-1363`) on every retry: the
  run is stuck.
- **Variant:** the purge's predicate on its alias `o` is judged on the purge's snapshot, so it can delete a row a
  withdrawal has just cleared.
- **Asked:** the purge and the reclaim keep a step grant whose step is `LEASED` at the grant's attempt, evaluated so that
  the recheck after a concurrent withdrawal is right; `clear_withdrawn_lease` fails when it updates no row unless this
  transaction gave that attempt back. Test, red first: lease, record the lease, purge as of now plus the window plus 61
  seconds, withdraw, pause: the pause succeeds.

## Minor 1 (also security minor 1). The give-back lost `AND leased_at IS NULL`

- **Where:** `readiness_gate.rs:1278-1282` (the read of a `rate_limit.returned`) and `:1342-1345` (its delete).
- **Scenario:** a mistake of the domain that emits `rate_limit.returned` for an attempt whose lease ran would now delete
  a grant that still counts, freeing a slot early, instead of failing.
- **Asked:** keep the filter except for the attempts this same transaction withdraws (`leased_at IS NULL OR (node,
  attempt)` among this commit's withdrawals), at least for `limit_scope = 'STEP'`. Test: a return of a recorded grant that
  was not withdrawn is refused.

## Minor 2. The consistency check does not look at the lease instant

- **Where:** `key_consistency`, `tests/support/flow_control.rs:564-625`.
- **Scenario:** nothing checks a step grant's `leased_at` against the journal (it must be empty, or the instant of the
  attempt's last `step.leased`, empty while the lease pass has not recorded it), nor that every granted, unleased step
  slot in `steps.flow_control_gates` has its grant row. The bugs D022-138 fixed would not have been seen by it.
- **Asked:** add both checks.

## Minor 3. One test does not test what its comment says

- **Where:** `withdrawn_leases::a_pause_after_a_withdrawal_gives_back_the_permit_and_the_slot`.
- **Scenario:** it passes with the old filter (`leased_at IS NULL`), because the withdrawal's own commit cleared the
  instant before the pause; its comment says it proves the give-back of a recorded instant.
- **Asked:** reword it, or add a pause in the same transaction as a withdrawal.

## Minor 4. The evidence dismisses failures

- **Where:** the evidence lines under T022-116.
- **Scenario:** the whole target had four failures in each run; three were set aside as "passed when rerun alone", one of
  them the lock-order test `rate_limit_locks::the_timeout_of_a_buffered_start_takes_the_run_then_its_counter_row_then_the_window`.
- **Asked:** another agent of the coordinator examines those failures and the two batch-rerun tests; add a task line for
  decisions D022-138 and D022-139 (task T022-117) with the tests seen failing first.

## Nit 1. The race test's barrier releases on any lock waiter

- **Where:** `withdrawn_leases.rs`, about lines 378-391.
- **Asked:** wait until `pg_blocking_pids` of the pass's backend contains the holder's backend.

## Nit 2. The down migration's comment is wrong

- **Where:** `20270217000000_flow_control_withdrawn_lease.down.sql`.
- **Scenario:** after a rollback, the older build's lease pass records the attempt's first lease (the withdrawn one), so
  the slot stops counting early; the comment says the grant stays unleased until the next lease is recorded.
- **Asked:** say so.
