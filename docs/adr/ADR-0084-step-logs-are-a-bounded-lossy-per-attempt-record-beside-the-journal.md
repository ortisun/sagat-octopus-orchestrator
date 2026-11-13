# ADR-0084: Step logs are a bounded, lossy, per-attempt record beside the journal

**Status:** Proposed (2026-11-13). Accepting it is the owner's, separately from the answers of OD-2026-11-13-14.
**Decided by:** Agent proposal for feature 041. The owner chose the feature (OD-2026-11-13-9) and, among the options of the design,
the store (O041-004), the ceilings (O041-005), the retention (O041-006), what is kept at a ceiling (O041-007) and the tail route
(O041-008) on 2026-11-13 ("Aceitar todas", OD-2026-11-13-14). The rules below that those answers do not cover (the fence, the absence
of a foreign key, the read order) are the agent's proposal.
**Refines / supersedes:** none. It applies [ADR-0027](ADR-0027-durable-content-rate-and-retention-quotas.md) (log bytes are counted
by the generated triggers and truncated, never refused) and [ADR-0015](ADR-0015-sdks-meet-the-server-by-minor-version-and-are-held-to-one-conformance-suite.md)
(a new capability is an additive protocol minor) without changing either. It is a deliberate exception to the `*_same_tenant_fk`
convention of the migrations, stated in rule 4.
**Enforced by:** nothing yet: the feature is not built. The checks that MUST exist when it is, each a task of
[design 041](../designs/041-step-logs.md): `tests/integration/claim-transition-efficiency/work_budget.rs` passing with its three
constants unmodified while a worker logs (T041-6, rule 2); in the new target `integration_step_logs`, a test that holds the run row
`FOR UPDATE` and sees a batch for that run commit (rule 3), a test that no foreign key leads from the log tables to `runs` or `steps`
(T041-5, rule 4), the duplicate, crash and stale-lease tests (T041-8, rules 5 and 6), the flood test (T041-8, rule 7) and the
held-transaction tail test on PostgreSQL 16 and 18 (T041-11, rule 9); the storage-classification manifest test and the runtime
privilege audit, unmodified (rule 8); the conformance scenarios `step-logs-limits` and `step-logs-transport-faults` in six SDKs
(T041-21, rules 6 and 7).

## Context

A step's code writes log lines that today stay on the worker's machine. Feature 041 sends them to the platform, stores them and
serves them from the run. Four facts constrain where they can live:

- The journal (`run_events`) is the authority for execution state. It is append-only by trigger, never purged, written under the run
  lock, and read by every projection.
- A lease and a completion have a database budget that fails a pull request when it moves (constitution 4.4.0; design 038): 38
  statements, 2 transactions and 5 rows for a lease, 50/1/11 or 47/1/9 for a completion, as equalities.
- The engine locks the run row `FOR UPDATE` while it leases or completes. An insert into a table that references `runs` takes
  `KEY SHARE` on that row, which conflicts with `FOR UPDATE`.
- Retained bytes are counted per tenant by triggers generated from the storage classification (ADR-0027). They fire on row inserts
  and deletes; dropping a partition fires none.

Log volume is set by customer code, not by the platform. A loop can write millions of lines.

What is not known: whether log batches, each of which updates the tenant's one `tenant_retained_usage` row at commit, slow the
completions that update the same row. The design measures it (T041-9, SC-041-007) before the SDKs are built.

## Decision

1. A step log is a diagnostic record. It MUST NOT be execution state: no transition, expression, compensation or review reads it,
   and it MUST NOT be written to the journal, a projection or telemetry.
2. Leasing, heartbeat and completion MUST NOT read or write the log tables, and capture MUST NOT add a statement, a transaction or a
   written row to a lease or a completion.
3. Storing a batch MUST NOT take, or wait for, a lock on a row of `runs`, `steps` or `run_events`. The lease is checked by a read of
   `steps` without a row lock.
4. The log tables MUST NOT have a foreign key to `runs` or `steps`. The run's existence in the tenant is proven by the lease read in
   the storing transaction. Whatever deletes a run (feature 020) MUST delete its log streams and usage row.
5. A batch MUST be accepted only while the attempt it names holds its lease (attempt, generation and lease token match). A stale
   holder MUST be refused, never merged.
6. Delivery is at-least-once and storage is idempotent: the server MUST answer only after the commit, and a record's identity is
   (attempt, generation, sequence), stored at most once.
7. The record is bounded and may lose lines. Ceilings per message, attempt, run and tenant MUST be enforced by the server from
   counters, before the insert, whatever the worker sends. At a ceiling the first lines are kept; what is not stored MUST be counted
   and marked where a reader sees it. Meeting a ceiling, a refused batch or an unreachable server MUST NOT fail, block, retry or
   delay a step beyond one bounded final flush.
8. Lines are rows of an ordinary PostgreSQL table, one row per line, classified `log` in the storage classification, charged and
   released by the triggers of ADR-0027, and purged by batched `DELETE` after `step_log_retention_days`. The table MUST NOT be
   partitioned while those triggers are how bytes are released.
9. Lines MUST be read in the order (storing transaction id, row id), and only below the oldest transaction still in progress, so
   that a page is final and a tail neither skips nor repeats. The worker's time is shown and MUST NOT order lines.
10. Log lines MUST NOT travel on `/runs/{runId}/stream`; the tail has a route of its own.

## Considered options

1. **Rows in PostgreSQL beside the journal, fenced by the lease, with no key to `runs`** — chosen. One store, no new component,
   duplicate removal by primary key, filters and paging in SQL, exact quota accounting with what exists, and the hot path untouched.
2. **Log lines as journal events** — lost. The journal is never purged and is written under the run lock; log volume would slow
   replay and every lease and completion, and could not be deleted.
3. **One row per batch, compressed** — lost for 0.3.0 (O041-004). About a tenth of the rows; filters, search and per-line identity
   move into the application. It is the next step if the measurement fails on storage.
4. **The blob store with an index in PostgreSQL** — lost for now. Cheapest per byte; committed blobs are never purged today (G69),
   and a tail and a filter would read objects.
5. **A table partitioned by time** — lost. Dropping a partition does not release quota bytes, retention differs per tenant, and it
   would be the first partitioned table of the database, with its own maintenance, grants and privilege audit.
6. **A foreign key to `runs`, as every run-scoped table has** — lost. It puts log ingest in the lock queue of leases and completions.
7. **A per-run sequence for the read order** — lost. It needs the run lock. A cursor per attempt grows with the number of iterations.
8. **Accepting batches after the lease ends, under a log capability of its own** — lost. A second token with its own lifetime and
   revocation, to save at most one flush interval of lines (G041-002).
9. **Keeping the last lines at a ceiling** — lost (O041-007). It needs deletes on the write path and makes a cursor unstable.

## Consequences

Good: a step that logs costs the engine's transactions nothing; a duplicate, a crash or a flood cannot corrupt a run or a quota;
retention works today, without waiting for feature 020.

Bad, and stated to the reader: lines can be missing. The tail of an attempt that lost its lease or its network is lost (G041-002)
and shown as "ended without its final flush". A long write transaction anywhere in the database delays the tail until it ends.
Text search scans one run's lines.

What becomes harder: every step that logs adds one transaction to the database beside its lease and completion, and one update of
the tenant usage row; if SC-041-007 fails, the next step is a grouped commit on the server, and after that a separate usage row,
which needs an amendment of ADR-0027. The missing foreign key is a duty on feature 020 (G041-004) that no constraint enforces; a
test asserts that every stream's run exists.

Compatibility: additive tables, an additive RPC (protocol minor 1.7) and three new REST operations. Runs in flight and workers
below 1.7 are unaffected. Rollback: `SAGAT_STEP_LOGS_ENABLED=false` stops capture without touching the database; an older binary
leaves the tables unread; the down migration refuses while a line exists.

## Evidence

None measured yet; this ADR is proposed with the design and nothing is built. The reasoning rests on facts read in the code at
`a4f6e9db`: the budgets in `work_budget.rs`, the quota triggers of `20261024000000_durable_quotas.up.sql`, the absence of any
`PARTITION BY` or run deletion in the migrations and stores, and the precedent of `script_executions.log` (1 MiB per attempt,
counted as `log`, purged by `purge_script_records`). The design is [docs/designs/041-step-logs.md](../designs/041-step-logs.md);
decisions D041-001 to D041-007 and D041-009, D041-011 and gaps G041-002, G041-004, G041-006 are in
[specs/041-step-logs/research.md](../../specs/041-step-logs/research.md). The measurement that can overturn rule 8 is T041-9.
