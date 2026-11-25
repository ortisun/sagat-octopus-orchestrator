# Correctness re-read of design 051 (2026-11-24)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: c095c5c5e36495cbe5a43ae3933c4220c7ef4718
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The second read, of the
benchmark alone (the hot-path remedies moved to design 052). The reviewer found every written figure of arithmetic
correct; the defects are in the assumptions and the method.

## Blockers

1. Each fan-out half-job runs 306 + 75 = 381 minutes, over the six-hour limit, and splitting a repetition across two
   runners breaks Requirement 051-003.
2. The 75 minutes to prepare and load three histories (3,000,000 chains through the products) has no measurement and
   implies about 2,000 transitions per second for every product; above 108 minutes every throughput job exceeds 360.
3. Fault (c) cannot be repeated for the second and third products of one crash job: with a quorum of one and commits
   refused without a standby, the second promotion leaves no standby; nothing rebuilds the old primary.

## Majors

1. Under Patroni, `kill -9` of the primary's PostgreSQL restarts it locally; failover waits `primary_start_timeout`.
2. Failed and open work have no stated latency; censored values can make the whole 99th percentile tail.
3. The sustained rule is ambiguous (by when, with which outcome; the saga's expected outcome is compensated).
4. The ladder has gaps: eight sustained rungs give only a lower bound; halving is unbounded and unbudgeted; one noisy trial
   per bisection point cannot rank results within the resolution.
5. Work and state carry over between rungs and turns (leftover backlog, the shared PostgreSQL's checkpoints, vacuum,
   cache and lag; restarting products to fit memory).
6. The order rotation cannot be balanced (unequal rung counts, five crash repetitions, cyclic orders unbalanced in
   precedence).
7. The timer burst measures the start rate, not timer firing, and no rung rule fits a fixed population.
8. The crash metrics cannot show a stall (the other replica keeps completing; in-flight work is left out; no tolerance;
   lost and failed mixed).
9. Repeated executions and transitions are not defined precisely (record at start and end; attempt numbers; shape
   retries; the saga's failed step; whether the sandbox reaches the socket).
10. How each adapter waits for results differs per product (polling against long polling).
11. Success criterion 051-002 compares latency at different loads; add a common absolute rate.
12. Non-functional requirement 051-001 and CUJ 2 fail about 40% of the time by chance (a new median inside an old range of
    three in 12 of 20 cases).
13. "At most 7 minutes per rung" and the free pilot are not credible (slow variants, a 4-vCPU runner cannot hold the
    topology or the histories).

## Nits

1. Fault (a) means something different per product; state each deployment and whether a balancer is used.
2. Equal timeouts may not be expressible in Windmill (its zombie-job timeout).
3. Success criterion 051-003's "work sent to a person" holds by construction once activities are declared retry-safe.
4. etcd, HAProxy and the Patroni agents share one core; detect uninjected failovers.
5. Measurement points of timer lateness, signal latency and warm-up arrivals are undefined.
6. A retried start answered "already exists" must count as accepted.

Counts: Blocker 3, Major 13, Nit 6
