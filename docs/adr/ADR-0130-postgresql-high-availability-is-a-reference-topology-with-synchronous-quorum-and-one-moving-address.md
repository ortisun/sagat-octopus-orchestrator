# ADR-0130: PostgreSQL high availability is a reference topology with a synchronous quorum and one moving address, and the platform acknowledges nothing a standby does not hold

**Status:** Proposed (2026-11-24)
**Decided by:** agent proposal (design 048), to be accepted under Owner decision 2026-11-24-4 once design 048's review is complete
**Refines / supersedes:** refines [ADR-0009](ADR-0009-single-region-first.md) (in-region high availability, now specified) and [ADR-0020](ADR-0020-the-scheduler-is-sharded-by-run-and-owned-by-lease-with-a-fence.md) (shard and worker leases survive a failover); is the in-region base of [ADR-0067](ADR-0067-a-cell-has-one-writable-region-and-fails-over-by-a-fenced-epoch.md) decision 6
**Enforced by:** the tests of design 048: the both-standbys-stopped fault test (Task 048-6), the waited-replay test (Task 048-7), the lease-extension fault test (Task 048-8), the conformance scenario `report-through-failover` (Task 048-9), the read-address routing test (Task 048-10), and the three-zone and Compose drills that gate the release (Tasks 048-16 and 048-17)

## Context

PostgreSQL is the only authority. Its high availability has been left to the operator: promotion and the switch of the
address are manual (`docs/runbooks/disaster-recovery.md`), the drill never points the platform at the promoted
standby, and the platform has no handling for a read-only database (SQLSTATE `25006` is classified internal), no
client bound on a query to a server that vanished, and no defence against PostgreSQL's success answer to a `COMMIT`
whose synchronous wait was cancelled. The SDKs give up a report after about 1.4 s of retries; worker leases last 30 s;
since feature 037 a lease lost mid-activity can become a review item. Design 048 has the measurements and references.

## Decision

1. The repository MUST carry and test a reference topology: CloudNativePG on Kubernetes and Patroni with etcd and
   HAProxy on Compose, three instances in three zones, synchronous quorum one of two standbys, commits refused when no
   synchronous standby is reachable, WAL archiving and base backups, everything pinned by digest.
2. The platform MUST connect through one address that the HA manager moves at promotion. The platform MUST NOT elect,
   choose or fence a primary itself.
3. A writer connection to a database in recovery MUST NOT be used; `25006` is unavailable and retryable and its
   connection closed; every statement MUST have a client bound of its transaction's timeout plus 2 s, closing the
   connection hard; connections MUST have a maximum lifetime; a process stays ready and answers a typed 503 while the
   database is unreachable, as Architecture decision 0082 decision 5 keeps it serving.
4. With synchronous standbys configured, the database MUST NOT cancel a commit's wait for a standby (the commit's
   timeout is disabled in the transaction's last statement; `client_connection_check_interval` is 0); a commit past its
   client bound is reported as an unknown outcome (`COMMIT_OUTCOME_UNKNOWN`), never as success or as "not applied".
5. A connection MUST commit a write that waits for the standby before its first use, so that whatever it reads is on a
   standby.
6. After a database restart or promotion, worker leases and attempt deadlines whose expiry fell in the gap MUST be
   extended by the gap (at most 300 s, with a floor of 10 s), once, by compare-and-set, before any other use of the new
   primary, and audited; shard leases are not extended; a change of region epoch extends nothing. This refines
   Architecture decision 0020 decision 2 for this case only.
7. SDKs MUST retry an unavailable report for at least the release gate plus one backoff (310 s by default); heartbeats
   are not retried.
8. A standby MUST NOT serve anything that decides, writes, authorizes or reveals a secret. It MAY serve only two reads,
   the journal pages of a terminal run and run listing and search, authorized on the primary and served by a standby only
   when its replay position has reached the primary's position read at authorization.

## Considered options

1. **A reference topology with a moving address, and the platform's side hardened (chosen).** The HA managers already
   do promotion well; the platform's gaps are on its own side, and only a tested reference proves them closed.
6. **Reading the warning of a cancelled wait** (this record's first draft). The driver does not surface it, and the
   transaction is visible before it could be read.
2. **Leave HA to the operator (today).** Each operator rebuilds it; the platform's side stays untested, and RPO 0 is
   not true when the standbys stall.
3. **Several hosts and `target_session_attrs` in every process.** The driver does not support it; each process would
   choose a primary, which is the election the platform must not do.
4. **Asynchronous replication by default.** Faster commits, RPO above zero, against the owner's objective.
5. **Longer leases instead of an extension.** Slows the recovery from every real worker crash to buy the rare failover.

## Consequences

- Good: a lost instance or zone is ridden through without a person; RPO 0 is true even when the standbys stall; a
  failover creates no retries or review items for work that finished during it.
- Bad: two HA managers to pin and follow; the replay paths gain a write; commits stop when two of three zones are lost
  (by choice; the preferred mode remains for an operator who accepts its RPO).
- Harder: every new replay path must commit a waited transaction; every new read must say whether it is on the list.
- Compatibility: additive migrations (continuity row, beacons); old binaries keep working; the SDK change is behaviour
  within the existing protocol.
- Rollback: the binaries; the topology is outside the platform.

## Evidence

Design 048 and its review reports; `tests/performance/reports/disaster-recovery.md` (the loopback promotion of
2026-10-11); feature 010's gaps 242, 247, 250 and 251. To be added when built: the fault tests above and
`tests/performance/reports/high-availability.md`.
