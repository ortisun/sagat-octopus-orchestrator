# egm-design (critic) on design 051, full round of 2026-11-26

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: b604f497564d3d7de3277c99cf38caf203e59f60
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing; the cost table's arithmetic is correct as written.

## Blocker

1. A throughput job does not fit its limit: the ladder is 12 rungs (1 + 6 doubling, 3 bisection, half-maximum,
   common rate), passes interleaving the products cost about 32 switches with waits and restarts, and the drain before
   each rung is separate from the rung's own; the free tier's worst case is 432 minutes (368 without the waits), the
   paid tier's 402. Fix: each product's ladder as one turn in an order rotated by repetition; one drain; 12 rungs; the
   plan check counts the worst case.

## Major

1. "The end as the product records it" means different clocks per product, and the live poll loads the products
   unequally. Fix: count live completions from the last activity's end record; no polling during a rung; state each
   product's clock and statement; report recorded end minus last activity's end.
2. "Sustained" judged after the drain lets about 20% over capacity pass; the ceilings have no rule; a product with no
   sustained rate is undefined. Fix: the backlog at the window's end; ceilings fixed before any pilot with reasoning;
   "no rate sustained" defined, and the common rate then.
3. The ranking discards the within-job pairing (CPU models differ between runners; steal unchecked). Fix: rank on the
   within-job ratio of each pair; record the CPU model and steal; mark or rerun rungs above 5% steal.
4. The reproducibility check fails by construction (quantized rates; ±2s from five covers about 86%). Fix: within the
   published range widened by one bisection step on the same CPU model, or the paired-ratio range.
5. The free tier's partition cannot be built by physical core on 2 cores × 2 threads; the Temporal sidecar slows its
   database alone. Fix: the product one core, the database and the driver the other; the sidecar in Temporal's partition.
6. A fixed 1:2 database-to-product split decides the ranking. Fix: a 3-vCPU, 12 GB budget for each product and its
   database together, the split chosen by the pilot and published; combined CPU seconds per thousand transitions.
7. Durability is not pinned (a session could set `synchronous_commit=off`), the shared configuration's origin is
   unsaid, and `kill -9` keeps the page cache. Fix: forbid durability session settings, checked by `SHOW`; a neutral rule
   for the configuration; state `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY=false`; Success criterion 051-003 as a process
   crash, storage loss untested.
8. Worker capacity undefined (activity body and duration; slots per worker differ by product; "activities in flight"
   not ensured). Fix: a defined activity body (echo and a 20 ms sleep); each product's worker concurrency published; the
   injector fires only with N activities in flight on the target.
9. The crash runs are not comparable (restart unsaid; 50% of a maximum from other machines; different absolute loads).
   Fix: restart after a fixed 5 s; the crash at the common absolute rate; baseline check; fractions of acknowledged
   starts; crash jobs after the chain-of-three throughput jobs.
10. The recovery criterion is noisy (p99 of 100 samples) and unreachable below 10 workflows per second. Fix: windows of
    200 samples; the median or 90th percentile against the baseline's worst; a cap reported as "more than".
11. Fault (c) on the free tier is not defined (a killed postmaster leaves backends; replay time depends on the
    checkpoint phase). Fix: `docker kill -s KILL` of the container, `docker start` on the same volume; a CHECKPOINT 60 s
    before; the WAL replayed reported.
12. The signal shape's meaning differs per product (an early signal buffered by Temporal, refused here without design
    054). Fix: the early-signal outcome and retry stated; latency from max(send, wait recorded); design 054's presence
    stated.
13. This platform's script containers escape its partition (created by the daemon outside its cpuset). Fix: under the
    product's cgroup and cpuset; the daemon's CPU charged to it.
14. WAL and PostgreSQL CPU are the cluster's but reported per product. Fix: autovacuum off on the stopped products'
    tables during a turn; WAL by database with `pg_walinspect`; background processes apart.
15. The free tier's 14 GB of disk is not budgeted. Fix: bytes per workflow measured by the pilot; the plan check
    refuses a plan whose projected disk exceeds free space less 2 GB; toolcaches freed; free space recorded per rung.

## Nit

1. Percentile convention (nearest rank; the censored bound).
2. Which arrivals are latency samples.
3. "Fired" in the timer burst undefined.
4. Fan-out repeats keyed by (workflow, step, iteration).
5. The order-effect test unnamed, its threshold and power unsaid.
6. The competitors' version rule; the pilot's tuning budget per product.
7. The Temporal sidecar's hop reported beside its latencies, included.
8. A start request's timeout and an unknown outcome's classification.
9. The free-tier timer burst's minutes and repetitions.
10. A 100,000-chain history may fit in memory; state it.
11. This platform's lease time to live decides faults (b) and (c); state its value.
12. The three-machine tier needs the clock offset between hosts.

Counts: Blocker 1, Major 15, Nit 12
