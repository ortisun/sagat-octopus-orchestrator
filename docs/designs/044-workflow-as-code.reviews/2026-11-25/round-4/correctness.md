# Correctness review of design 044, round 4 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: ba5a3fdd02abfc57e03b76efc4a9fb2e237b5c45
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Most earlier Blockers
and Majors are closed; five partly closed ones return below, and finding 4 is new.

## Major

1. Cooperative cancellation cannot work: today's cancel route calls `stop_run`, and the drain cancels the workflow-task
   step and ends the run in one pass; the function's answer is refused; no cancellation-request event, no deadline
   pass. Give code runs a cancel arm with `run.cancel_requested` and a deadline pass; drop the stop-reason exemption.
2. A refused terminal command loops or deadlocks (the whole completion refused keeps the run pending; partial commit
   shifts command numbers). Commit the other commands, move S, record the deferral, and have the SDK re-emit the
   terminal command last.
3. The saga engine reads `sagas`, `saga_region`, `enclosing_scope`, `scope_chain` and `policies` from the graph and falls
   back to defaults; compose every accessor, fail closed in `policies`, test non-default policies.
4. Signal consumption is not journaled, chained or validated; record it, chain it, refuse a sequence beyond the window.
5. Per-command body digests are needed by `verify_run`, the reset, the move and the export, and are stored nowhere.
6. The origin of an event is not stored, and the reconciler's partial index sees no root event; add `run_events.origin`;
   scan the primary key.

## Nit

7. The saga dependency rule is too conservative; B depends on A when A's outcome is at or before the start of the task
   that recorded B.
8. The completion that crosses 51,200 can add about 513 events; the review transition from `DECIDER` is illegal today.
9. The page bound ignores the 4 MiB cap per page.
10. The statement budget omits approval rows, look-back reads and gate rows.
11. A pre-check kill test cannot fail; the generator omits failed tasks and immediate effects; the differential test's
    coverage.
12. Gate grants and approval decisions of gated steps are waking but not visible.
13. Retry and move do not reset the failure count.
14. A `Completed` drain shows the run as `CANCELLING`.
15. Load the overlay row of the addressed step even when terminal.
16. The closing window refuses markers, set-state and update completions.
17. Requirements without tests (first cancellation, two bounds, two caps, the time rule).
18. `WriteBlob` charges before the completion commits.
19. Drift in Architecture decisions 0103 to 0108.
20. Continue-as-new in one transaction or two.

Counts: Blocker 0, Major 6, Nit 14
