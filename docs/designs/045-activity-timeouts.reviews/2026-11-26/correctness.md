# Correctness review of design 045, round 4 (2026-11-26)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 5d2d7833
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author; every finding and its severity kept, with the author's answer.

Counts: Blocker 0, Major 1, Nit 8

Round 3: Majors 2 to 5 and Nits 1 to 10 closed; Major 1 partly closed (below).

## Major

1. **A withdrawal during a pause could leave a deadline from before the pause**, and the pass would end the step during the pause.
   *Answer: fixed*: a withdrawal while paused writes the start deadline as null; every exit from a live attempt clears it; the pass's
   re-check refuses a start deadline on a paused run; the tests of a withdrawal and of an operator's retry during a pause are named.

## Nit (each fixed)

1. The delay is in the fingerprint whenever present, at every level. 2. `delaySource` is recorded at levels 1 and 2. 3. Architecture
decision 0110's refinement of 0082, its decisions 3 and 7 and the cutoff of decision 6 aligned. 4. `DOCUMENT_NEEDS_REPUBLISH` is a
listed reason with a fixture. 5. Keeping `ready_at` names the statement change and the refinement of Architecture decision 0098
decision 3, with a test that re-claims from one silent host. 6. The replay comparison names its columns per level, and `StepState`
carries the attempt deadline and its cause at levels 1 and 2. 7. `step.failed` from `WAITING` is not a new transition. 8. The claim
test's "plus" case sets the deadline minutes ahead.
