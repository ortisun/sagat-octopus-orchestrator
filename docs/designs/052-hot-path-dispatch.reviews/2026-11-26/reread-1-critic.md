# Targeted re-read 1 of design 052: egm-design (critic) (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 95bf8b463272e804150aa5b2124484d2c4ec597f
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of
`critic_report.md` (at 2348509a, which carried the queue and the next-step lease), not a full review.

Full round: both Blockers and Majors 1 to 5, 8 and 9 moot (the queue and the next-step lease deferred, their needs
recorded in Open Questions 2 and 3); Majors 6 and 7 resolved; Nits 1 to 4, 7 and 8 moot, 5, 6 and 9 resolved.

## Major

1. The spread's offsets are a random draw fixed for each process's life: two instances start within an eighth of a
   window about 23% of the time, and the benchmark measures whatever the draw gave. Fix: `⌊rank × len / n⌋` over the
   live engines, refreshed at each heartbeat.
2. A scenario change cannot merge ("not comparable" fails the job), and the old sd outlives it. Fix: a neutral pass
   with a notice, and the pilot re-run with the new constant in the same pull request.

## Nit

1. The second fetch shifts twice (past the tried candidates and without the probed runs); pick one.
2. The contended flag can flip while polls are in flight, breaking the in-process permutation; decide once per round.
3. The cap on unary 50 ms answers is per process; say so.
4. The receiver looks keys up by the activity's token while the registry is keyed by name; index it by the token.

Counts: Blocker 0, Major 2, Nit 4

All six were fixed at the next commit.
