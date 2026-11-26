# egm-review Stage 7 design lens, targeted re-read, round 8 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 3884f9f3db9c7ec9c7405f2970a02985c4249711
Reviewer: egm-design, feature-spec standard, FeatureRange from the start to the Detailed Implementation Plan
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 7's findings,
4, 5, 8, 9 and 10 are closed and 1, 2, 3, 6 and 7 partly closed. Claims were checked against the saga, approval and
transport code.

## Major

A. The can-act set leaves out rows the saga machinery reads (members of a closed child inside an open parent, a
   compensating scope's members), nested-scope closing is contradictory, and the partial index cannot hold succeeded
   steps: a closed child stays compensable by its parent; count by the outermost scope; an index; a test.
B. The code arm of the saga plan is quadratic (millions of `after` links) and no per-step outcome sequence is stored:
   store `outcome_seq`; a linear form; a budget test.
C. `run_initiators` silently changes graph-only trees: write rows only in trees that contain a code run.
D. Large values have no legal encoding: `WriteBlob` is allowed only above 256 KiB while the message must fit 256 KiB;
   allow blobs for any input, result, side-effect value and output; declared state inline, one set-state per completion.
E. A terminal command issued outside a pending saga body goes to review on the intended cancellation path: the SDK
   aborts every open scope before a terminal or continue-as-new command; a conformance scenario.

## Nit

1. Requirement 044-027's window size against section 6.3's.
2. The claim's 51,200 refusal inside an open window.
3. A null bound for graph approvals.
4. `plan_entries` is in `execution/sagas.rs`.
5. The unseen-events check excludes the completion's own immediate effects.
6. `version: active` resolved at the completion and pinned at the handoff.
7. The poll answer's 4 MiB against the clients' 4 MiB.

Counts: Blocker 0, Major 5, Nit 7
