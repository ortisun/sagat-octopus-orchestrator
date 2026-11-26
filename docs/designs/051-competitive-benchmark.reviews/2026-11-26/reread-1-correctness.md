# Targeted re-read 1 of design 051: correctness (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 730a7daa32dab31852ecb2a581fac1150dc001f0
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`correctness-full.md`, not a full review; every timing figure and campaign total was checked and is correct as written.
The reviewer read files only and ran nothing.

Full round: the Blocker, Majors 1 to 3, 5, 6 and 8 to 11 and all Nits resolved; Majors 4 and 7 partly (below).

## Major

1. The wait between rungs waits for any earlier rung's open run (one lost run makes every later gap wait 3 minutes,
   sustained rungs may leave 1% open): 30 minutes per turn, a 391-minute job. Fix: wait for the previous rung's runs
   only, count a run still open as stuck and exclude it; cap the waits per turn and mark rungs after the cap.
2. The worker-bound rule fires on the fan-out at low rates (100 activities of 20 ms hold a worker at its slots), so no
   fan-out result, and other ladders bisect to just under the threshold. Fix: worker-bound by slot utilisation (rate ×
   activity-seconds at 80% of the slots), ending the ladder as "worker-bound at", the pilot raising the slots.
3. "Tied when the range includes the band" reads wrongly ([1.0, 1.0] would not be a tie). Fix: tied unless all ratios
   exceed 1.1 or all fall below 1/1.1.
4. (Plausible) No fallback when a crash deployment cannot hold the common rate. Fix: a crash confirmation job; half the
   common rate for all three if any fails, stated.

## Nit

1. Failed arrivals and orphans per fault, as fractions of arrivals.
2. The end row's `xmin` read again at the turn's end, a change failing the rung.
3. `pg_stat_statements_reset()` after each restore and at each rung's start.
4. The crash job's warm-up, stop and trigger missing from its minutes (about 172 per job).
5. Ranking a pair only on jobs where both maxima are numbers; "more than" latencies.
6. The calibration's reference and thread.
7. The reconciler makes the review column zero by configuration; say so.
8. Two matrices in one stage could run 17 jobs at once.

Counts: Blocker 0, Major 4, Nit 8

All twelve were fixed at the next commit (the crash job at 173 minutes, a campaign at 20,597 runner minutes and about
41.8 hours).
