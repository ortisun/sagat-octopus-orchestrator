# Targeted re-read 1 of design 054: architecture (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: 3798bec94b1ecef532da455f7e2d369121f8a770
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. A targeted re-read of the
fixes of `../architecture.md` and of the text changed since (Owner decision 2026-11-25-2, step 4), not a full review. The
reviewer read files only and ran nothing.

Findings 1 to 4 and 6 to 11 of the architecture review: resolved (7 and 9 mostly; new findings 3 and 4); 5 partly (new finding 2).

Counts: Blocker 0, Major 3, Nit 4

## Major

1. **Design 045 on this branch, and design 044, still give the hold's older body, a drift guard and the floor in 0098's trigger;**
   slice 2 accepted their "equivalent", so the first to land would break the other's migration. *Answer: fixed:* slice 2 depends
   on the hold and registry of Architecture decision 0112 only (Task 054-18, or design 044 or 045 once it cites the record); Gap
   054-012 makes their revision a precondition of Tasks 045-6 and 044-6 (this design may not edit their texts).
2. **No task builds the source rule over `run_version` comparisons.** *Answer: fixed:* Task 054-9 builds it with fixtures, an
   allowlist and a failing fixture.
3. **The refusal while pending bytes are above 2 MiB has no reason of its own, no place in the order and no test.** *Answer:
   fixed:* `SIGNAL_PENDING_BYTES_FULL`, placed in Requirement 054-005's order in the application after the ledger's read, tested by
   Task 054-9, stated in Architecture decision 0111 decision 4, with its metric outcome and REST reason.

## Nit

4. Terms calls the invariant's check a helper, the rest a trigger. *Answer: fixed.*
5. Open Questions' range of decisions. *Answer: fixed* (054-001 to 054-017; Gaps 054-001 to 054-011).
6. Complexity Tracking omits the two quota resources, the move limit and the test trigger. *Answer: fixed:* three rows.
7. Tasks 054-4 and 054-6 change the run's byte ledger without a gate. *Answer: fixed:* Owner decision 2026-11-26-540 names the
   ledger (the execution core features 014 and 016 share) and both tasks carry the gate.
