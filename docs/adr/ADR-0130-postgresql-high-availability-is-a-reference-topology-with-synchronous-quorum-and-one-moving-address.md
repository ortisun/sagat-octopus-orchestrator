# ADR-0130: PostgreSQL high availability is a reference topology with a synchronous quorum and one moving address, and the platform acknowledges nothing a standby does not hold

**Status:** Proposed (2026-11-24; revised 2026-11-26 with design 048's split)
**Decided by:** agent proposal (design 048), to be accepted under Owner decision 2026-11-24-4 once design 048's review is complete
**Refines / supersedes:** refines [ADR-0009](ADR-0009-single-region-first.md) (in-region high availability, now specified) and [ADR-0082](ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md) (the serving gate becomes a set of reasons, with `DATABASE_UNCONFIRMED`; readiness unchanged); is the in-region base of [ADR-0067](ADR-0067-a-cell-has-one-writable-region-and-fails-over-by-a-fenced-epoch.md) decision 6; supersedes nothing
**Enforced by:** design 048's tests on the replicated fixture: the dead-peer test and `scripts/check-vendor.py` (Task 048-4), the read-only and `synchronous_commit` tests (Task 048-5), the proving test, the unknown-outcome and pool-exhaustion tests and the architecture checks of commits, writing calls and cancel calls (Task 048-6), the proving test of the unlogged marker, the restart-in-place, overtaken-confirmation and one-beacon tests (Task 048-7), the gate-reasons test (Task 048-8), the conformance scenario `report-through-failover` (Task 048-9), the cross-tenant 404 test, the lagging-standby test and the read-pool rule (Task 048-10), the digest check of the references (Task 048-3), and the Compose drill that gates every release (Task 048-13)

## Context

PostgreSQL is the only authority. Its high availability has been left to the operator: promotion and the switch of the
address are manual, the drill never points the platform at the promoted standby, `25006` is classified internal, nothing
bounds a commit on the client, the kernel's default retransmission keeps a statement to a vanished server waiting for
about 15 minutes, nothing pins `synchronous_commit`, and a primary that restarts after a crash marks committed what no
standby holds, keeping its postmaster's start time when only a backend crashed. The SDKs give up a report after about
1.4 s; worker leases last 30 s; since feature 037 a lease lost mid-activity can become a review item. Design 048 has the
references. Rolling upgrades are a separate decision, [ADR-0131](ADR-0131-migrations-expand-before-and-contract-after-a-release-and-a-binary-refuses-a-schema-it-cannot-use.md) (design 053).

## Decision

1. The repository MUST carry and test a reference topology: CloudNativePG on Kubernetes (namespace-scoped operator,
   managed runtime role, failover quorum) and Patroni 4 with etcd and HAProxy on Compose (a closed control network), three
   PostgreSQL 16 nodes, synchronous quorum one of two standbys, commits refused when no synchronous standby is reachable,
   everything pinned by digest.
2. The platform MUST connect through one address that the HA manager moves at promotion, and MUST NOT elect, choose or
   fence a primary itself.
3. Every database socket MUST set keepalive (4 s idle, 2 s interval, 3 probes) and, on Linux, `TCP_USER_TIMEOUT` of
   10 s, so that a dead peer is found within 10 s (a vendored `sqlx-core` until upstream has the options). A writer
   connection MUST NOT be used on a server in recovery or read-only, or whose `synchronous_commit` is weaker than `on`;
   the writer pools pin it at connection. A `25006` met by a writer is an unavailable server, and its connection is closed.
4. Every write MUST be in a transaction or one writing function call, and every commit or such call MUST go through one
   function with a commit deadline. The outcome is applied only on the server's `COMMIT` (or the call's result) and not
   applied only on `ROLLBACK` or an error of class 23 or 40; every other result is unknown, never "not applied". A commit
   past a hard limit is detached from the pool and kept until its backend answers, at most a pool's worth per process.
   The platform MUST NOT cancel or terminate a backend that waits for a standby.
5. A server's identity is its start time and the token of an unlogged one-row table, which a crash restart and a
   promotion empty. When a process sees a new identity it MUST serve nothing, discard every connection created before,
   and confirm by one waited write of its own on a connection of its own; the confirmation opens the gate only if the
   identity it read is still the newest. The serving gate is a set of reasons, each cleared by its own owner. A command
   that runs once confirms before its first statement.
6. Leases are not extended across a failover by this decision; the lease time to live is sized to the expected failover,
   and carrying leases across a longer one is a follow-up decision.
7. SDKs MUST retry an unavailable or failed-transport report for a window that covers the release gate (310 s by
   default), with a last attempt clamped to the window's end; heartbeats keep their own rule.
8. A standby MUST NOT serve anything that decides, writes, authorizes, misses or reveals a secret. It MAY serve one read,
   the events of a terminal run, authorized on the primary together with the run's highest sequence, and served by a
   standby only up to that sequence and only when the standby holds it, marked without a value.

## Considered options

1. **A reference topology with a moving address, and the platform's side hardened (chosen).** The HA managers already
   do promotion well; the platform's gaps are on its own side, and only a tested reference proves them closed.
2. **Leave HA to the operator (today).** Each operator rebuilds it; the platform's side stays untested.
3. **Several hosts and `target_session_attrs` in every process.** The driver does not support it; each process would
   choose a primary.
4. **Asynchronous replication by default.** Faster commits, RPO above zero, against the owner's objective.
5. **Extending leases after a restart or promotion** (this record's second draft). Its detection, the fencing of every
   reader of an expiry and its exactly-once execution need a decision of their own; deferred.
6. **Reading the server's warning after a cancelled wait, and a waited write on every replay path** (this record's first
   draft). The driver does not surface the warning, and one confirmation per process after a change of identity covers
   what a replay could read.
7. **The postmaster's start time alone as the identity** (this record's third draft). A backend's crash restarts the
   server in place under the same start time.

## Consequences

- Good: a lost node or zone is ridden through without a person; nothing unreplicated is read except after an operator's
  forbidden act; a failover shorter than a lease's remainder creates no retries or review items for delivered leases of
  workers on current SDKs.
- Bad: two HA managers to pin and follow; one commit function for every commit site; a moment of closed gate after a
  restart or promotion; commits stop when both standbys are gone (by choice; the preferred mode remains for an operator
  who accepts its RPO); up to twice a pool's connections per process while commits wait; a vendored `sqlx-core` until
  upstream has the socket options; a claim with an unknown outcome may still become a review item until delivery
  acknowledgement exists.
- Harder: every new commit and writing call goes through the function (checked); every new read must say whether it is
  the one a standby may serve.
- Compatibility: additive tables; old binaries keep working; the SDK change is behaviour within the existing protocol.
- Rollback: the binaries; the topology is outside the platform.

## Evidence

Design 048 and its review reports; `tests/performance/reports/disaster-recovery.md` (the loopback promotion of
2026-10-11); feature 010's gaps 242, 247, 250 and 251. To be added when built: the two proving tests' results, the fault
tests and `tests/performance/reports/high-availability.md`.
