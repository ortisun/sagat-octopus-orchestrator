# ADR-0084: Step logs are a bounded, lossy, per-attempt record beside the journal

**Status:** Accepted (2026-11-13, OD-2026-11-13-17: the owner's "Aceito tudo"); revised the same day after review, before any code (see Revision)
**Decided by:** owner, on the agent's proposal (OD-2026-11-13-17, accepting the text of commit 35b752b3). The owner chose the feature
(OD-2026-11-13-9) and, among the options of the design, the store (O041-004), the ceilings (O041-005), the retention (O041-006), what
is kept at a ceiling (O041-007) and the tail route (O041-008) (OD-2026-11-13-14). The revision of 2026-11-13 is the agent's, answering
the architecture, security and correctness reviews of commit 2e90dbb8, and is put to the owner with the design (T041-2).
**Refines / supersedes:** none. It applies [ADR-0027](ADR-0027-durable-content-rate-and-retention-quotas.md) (log bytes are counted
by the generated triggers and truncated, never refused) and [ADR-0015](ADR-0015-sdks-meet-the-server-by-minor-version-and-are-held-to-one-conformance-suite.md)
(a new capability is an additive protocol minor) without changing either. It is a deliberate exception to the `*_same_tenant_fk`
convention of the migrations, stated in rule 4.
**Enforced by:** nothing yet: the feature is not built. The checks that MUST exist when it is, each a task of
[design 041](../designs/041-step-logs.md): in `tests/integration/claim-transition-efficiency/`, `work_budget.rs` passing with its
constants unmodified while a worker logs, the lock-free test (a batch commits under `lock_timeout = 50ms` while the run row is held
`FOR UPDATE` and the leased step row updated), the tenant-row barrier test (a completion with output commits under
`lock_timeout = 100ms` while an ingest is held after its inserts) and the no-foreign-key test (T041-5, T041-6; rules 2, 3, 4); in the
new target `integration_step_logs`, the duplicate, crash, revocation, expired-lease, ceiling-concurrency and lying-marker tests
(T041-8; rules 5, 6, 7), the lock-order barrier test (T041-13; rule 11), the held-transaction tail and end tests on PostgreSQL 16 and
18 (T041-11, T041-12; rule 9); the storage-classification manifest test and the runtime privilege audit, unmodified (rule 8); the
conformance scenarios `step-logs-limits` and `step-logs-transport-faults` in six SDKs (T041-21; rules 6, 7). Each gets its row in
`docs/engineering/architecture-principles.md` when it is built.

## Context

A step's code writes log lines that today stay on the worker's machine. Feature 041 sends them to the platform, stores them and
serves them from the run. These facts constrain where they can live:

- The journal (`run_events`) is the authority for execution state. It is append-only by trigger, never purged, written under the run
  lock, and read by every projection.
- A lease and a completion have a database budget that fails a pull request when it moves (constitution 4.4.0; design 038): 38
  statements, 2 transactions and 5 rows for a lease, 50/1/11 or 47/1/9 for a completion, as equalities.
- The engine locks the run row `FOR UPDATE` while it leases or completes. An insert into a table that references `runs` takes
  `KEY SHARE` on that row, which conflicts with `FOR UPDATE`.
- Retained bytes are counted per tenant by triggers generated from the storage classification (ADR-0027). Each inserted row fires a
  per-row trigger; at commit a deferred finalizer takes the content and log policy fences and then the tenant's
  `tenant_retained_usage` row `FOR UPDATE`, and raises when the limit is passed. `sagat_quota_lock_log_usage` takes that row early and
  holds it to commit. A completion that stores output takes the same row in its commit while it holds the run lock. Dropping a
  partition fires no trigger.
- A worker report is fenced by the lease token and also by the caller's role, membership and activity binding.

Log volume is set by customer code, not by the platform. A loop can write millions of lines.

What is not known: how much a log batch's commit, which still takes the tenant row, slows completions of the same tenant. The design
measures it before the RPC is built (T041-6) and again end to end (T041-9).

## Decision

1. A step log is a diagnostic record. It MUST NOT be execution state: no transition, expression, compensation or review reads it,
   and it MUST NOT be written to the journal, a projection or telemetry.
2. Leasing, heartbeat and completion MUST NOT read or write the log tables, and capture MUST NOT add a statement, a transaction or a
   written row to a lease or a completion.
3. Storing a batch MUST NOT take, or wait for, a lock on a row of `runs`, `steps` or `run_events`. It MUST NOT take the tenant's
   `tenant_retained_usage` row before its commit, except when usage plus the batch is within 2 MiB of the tenant's log limit; then it
   takes it as the last statement before its insert. The whole ingest runs under a lock timeout of 200 ms, answered as "retry later".
4. The log tables MUST NOT have a foreign key to `runs` or `steps`. The run's existence in the tenant is proven by the lease read in
   the storing transaction. Whatever deletes a run (feature 020, G1618 there) MUST delete its log streams, lines and usage row.
5. A batch MUST be accepted only from a caller that a report would accept short of the run lock: `worker:execute`, not the
   database-runner or runner role, a current membership, an activity binding that covers the step, a session of the same subject
   accepted at 1.7 or later, and a lease that is `LEASED` or `RUNNING` with the same attempt, generation and token, unexpired, on a run
   neither finished nor stopping. Anything else MUST be refused and store nothing.
6. Delivery is at-least-once and storage is idempotent: the server MUST answer only after the commit, and a record's identity is
   (attempt, generation, sequence), stored at most once. After an attempt's `final` batch is recorded, later batches MUST store nothing.
7. The record is bounded and may lose lines. Ceilings per message, attempt, run and tenant MUST be enforced by the server from
   counters, before the insert, on the text as it will be stored (after redaction and cutting), whatever the worker sends; counters
   MUST be updated from the rows actually inserted. At a ceiling the first lines are kept; what is not stored MUST be counted where a
   reader sees it. A count a worker reports for its own drops MUST be derived by the server from sequence ranges and clamped at the
   attempt's ceiling. Markers MUST NOT carry log-classified bytes. Meeting a ceiling, a refused batch or an unreachable server MUST
   NOT fail, block, retry or delay a step beyond one bounded final flush.
8. Lines are rows of an ordinary PostgreSQL table, one row per line, classified `log` in the storage classification, charged and
   released by the triggers of ADR-0027, and purged by batched `DELETE` once the last line is older than the retention in force at
   purge time. The table MUST NOT be partitioned while those triggers are how bytes are released.
9. Lines MUST be read in the order (storing transaction id, row id), and only below the oldest transaction still in progress, both
   computed in the reading statement, so that a page is final and a tail neither skips nor repeats. A storing transaction MUST obtain
   its id before it reads the lease, and a tail MUST NOT end until the horizon has passed every transaction that could have seen the
   lease live. The worker's time is shown and MUST NOT order lines.
10. Log lines MUST NOT travel on `/runs/{runId}/stream`; the tail has a route of its own.
11. Every path that writes step logs MUST take its locks in one order: the namespace secrets lock and the membership row, the stream
    row, the run's usage row, the content policy fence, the log policy fence, the tenant usage row.

## Considered options

1. **Rows in PostgreSQL beside the journal, fenced like a report, with no key to `runs` and the tenant row left to the commit** —
   chosen. One store, no new component, duplicate removal by primary key, filters and paging in SQL, exact quota accounting with what
   exists, and no statement on the hot path.
2. **Log lines as journal events** — lost. The journal is never purged and is written under the run lock.
3. **One row per batch, compressed** — lost for 0.3.0 (O041-004). The next step if the measurement fails on storage.
4. **The blob store with an index in PostgreSQL** — lost for now. Committed blobs are never purged today (G69).
5. **A table partitioned by time** — lost. Dropping a partition does not release quota bytes, and retention differs per tenant.
6. **A foreign key to `runs`, as every run-scoped table has** — lost. It puts log ingest in the lock queue of leases and completions.
7. **Taking the tenant allowance under the row lock in every batch**, as script logs do — lost in review. It holds the row a
   completion with output needs, for the whole ingest, in every batch of every step that logs.
8. **A per-run sequence for the read order** — lost. It needs the run lock.
9. **Accepting batches after the lease ends** — lost. A second token to save at most one flush interval (G041-002).
10. **Keeping the last lines at a ceiling** — lost (O041-007).

## Consequences

Good: a step that logs adds no statement to the engine's transactions; a duplicate, a crash, a revoked worker or a flood cannot
corrupt a run or a quota; retention works today, without waiting for feature 020.

Bad, and stated to the reader: lines can be missing. The tail of an attempt that lost its lease or its network is lost (G041-002)
and shown as "ended without its final flush". A long write transaction anywhere in the database delays the tail until it ends.
Text search scans one run's lines.

What is not free, said plainly: every step that logs adds transactions beside its lease and completion, each of which takes the
tenant usage row in its commit and fires a per-row trigger for each line. A completion with output of the same tenant can wait for
one such commit, and near the tenant's limit for one batch's insert and commit (bounded by the 200 ms lock timeout). The criterion
is SC-041-007 of the design; if it fails, the next step is a grouped commit on the server, and after that a separate usage row,
which needs an amendment of ADR-0027. The missing foreign key is a duty on feature 020 (G1618) that no constraint enforces; a test
asserts that every stream's run exists.

Compatibility: additive tables, an additive RPC (protocol minor 1.7) and new REST operations. Runs in flight and workers below 1.7
are unaffected. Rollback: `SAGAT_STEP_LOGS_ENABLED=false` stops capture without touching the database (it is the default until the
purge pass exists); an older binary leaves the tables unread; the down migration refuses while a line exists.

## Revision (2026-11-13, after review, before any code)

The owner accepted the text of commit 35b752b3. The architecture, security and correctness reviews of commit 2e90dbb8 found that the
ingest held the tenant's retained-usage row for its whole transaction, so completions with output would queue behind log batches
(the blocker); that the fence was the lease token alone, with no binding, membership, expiry or session check; that ceilings were
applied before redaction and counters updated from the batch rather than the rows stored; that a worker's drop count was trusted;
that markers carried log bytes; that a tail could end before a late batch committed; and that the script marker could take locks in
the opposite order. Decisions 3, 5, 6, 7, 8 and 9 were rewritten and decision 11 added; option 7 was added. The chosen option is
unchanged. The reports are in `docs/designs/041-step-logs.reviews/2026-11-13/`.

## Evidence

None measured yet; nothing is built. The reasoning rests on facts read in the code at `2e90dbb8`: the budgets in `work_budget.rs`;
the trigger functions and `sagat_quota_lock_log_usage` of `20261024000000_durable_quotas.up.sql` (lines 150-370);
`content_usage::take_log_prefix`; the heartbeat's checks in `crates/application/src/worker.rs` (`heartbeat`, `ensure_bound`) and
`current_worker_authorized` in `crates/infra-postgres/src/execution.rs`; `steps_lease_consistent`; the absence of any
`PARTITION BY` or run deletion; and the precedent of `script_executions.log`. The design is
[docs/designs/041-step-logs.md](../designs/041-step-logs.md); decisions D041-001 to D041-007, D041-009, D041-011, D041-013, D041-015,
D041-017 to D041-020 and D041-023, and gaps G041-002, G041-004, G041-006, are in
[specs/041-step-logs/research.md](../../specs/041-step-logs/research.md). The measurements that can overturn rule 3 are the spike of
T041-6 and T041-9.
