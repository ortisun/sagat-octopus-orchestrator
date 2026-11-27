# Targeted re-read 6 of design 052: security (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 9dcbc22eee9238cc99a25289368680b530888882
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation of the
changes that answered `reread-2-correctness.md` and `reread-2-architecture.md`, checked with no Blocker or Major (the
flag and round rules, rank 0, the gate's scenario rule and the permit hand-off open nothing). The reviewer read files
only and ran nothing.

No Blocker, no Major.

## Nit

1. "Beside the repository's other checkers" places `scripts/check-chart-wake.py` in the `architecture` job, which runs
   only on full runs or changes of `scripts/`, so a chart-only pull request would not run it. Fix: a step of the
   `documentation` job, which runs on every pull request; the checker fails, never skips, without `helm`. (Suspicion,
   not graded: a check on every pull request may need constitution 5.1.0 to admit it.)

Counts: Blocker 0, Major 0, Nit 1

Fixed at the next commit, the step waiting for 5.1.0 to admit it (Task 052-10), the checker running in the
`architecture` job and at every full run and release until then.
