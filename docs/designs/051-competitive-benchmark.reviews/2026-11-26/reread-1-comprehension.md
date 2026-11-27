# Targeted re-read 1 of design 051: egm-comprehension (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 174931d70d47852769614ed011d1f6475f7b4931
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`comprehension_report.md`, not a full review; the timing and campaign totals were rechecked and hold.

Full round: both Blockers, Majors 1, 2, 4, 5 and 7 to 10, and Nits 1 to 3, 5, 6 and 8 to 14 resolved; Majors 3 and 6
partly, Nit 4 not, Nit 7 partly (below).

## Major

1. The workflow's stages start with the pilots, while "A campaign" runs them once per method version and leaves them
   out of the totals. Fix: the pilots a job kind of their own; their cost stated.
2. The plan check accepts 350 minutes, above the `run.sh` step's 340-minute timeout.
3. A ladder ending "worker-bound at" a rate has no defined maximum in the ranking or the common rate.

## Nit

1. The common rate's fallback reads two ways.
2. "One bisection step" and "one eighth" not stated as a ratio.
3. Eleven rungs of three products make 32 switches, not 35.
4. The saga's five transitions against the definition.
5. Re-runs budgeted only in throughput jobs.
6. The timer burst has no stop (3 × 32 + 20 = 116).
7. The history job's 60 minutes per product unbroken; the image build named twice.
8. The recovery cap's confirming windows.
9. The steal sentence is hard to parse.

Counts: Blocker 0, Major 3, Nit 9

All twelve were fixed at the next commit (the plan check at 340 minutes with the step at 345; the pilots at most 2,040
runner minutes once per method version; a campaign at 20,657 runner minutes and about 42.7 hours).
