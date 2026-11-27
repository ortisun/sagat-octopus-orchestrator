# Correctness review of design 048, round 3 (2026-11-24)

Doc: docs/designs/048-high-availability-and-rolling-upgrades.md
Reviewed-commit: 29a9c299a5429b22d468a1172d344148a3672d9d
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
(the design, Architecture decisions 0130 and 0131, the code, the pinned sqlx 0.9.0 source, `sdks/SPEC.md`) and ran
nothing. The author answered this report by moving the lease extension out of the design (its findings Blockers 2 and
3, Majors 1, 4 and 5, Nits 5 and 8 then have no subject) before the rest of round 3 ran, and started round 4 on the
revised text.

## Blockers

1. The kernel's retransmission limit does not bound a statement whose bytes the server already acknowledged, and sqlx
   0.9 sets no keepalive (`sqlx-core/src/net/socket/mod.rs`); a commit to a powered-off primary holds its pool slot for
   ever; on Compose, HAProxy is the peer. Fix: keepalive and `TCP_USER_TIMEOUT` on each socket; HAProxy's checks; a fault
   test that drops packets after the commit was acknowledged.
2. Heartbeats and keeper rounds overwrite the continuity row before the extension runs.
3. The caller whose connection triggered the check still uses it; the lost-lease pass can reclaim first.

## Majors

1. The serving gate is one boolean, and the registry reopens it unconditionally on re-registration.
2. The beacon runs inside the acquire timeout, so every timed-out attempt leaves a waiting backend behind (about 800 a
   minute across a fleet).
3. Autocommit writes (about 55 pool-level executes in 11 files, the heartbeat among them) bypass `commit_safely`.
4. The cap on the gap is ambiguous ("clamped" and "at most"), and a stale row is not covered.
5. A single continuity row is written by every keeper round and heartbeat.
6. The stale read does not fix the order of the position read and the data query on the standby, nor the fallback on a
   standby error.
7. A retried concurrent index build leaves an invalid index and records success.
8. Deactivation after a contract migration re-admits N-1.
9. Activation can race a registering N-1 process; check `oldest_compatible` inside the registry's functions.
10. The conformance scenario `report-through-failover` cannot fail on today's SDKs (five attempts and `Retry-After`
    survive a 2 s outage).
11. The retransmission test cannot show what it claims (the sysctl is per network namespace; a pause still acknowledges).
12. The upsert needs `SELECT` on the conflict column; the marker needs `UPDATE`.
13. A `CONNECTION LIMIT` equal to the connection budget leaves no headroom for surge pods and registry pools.

## Nits

1. sqlx 0.9 already defaults the maximum lifetime to 30 minutes.
2. Reclassifying `25006` hides the platform's own read-only transactions' bugs.
3. On PostgreSQL 17 and later, `transaction_timeout` may end a waiting commit; set it to 0 in the writer pools.
4. The residual omits the HA manager's own fast shutdown of waiting backends.
5. The 60 s budget omits the extension's own time.
6. The stale read rarely hits under load; compare against the run's terminal position.
7. A gated process still writes heartbeats and releases shards; `migrate`'s handshake on a fresh or 0.2.x database.
8. Architecture decision 0130 and the design disagree on the extension window.

Counts: Blocker 3, Major 13, Nit 8
