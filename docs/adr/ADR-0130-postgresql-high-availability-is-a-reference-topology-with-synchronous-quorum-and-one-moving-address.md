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
3. A connection to a database in recovery MUST NOT be used; `25006` and `57P0x` are unavailable and retryable; a
   connection that returned `25006` MUST be closed; every statement MUST have a client bound of the server's timeout
   plus 2 s; connections MUST have a maximum lifetime; readiness MUST follow writability, liveness MUST NOT.
4. With synchronous standbys configured, an operation MUST NOT be acknowledged on a `COMMIT` that returned after a
   cancelled or terminated synchronous wait, and an answer that acknowledges earlier work without writing MUST follow a
   waited commit of its own transaction.
5. After a failover, leases whose expiry fell inside the outage MUST be extended by the outage's length, once, before
   any lease is reclaimed; the extension never shortens a lease or changes a fence and is audited.
6. SDKs MUST retry an unavailable report for a window of at least the failover budget plus one backoff (120 s by
   default).
7. A standby MUST NOT serve anything that decides, writes, authorizes or reveals a secret. It MAY serve only the closed
   list of stale-tolerant reads of `docs/architecture/storage-and-events.md`, authorized on the primary, refused beyond
   5 s of replay lag with a fallback to the primary, and marked as possibly stale.

## Considered options

1. **A reference topology with a moving address, and the platform's side hardened (chosen).** The HA managers already
   do promotion well; the platform's gaps are on its own side, and only a tested reference proves them closed.
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
