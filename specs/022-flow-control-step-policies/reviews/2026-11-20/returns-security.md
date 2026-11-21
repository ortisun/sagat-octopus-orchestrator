# Security review of T022-021, T022-099 and T022-100 (2026-11-20, second set)

Reviewer: `security-reviewer` subagent, read-only, given the working-tree diff of the three tasks (the domain's pause,
return and re-decision, the adapter's waiting kind, the tests) with the spec (FR-022-046) and the code around it (not the
author's conclusions).

## Findings

1. minor: pause and resume now lock keys out of order (the returns' windows, then the rearm's semaphores; a resume's
   round, then the rearm): a deadlock (`40P01`) with a concurrent decision, an availability problem only.
2. nit: each pause and resume cycle counts a second acquisition of the same attempt; a release is not counted; the pause
   audit record does not say what it gave back (the journal does).
3. nit: a step granted while its run is paused takes and gives back in one commit (lock work, two journal events).
4. nit: the doc comment "a gate already decided for this attempt never is again" is now wrong.
5. minor (suspicion): a resume re-evaluates the key and the ceilings, so a step that held a grant can fail
   `FLOW_CONTROL_LIMIT` or `FLOW_CONTROL_KEY_INVALID` on resume.
6. minor (suspicion): a pause and resume cycle writes 2 + 4N journal events (N granted, unleased steps).
- Test gaps: a cancellation or deadline of a paused run whose step was given back; a pause racing the grant routine.
- Checked with no finding: the tenant boundary (`run:operate` before the run is opened; every statement bound to tenant
  and run; a key frees only for runs of the same tenant and namespace); nobody gets more than a key's limit (returns under
  the key locks, the reservation rule on resume, the grant routine skips paused runs under the same run lock); no key value
  or digest in the new errors, events, projections or test output; no new grant or dynamic SQL.

## Resolution (author, 2026-11-20)

| Finding | Resolution | Evidence |
|---|---|---|
| 1 | Fixed: a pause or resume locks the keys of its returns and of the run's waiters in one sorted pass before it writes any (`rearmed_keys` in `persist_gate_returns` and in the readiness round) | `returns::a_pause_locks_its_keys_in_the_one_sorted_order` (a held semaphore row, a `NOWAIT` probe of the window), red with the rearmed keys left out of the pass |
| 2 | Recorded: ADR-0096 (consequences) and G022-001 (telemetry); the journal records every return | review only |
| 3 | Accepted and stated in ADR-0096 (option 3, the uniform alternative, would leave a wait the journal does not explain) | `returns::a_retry_due_while_paused_is_granted_and_given_back_in_one_commit` |
| 4 | Fixed | `flow_control.rs` doc comment |
| 5 | Accepted and stated in ADR-0096 (consequences): a resume is a readiness decision like any other | review only |
| 6 | Accepted: bounded by the gated steps of the run; the operation needs `run:operate` and an idempotency key | review only |
| Test gaps | Cancellation of a paused run whose step was given back: tested. The pause against the grant routine is serialized by the run's lock (`SKIP LOCKED` in the routine), as the reviewer found | `returns::a_cancellation_of_a_paused_run_ends_the_step_it_gave_back` |
