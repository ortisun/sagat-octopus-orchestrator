# egm-review Stage 7 design lens, targeted re-read, round 7 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: e0a23b09f7ddbe225e80110c2cf25f3da960b730
Reviewer: egm-design, feature-spec standard, FeatureRange from the start to the Detailed Implementation Plan
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 6's findings,
1 to 3 and 6 to 13 are closed and 4 and 5 partly closed. Claims were checked against the saga code and the migrations.

## Major

1. The can-act count reads `run_saga_entries`, which is written only when an abort settles, so completed members of
   an open scope are never counted: derive membership from the overlay's scope and the run's sagas; a long-scope test.
2. Nothing computes the journal-derived compensation order: `reverse_dependencies` walks graph edges, which overlay
   nodes lack; record the issuing `S(k)` and outcome sequences and give `plan_entries` a code arm.
3. Nested code saga scopes are unspecified: refuse them or carry the parent scope.
4. The `RETAINED_BYTES_LIMIT` closing window is not usable: the function is never told, the window has no size, end or
   column.
5. Copied initiator rows keep another run's sequences, so an inherited initiator escapes a child's approval: copy them
   with `first_seq = 0`.
6. The new messages exceed the gRPC transport limits (320 KiB decode; 4 MiB client receive): per-RPC limits, one replay
   check item per message, pages below 4 MiB with framing; contract tests.

## Nit

7. Requirement 044-027's window list is shorter than section 6.3's.
8. A 1,000 ms workflow task timeout and a cold replay of up to 5.5 s.
9. The sticky name and the `sagat.workflow` quota bucket.
10. Worker queries on an ended run.

Counts: Blocker 0, Major 6, Nit 4
