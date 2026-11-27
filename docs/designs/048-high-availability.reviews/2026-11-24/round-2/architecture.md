# Architecture review of design 048, round 2 (2026-11-24)

Doc: docs/designs/048-high-availability-and-rolling-upgrades.md
Reviewed-commit: 70c9f3f7464087d31e7cc0dc62966570be38f6f1
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files
only, did not read the earlier reports, and found the arithmetic and the owner gate of Task 048-16 right.

## Blockers

1. Readiness contradicts Architecture decision 0082 decision 5 (readiness fails when the registry's last answer is older
   than the liveness window) and requirement 001-002; Architecture decision 0130 misreads it.
2. A commit gets no client-side time limit: requirement 048-010's bound is "none when the timeout is 0", and requirement
   048-012 sets it to 0 for the commit. Fix: a separate commit deadline with a default and a ceiling (feature 032's
   D1585: 10 s, at most 60 s, at least twice the statement timeout).
3. The lease extension is unbounded work inside `after_connect` (3 s acquire timeout, 5 s statement timeout, batches,
   no ceiling), policy in pool plumbing, and a crash between batches extends twice. Fix: compare without a lock; keep the
   serving gate closed until one process's application service has extended, in one transaction, tested at the ceiling.

## Majors

1. The premise of requirement 048-012 is unproven (feature 032's D1566 and gap 1578); the reviewer reads that the
   statement timer is disabled before the commit; Task 048-6 needs a negative control.
2. `transaction_timeout` 60 s is only in the Kubernetes paragraph, untested, and unsafe for the owner role and a commit
   that waits by design.
3. Each failed connection attempt leaks a waiting backend holding the continuity row's lock; all connection setup and
   every keeper round serialize on one row.
4. The failover budget leaves out the surviving standby's reattachment, during which every beacon waits.
5. The 6 s lower bound leaves fast failovers unextended, against requirement 048-016; the 10 s floor equals the longest
   heartbeat interval.
6. What is extended is not named by table; a deadline in the projection would need a journal event; Architecture decision
   0130's refinement of 0020 decision 2 is wrong (that decision is about shard leases); `execution-semantics.md` is not
   updated.
7. Architecture decision 0130 and feature 032 still describe the dropped waited replay (Task 048-7 as "the waited replay
   for all"); two beacon tables; no exemption for the read pool and passive processes.
8. The error vocabulary is not reconciled with `OUTCOME_UNKNOWN`, `CommitOutcomeUnknown` and the parity rule of the
   unknown-outcome codes; "retryable by idempotency key" is false for unkeyed writes.
9. Several contract changes have no task (`SCHEMA_TOO_OLD`, `BINARY_TOO_OLD`, the staleness header, the read address,
   the report window setting, a release field in the registry).
10. A contract migration can break release N-1 processes that have not activated; `migrate` should require the
    capability named by `-- contract-of` to be active.
11. The schema handshake does not cover every process (CLI subcommands, the `secret-cleanup` job, `migrate`), and its
    release identity is undefined (the last migration of a release, hotfix ordering).
12. The rollback after activation contradicts itself.
13. Workers on older SDKs do not get the guarantee of requirement 048-016.
14. The evidence for "no commit after the fork" can pass without checking anything: CloudNativePG rewinds a former primary
    before the drill reads it. Fix: capture its WAL first; a negative control.
15. Rules that say "every" have no failing check (client limits, the commit wrapper, `after_connect` on every pool, the
    handshake in every entry point, the read-pool rule).

## Nits

1. Architecture decision 0130's options are numbered 1, 6, 2, 3, 4, 5.
2. "Three zones" for Compose; "at least 310 s" while configurable lower.
3. The staleness header exposes the global WAL position, and under synchronous replication the standby rarely reaches
   the primary's current position.
4. The exporter of `database_backends_waiting_for_standby` is not named; the type of `schema_gate_closed`.
5. Tenants whose leases changed see no audit record.
6. Clock-skew assumption of the gap; clamp negative gaps.
7. In a transactional migration the unit of retry is the migration; allow the validated-check-then-`SET NOT NULL` pattern
   in a contract migration.
8. TCP keepalive's mechanism in sqlx 0.9.
9. Broker high availability is missing from the Non-Goals.
10. The handshake function in the runtime-privilege audit and under Architecture decision 0020 decision 9.
11. The Constitution Check cites 4.7.0; link the 5.0.0 Sync Impact Report that must name Architecture decision 0130.
12. Two refusals for one case (`BINARY_TOO_OLD` and `BINARY_CAPABILITY_MISSING`).

Counts: Blocker 3, Major 15, Nit 12
