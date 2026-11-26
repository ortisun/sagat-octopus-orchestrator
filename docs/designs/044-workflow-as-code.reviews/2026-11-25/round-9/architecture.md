# Architecture review of design 044, round 9 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 842f013e1e19a2d4e09617fc0abf88296275b860
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 8's Majors 1, 3
and 5 are closed, 2 and 4 partly closed; Nits 6 to 13 closed, 14 closed in text. Layering, the constitution gating,
the code claims, compatibility, operability and the fitness functions hold.

## Major

A. A move item's verdicts do not fit feature 018's item model (`reason` is only a skip or defer cause, failures use
   `errorCode`): map each verdict to `SUCCEEDED`, `SKIPPED` with new skip reasons, `DEFERRED` with `RUN_BUSY`, or
   `FAILED` with an error code; a test.
B. "A workflow type is unique in its namespace" has no storage, so concurrent publications both commit: a
   `code_workflow_types` table and type and digest columns on `workflow_versions`; a barrier test.
C. The overlay cache can be filled by a transaction that never commits: fill after commit, drop on any count mismatch;
   a test with an injected failure.

## Nit

1. The batch route is `/batch-operations`.
2. "Forced cancellation" is undefined; say the cancellation deadline.
3. The cancellation source check's allowlist omits the acknowledgement, the continue-as-new path and the latch.
4. Enforced-by names no task writes (ADR-0103, ADR-0106).
5. The hold function's body comparison supersedes Decision 044-193 silently; make Gap 044-026 a precondition.
6. No test for refusing a manual-review hold on `~workflow`.
7. Task 044-2 lists neither `firstDifference` nor the node-spec baseline.
8. The down migration's refusals are incomplete.
9. Drafts 049 (journal buckets) and 052 (next step in the completion answer) are not in section 12.
10. Task 044-1 should add the new checks to the principles table.
11. What the move rule becomes if Gap 044-025's wording is kept.
12. Architecture decision 0108 should list `maxBytes` and the per-signal schema as a refinement of 0111.
13. Task 044-9 names no application use case or `RunTx` method.
14. `sagat_move_code_run` does not refuse an item of a dry-run job.
15. Task 044-18's contradictory CI sentence.
16. The closing window's count in Requirement 044-027 against section 6.3.

Counts: Blocker 0, Major 3, Nit 16
