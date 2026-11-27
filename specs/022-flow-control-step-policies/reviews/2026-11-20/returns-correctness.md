# Correctness review of T022-021, T022-099 and T022-100 (2026-11-20, second set)

Reviewer: `correctness-reviewer` subagent, read-only, given the working-tree diff with the spec (FR-022-040, FR-022-046),
the plan (T29), the data model (invariants 11 and 13), D022-028 and the code around it (not the author's conclusions).

## Findings

1. **Major: a pause, and now nearly every resume, locks key rows out of the D022-038 order** (returns first, then the
   unsorted rearm of the run's waiter keys; a resume's round, then the rearm).
2. **Major: "ask again on retry" has no test**: the new re-decision over an `ENDED` slot is what lets an operator's retry
   after a hold for review work; before the change that path failed with an invariant error.
3. minor: the code exempts paused steps from invariant 11 more broadly than `data-model.md` says, keyed on the slot
   phase (a hold, then a pause, then a retry also waits undecided); `waiting_kind` no longer implies a waiter row.
4. minor: a pause treats a granted step and a queued step differently: a queued step's deadline runs during the pause;
   a returned step cannot be held for review while paused.
5. minor: the bound helper is close to vacuous (it counts `READY` steps only and no rate grants); `lease_step(...)
   .is_err()` passes on any paused run; untested: a cancellation of a paused run after a return, the retry pass while
   paused against PostgreSQL, a resume that grants and is leased.
6. nit: `pausing = paused && stop.is_none()` is redundant.
- Checked with no defect: replay determinism (journal alone against folded) for a pause-return, a retry while paused, a
  grant while paused, an operator's retry while paused, a resume, a cancellation and a stop while paused; no gated step
  undecided outside a pause; a paused run's queued step as before; races with the grant and timeout passes; the resume's
  re-decision (rows, attempt, hints).

## Resolution (author, 2026-11-20)

| Finding | Resolution | Evidence |
|---|---|---|
| 1 | Fixed: one sorted pass over the returns and the run's waiter keys, in `persist` and in the round | `returns::a_pause_locks_its_keys_in_the_one_sorted_order`, red before the fix |
| 2 | Fixed: tests of a hold then an operator's retry, domain and PostgreSQL | `readiness_gate::a_review_hold_gives_back_and_a_retry_asks_again_for_the_same_attempt`, `returns::a_hold_for_review_gives_back_a_granted_unleased_step` (now with the retry); red with the re-decision over `ENDED` refused |
| 3 | Fixed in part: one domain predicate (`RunState::waits_for_resume`, paused and not stopping); the texts are G022-007 (the approved data model changes only on re-approval); the exception is ADR-0096 (proposed, O1175) | review only |
| 4 | Recorded: G022-008 (a queued step's deadline runs during a pause, not introduced here) and option (c) of O1175; holding a returned step for review while paused is refused as for any waiting step | review only |
| 5 | Fixed: the bound helper counts holders and unleased rate grants of steps that are not leased, per key; tests of a cancellation of a paused run and of the retry pass while paused; a resume that grants and leases is in the domain test | `returns::{a_cancellation_of_a_paused_run_ends_the_step_it_gave_back, a_retry_due_while_paused_is_granted_and_given_back_in_one_commit}`, `readiness_gate::a_step_granted_while_its_run_is_paused_gives_its_gates_back_in_that_commit` |
| 6 | Kept, in the one predicate, so that the rule reads alone | review only |
