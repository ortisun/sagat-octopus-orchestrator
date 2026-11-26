# ADR-0130: PostgreSQL high availability is a reference topology with a synchronous quorum and one moving address, and the platform acknowledges nothing a standby does not hold

**Status:** Proposed (2026-11-24; revised 2026-11-26 after design 048's full review round)
**Decided by:** agent proposal (design 048), to be accepted under Owner decision 2026-11-24-4 once design 048's review is complete
**Refines / supersedes:** refines [ADR-0009](ADR-0009-single-region-first.md) (in-region high availability, now specified), [ADR-0082](ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md) (the serving gate gains a set of reasons; the registry's row carries the confirmed server identity; its decision 3's `details.reason` is no longer shown before authentication, where every `UNAVAILABLE` is one constant answer, and reasons go to operators), [ADR-0078](ADR-0078-capacity-group-lock-order-and-possession-protocol.md) (one `CommitOutcomeUnknown` and one rule for a definite rollback) and [ADR-0014](ADR-0014-transactions-begin-where-nothing-can-cut-them-off.md) (writes open `BEGIN READ WRITE` in `WriteTx`); supersedes in part [ADR-0067](ADR-0067-a-cell-has-one-writable-region-and-fails-over-by-a-fenced-epoch.md): its decision 6(b) (a waited write on every replay path) and 6(d) (the barrier on reconnect) are replaced by this decision's 4 and 5, and its "no unlogged tables" consequence admits `server_incarnation`, which holds no state of record
**Enforced by (pending until built):** design 048's dead-peer test and `scripts/check-vendor.py` (Task 048-5); the proving test (Task 048-6); the commit tests (Task 048-7); the read-only sessions, the `synchronous_commit` test and the architecture rules (Task 048-8); the gate's reasons (Task 048-9); the proving test of the unlogged marker and the identity tests (Task 048-10); the conformance scenario (Task 048-11); the lost-lease pass's wait (Task 048-12); the reference checks (Tasks 048-3 and 048-4); and the Compose drill that gates every release (Task 048-14)

## Context

PostgreSQL is the only authority. Its high availability has been left to the operator: promotion and the switch of the
address are manual; `25006` is classified internal; nothing bounds a commit on the client, and sqlx cannot report or
bound it; the kernel keeps a statement to a vanished server waiting for about 15 minutes; nothing pins
`synchronous_commit`; and a primary that restarts after a crash marks committed what no standby holds, keeping its
postmaster's start time when only a backend crashed. The SDKs give up a report within about 3 s and a heartbeat until the
next one; worker leases last 30 s. Design 048 has the references; rolling upgrades are
[ADR-0131](ADR-0131-migrations-expand-before-and-contract-after-a-release-and-a-binary-refuses-a-schema-it-cannot-use.md).

## Decision

1. The repository MUST carry and test a reference topology: Patroni 4 with etcd and HAProxy on Compose, which gates every
   release, and CloudNativePG on Kubernetes, not offered for production until its drill has run; three PostgreSQL 16
   nodes, synchronous quorum one of two standbys, commits refused without a synchronous standby, closed networks and
   `pg_hba` for the database, everything pinned by digest. Running without a synchronous standby is an operator's choice
   the platform announces at start and in a metric.
2. The platform MUST connect through one address that the HA manager moves at promotion, and MUST NOT elect, choose or
   fence a primary itself.
3. Every database socket MUST set keepalive (4 s idle, 2 s interval, 3 probes) and, on Linux, `TCP_USER_TIMEOUT` of
   10 s. Every connection MUST start read-only with `synchronous_commit=on`, and refuse a server in recovery, one whose
   `synchronous_commit` is weaker, or, when required, one without synchronous standby names.
4. Every write MUST be a `WriteTx`, whose only commit has a deadline. The outcome is applied on success, not applied on a
   rollback the server confirms (class 23, `40001`, `40P01`), and unknown otherwise, never "not applied". The platform
   MUST NOT cancel or terminate a backend that waits for a standby.
5. A server's identity is its start time and the token of an unlogged one-row table, which a crash restart and a promotion
   empty. When a process observes a new identity, an unknown commit outcome or a read-only refusal, it MUST serve nothing
   and accept no connection to a superseded server until one write of its own (the registry's confirmation) has reached
   a standby. The serving gate is a set of reasons, each cleared by its owner; a write command waits for a confirmed
   identity.
6. Leases are not extended across a failover: SDKs retry heartbeats within the lease's remainder, the lease time to live
   is sized to the expected failover, and the lost-lease pass waits after a confirmation.
7. SDKs MUST retry an unavailable or failed-transport report for a window that covers the release gate (310 s), with a
   last attempt at the window's end.
8. No statement of the platform runs on a standby.

## Considered options

1. **A reference topology with a moving address, and the platform's side hardened (chosen).**
2. **Leave HA to the operator (today).** Each operator rebuilds it; the platform's side stays untested.
3. **Several hosts and `target_session_attrs`.** The driver does not support it.
4. **Asynchronous replication by default.** RPO above zero.
5. **Extending leases after a restart or promotion.** A decision of its own; deferred.
6. **A waited write on every replay path** (ADR-0067 decision 6(b)). A write per duplicate answer; the confirmation after
   a change of server and the gate on an unknown outcome cover what a replay could read.
7. **The start time alone as the identity.** A backend's crash restarts the server in place under the same start time.
8. **Detaching a waiting commit's connection, and reads from a standby** (earlier drafts). The first doubled the
   connection budget; the second gained no availability and conflicted with feature 032's refusal of stale reads.

## Consequences

- Good: a lost node is ridden through without a person; nothing unreplicated is read except after an operator's
  forbidden act; a failover shorter than a lease's remainder less one heartbeat creates no retries or review items for
  delivered leases of workers on current SDKs.
- Bad: two HA managers to pin and follow; every write moves to `WriteTx`; a moment of closed gate after a restart,
  promotion or unknown outcome; when both standbys are gone, writes and reads stop (by choice); a vendored `sqlx-core`
  until upstream has the socket options; a claim with an unknown outcome may still become a review item until delivery
  acknowledgement exists.
- Harder: a write outside `WriteTx` fails at once; every new command that writes waits for a confirmed identity.
- Compatibility: additive tables and columns; the SDK changes are behaviour within the existing protocol.
- Rollback: the binaries; the topology is outside the platform.

## Evidence

Design 048 and its review reports; `tests/performance/reports/disaster-recovery.md` (the loopback promotion of
2026-10-11); feature 010's gaps 242, 247, 250 and 251. To be added when built: the two proving tests' results, the fault
tests and `tests/performance/reports/high-availability.md`.
