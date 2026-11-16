# egm-review Stage 5 (Initial Zoom Out) of design 043 (2026-11-15)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: ce2e10e2eab95102e658d370dcef1fcd4a48faeb
Reviewer: egm-zoom-out (Stage 5 of egm-review only; Stages 6 to 9 not run, see README.md)
Standard: zoom-out-standard (EGM 0.1.0, tree hash 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9)
Note: condensed by the author from the reviewer's output; every finding and severity is kept. This is not the
`egm-review` summary report of an approval record, because the run is not complete.

## Major

1. Lines 181, 397-398, 695, 797, 805-806: the doc says `sagat-application` has no runtime; the code has `tokio` (`rt`)
   and the rule is narrowed (D043-150, D043-154, G043-052). ADR-0090's "Enforced by" is stale too.
2. Lines 624-634 against 456, 470-471, 486-489, 612-619, 200-201: the amendments block sits on top of the text it
   changes (the `oneOf` bound, the publication codes and `CLASSIFICATION_TOO_COMPLEX`, the lookup charge, the 1 KiB name
   limit missing from NFR-043-BOUND). Fold them in.
3. Lines 412-416, 439, 462-477, 492-498, 506-520: Technical Plan 3 and 4 go into code-level detail (regex engine
   tuning, the DFS colouring, the single-flight `Condvar`, map order, each pattern edge case); every implementation
   adjustment then changes the architecture section.

## Nit

4. Stale register ranges (line 41); Open Question 2 is answered by G043-017 except for CI and production numbers.
5. Dozens of `file:line` references pinned to `c2e9341f`; symbol names would serve.
6. T043-3 says to build the queue and then that it is built in the adapter; split the task.
7. Rust privacy already enforces most of architecture rule 4; keep only what the compiler cannot, or say why.
8. `off_worker` exists twice, in two adapters; say why no shared helper.
9. The fair permit queue: Complexity Tracking does not record the simplest option (a plain semaphore with a timed
   acquire plus the existing rate limiter and pass fairness).
10. The legacy continue-as-new kept as a tests-only production path costs a rule and an allowed site; a follow-up.
11. Alternatives Considered does not record "subset pre-check, then `jsonschema`".

Counts: Blocker 0, Major 3, Nit 8
