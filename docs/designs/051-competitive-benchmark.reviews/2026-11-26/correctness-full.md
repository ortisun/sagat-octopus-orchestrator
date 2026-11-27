# Correctness review of design 051, full round of 2026-11-26

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 831873036bbf0a4a0e5ec51d988892a4d1f8331c
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only,
ran nothing, and did not repeat the reports beside this one or those of 2026-11-24.

## Blocker

1. The recovery estimator reports a recovery when nothing degraded: a window passes the worst of five baseline windows
   with probability between 2/3 and 5/6, so three in a row take 4.4 to 6.6 windows (87 to 132 s at 10 workflows per
   second), about 1 measurement in 32 waits 280 s or more; nothing tests the estimator against a known recovery. Fix: a
   baseline of 20 windows (p = 20/21, three in a row with probability 0.86); recovery from the start of the first passing
   triple; a null-fault control in every crash job, published; a unit test against a simulated 30 s recovery.

## Major

1. The pre-fault check compares two deployments and fails a third of the time by chance ((n − 1)/(n + 1) = 4/6). Fix:
   the crash deployment sustains the common rate by Requirement 051-006's rule during its own pre-fault period.
2. Fault (c)'s repeat and review columns are decided by the worker client's report budget (the Go worker gives up after
   five attempts, 1.5 to 3 s; the heartbeat stops; the lease lapses), not the server. Fix: each product's report-retry
   budget in `shapes.md`, set to cover the restart where the client allows it, a deviation where not.
3. The history of 100,000 holds only for the first rung; the common-rate rung runs on databases 3.3 times apart in size.
   Fix: rows and bytes recorded per rung; the common-rate rungs run first after a restore.
4. Leftover work of an unsustained rung contaminates the next (cancelling 36,000 workflows does not fit a 60 s drain).
   Fix: between rungs, wait until no earlier run is open (at most 3 minutes, recorded; the job's worst case 344); the
   backlog scoped to the rung's arrivals; a rung that started with open work marked.
5. The workflow's end is stamped before its commit, differently per product (this platform's `finished_at` from
   `clock_timestamp()` after the run lock). Fix: `track_commit_timestamp` on for all, the end as
   `pg_xact_commit_timestamp(xmin)` of the end row; or the stamp-to-commit gap measured and published.
6. The driver's own sending delay is never measured. Fix: every arrival's send lag recorded; a rung whose 99th
   percentile lag exceeds a bound marked; the no-op adapter delayed like each product.
7. A maximum can be a worker-slot setting (this platform's Go worker runs 10 tasks at once by default: 500 activities
   per second with a 20 ms sleep); the timer burst's lateness measures the slot count. Fix: a rung "worker-bound" when a
   worker sits at its slot count for over 10% of the window; the slot-bound floor beside the timer burst.
8. This platform's graph timers sleep a constant delay from now, not until an absolute instant. Fix: each product's
   mechanism in `shapes.md`; a start delay as a deviation, or "not run natively".
9. The signal shape measures the driver here (refused early signals retried every 100 ms) and cannot be computed for
   Temporal (no history event for a blocked wait). Fix: a correlated-event wait with lookback, or capped retries
   published; Temporal's wait as the time of the workflow task that blocked.
10. The history is loaded before the pilots fix settings it freezes (Temporal's `numHistoryShards`). Fix: pilots on a
    small history, then the history job, then a confirmation pilot; a bounded closed-loop load checked for exactly
    100,000; the frozen settings listed.
11. The 5% steal rule may never fire on the runners' hypervisor. Fix: the pilot shows the counter moves, or a calibration
    loop between rungs.

## Nit

1. WAL by database is incomplete (records without a block reference); since one product runs per turn, use the turn's
   LSN difference.
2. Read Temporal's outcome and close time from persistence, not lagging SQL visibility.
3. Orphan starts counted apart; each workflow's completion counted once.
4. The crash classes are not a partition; repeats over activity executions.
5. The censored bound is an instant, not a latency.
6. Ranking with "at least" or "no rate", and intransitive ties.
7. One interval for the per-transition figures.
8. PostgreSQL's background work belongs to the product's figure; peak memory as anonymous plus shared memory.
9. The fan-out's completion record per product.

Counts: Blocker 1, Major 11, Nit 9
