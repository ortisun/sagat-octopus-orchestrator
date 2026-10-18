# Actual late reports versus first certification

Bounded B1b.2a.1d.3b.1. Sol/high researched actual source paths and fixture topology read-only.
Root alone wrote code and executed tests. One fresh Astra/xhigh review found no concrete issue;
the reviewer performed no tests. The user's earlier current-review choice applies; no external
CLI review ran.

The fixture explicitly observes zero chain_segments: ordinary root definitions without a
continuation policy do not initialize a chain. Both actual operations queue on the original Run
tuple. This does not establish a chain-member barrier. Two merely blocked sessions would be
insufficient: the test identifies the first waiting run SELECT behind RunHold and the second
session's dependency on that exact first query, including the holder relationship.

The PostgreSQL16 documentation defines pg_blocking_pids to include both conflicting lock holders
and earlier conflicting waiters. The latter supports this test's observed queue direction:
[PostgreSQL16 session information functions](https://www.postgresql.org/docs/16/functions-info.html).
The test-only observer polls that condition under a deadline; it does not sleep or infer order
from task construction. Pinned futures avoid requiring Flow's tracing guard to be Send.

First execution passed1test with4fresh actual HTTP/gRPC cases: completion/failure × each queue
order. Reports use a new key, return Aborted/STALE_ATTEMPT, and leave the source/run/journal
oracle unchanged. First certification produces one fact and audit; replay returns the same record.
No production code, privilege migration, guards or definition shape changed.

This tests already stale reports after successful closure. It does not prove an in-flight
successful producer versus closure, other producers, chain membership, source UPDATE
immutability, uncertain COMMIT/restart, observations, release activation or full nesting.
