# ADR-0096: A paused run gives back what its granted, unleased steps hold, and they ask again on resume

**Status:** Accepted (2026-11-20, OD-2026-11-20-2, "Aceitar ADR-0096": O1175 option (a))
**Decided by:** agent proposal (feature 022, T022-100, 2026-11-20) after the security, correctness and architecture reviews of that day (`specs/022-flow-control-step-policies/reviews/2026-11-20/`); accepted by the owner (OD-2026-11-20-2)
**Refines / supersedes:** partly supersedes ADR-0094 decisions 2 and 3 (accepted 2026-11-19, OD-2026-11-19-3): a step that a pause made give back its gates is not asked while the run is paused, and `persist` accepts it waiting without a decision. The rest of ADR-0094 stands.
**Enforced by:** `crates/domain/tests/readiness_gate.rs::{a_pause_returns_the_gates_of_a_granted_unleased_step_and_its_resume_asks_again, a_step_granted_while_its_run_is_paused_gives_its_gates_back_in_that_commit, a_review_hold_gives_back_and_a_retry_asks_again_for_the_same_attempt}`; `integration_flow_control_step_policies::returns` (one test per path: pause, hold for review, cancellation, run end, retry after a hold, retry pass while paused, cancellation while paused, and the pause's sorted lock order against a held key row); `RunState::waits_for_resume`, the one predicate the planner, invariant 11 (`flow_control_breach`) and `persist` share.

## Context

FR-022-046 (approved) says that "pausing a run MUST return the slot (`rate_limit.returned`) and the permit
(`concurrency.released`) of each of its gated steps that is granted and not leased, and make them wait again, and
resuming it MUST make them ask again". Plan T29 and D022-028 say the same, so that granted, unleased steps never exceed a
key's limit (invariant 13) and a paused run does not hold a key it cannot use.

ADR-0094, accepted on 2026-11-19, says the planner asks `Evaluation::Gates` for every gated step that waits without a
decision, "running or paused" (decision 2), and that `persist` refuses a gated step waiting without a decision
(decision 3). A step that a pause made give back its gates waits without a decision until the resume: the two texts
disagree. The build of T022-100 follows the spec; this ADR records the exception, which the owner accepted (OD-2026-11-20-2).

Facts the design rests on:

- `verify_run` replays the journal alone, without knowing which nodes are gated, and must reach the state the
  transactions folded. A wait must therefore follow from journaled events: a step that becomes ready while the run is
  paused (the retry pass, an operator's retry) is decided in that commit, as ADR-0094 says; a return of a step that is
  `READY` while its run is paused makes it wait, in the reducer, from the return events themselves.
- A slot that an attempt gave back before its lease ends `ENDED` for that attempt; asking again is for the same attempt.

## Decision

1. Pausing a run MUST settle it (`execution::pause_run` returns a `Progress`): the planner gives back, in the pause's
   commit, the permit and the slot of every gated root step that is granted for its next attempt and not leased, and the
   reducer makes it wait again (`WAITING`, `Gate`) with no decision of that attempt.
2. While the run is paused, the planner MUST NOT ask such a step (`RunState::waits_for_resume`), and `persist` and
   invariant 11 accept it waiting without a decision; its `waiting_kind` is its first declared gate, rate first. Any
   other gated step that waits without a decision is still refused (ADR-0094 decision 3).
3. A step made ready while the run is paused is decided in that commit (ADR-0094 decision 2); a grant is given back at
   once by decision 1, a wait keeps its place in its queue.
4. Resuming asks again, for the same attempt; the reducer accepts a decision over a slot of that attempt that was given
   back unleased (`ENDED`). An operator's retry after a hold for review asks the same way.
5. A pause or a resume locks every key it touches (the returns and the keys the run's waiters are on) in one sorted pass,
   semaphores then windows, before any of them is written (D022-038).
6. *Added 2026-11-26, delegate (OD-2026-11-20-6; feature 022 decisions D022-138 and D022-139; ADR-0098 decision 3):* a
   step whose lease is withdrawn (`step.lease_withdrawn`, nothing ran) is again granted and not leased: it keeps its
   permit and its slot, is leased again without a new decision, and is given back by decisions 1 to 4 like any other.
   Its rate grant MUST count as unleased again: the withdrawal's commit clears the lease instant the scheduler may have
   recorded, the scheduler records the attempt's last lease, and a give-back deletes the grant whatever lease instant a
   withdrawn lease had left on it.

## Considered options

1. **Give back on pause, ask on resume, decide what becomes ready while paused** — chosen (OD-2026-11-20-2): it is FR-022-046 as
   written, it keeps the journal the only source of truth for every wait, and it changes nothing for a step already
   queued.
2. **Keep ADR-0094 as written: a paused run keeps its grants** — no exception, but a paused run holds keys it cannot use
   and can exceed a key's limit of granted, unleased steps for the length of the pause (against FR-022-046 and
   invariant 13).
3. **Every gated step of a paused run waits undecided, including one made ready while paused** — uniform, but the
   journal alone would then show such a step `READY` (no event says it waits), so `verify_run` could not agree without a
   new journal fact.

## Consequences

- Good: a paused run holds no granted, unleased step; the key serves its other waiters meanwhile.
- Bad: a step granted while its run is paused takes and gives back in one commit (key locks and two journal events per
  retry); the acquisition counters count a re-decision on resume again.
- Bad: a step given back on pause goes to the back of its key's queue on resume, and its key is evaluated again over
  the run's data: a key that no longer evaluates fails `FLOW_CONTROL_KEY_INVALID`, and a key that must be created again
  can meet the namespace's live-keys ceiling (`FLOW_CONTROL_LIMIT`), as any readiness decision can.
- Unchanged and still open: a step already queued when its run pauses keeps its deadline, which runs during the pause
  (G022-008).
- Compatibility: no journal event or contract changes; no gated run exists in production before the capabilities are
  listed (D022-037).

## Evidence

The tests under "Enforced by", each seen failing under a hand mutation (the planner's return on pause removed, the
re-decision over an `ENDED` slot refused, the exemption of invariant 11 removed, the planner asking while paused, the
pause's key locks taken out of order). Spec FR-022-040, FR-022-046; plan T29; research D022-028, D022-048.
