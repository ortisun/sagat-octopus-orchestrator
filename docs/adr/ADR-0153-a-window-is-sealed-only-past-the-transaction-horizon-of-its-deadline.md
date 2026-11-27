# ADR-0153: A trigger window is sealed only past the transaction horizon of its deadline, and trigger operators are activated through the instance registry

**Status:** Accepted (2026-11-26, delegate, Owner decision 2026-11-24-4)
**Decided by:** agent proposal with the third revision of design 042, accepted by the agent preparing slices 2 to 5 of
design 042 for build under the owner's delegation (Owner decision 2026-11-24-4); recorded as Owner decision
2026-11-26-136. The owner may overrule it.
**Refines / supersedes:** refines ADR-0086 decision 4 (the admission grace `admissionGraceMs` is replaced by the
transaction horizon below; the backlog rule and `maxHoldMs` stay) and ADR-0086's rollout (the fleet rule becomes a
requirement of the instance-capability registry, ADR-0082); supersedes neither.
**Enforced by:** the race test that holds an admission transaction open past a window's deadline with a held lock and
asserts that the window is not sealed until it commits (Task 042-55); the activation test of the two capabilities
(Task 042-54). Until they exist: review only.

## Context

An event's `received_at` is taken by `clock_timestamp()` when the inbox row is inserted and becomes visible when the
admission transaction commits, later. ADR-0086 bounded that gap with a grace "at least the admission statement timeout".
On PostgreSQL 16, which the platform runs (`infra/docker/compose.yaml`), there is no transaction timeout, and an
admission transaction is several statements, each bounded by the pool's `statement_timeout` (5 seconds by default,
`crates/infra-postgres/src/pool.rs`) with idle gaps bounded only by `idle_in_transaction_timeout` (60 seconds). So no
fixed grace is a proof. Design 040 reads the audit trail by the same kind of horizon (ADR-0150).

The instance-capability registry of ADR-0082 now exists (`20270113000000_instance_registry.up.sql`), so the fleet rule
of design 042, section 10 (Gaps 042-006 and 042-140) can be enforced instead of written in a runbook.

## Decision

1. When the sealing pass first finds a window whose deadline `D` has passed on the database clock, it MUST record on the
   window `seal_horizon = pg_snapshot_xmax(pg_current_snapshot())`. It MUST NOT seal the window until
   `pg_snapshot_xmin(pg_current_snapshot()) >= seal_horizon`. The admission statement MUST take its transaction id
   (`pg_current_xact_id()`, in a common table expression the insert reads) before it evaluates `received_at`, in the
   same statement, so that it adds no statement. Then any admission that took a `received_at` at or before `D` had its
   transaction id before the horizon was read, and once the horizon is passed it has committed (and is visible) or
   aborted. Should an element still be applied after its window sealed, it is late under ADR-0086 decision 4: it joins
   an open window or opens one, flagged and counted, and is never lost.
2. The backlog rule of ADR-0086 decision 4 stays: after the horizon, a window is not sealed while the tenant and
   namespace hold a pending event with `attempts = 0` that the trigger could match and that was received at or before
   `D`, unless `D + maxHoldMs` has passed (60 seconds by default, at most 10 minutes).
3. `admissionGraceMs` is withdrawn; no configuration names it.
4. The capabilities `collection-operators-v1` and `trigger-operators-v1` MUST be values of the instance registry. The
   definition service MUST refuse to publish a chained `forEach` while the requirement `collection-operators-v1` is not
   recorded for both kinds, and a trigger chain while `trigger-operators-v1` is not.

## Considered options

1. **The transaction horizon** — exact on any PostgreSQL version, no tuning, the technique ADR-0150 uses.
2. **A grace from timeouts** (ADR-0086) — not a proof on PostgreSQL 16, and a long grace delays every window.
3. **`transaction_timeout`** (PostgreSQL 17) — would bound the grace, but the platform runs 16.

## Consequences

A long-running transaction anywhere in the database holds the horizon back and delays sealing (the pass's lag shows
it); it never makes a window lose an element. The window row gains one `xid8` column. The trigger half of design 042
enters release 0.3.0 (Owner decision 2026-11-24-1) under this refinement.

## Evidence

Design 042, sections 4, 6 and 10, third revision (`docs/designs/042-stream-operators.md`); research Decisions 042-161
to 042-166.

## Revision of 2026-11-27 (design 042, architecture review of the third revision)

Recorded without changing the decisions above, under Owner decision 2026-11-24-4. Decision 1: the admission statement
orders its two reads with `MATERIALIZED` common table expressions in sequence, the transaction id first, since a plain
common table expression does not fix the order of evaluation; transaction ids are cluster-wide, which only makes the
horizon more conservative. Decision 4: a process lists a capability only where its configuration runs it (ADR-0082),
the requirement is also checked when `switch` activates a chained version, and the registry's removal `verify` refuses
to remove a requirement while a chain, a window that is not final or a chained version exists. A binary from before the
instance registry does not register, so the registry cannot refuse it; for such binaries the upgrade rule of
`docs/runbooks/scaling.md` is the only guard (design 042, Gap 042-006).
