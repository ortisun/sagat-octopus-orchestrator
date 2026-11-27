# Targeted re-read 1 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 039bfada3132857347aa93769e20eda8854a5cc3
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the
fixes of the full round (`security-full.md`, at 1e283510), not a full review. The reviewer read files only and ran
nothing.

Full round: Majors 1, 2, 5, 6, 7 resolved; Majors 4 and 8 moot (removed); Major 3 partly (below); Nits 3, 4, 8, 9
resolved; Nits 1, 2, 5, 6, 10 moot; Nit 7 partly (the owner role and the migrate run on Kubernetes); Nit 11 open (an
orphaned backend holds its slot while the standbys are down, uncounted).

## Major

1. (Verified in code.) The deferred quota triggers raise `54000` at `COMMIT`, which the new rule would make unknown,
   closing the gate: any tenant going over its quota would close a process for every tenant. Fix: an `ERROR` answer on a
   live session is a definite rollback; only `FATAL`, `PANIC`, I/O and the deadline are unknown.
2. `sagat_server_identity()` inserts the token on a session started read-only, so after any crash restart or promotion
   every connection fails with `25006` and no process can confirm. Fix: the function only reads; the confirmation's
   `WriteTx` inserts; the registry's connection is admitted while unconfirmed.
3. (Full-round Major 3, partly.) The window of reads from an isolated former primary is not stated, and the revocation
   check runs only once ready. Fix: state the bound (about 12 + 10 s) and assert it.

## Nit

1. The lint misses `SET ... READ WRITE`, `SESSION CHARACTERISTICS`, `START TRANSACTION` and raw transaction control.
2. A `Retry-After` that differs by reason before authentication reveals a failover.
3. The policy must pair the namespace and pod selectors in one entry; `denyCidrs` must cover the cluster's pod and
   Service ranges.
4. `DATABASE_NOT_SYNCHRONOUS` can be one heartbeat late, and clearing it reopens without a confirmation.
5. A command proceeds on "no live row", which also describes a platform that was down at a crash restart.
6. (Full-round Nit 7, partly.) The owner role and the migrate run on Kubernetes.
7. (Full-round Nit 11, open.) Orphaned backends in the connection budget.

Counts: Blocker 0, Major 3, Nit 7

All ten were fixed at `e56630a2`.
