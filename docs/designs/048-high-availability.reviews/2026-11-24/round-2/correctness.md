# Correctness review of design 048, round 2 (2026-11-24)

Doc: docs/designs/048-high-availability-and-rolling-upgrades.md
Reviewed-commit: 70c9f3f7464087d31e7cc0dc62966570be38f6f1
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
(the design, Architecture decisions 0130 and 0131, the code, the pinned sqlx 0.9.0 source) and ran nothing.

## Blocker

1. The commit rule rests on an unproven PostgreSQL behaviour: as the reviewer reads the source, `finish_xact_command()`
   turns the statement timeout off before the commit, so the server's timeout never reaches the wait for the standby
   (feature 032 lists it as unproven, gap 1578). If so, folding the timeout's reset into 232 commit sites protects
   nothing; if not, autocommit writes (about 23 sites) escape the rule. Fix: a proving test first; then either drop the
   folding, or enforce it in one commit primitive and forbid autocommit writes.

## Majors

1. Nothing bounds `COMMIT` on the client: the bound is derived from a timeout set to 0 for the commit; sqlx has no TCP
   keepalive option, and keepalive does not fire with unacknowledged data.
2. Other commits with an unknown outcome (cut by HAProxy, by a `kill -9`, by `57P01` after the commit record) are
   reported as "nothing applied". Fix: any failure after `COMMIT` was sent is an unknown outcome.
3. With both standbys gone, waiting backends pile up until `max_connections` is used up.
4. The extension does not fire after a crash restart in place (`restart_after_crash`): start time and timeline stay the
   same. Fix: an unlogged marker row.
5. The extension's cap (300 s from the last round) and the SDK window (310 s from the first failure, `Retry-After`
   unstated) do not cover the release gate at its boundary.
6. The extension inside `after_connect` can stop every connection after a failover (the 3 s acquire timeout, batches,
   the standby's attachment); and every new connection queues on one row.
7. The readiness rule contradicts Architecture decision 0082 decision 5 (not ready after 30 s of registry silence).
8. The schema and capability handshake leaves a window in which a stale binary that was cut off writes after it
   reconnects. Fix: check in the `after_connect` transaction.
9. The prescribed migration patterns cannot run under sqlx (a file is one simple query, one implicit transaction; retry
   per statement is impossible; `SET lock_timeout` leaks).
10. A `transaction_timeout` of 60 s for the owner role would kill migrations.
11. "A migration cannot stall the hot path for more than its lock timeout" is false; locks are held to the commit.
12. Architecture decision 0130 does not match the design (a replay write, Task 048-7's name) and drops Architecture
    decision 0067's durability lease without stating the residual.
13. Rollback after activation cannot work: nothing lowers the oldest compatible release.
14. The planned fault tests have no synchronous standby (`tests/fault/failover` has none), so they pass without testing.

## Nits

1. The floor of 10 s applied to leases only can put a lease past its attempt deadline.
2. The position check compares positions without the timeline, and must run on the same standby connection.
3. The read pool and passive processes cannot write the beacon.
4. `sdks/SPEC.md` already allows `CompleteStep` five attempts; `Retry-After` overrides the cap.
5. The census-based refusal ignores silent processes and operator commands.
6. Closing a `25006` connection needs a pool hook.
7. The extension's scope does not list script leases and the deadlines left on the wall clock.
8. Record the continuity values on every branch, not only when extending.

Counts: Blocker 1, Major 14, Nit 8
