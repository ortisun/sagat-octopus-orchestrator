# Targeted re-read 1 of design 051: egm-design (critic) (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 817126fd7c6bbc2f44a769b708020872c2741d3c
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of
`critic_report.md` (at b604f497), not a full review.

Full round: Majors 1 to 6, 8 to 11 and 13 to 15 and all Nits resolved; the Blocker and Majors 7 and 12 partly (below).

## Major

1. The throughput job's 3 minutes of slack leave out the daemon's restarts, the calibration loop, the probes and image
   checks, the `pg_stat_statements` scan and the commit-time reads. Fix: a per-rung and per-turn overhead in the worst
   case; one doubling or one re-run fewer if it then exceeds 340.
2. The daemon's default control-group parent also captures the driver's container during this platform's turn. Fix:
   an explicit `cgroup_parent` for the driver and metrics containers, confirmed at each turn's start.
3. Durability can still change out of sight (startup parameters are not `SET` statements; `commit_delay` limited only
   for non-superusers). Fix: an allowlist of startup parameters; the roles not superusers, with no `SET` grant on the
   two, checked through SQL.
4. The ladder's own runs swamp the 100,000-workflow history, faster products more. Fix: a deviation, the rows at each
   maximum reported.

## Nit

1. The driver's retry and its count when a product refuses an early signal.
2. "A changed `xmin` fails the rung" fails by construction if a product rewrites its end row; the pilot confirms it.
3. A reproduction judged on the median of its repetitions per cell.
4. The archives and images in the projected disk use.

Counts: Blocker 0, Major 4, Nit 4

All eight were fixed at the next commit: a per-rung overhead of 0.5 and a per-turn overhead of 4 minutes, at most five
doublings and one re-run (a throughput job at 336 minutes; a campaign at 21,505 runner minutes and about 44 hours).
