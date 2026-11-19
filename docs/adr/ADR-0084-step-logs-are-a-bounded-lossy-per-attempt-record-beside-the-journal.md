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

Revision accepted 2026-11-15, OD-2026-11-15-4.

The owner accepted the text of commit 35b752b3. The architecture, security and correctness reviews of commit 2e90dbb8 found that the
ingest held the tenant's retained-usage row for its whole transaction, so completions with output would queue behind log batches
(the blocker); that the fence was the lease token alone, with no binding, membership, expiry or session check; that ceilings were
applied before redaction and counters updated from the batch rather than the rows stored; that a worker's drop count was trusted;
that markers carried log bytes; that a tail could end before a late batch committed; and that the script marker could take locks in
the opposite order. Decisions 3, 5, 6, 7, 8 and 9 were rewritten and decision 11 added; option 7 was added. The chosen option is
unchanged. The reports are in `docs/designs/041-step-logs.reviews/2026-11-13/`.

## Revision (2026-11-15), accepted 2026-11-15 (OD-2026-11-15-4)

Revision accepted 2026-11-15, OD-2026-11-15-4.

Note (2026-11-15, OD-2026-11-15-9): the owner kept a script-output marker. Its write takes the stream row before the
fences and the tenant usage row, so it follows decision 11 with no exception (design 041, section 7); the words "the
script marker is removed" of decision 11 no longer describe the design. The tenant's step-log bytes are a resource and a
storage class of their own (OD-2026-11-15-5, ADR-0027 Amendment of 2026-11-15), so in decision 8 the lines are classified
`step_log`, not `log`, and in decision 11 "the content policy fence, the log policy fence" reads "the policy fences of
the classes the transaction charges, in ADR-0027's order (content, log, step-log)"; decision 3 applies to the new
resource. In "Enforced by", the end test is the one of design 041's T041-12: an ingest with the largest id below the end
horizon, held open. These readings follow from the owner's answers of the same day and change no rule's intent.

The Status line above is unchanged; the decisions below replace those of the same numbers above. The egm-review and the second round of the
architecture, security and correctness reviewers (`docs/designs/041-step-logs.reviews/2026-11-15/`) found that the end
rule of decision 9 repeats the defect feature 018 fixed in D1958; that the locking path near the tenant limit of decision
3 holds the row a completion needs and that a completion waits for every ingest commit queued ahead of it, not one; that
the session and binding checks of decision 5 need the lease's namespace and activity and omitted the session's expiry,
namespace and a replaced token; that at a ceiling the worker stopped before its final batch, so drops could not be
counted (decision 7); that the lock order of decision 11 took locks held to commit first and had a script-marker
exception; and that rules 1, 2 and 10 had no check. Its re-audit added that a purge computed at purge time from the
retention in force deletes retroactively, which ADR-0027 forbids (decision 8). The revision replaces these decisions:

- **3.** Storing a batch MUST NOT take, or wait for, a lock on a row of `runs`, `steps` or `run_events`, and MUST NOT
  take the tenant's `tenant_retained_usage` row before its commit. Over the tenant's allowance, by a read without a lock
  or by the commit-time finalizer's raise, the batch MUST store no line and MUST be counted on its stream. The rule of
  ADR-0027 that keeps what fits applies to the terminal reports it was written for, not to step logs, which are lossy
  (decision 7). The whole ingest runs under a lock timeout of 200 ms, answered as "retry later".
- **5.** A batch MUST be accepted only from a caller that a report would accept short of the run lock: `worker:execute`,
  not the database-runner or runner role (in the token and in the locked membership read), a current membership, an
  activity binding that covers the step's `activity`, a session matched by tenant, id, the lease's namespace, subject and
  current token hash, unexpired and accepted at 1.7 or later, and a lease that is `LEASED` or `RUNNING` for the same run,
  node and instance path, with the same attempt, generation and token, unexpired, on a run neither finished nor
  stopping; the run, namespace, node and path a batch is stored under MUST come from that lease, never from the
  request. The lease is read first, without a
  lock; the binding and membership checks, whose locks are held to commit, are the last statements before the insert.
  Anything else MUST be refused and store nothing, and a caller who fails the binding or membership check MUST learn
  nothing else about the stream.
- **7.** The record is bounded and may lose lines. Ceilings per message, attempt, run and tenant MUST be enforced by the
  server from counters, before the insert, on the text as it will be stored (after redaction and cutting), whatever the
  worker sends; counters MUST be updated from the rows actually inserted. At a ceiling the first lines are kept. Every
  sequence the worker numbered and the server did not store MUST be counted once, as dropped by the worker or by the
  server, from the gaps in the sequences the server received and the highest sequence the final batch declares; a
  ceiling MUST still let the worker send its final batch. Markers MUST NOT carry log-classified text. Meeting a ceiling,
  a refused batch or an unreachable server MUST NOT fail, block, retry or delay a step beyond one bounded final flush.
- **8.** Lines are rows of an ordinary PostgreSQL table, one row per line, classified `log` in the storage classification,
  charged and released by the triggers of ADR-0027, and purged by batched `DELETE` once their stream has expired. A
  stream MUST keep the retention its namespace had when the stream was created, as ADR-0027 requires of every retention
  ("lowering policy does not rewrite stored expiry … or authorize retroactive deletion"); removal MUST begin by marking
  the stream, after which no line of it is served and no batch is stored. The table MUST NOT be partitioned while those
  triggers are how bytes are released.
- **9.** Lines MUST be read in the order (storing transaction id, sequence), and only below the oldest transaction still
  in progress, both computed in the reading statement, so that a page is final and a tail neither skips nor repeats. A
  storing transaction MUST obtain its id before it reads the lease. A read MUST NOT declare a run's log complete until
  the run is finished, `pg_snapshot_xmin` is strictly greater than an id taken by a transaction begun after the run was
  seen finished (the rule of D1958 of feature 018), and the page is empty. The worker's time is shown and MUST NOT order
  lines.
- **11.** Every path that writes step logs MUST take its locks in one order: the stream row, the run's usage row, the
  binding lock, the namespace secrets lock, the membership row, then, at commit, the content policy fence, the log policy
  fence and the tenant usage row. No path takes a stream row after any later lock. No other exception exists (the script
  marker is removed).

Option 7 ("Taking the tenant allowance under the row lock in every batch") now also covers taking it only near the
limit. The paragraph "What is not free" of the Consequences becomes: every step that logs adds transactions beside its
lease and completion, each of which takes the tenant usage row at its commit and fires a deferred trigger call per row;
a completion with output of the same tenant can wait for the ingest commits queued ahead of it on that row (at most the
tenant's tickets on every replica) and, when an exclusive taker of the namespace secrets lock is queued, for the ingests
of that namespace that hold it shared. The spike of design 041 (T041-31) measures both before the schema is fixed; a
failure stops the feature for a design change.

The paragraph "Bad, and stated to the reader" reads "any open transaction of the cluster, prepared ones included" where
it says "a long write transaction anywhere in the database". The Evidence's "spike of T041-6" is the spike of T041-31.

"Enforced by" is replaced, when built, by: in `tests/integration/claim-transition-efficiency/`, `work_budget.rs` passing
with its constants unmodified while ingest runs, the lock-free test and the no-foreign-key test (rules 2, 3, 4); in
`integration_step_logs`, the duplicate, gap, crash, revocation, session, expired-lease and ceiling-concurrency tests
(rules 5, 6, 7), the purge and retention tests (rule 8), the held-transaction read and end tests on PostgreSQL 16 and 18
(rule 9); the storage-classification manifest test and the runtime privilege audit (rule 8); the conformance scenarios
`step-logs-limits` and `step-logs-transport-faults` in six SDKs (rules 6, 7); and a source-text test that the worker and execution modules never name the log tables or
their port (rules 1 and 2, T041-5), the barrier tests with K held ingests and with a queued `lock_secrets` waiter (rule 3,
T041-6), the session and authority cases (rule 5, T041-6), the end test with a held transaction whose id equals the end
horizon on PostgreSQL 16 and 18 (rule 9, T041-12), and a contract test that `/runs/{runId}/stream` emits only `graph`,
`reconnect` and `end` (rule 10, T041-12).

Note (2026-11-16, OD-2026-11-16-7 and OD-2026-11-16-10): on the approval path the owner chose ("Corrigir e aceitar o
resto"), an ingest reads the binding's patterns and the caller's membership without a lock: it takes no binding lock, no
namespace secrets lock and no membership lock, so it never delays a report, a lease, a heartbeat, a secret change or a
managed-database admission, and a batch that commits just after its worker is revoked is still stored. In decision 11,
an ingest therefore takes the stream row, then at commit the fences and the tenant usage row. There is no run ceiling
("Retirar", OD-2026-11-16-10): the attempt ceilings and the tenant's step-log quota bound what a run stores, and no run
usage row exists, so decision 7's ceilings are the attempt's and the tenant's. The script-output marker is written in a
transaction of its own right after the script report commits, and a crash between the two loses that one marker, as
decision 7's lossy record allows. In "Enforced by", the barrier tests are those of design 041's SC-041-001 (one ingest
held after its quota flush, K − 1 queued behind it), and decision 11 is checked by the lock-order test of design 041
(T041-6, T041-10, T041-13, T041-30). The stop rule of decision 5 and the place of the policy fences in decision 11 are
still the owner's question O041-025. Decision 5's "in the locked membership read" and "whose locks are held to commit"
read, for an ingest, as a membership and a binding read without a lock; the Consequences' clause on a queued taker of
the namespace secrets lock no longer applies to an ingest. In decision 9, "and the page is empty" reads "and the read's
scan ended because no row below the horizon was left in the index range it reads, neither the page limit nor the scan
bound having stopped it, whether or not a filter matched" (design 041, D041-084): a filtered read can otherwise declare
the log complete before it has scanned to the end.

Note (2026-11-18, OD-2026-11-16-11): the owner answered O041-025 with (a), "Sim, como os reports" ("yes, as the
reports"). Decision 5's "a run neither finished nor stopping" reads: a batch is accepted when a report of its attempt
would be, short of the run lock: the step is not cancel-requested, the run is not finished, and the run is not stopping
unless the node is a compensation or reconciliation node. One domain function, `report_fence_allows`
(`crates/domain/src/execution/state.rs`), states that rule, and both `attempt_authority_fence` and the ingest's lease
read call it. Decision 11's order starts, for every writer of step logs, with the shared policy fences of the
namespace's step-log resources (tenant then namespace), before the stream row, as ADR-0027 asks of every authoritative
limit read; then the stream row, then at commit the fences of the charged classes and the tenant usage row.

Note (2026-11-19, design 041 T041-31): the spike before the schema ran
(`specs/041-step-logs/validation-report.md`). Workload (a) failed SC-041-007 at both rows, so the condition of this
ADR's "Enforced by" fired: the feature stops for a design change and a new review, asked as O041-036 of
`specs/041-step-logs/research.md`. The completion's wait on the tenant usage row was small on average; whether it
explains the tail is not settled by that run.

Note (2026-11-19, OD-2026-11-19-1): the owner answered O041-036 with (d), "Redesenhar (CompleteStep)". The text above is
unchanged. [ADR-0095](ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md), Proposed, would supersede
decisions 2, 6 and 11 and the session clause of decision 5 for a carry (an attempt's last records ride on its report,
at most five statements in the report's transaction) and refine decisions 3, 7 and 9, once the owner accepts it (O041-038)
after design 041's spike T041-35.

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
