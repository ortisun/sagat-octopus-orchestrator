# Correctness review of design 048, round 4 (2026-11-25)

Doc: docs/designs/048-high-availability-and-rolling-upgrades.md
Reviewed-commit: f4ec8bbdb5a070d6a5da989cd8ce0fc3440f6926
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
(the design, Architecture decisions 0130 and 0131, the code, the pinned sqlx 0.9.0 source) and ran nothing.

## Blocker

1. Restart detection probably misses a crash restart in place: `pg_postmaster_start_time()` is set once when the
   postmaster starts, and a backend's `kill -9` reinitializes shared memory under the same postmaster; a retried request
   then acknowledges a commit no standby holds. Fix: assert the behaviour first in Task 048-7; an unlogged marker that
   crash recovery and promotion both reset, checked with the start time and the recovery state in one statement.

## Majors

1. The 60 s hard limit cannot free the pool slot: dropping a `Transaction<'static>` queues a rollback and returns the
   connection through a ping that waits for the outstanding commit. Fix: own the `PoolConnection` and detach it.
2. Orphaned waiting backends exhaust the role's `CONNECTION LIMIT` and `max_connections` (53300 for every pool).
3. The unknown-outcome rule lists unknown errors, so anything unlisted (`Protocol`, `WorkerCrashed`, `PoolClosed`, a
   `JoinError`, a `ROLLBACK` command tag) reads as "not applied". Fix: invert it.
4. The registry pool (one connection) cannot carry the beacon; registry writes are autocommit function calls a text
   check cannot see.
5. One-shot commands (`secret-cleanup`, `provision-tenant`, reseal, `migrate`) are undefined under requirement 048-013.
6. Connections to a superseded server survive (the confirmation's clear is not compare-and-clear; conntrack keeps
   connections to an isolated old primary; `before_acquire` discards only after a `25006`).
7. The `25006` classification asks the new primary through the moved address, and misses the realistic case
   (`default_transaction_read_only`).
8. The keepalive bound is about 13 s, not 12 s (with `TCP_USER_TIMEOUT` set, the probe count is ignored); the test
   through docker-proxy cannot see a dead peer; the Compose drill goes through HAProxy.
9. The handshake never re-runs, so the gate of CUJ 4 cannot reopen; the design and Architecture decision 0131 disagree.
10. Migration identity across releases is undefined (a hotfix migration not merged forward blocks the next `migrate`;
    contract migrations counted by `SCHEMA_TOO_OLD`).
11. N-1 and N carry the same version (`Cargo.toml` is `0.2.0` on develop), so the version gates and the mixed-writer test
    cannot fail.
12. The state model of `oldest_compatible` is undefined (two capabilities, deactivation, rows inserted by migrations).
13. Deactivation races the fleet's learning window (up to one heartbeat).
14. An N-1 process the census cannot see still serves after reconnecting.
15. The standby read's completeness proof is not an invariant (a new compensation segment on a terminal run).
16. Readiness today fails on a failed storage ping and on a closed gate (`crates/api-http/src/routes/health.rs`); the
    failover budget omits the probe period and endpoint propagation.
17. The positive case of `report-through-failover` depends on jitter.
18. A claim with an unknown outcome becomes a spurious review item when its undelivered lease expires.
19. The WAL comparison cannot map commits to acknowledgments (`pg_waldump` does not decode tuples).
20. The invalid-index retry can loop for ever, and a concurrently built unique index is not safe for N-1.
21. The platform never pins `synchronous_commit`; a role or database setting can void RPO 0.

## Nits

1. "Two reads" remains in three places.
2. Architecture decision 0130 option 6 says "a beacon per connection".
3. Open Question 6 still names the lease refinement.
4. Adding a capability touches three tables; one blocking lock per migration forces three migrations or a waiver.
5. Releases compared as text; specify semver order.
6. `before_acquire` discards after sqlx's ping.
7. A refusal in `after_connect` storms sqlx's connect retry loop and the log; check the gate before acquiring.
8. Assert the commit's command tag.
9. The proving test must assert that `COMMIT` has not returned past the timeout plus a margin.
10. A migration holds its lock through its own synchronous commit.
11. Whether a refused process's heartbeat row stays live for the census; the retry of `55P03` has no bound.
12. The checksum file is written only at release cut.

Counts: Blocker 1, Major 21, Nit 12
