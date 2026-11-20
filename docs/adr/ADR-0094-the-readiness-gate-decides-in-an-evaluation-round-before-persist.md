# ADR-0094: The readiness gate decides in an evaluation round, before `persist`

**Status:** Accepted (2026-11-19, OD-2026-11-19-3, "Sim, como recomendado": O1171 option (a))
**Decided by:** agent proposal (feature 022, T022-096, 2026-11-19) after the security, correctness and architecture reviews of that day (`specs/022-flow-control-step-policies/reviews/2026-11-18/`); accepted by the owner (OD-2026-11-19-3)
**Partly superseded by:** ADR-0096 (accepted 2026-11-20, OD-2026-11-20-2) for decisions 2 and 3: a step that a pause made give back its gates is not asked while its run is paused, and `persist` accepts it waiting without a decision. The rest of this ADR stands.
**Refines / supersedes:** refines ADR-0089 decision 2 ("One adapter at the persistence point"), where it says that `persist` collects the gate keys after the reducer. The rest of ADR-0089 stands, decision 1 (the reducer's mapping) included.
**Enforced by:** `crates/domain/tests/readiness_gate.rs` (the planner asks `Evaluation::Gates`, an answer that leaves a declared gate undecided or fails anything but an asked step's key is refused); `integration_flow_control_step_policies::readiness` (one test per readiness source, the release and the decision on one key counted once, the sorted lock order with a held lock); `verify_run` (the journal alone agrees; invariant 11); `persist` refuses to write a gated step waiting without a decision.

## Context

ADR-0089 decision 2 places the readiness gate in `persist`: after the reducer, `persist` collects the gate keys the
transition touched, locks them once (semaphores, then windows, sorted, before usage rows) and journals a grant or a wait for
each gated step. Building it (T022-096) showed two things that decision does not let the build do:

- `persist` charges the run's usage rows (slot 8b) first: it cannot lock a key row before them, and a key row locked
  after a usage row breaks the lock order of ADR-0050 decision 7 and D022-038.
- A key that does not evaluate fails its step `FLOW_CONTROL_KEY_INVALID` through the step's failure route (FR-022-054).
  Routing a failure is the planner's work, and the planner may then ask for another evaluation (a condition on the error
  route); `persist` cannot run the planner or answer an evaluation.

## Decision

1. The reducer maps the readiness events of a gated node to `WAITING`, as ADR-0089 decision 1 says
   (`RunState::with_gated_steps`).
2. Once the planner has nothing else to emit it asks `Evaluation::Gates` for every step that waits without a decision of
   each gate it declares, running or paused (*partly superseded by ADR-0096 decision 2: not a step a pause gave back, while its run is paused*). The application evaluates the keys over the run's data; the adapter
   (`RunTx::decide_step_gates`) files and locks, once, every key the transaction decides or gives back under (semaphores,
   then windows, each sorted), gives back what the transaction's events release first, counts, and returns the grants or
   the waits. The domain checks the answer and journals it in the same commit.
3. `persist` writes no task-ready hint for a step left waiting, refuses to write a gated step waiting without a decision
   (*partly superseded by ADR-0096 decision 2: except a step a pause gave back, while its run is paused*),
   and gives back first, under the same sorted locks, whatever the round did not.
4. Every consumer of `Progress::pending` answers `Evaluation::Gates` or refuses (fail closed). The collection- and
   loop-finish paths, which write their successor's step row directly, refuse a gated successor (G1197) until they are
   integrated with the round.

## Alternatives

- **As ADR-0089 decision 2 says, inside `persist`.** Locks the keys after the usage rows (out of order) and cannot route a
  key failure.
- **Before the transition, in each command.** The "guard each caller" shape ADR-0089 rejected: the next readiness source
  would miss it.

## Consequences

- Safety does not depend on the drivers: the reducer never makes a gated step `READY`, and `persist` refuses an undecided
  wait. What depends on a driver answering `Evaluation::Gates` is liveness: a driver that does not refuses its command.
- A new consumer of `Progress::pending` must answer the round or refuse it (G1197 names the two that refuse today).
