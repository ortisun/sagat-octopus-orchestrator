# Correctness review of design 048, full round of 2026-11-26

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 1e28351031ae51b7b6625a86f4c4350790ffd2c1
Reviewer: correctness (full round; the approval's correctness report is the re-read that follows the fixes)
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
only (the design, the ADR, the pool, readiness, the keeper, the SDK specification and sqlx 0.9.0's source) and ran
nothing.

## Major

1. `commit_safely` cannot be built on sqlx 0.9's public API: no command tag, no transaction status, no way to take a
   `Transaction`'s `PoolConnection`. Fix: vendor `sqlx-postgres` too, or a wrapper that owns the connection and sends
   `BEGIN` and `COMMIT` itself.
2. Process start fails: `after_connect` refuses every connection until a confirmation, but pools connect eagerly. Fix:
   confirm before building pools, or connect lazily; say how fixtures confirm.
3. "Still the newest observed" is undefined: after a crash restart in place `after_connect` sees an absent token and the
   confirmation reads a new one, so the compare can never succeed. Fix: an absent token is "unconfirmed", not an
   identity; the confirmation's own read counts as an observation.
4. Setting the reason is not compare-and-set, so a wake-up can be lost after a clear. Fix: set it under the gate's lock
   only if the read identity differs from the confirmed one; keep the confirmation task alive and signal it.
5. sqlx stamps a connection's creation after `after_connect` returns, so a connection that read the old identity just
   before the change survives the discard by creation time. Fix: discard by the identity read or a counter.
6. The confirmation connection pins neither `synchronous_commit` nor the checks, so a role setting of `local` opens the
   gate on unreplicated state. Fix: the same startup options and checks.
7. The lease arithmetic is off by one heartbeat interval: the safe outage is 30 − 2 × 10 = 10 s, not 20 s. Fix: state it,
   or keep heartbeats retrying until the lease expires.
8. The failover budget leaves out readiness's registry rule (it waits for the next beat, up to 10 s) and the beacon's
   deadline (a late answer thrown away plus a backoff). Fix: beat at once when the gate clears; the confirmation waits
   with no deadline; rederive the budget.
9. A repetition's 4 minutes after the fault are below the 300 s gate, the 310 s window and the 90% recovery held 30 s.
10. The release gate leaves out the integrity oracle and the fleet's recovery. Fix: gate on the oracle, every process
    ready, the 90% recovery within 300 s.
11. The negative control cannot be produced as written (asynchronous WAL still ships; the platform refuses connections
    when the standby names are empty). Fix: block replication first, commit through the drill's own connection, then
    isolate.
12. The ledger's key (run, sequence) cannot tell a lost completion from one redone after its lease expired. Fix: key by
    attempt, generation and lease token, and compare content.

## Nit

1. Readiness, the passes and the registry beats also acquire while unconfirmed; every path must check the gate first.
2. `execute_safely` must drain to ReadyForQuery; forbid `fetch()` for writing calls.
3. The keeper waits up to 3.6 s after a round plus the round's time, not 3 s.
4. The connection formulas of Task 048-3, Requirement 048-012 and the ADR disagree; the ADR's 1.4 s against 3 s.
5. The vendored change also adds `socket2` to `sqlx-core`'s manifest.
6. "The connection that met a `25006` is closed": no public API on a `Transaction` does it.
7. The drill copies a running former primary; fence it first; name the networks; "a node stopped" on Compose.
8. CUJ 2's "the pool is not exhausted" is false once the cap is reached; "the retry finds it" only if the standby returns
   in time.
9. Task 048-8 before Task 048-7; Task 048-9 before Task 048-13.

Counts: Blocker 0, Major 12, Nit 9
