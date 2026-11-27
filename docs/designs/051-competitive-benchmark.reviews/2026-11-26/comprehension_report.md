# egm-comprehension on design 051, full round of 2026-11-26

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 831873036bbf0a4a0e5ec51d988892a4d1f8331c
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing. Run on the text revised after `zoom_out_report.md` and `critic_report.md` (at b604f497).

## Blocker

1. The common rate depends on results that do not exist yet: a rung at it inside every product's turn, while the first
   product of a job ends before the others have a maximum; Requirement 051-009 uses a second definition ("medians" of
   what; a crash runner on an unseen CPU model). Fix: one common rate per variant per campaign from earlier jobs or the
   pilots, half the lowest median maximum, with a fallback; the common-rate rungs in a later job.
2. One PostgreSQL server or one per product, both supported by the text and not both buildable (a data directory
   restores a whole server; `shared_buffers` per product; autovacuum off on "the other products' databases"; WAL by
   database). Fix: say which; drop what the other implies.

## Major

1. The steal re-run breaks the worst case (up to 560 minutes). Fix: cap the re-runs per job and recompute.
2. The 25-minute crash measurement is not broken down and does not hold at low rates; "the database's restoration"
   unexplained.
3. Whether the pilots repeat per campaign; the 25 hours ignores the jobs' order (about 31 hours with pilots).
4. One matrix with `max-parallel: 12` cannot order the jobs; several matrices overlap past the CI headroom.
5. Latency ties are undefined.
6. Whether the starting rate is per variant or per product; "one bisection step" as a ratio band undefined.
7. "Work sent to a person" undefined per product.
8. Temporal's fault (a) kills a history process, which is not behind the load balancer.
9. Success criterion 051-001 and the Verification Plan ask for variants Temporal never runs.
10. The sidecar charged to Temporal contradicts "the harness on its own thread".

## Nit

1. The timer burst's starts fall inside its sleep (10 + 10, not 5 + 10 + 10); unfired timers unsaid.
2. "Reach 1%" should be "exceed 1%".
3. Loading per job is about 40% of a job, not "a fifth".
4. Twelve rungs times three products is 35 switches, not thirty.
5. "Five repetitions" in the Overview; crashes repeat six times.
6. Terms used before definition (ladder, pilot, history, turn, sidecar, partition, engine replica, this platform's load
   balancer).
7. The fan-out reads two ways; the saga counts a failed step as a transition.
8. The 10 s start limit has no starting point.
9. The recovery time's end point, the windows' order, a failed baseline check.
10. Script variants' backlog cannot be computed during a rung.
11. Design 053 defines transition latency to the next lease; design 049 also uses the shapes.
12. Owner decision 2026-11-24-1's record does not mention crash recovery or pull request 273.
13. "One variant per job" only implied.
14. Reproduction ranges by CPU model with fewer than five repetitions of a model.

Counts: Blocker 2, Major 10, Nit 14
