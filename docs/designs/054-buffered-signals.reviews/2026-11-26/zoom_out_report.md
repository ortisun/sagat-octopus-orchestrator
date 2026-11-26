# Stage 5 (Initial Zoom Out) review of design 054 (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: a49668f8d0b3c2c7fac300e7a122cdef5b5dd05d
Reviewer: egm-review, stage 5 (the egm-zoom-out lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. The author's answer
follows each finding. The reviewer read the doc, Architecture decision 0111, the register, design 044's signal passages and
the code it cites; it did not read the constitution.

Counts: Blocker 1, Major 3, Nit 6

Scope verdict of the reviewer: the opt-in inbox of the run is the right core.

## Blocker

1. **The queue journey cannot be built with root-only consumers.** A root graph is acyclic (`check_acyclic`,
   `crates/domain/src/workflow.rs`), so each root `waitEvent` is entered at most once per run; CUJ 2's "handles orders and
   continues as new after 100" cannot be written. Fix: restate CUJ 2 as one order per segment with continue-as-new, or admit
   loop-body waits.
   *Answer: fixed* by the first option. The Problem, CUJ 2 and Success criterion 054-002 now say a graph version handles one
   order per run segment (its entry an activity, as a continuation requires) and moves the rest; a code version (design 044)
   loops over its channel in one run. Body waits stay Gap 054-002; `maxPending` up to 1,000 serves such a backlog and code
   runs.

## Major

2. **The design rebuilds a retained-bytes ledger that exists** (`run_instance_usage.logical_output_bytes`, capped by a check,
   read by `logical_output_bytes()`). Fix: charge pending bytes to that ledger or a sibling column summed by the method.
   *Answer: fixed.* `runs.pending_signal_bytes`, `retained_bytes()` and the fitness rule are gone; the ledger gains
   `pending_signal_bytes` under one check of the sum; only the three sites that sum `RunData.outputs` change (Technical Plans 2
   and 6, Requirement 054-008, Task 054-7; Decision 054-008 revised).
3. **Design 044 disagrees on the shared inbox:** it says consumed rows are "marked consumed", and refuses undeclared names on code
   runs while Requirement 054-003 says "exactly as today". Fix: state the inbox contract once in 054 and defer code runs to 044.
   *Answer: fixed in 054; the citation in 044 is recorded.* Technical Plan 10 states the contract (`consume_pending_signals`, all
   or nothing, rows deleted); Requirement 054-003 is a rule of graph runs and defers code runs to design 044; Gap 054-006 asks
   design 044 to cite the contract (this design may not edit design 044).
4. **Three designs each create or compare one shared database function** (and the floor's rule): fragile. Fix: one prerequisite
   change.
   *Answer: fixed.* Task 054-18 is that prerequisite, its own migration and pull request, landed once by whichever of designs
   044, 045 and 054 builds first; each design adds only a capability and a predicate; the compare stays only as a drift guard
   (Technical Plan 15; Gap 054-007 for designs 044 and 045 to cite it).

## Nit

5. **`buffered: true` is speculative.** *Answer: fixed in part:* `buffered` is optional and, when present, `true`, so design 044's
   documents stay valid and no author must write it (Decision 054-009 revised).
6. **The first-waiter rule borrows feature 004's `event_since`.** The reviewer proposed the sequence of `wait.started`, which it
   believed is in the domain's state. *Answer: fixed differently:* the state holds no sequence (`StepState`); the entry time is the
   wait's deadline less its node's `timeoutMs`, both in the domain (Requirement 054-004; Decision 054-003 revised).
7. **The publication byte-sum check promises more than it delivers.** *Answer: contested:* it guarantees that every declared
   `maxPending` is reachable when outputs are small, and bounds what senders can pin in a run to what the author declared;
   Requirement 054-007 states the shared-room caveat. Kept with the Studio's warning.
8. **A new planner evaluation where the correlation's would do.** *Answer: fixed:* `Evaluation::Correlation` carries
   `take_buffered` and `Resolution::Correlation` the taken signal (Technical Plan 4; Decision 054-011 revised).
9. **SDK list methods go beyond the operator need.** *Answer: contested:* the client that sent a signal (CUJ 1's approver) needs to
   see what is pending without the Studio, and the brief of this design asks for the six SDK clients; the method is one route
   call.
10. **SQL detail in the architecture sections.** *Answer: contested:* the table's shape, keys and the one consuming statement are
    what the codebase reviewers and the builders check the invariants against, as in the sibling design 045.
