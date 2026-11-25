# Partitioning of the journal tables, joined to the retention of feature 020

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

`run_events`, `payloads`, `runs` and `steps` grow without bound: nothing deletes a run, an event or a payload
(Architecture decision 0054: "Nothing is ever deleted today"). Feature 020 designs retention as a verified archive of
each run followed by a purge of that run in bounded chunks. This design gives that retention a cheap path where it can
have one, and makes the per-run path cheap everywhere:

1. **A journal bucket per run.** Every run is given, when it is created, the UTC month of its `created_at`, its
   *journal bucket*, stored on the run row and never changed. Everything the run writes to the partitioned tables goes
   to that bucket's partition, so a run's events and payloads are in one partition however long the run lives.
2. **`run_events` is partitioned by bucket**, online: the existing table becomes the *legacy* partition by
   `ATTACH PARTITION`, with no copy and no index build under a lock that blocks the hot path.
3. **Payloads that belong to a run move to `run_payloads`**, partitioned by bucket and keyed by the run, so that a
   run's payloads are its own: purging one run deletes rows selected by its key and needs no check of references from
   other runs (which removes gap 1099 of feature 020 and its 22 reference indexes). Payloads that belong to no run
   (schedule and batch inputs, feature 020's tombstone) stay in `payloads`.
4. **No foreign key points into or out of a partitioned journal table.** The integrity those keys gave is kept by
   triggers that check the same facts on every write and take no lock when a partition is detached or dropped. The
   checks are listed by a registry that also covers the references that never had a foreign key.
5. **Feature 020's per-run purge is the main path; dropping a month is the fast path** when every run in it is purged
   under its namespace's policy: one `DETACH PARTITION CONCURRENTLY` and one `DROP TABLE` per partitioned table instead
   of deleting its journal and payload rows, with the tenants' content quota released from the bytes counted before the
   drop. A month whose runs are not all purgeable stays, and is purged run by run.
6. **`runs`, `steps` and the audit trail are not partitioned in release 0.3.0**, for the reasons in the Technical Plan.

Dropping journal rows, by month or by run, changes an obligation of principle I (the journal is append-only); feature
020 already carries that amendment (its Task T020-061). The partitioning itself needs none, and is built first.

The cost on the hot path is measured by design 038's statement budget, which gains a count of the relations each lease
and completion locks and of its round trips.

**Terms used below.** The *journal* is the append-only record of what happened in each run: `run_events` holds one
row per event with a per-run sequence (`seq`), and *payloads* hold the values (inputs, outputs, event bodies) the
events and the state rows refer to by digest. A *partition* is one physical table of a table partitioned by PostgreSQL;
*pruning* is the planner's choice of the partitions a statement must open. *Feature 020* designs retention: a
namespace's policy says how long a finished run stays (*hot retention*), its archive is written to an object store and
read back to check it (*verified*), and only then is the run *purged*, leaving a small *stub* row whose digests point to
a per-tenant *tombstone* payload. The *run lock* is the row lock on the run that every transaction that appends to it
takes first. *Design 038's budget* is the test that counts the statements, transactions and rows of a worker's lease
and of a completion and fails when they change. The *instance registry* (Architecture decision 0082) records which
processes are live and what they support; its *census* says whether a process of an older release is still running,
and its *activation* switches a capability on only when none is.

## The Problem

From the code on `develop` at `546d5aa5`:

- **Nothing shrinks.** `run_events` and `payloads` are append-only by triggers that refuse `UPDATE`, `DELETE` and
  `TRUNCATE` (`crates/infra-postgres/migrations/20261009000100_execution.up.sql`); `runs`, `steps` and the collection
  tables cannot be deleted either (`leaf_source_no_delete`, migration `20261211000000`). No table is partitioned (the
  only `PARTITION BY` in the code are window functions). Feature 010 recorded it as gap 241; the reports measure a
  suspended run at 2,459 to 3,219 bytes with its indexes (`tests/performance/reports/durable-execution.md`,
  `scalability-ha.md`) and a collection at about 2,335 journal bytes per item (`workflow-primitives.md`).
- **Feature 020's purge is expensive where the data is.** Its purge deletes a run's rows in bounded chunks through a
  definer function (`specs/020-visibility-analytics-archival/data-model.md`). Payloads are content-addressed per tenant,
  `(tenant_id, digest)`, and shared by every row that holds the same bytes: 22 columns of 14 tables reference them by
  foreign key (12 tables owned by a run, 2 not), and `collection_scope_items.item_digest` references them with no foreign
  key at all (migration `20261029000000`; `crates/infra-postgres/src/primitives.rs`). Deleting a run's payloads needs a
  check that no other row references them and an index on each referencing column (gap 1099 of feature 020), which adds
  write cost to the hottest tables. Architecture decision 0054 rejected partition drop because "partitions do not align
  with run lifecycles (a long run spans many), it needs the journal re-keyed (gap 241), and it cannot verify an archive
  per run before dropping".
- **What a partition key must survive.** Run identifiers are not all time-ordered: a run started through the API gets a
  UUID version 7, but a child run's identifier is derived from a SHA-256 of its parent and node (`child_run_id`,
  `crates/application/src/execution.rs`). An event's own time would split a long run across partitions. Every read of
  `run_events` is by `(tenant_id, run_id)`; none reads by time or by a global cursor; no foreign key points to
  `run_events`. A run's input payload is written before its run row (`crates/infra-postgres/src/execution.rs`), and
  `runs.created_at` is set by the application, not by the database.
- **Foreign keys and partitions do not mix cheaply.** A detach of a partition of a referenced table checks every
  referencing table under a `SHARE` lock, which blocks writes to `runs` and `steps` while it scans; dropping a table that
  has a foreign key to `runs` takes `ACCESS EXCLUSIVE` on `runs` to remove the key's triggers; before PostgreSQL 18 a
  foreign key on a partitioned referencing table cannot be added `NOT VALID`. And foreign-key checks run as cached
  internal plans that, once generic, open every partition of the referenced table.
- **Partitions cost locks.** With four and eight API replicas the database already spends 4% to 11% of active backends
  on `LWLock:LockManager` (`tests/performance/reports/scalability-ha.md`, section of 2026-11-12). A statement the
  planner cannot prune at plan time opens, and locks, every partition.
- **Dropping fires no row trigger.** The content quota (Architecture decision 0027) is kept by row triggers; a `DROP
  TABLE` releases nothing, which is why Architecture decision 0084 rejected time partitions for step logs.
- **The checks that read the catalog** (the storage-classification test, `crates/infra-postgres/src/privilege.rs`, the
  runtime-privilege audit) treat every relation of kind `r` as a table; a partition is one.

## Requirements

- **Requirement 049-001 The journal bucket.** Every run MUST have a `journal_bucket`: the first day of the UTC month of
  its `created_at`, or the legacy value `1970-01-01` for a run created before the switch of Requirement 049-010. The
  creating transaction MUST compute it once from the `created_at` it binds and pass it explicitly to every row it writes
  (the input payload, written before the run row, included). A check constraint on `runs` MUST refuse a bucket that is
  neither the legacy value nor the month of `created_at`, and a trigger MUST refuse a change of either.
- **Requirement 049-002 One partition per run, enforced.** Every row of a partitioned journal table MUST carry its
  run's bucket. A row trigger on each partitioned table MUST refuse a row whose `(tenant_id, run_id, journal_bucket)` is
  not a run's; it replaces the foreign key to `runs`. A suite that runs after the switch, with at least two dated
  partitions and a clock the test controls across a month boundary, MUST cover every insert site.
- **Requirement 049-003 `run_events` partitioned.** `run_events` MUST be partitioned by range of `journal_bucket`, one
  partition per month and one legacy partition. Its primary key becomes `(tenant_id, journal_bucket, run_id, seq)`; the
  append-only and no-truncate triggers stay `ENABLE ALWAYS` on every partition (the statement-level truncate trigger is
  created on each partition, since it is not inherited); the runtime role keeps `SELECT` and `INSERT` on the parent
  only and has no privilege on a partition.
- **Requirement 049-004 Online.** The partitioning of `run_events` MUST NOT copy its rows and MUST NOT build an index or
  scan the table while it holds a lock that blocks inserts. Every lock that blocks inserts MUST be both waited for under
  a `lock_timeout` of 2 s (design 048), with the migration retried, and held only for statements that touch the catalog;
  the measured wait plus hold is Success criterion 049-003.
- **Requirement 049-005 Run payloads.** A payload referenced by a row that belongs to a run MUST be stored in
  `run_payloads`, keyed `(tenant_id, journal_bucket, run_id, digest)` and partitioned like `run_events`. A run MAY share
  a payload between its own rows and MUST NOT reference another run's payload: a child, a reset, a rerun or a
  cross-namespace operation that starts from another run's value writes its own copy. Payloads referenced only by rows
  that do not belong to a run (`schedule_revisions.input_digest`, `batch_jobs.payload_digest`, feature 020's tombstone)
  stay in `payloads`.
- **Requirement 049-006 References are checked without foreign keys.** Every column that references a run's payload
  MUST be listed in feature 020's reference registry (`crates/infra-postgres/run-content-references.json`), foreign key
  or not, and MUST be checked on insert and update by a trigger that the payload exists in `run_payloads` for the same
  tenant, bucket and run (or, for a purged run's stub, that it is the tenant's tombstone in `payloads`). Its table gains
  `journal_bucket` (`NOT NULL`, constant default the legacy value, so no rewrite). A migration test MUST fail when a
  digest column of a run-owned table is in neither the registry nor its exclusion list, and when a run-owned table has a
  foreign key to `payloads` other than the two shared columns.
- **Requirement 049-007 The move of existing payloads.** Payloads of runs created before the switch MUST be copied into
  `run_payloads` (legacy bucket) by a resumable job run as the owner role, one run per transaction, after the census
  shows no process of the release before this one, so that no older process writes a payload behind the job's cursor.
  Until the contract step, every new payload write MUST write both tables, and the content quota MUST be charged by
  `payloads` only, as today (`run_payloads` has no quota trigger until then). Before the contract, a check MUST find, for
  every registered reference column of every run, a matching `run_payloads` row.
- **Requirement 049-008 Partitions exist ahead.** A definer function, `ensure_journal_buckets()`, owned by the owner
  role, with no argument, a pinned `search_path` and a transaction-level advisory lock, MUST create the partitions of the
  current month and the next two for every partitioned table, each as a standalone table with its triggers, checked by
  a constraint, then attached, under `lock_timeout`. The engine's housekeeping calls it at start and daily. A metric MUST
  give the months that exist ahead, and an alert MUST fire below two. A row for a bucket with no partition fails its
  transaction with the typed error `JOURNAL_BUCKET_MISSING` (retryable); a row for a retired bucket fails with
  `RUN_PURGED` (not retryable). Nothing writes to a default partition, and there is none.
- **Requirement 049-009 A month is dropped only when every run in it is purged.** A bucket MAY be retired only when
  every run of it is purgeable under feature 020 (terminal, its tree terminal, past its namespace's hot retention, every
  sink applied or held past its hold, not `PARTIAL`, not `COMPENSATION_FAILED`, no unresolved outcome or review) and its
  archive is `VERIFIED` (or its policy is `archive: NONE`). Retirement MUST, in this order: purge each run with feature
  020's purge in its mark-only form (under the run lock: re-check eligibility, write the stub, delete the run's rows of
  the unpartitioned run-owned tables), record per tenant the bytes of the bucket's `run_payloads` partition, detach the
  bucket's partitions concurrently, check that they hold no row of a run that is not purged, release the recorded bytes
  through the quota's accounting function in one audited transaction, and drop them. Each step is recorded in a
  retirement row, so that a crash at any point resumes, including a detach left pending (`DETACH ... FINALIZE`). A
  refusal at any step leaves every partition attached and every row in place.
- **Requirement 049-010 The switch.** New runs get a dated bucket only after the registry's activation of the
  capability `journal-buckets`, which refuses while a live process lacks it and refuses, afterwards, a process that lacks
  it. Before it, every run is legacy. At the switch the legacy defaults of `journal_bucket` are dropped, so that a writer
  that leaves the column out fails instead of writing the legacy bucket.
- **Requirement 049-011 Every statement names its bucket.** Every statement that reads or writes a partitioned journal
  table MUST compare `journal_bucket` by equality with a bind parameter or a literal, so that the planner prunes to one
  partition. A static check over the store sources and the function bodies of the migrations MUST fail on a statement
  that touches a partitioned journal table without it.
- **Requirement 049-012 The hot path is budgeted.** Design 038's budget test MUST record, besides statements,
  transactions and rows written, the distinct relations locked during a lease and a completion (from `pg_locks` at
  commit, triggers' internal statements included) and the round trips, in the change that partitions each table.
  Partitioning MUST NOT change the statement count of a lease or a completion once the switch is done, and MUST NOT
  raise the relations locked by more than one partition and its indexes per partitioned table the operation writes.
  During the dual write of Requirement 049-007, the budget records the extra statement per payload write as a temporary
  line that the contract step removes.
- **Requirement 049-013 The catalog checks know partitions.** The storage-classification test, the startup privilege
  check and the runtime-privilege audit MUST treat a partition as its parent: classification and grants are of the
  parent; a partition with any grant to the runtime role, or with a trigger that is missing or not enabled always, MUST
  fail them.
- **Requirement 049-014 Feature 020 uses the bucket.** Feature 020's purge of one run MUST delete that run's rows of
  the partitioned tables by `(tenant_id, journal_bucket, run_id)`, with no check of references from other runs. Its stub
  is unchanged: the digests of a purged run point to the tenant's tombstone in `payloads`, and the run row keeps its
  `NOT NULL` columns, so that an older reader reads a stub as feature 020 designed.
- **Requirement 049-015 Features that will reference payloads follow the rule.** The approved designs that plan new
  references to run content (feature 022: `test_run_pins.payload_digest` and the step cache's obligations; feature 026:
  `approval_requests.summary_digest`, `approval_decision_receipts.input_digest`; feature 030: the inputs and results of
  `service_operations`, each run with its own copy; feature 025: transcript bodies) MUST reference `run_payloads` under
  Requirements 049-005 and 049-006; their data models are amended with this design's delivery.

### Success criteria

- **Success criterion 049-001** After the switch, the budget test's statement counts for a lease and both completions
  equal the counts before partitioning, and the relations locked rise by at most one partition and its indexes per
  partitioned table written.
- **Success criterion 049-002** On a database holding 12 monthly buckets of 1,000,000 runs of the chain shape each, the
  retirement of one bucket is measured end to end (the purge of its runs included) and from the start of the detach to
  the end of the drop; both are reported at the median and the maximum of five repetitions, and the chain shape's 99th
  percentile latency, measured by design 051's driver during the retirement, rises by less than 10%. The target for the
  detach-to-drop part is at most 120 s: a concurrent detach waits twice for the transactions older than it, and design
  048's reference topology bounds every platform transaction at 60 s (`transaction_timeout`, PostgreSQL 17 and later), so
  2 × 60 s = 120 s; on PostgreSQL 16 the maximum is reported without a bound.
- **Success criterion 049-003** The partitioning of a `run_events` of 100 GiB, under the chain shape at half its maximum
  sustained rate, blocks no insert for more than 2 s of wait plus the measured hold of its catalog statements, as shown
  by the driver's per-second counter and by the migration's own timing of each lock.
- **Success criterion 049-004** The payload move can be stopped and restarted at any point and ends with every
  registered reference of every legacy run matched by a `run_payloads` row, checked by a digest of each run's referenced
  payloads before and after.
- **Success criterion 049-005** After a retirement, every affected tenant's content usage equals a fresh reconciliation
  of its rows.

### User stories

- **User story 049-01** An operator whose namespaces all keep 90 days of history sees the journal stop growing after 90
  days plus one month, without a nightly purge that fights the workload.
- **User story 049-02** A tenant whose data must be erased has its runs purged one by one, at a cost proportional to its
  runs, not to everyone's.
- **User story 049-03** A developer adding a statement on the journal learns at once, from a check, that it must name
  its bucket.

## Goals

- A per-run purge whose cost is a run's own rows, with no cross-run reference check.
- Retention that costs a drop per month where every namespace in the database has a policy no longer than the month's
  age; elsewhere, the per-run purge.
- A journal whose hot partitions stay the size of a few months, so their indexes stay in memory.
- No change in what a transition costs in statements, and a measured, bounded change in what it locks.

## Non-Goals

- **Retention policy and archival themselves** are feature 020's (its requirements 020-030 to 020-038); this design gives
  them a storage layout and the drop path.
- **Partitioning `runs`, `steps` and the audit trail in 0.3.0** (Technical Plan, "What is not partitioned").
- **Dropping audit records.** No audit retention exists; deleting audit evidence would be a new, irreversible decision of
  the owner's, not this design's.
- **Moving a live run's rows to another bucket** so that its old month can drop. A long-lived run holds its month; the
  rest of the month is purged run by run.
- **Partitioning the outbox, the inbox, the command deduplication and the webhook deliveries.** They already delete by
  age in small, indexed batches.
- **Changing feature 041's step logs.** Their tables are their own (Architecture decision 0084).

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6, with its reasoning; the acceptance of the
decision record is under Owner decision 2026-11-24-4 (pull request 274).

1. **What is the partition key?** The UTC month of the run's `created_at`, stored on the run (`journal_bucket`). The run
   identifier is not time-ordered for child runs; the event's time splits runs; a key stored on the run and computed once
   by the creating transaction keeps a run in one partition and is known to every writer. Delegate (Owner decision
   2026-11-20-6).
2. **Which width?** One calendar month. With hot retention R days, the database holds about `ceil(R / 30) + 3` buckets
   (the retained months, the current one, two ahead); 90 days gives 3 + 3 = 6 and 365 days gives 13 + 3 = 16. A week
   would multiply the partitions, and the planning and locking of any statement that is not pruned, by four for the same
   retention. Delegate (Owner decision 2026-11-20-6).
3. **Deduplicate payloads within a run, a bucket, or a tenant?** Within a run. Within a bucket would bring the reference
   check back for a single run's purge (erasure, a long-lived run's neighbours), which is what makes feature 020's purge
   expensive. The cost is a copy where two runs hold the same bytes, which the tenant's content quota counts; Task 049-2
   measures how often that happens in the benchmark shapes. Delegate (Owner decision 2026-11-20-6).
4. **Foreign keys or triggers?** Triggers. A foreign key into a partitioned table makes every detach scan the
   referencing tables under a lock that blocks their writes, and a foreign key out of a partition makes its drop lock
   `runs`; the same existence checks as row triggers take no lock at a detach or a drop. The cost is a trigger call per
   write instead of the built-in check, which Task 049-3 measures against the checks it replaces. Delegate (Owner decision
   2026-11-20-6).
5. **Partition `steps`?** Not in 0.3.0. Its partial indexes are read by the scheduler's passes, and today by the claim's
   candidate scan, without a run identifier, so every partition would be opened by them, and its rows are few per run
   (three for a chain), so the purge by run costs little. Design 051's queue removes the claim's scan; Task 049-16
   measures the passes with partitioned `steps` and records the decision. Delegate (Owner decision 2026-11-20-6).
6. **A default partition as a safety net?** No. A default partition makes every later attach scan it, and rows that
   should fail would land there silently. A missing partition fails loudly and the alert fires two months before.
   Delegate (Owner decision 2026-11-20-6).
7. **How are statements planned so that only one partition is locked?** Decided by Task 049-3's measurement, among
   three: statements on partitioned tables sent unnamed (planned with their parameters, one more round trip each), the
   runtime role's `plan_cache_mode` set to `force_custom_plan` (planning every statement), or generic plans where the
   PostgreSQL version locks only the partitions left after initial pruning. The measurement counts locks, round trips and
   latency, triggers' internal statements included, on PostgreSQL 16 and 18; this design is amended with the choice
   before Task 049-8 starts. Delegate (Owner decision 2026-11-20-6).
8. **The decision record.**
   [Architecture decision 0132](../adr/ADR-0132-the-journal-is-partitioned-by-the-month-a-run-was-created-and-a-run-owns-its-payloads.md)
   records the key, the ownership of payloads, the triggers in place of foreign keys and the rule for dropping a month;
   it refines Architecture decision 0054 in part (its decisions 4 and 6: a second purge path for whole months, and the
   registry as the source of references) and reverses its rejected option 3 for whole months only.

## Owner decisions needed

- **The amendment of principle I that feature 020 carries** (its Task T020-061, the text in Architecture decision 0054,
  "Required amendment of principle I"): dropping a month and the payload cleanup delete journal rows, as feature 020's
  purge does. Recommendation: fold it into the constitution 5.0.0 amendment of Owner decision 2026-11-24-2, so that
  release 0.3.0 can retire data at all. Gates Tasks 049-12 (its cleanup) and 049-14.

## Critical User Journeys (CUJs)

### CUJ 1: A month is retired

*   **Given**: every namespace with 90 days of hot retention and archives required; bucket 2026-12, whose runs all ended
    before 2027-01-01 and were archived and verified.
*   **When**: the operator's retirement job runs after 2027-04-01.
*   **Then**: it purges each run of the bucket in its mark-only form (stub, unpartitioned rows), counts the bucket's
    payload bytes per tenant, detaches the December partitions of `run_events` and `run_payloads` concurrently, checks
    them, releases the counted bytes, and drops them; the hot path never waits more than a lock timeout.

### CUJ 2: A run that lives a year

*   **Given**: one run of bucket 2026-12 still waiting for an approval in 2027-06.
*   **When**: the retirement job reaches December's bucket.
*   **Then**: the bucket stays; feature 020 purges every other run of it one by one, by its key; the metric
    `journal_bucket_held_runs{reason="LIVE"}` shows one; when the run ends and becomes purgeable, the bucket is retired.

### CUJ 3: The upgrade

*   **Given**: a database of the previous release with 100 GiB of `run_events`.
*   **When**: the release's expand migrations run while it serves.
*   **Then**: `run_events` becomes a partitioned table whose legacy partition is the old table, without a copy or an index
    build under a blocking lock; new runs stay legacy until the operator activates `journal-buckets` after the census;
    the payload move runs in the background after the census.

### CUJ 4: A statement without its bucket

*   **Given**: a change that adds `SELECT ... FROM run_events WHERE tenant_id = $1 AND run_id = $2`.
*   **When**: its pull request runs the static check.
*   **Then**: the check fails and names the statement; the author adds `AND journal_bucket = $3`.

## Technical Plan (Architecture)

### Layout

| Table | Partitioned | Key | Primary key |
|---|---|---|---|
| `run_events` | yes, by range of `journal_bucket` | the run's month | `(tenant_id, journal_bucket, run_id, seq)` |
| `run_payloads` (new) | yes, by range of `journal_bucket` | the run's month | `(tenant_id, journal_bucket, run_id, digest)` |
| `payloads` | no | | `(tenant_id, digest)`: the two shared columns, the tombstones, and legacy rows until the cleanup |
| `runs` | no | | `id`, with `journal_bucket` and feature 020's stub columns |
| `steps` and the other run-owned tables | no | | unchanged, each with `journal_bucket` for its reference check |
| `audit_records` | no | | unchanged |
| `ready_steps` (design 051) | no | | small and short-lived |

Partition names are `<table>_b<YYYYMM>` and `<table>_legacy`.

### `run_events`, online

The migrations, each in its own file, all with `lock_timeout` 2 s and retried by `migrate` (design 048):

1. *Expand.* `ALTER TABLE runs ADD COLUMN journal_bucket date NOT NULL DEFAULT '1970-01-01'` and the same on
   `run_events` (a constant default: no rewrite), the check constraint and the immutability trigger of Requirement
   049-001 on `runs` (`NOT VALID`, validated in step 3), and the bucket trigger of Requirement 049-002 on `run_events`.
2. *Expand, without a transaction.* `CREATE UNIQUE INDEX CONCURRENTLY run_events_bucket_key ON run_events (tenant_id,
   journal_bucket, run_id, seq)`, preceded by `DROP INDEX CONCURRENTLY IF EXISTS` of the same name, so that a build
   stopped by a timeout is not left invalid.
3. *Expand.* `ALTER TABLE run_events DROP CONSTRAINT run_events_pkey, ADD CONSTRAINT run_events_pkey PRIMARY KEY USING
   INDEX run_events_bucket_key` (no scan: `journal_bucket` is already `NOT NULL`); add `CHECK (journal_bucket =
   '1970-01-01') NOT VALID`; validate both checks (`SHARE UPDATE EXCLUSIVE`, which blocks no insert or read).
4. *Expand, one short transaction.* Rename `run_events` to `run_events_legacy`; create the partitioned `run_events` with
   the same columns and the same primary key; attach `run_events_legacy` `FOR VALUES FROM (MINVALUE) TO ('1970-01-02')`
   (no scan: the validated check proves the range; no index build: the primary key already backs a constraint); create
   the current month's and the next two months' partitions; put the triggers and the grants on the parent. The legacy
   partition keeps its old foreign keys to `runs` and `payloads` on itself; the parent has none. Only catalog statements
   run under the `ACCESS EXCLUSIVE` lock of the rename.

The rename is the one exception to design 048's migration lint (Requirement 048-032), recorded in the lint's allow-list
with this design as its reason. Old binaries keep working: they insert into `run_events` by name; until the switch every
run is legacy and the default fills the column.

### `run_payloads` and the move

1. *Expand.* Create `run_payloads` partitioned like `run_events`, with the append-only, no-truncate and bucket triggers,
   grants `SELECT, INSERT` on the parent only, and its storage classification; no quota trigger yet. Add `journal_bucket`
   to the 10 run-owned referencing tables besides `runs` and `run_events`, and the reference triggers of Requirement
   049-006 to every registered column, which accept a legacy run's reference found in `payloads` until the contract.
2. *Dual write.* Release N's adapter writes each run payload to both tables, with the bucket the creating transaction
   computed; one extra statement per payload write, a temporary line of the budget.
3. *Move*, after the census shows no process of the previous release: `control-plane journal move-payloads`, as the
   owner role, walks legacy runs by identifier, copies in one transaction per run the payloads its registered references
   name into `run_payloads` (`INSERT ... SELECT ... ON CONFLICT DO NOTHING`), and records its position. Its rate and its
   pause threshold are settings, with defaults set from Task 049-2's measurement so that the measured volume moves
   within one release. It ends with the check of Requirement 049-007 over every registered column of every run.
4. *Contract, the next release*, when the census shows no process of release N: the reference triggers stop accepting
   `payloads` for run-owned columns, the readers read `run_payloads` only (a registry capability), the dual write stops,
   `run_payloads` gets its quota triggers and the content usage of each tenant is reconciled in the same owner
   transaction, and the legacy run payloads of `payloads` are deleted under feature 020's purge authorization (a private
   authorization row that the append-only trigger consults; the trigger is never disabled).

The 20 run-owned columns with a foreign key are `runs.input_digest`, `runs.result_digest`, `run_events.payload_digest`,
`steps.output_digest`, two of `iteration_scopes`, two of `collection_iterations`, `iteration_checkpoints`,
`workflow_operation_states.state_digest`, `operation_update_receipts.result_digest` and `arguments_digest`, two of
`loop_scopes`, two of `loop_iterations`, two of `loop_checkpoints` and two of `chain_segments`: 2 + 1 + 1 + 2 + 2 + 1 +
1 + 2 + 2 + 2 + 2 + 2 = 20 in 12 tables. The registry adds `collection_scope_items.item_digest`, which has no foreign
key, for 21 checked columns. The two shared ones are `schedule_revisions.input_digest` and `batch_jobs.payload_digest`:
20 + 2 = 22 foreign-key columns in 14 tables, the count of feature 020's gap 1099.

### Locks and plans

A statement planned with the bucket's value prunes to one partition and locks it and its indexes. How plans are made is
Open Question 7, decided by Task 049-3 before the build of `run_events` relies on it. The relations a transaction locks,
triggers' statements included, are counted by the budget (Requirement 049-012), so whichever choice is made, a
regression fails a test.

### Retirement

`control-plane journal retire --bucket YYYY-MM`, as the owner role, run by the operator (a scheduled job of the chart,
with an owner credential in an operator Secret that no platform pod mounts), never by the platform's processes. It reads
feature 020's job state, runs outside a transaction where a step needs it, sets `statement_timeout = 0` for its session
and a `lock_timeout` for each lock it takes, and records every step in `journal_retirements` (a platform-level table
with its audit, as `platform_requirement_history` is for the registry):

1. Check every run of the bucket (an index on `runs (journal_bucket)` serves it); stop if any is not purgeable, and count
   the held runs by reason (`NO_POLICY`, `NOT_ELIGIBLE`, `NOT_VERIFIED`, `LIVE`) in `journal_bucket_held_runs`.
2. Purge each run in feature 020's mark-only form, in chunks, each under its run lock: re-check eligibility, write the
   stub (tombstone digests in `payloads`), delete the run's rows of the unpartitioned run-owned tables. A purged run
   refuses any later append.
3. Count the bytes of the bucket's `run_payloads` partition per tenant and record them.
4. `ALTER TABLE ... DETACH PARTITION ... CONCURRENTLY` for each partitioned table's partition of the bucket; on resume, a
   detach left pending is finished with `DETACH PARTITION ... FINALIZE`.
5. Check that no row of the detached tables belongs to a run that is not purged; if one does, attach the partition
   again and stop.
6. Release the recorded bytes per tenant through the content quota's accounting function, in one transaction with the
   retirement's audit record, and mark the step done.
7. `DROP TABLE` the detached partitions. They have no foreign key, so the drop locks nothing of the platform's.

The legacy partition is retired the same way when every legacy run is purged; it alone still has foreign keys, which
step 7 drops first under `lock_timeout` with a retry, since removing a foreign key to `runs` locks `runs` briefly.

### What is not partitioned, and why

- **`runs`.** A purged run keeps its stub (feature 020, owner question 1082), so its rows do not leave with a bucket. Its
  queries by status and by shard read across runs of every age, which would open every partition.
- **`steps`.** Open Question 5. Its rows of a purged run are deleted by feature 020's purge by `(tenant_id, run_id)`, a
  few rows per run.
- **`audit_records`.** No audit record may be dropped (Non-Goals), and its key `(id)` would have to become
  `(id, created_at)`, losing the uniqueness of `id` that `crates/infra-postgres/src/chain_progress.rs` relies on.
- **The outbox, the inbox, the deduplication table and the webhook deliveries.** They are deleted by age already, in
  small batches on their own indexes.

### Interactions

- **Feature 020** builds the purge, the mark-only form, the registry and the purge authorization; this design uses them
  and orders its Tasks 049-12 and 049-14 after feature 020's Tasks T020-045 and T020-046.
- **Feature 032** adds `run_events.region_epoch` (Architecture decision 0067, decision 9). A column added to the
  partitioned parent is added to every partition; whichever design lands second writes its migration against the
  other's schema; `ensure_journal_buckets()` runs only in the active region.
- **Design 048**'s rules apply to every migration here, with the one recorded exception of step 4's rename.
- **Design 051**'s queue is not partitioned.
- **Design 041**'s step logs are not touched.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A statement forgets the bucket and opens every partition | The static check of Requirement 049-011; the lock count in the budget |
| A run's rows land in two buckets | The bucket is computed once and bound; the bucket trigger refuses a mismatch; the legacy defaults are dropped at the switch; the suite after the switch with a controlled clock |
| A reference points to a payload that is not there | The reference triggers on every registered column; the registry test; the move's final check |
| No partition for next month | Created three months ahead daily; an alert below two; a missing one fails loudly and retryably |
| A trigger per write costs more than the foreign-key check it replaces | Task 049-3 measures both; the budget records latency and round trips |
| The rename queues behind a long transaction | `lock_timeout` 2 s and a retry; only catalog statements under the lock |
| The payload move doubles payload bytes for a while | It copies only what registered references name; the quota is charged once until the contract; the cleanup deletes the legacy rows; the report states the peak |
| A long run holds a month | The rest of the month is purged by run; held runs are a metric with a reason; the month is retired when they end |
| Retirement races an append | Runs are purged under their run lock before the detach; a purged run refuses appends; the check after the detach reattaches on any surprise |
| A drop does not release the tenant's quota | Bytes counted before the detach and released in an audited transaction; Success criterion 049-005 |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass with conditions | The partitioning keeps the journal append-only with a monotonic per-run sequence, a run's rows together, and the bucket enforced by trigger; dropping a month and the payload cleanup delete journal rows, so they wait for the amendment of principle I that feature 020 carries (Owner decisions needed; Complexity Tracking) |
| II. Modular domain, ports and adapters | Pass | All changes are in `infra-postgres`, its migrations and the control plane's commands; the bucket is a storage fact, not a domain field |
| III. External effects are at-least-once; sagas compensate | n/a | No effect changes |
| IV. Compatibility is versioned and pinned | Pass | Expand and contract by design 048's rules; old binaries write the legacy bucket; the switch is a registry activation that refuses older processes |
| V. Code, tenant boundary and secrets are isolated | Pass | Every key starts with `tenant_id`; the runtime role has no privilege on a partition; the definer function takes no argument; retirement's owner credential is in no platform pod |
| VI. Observability and audit are part of the feature | Pass | Months ahead, held runs by reason, retirement steps and the move's progress as metrics with closed labels; each retirement audited in a platform-level table |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | Not touched |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Crash at each retirement step and inside the detach, a missing partition, the move stopped and resumed, an append racing a retirement, a suite after the switch across a month boundary |
| Delivery Constraints | Pass | PostgreSQL stays the only authority; nothing is dropped that feature 020 has not archived and verified |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| `journal_bucket` on runs and on the run-owned tables | The key must be on the run, known to every writer, and the same for all of a run's rows | The run identifier: child identifiers are hashes; the event's time: splits runs |
| `run_payloads` and a move across two releases | A run must own its payloads for a single run's purge, or a month's drop, to need no reference check | Keeping `payloads` shared: feature 020's 22 reference indexes on the hottest tables |
| Triggers in place of foreign keys on the partitioned tables | Foreign keys make detaches scan and lock the referencing tables and drops lock `runs` | Foreign keys: the drop path would block the hot path |
| A deletion of journal rows by month | Retention at the size of a month's data | Waiting for the amendment before the partitioning: the partitioning is useful to the per-run purge and the hot path without it, so only the deleting tasks wait |

## Alternatives Considered

- **Delete per run only** (feature 020 as drafted). Correct, and the main path here, made cheaper by run-owned payloads;
  at scale the drop path avoids millions of deletes and the bloat vacuum must catch up with.
- **Partition by event time.** Splits a long run's events, loses the uniqueness of a run's sequence, and cannot drop a
  month while any run that wrote to it lives.
- **Partition by hash of the tenant.** Spreads vacuum and indexes, drops nothing, and does not follow retention, which
  is per namespace.
- **Foreign keys to `run_payloads` from the 21 columns.** Integrity by the database's own mechanism, at the price of a
  `SHARE` scan of every referencing table at each detach.
- **A separate database per period** (archive databases). Reads of an old run would cross databases; feature 020's
  archive already serves the old runs.
- **The `pg_partman` extension.** It manages time partitions well, but it is an extension the managed PostgreSQL targets
  may not offer, and the creation rule here is a few statements; the definer function is enough.

## Non-Functional Requirements

- **Non-functional requirement 049-001 Hot path.** As Requirement 049-012 and Success criterion 049-001.
- **Non-functional requirement 049-002 Online.** As Requirement 049-004 and Success criterion 049-003.
- **Non-functional requirement 049-003 Security.** The definer functions take no argument from a caller, pin their
  `search_path` and are executable only by the roles named in their migration; the retirement command's owner
  credential lives in an operator Secret that no platform pod mounts.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T049-1 Write Architecture decision 0132, the refinement note in Architecture decision 0054, the note in feature 020's research register, and the layout in `docs/architecture/storage-and-events.md`.
  Refs: Requirement 049-001, Requirement 049-005, Requirement 049-014
- [ ] T049-2 Measure on the benchmark shapes how many payload bytes are shared between runs today, and record it.
  Refs: Requirement 049-005
- [ ] T049-3 Measure locks, round trips, planning time and latency with 1, 6, 16 and 60 partitions for the three planning choices of Open Question 7, triggers against foreign-key checks, and the locks of attach and detach, on PostgreSQL 16 and 18; record the choice and amend this design.
  Refs: Requirement 049-006, Requirement 049-011, Requirement 049-012
- [ ] T049-4 Add the relations-locked and round-trip counts to design 038's budget test, at today's values.
  Refs: Requirement 049-012
- [ ] T049-5 Add `journal_bucket` to `runs` with its check and immutability trigger, compute it once per creating transaction from a clock tests control, and bind it to every write.
  Refs: Requirement 049-001
- [ ] T049-6 Write the static check that every statement on a partitioned journal table compares its bucket to a parameter or a literal, over the store sources and the migrations' function bodies, with negative fixtures.
  Refs: Requirement 049-011
- [ ] T049-7 Teach the storage-classification test, the startup privilege check and the runtime-privilege audit about partitions.
  Refs: Requirement 049-013
- [ ] T049-8 Partition `run_events` online by the four migrations, with the bucket trigger, the migration tests (indexes unchanged by step 4) and a measurement under load.
  Refs: Requirement 049-002, Requirement 049-003, Requirement 049-004, Success criterion 049-003
- [ ] T049-9 Add `ensure_journal_buckets()`, its call from housekeeping, the months-ahead metric and alert, and the two typed errors.
  Refs: Requirement 049-008
- [ ] T049-10 Create `run_payloads`, add `journal_bucket` and the reference triggers to the registered columns, the registry test, and the dual write.
  Refs: Requirement 049-005, Requirement 049-006, Requirement 049-007
- [ ] T049-11 Write `control-plane journal move-payloads` with the census gate, resume, rate and the final reference check.
  Refs: Requirement 049-007, Success criterion 049-004
- [ ] T049-12 Write the contract step (triggers narrowed, readers switched, dual write stopped, quota reconciled) and the authorized cleanup of legacy payloads, for the next release.
  Refs: Requirement 049-006, Requirement 049-007
  Owner-gate: pending, the amendment of principle I that feature 020's Task T020-061 carries (Owner decisions needed)
- [ ] T049-13 Write the switch as the registry's activation of `journal-buckets`, dropping the legacy defaults, with a suite that runs after it across a month boundary with two dated partitions.
  Refs: Requirement 049-002, Requirement 049-010
- [ ] T049-14 Write `control-plane journal retire` and its scheduled job, with the crash test at every step and inside the detach, the race with an append, the quota release and its reconciliation test, on the post-contract schema.
  Refs: Requirement 049-009, Success criterion 049-002, Success criterion 049-005
  Owner-gate: pending, the amendment of principle I that feature 020's Task T020-061 carries (Owner decisions needed)
- [ ] T049-15 Amend the data models of features 022, 026, 030 and 025 so that their new run-content references follow Requirements 049-005 and 049-006.
  Refs: Requirement 049-015
- [ ] T049-16 Measure the scheduler's passes with `steps` partitioned by bucket after design 051's queue, and record the decision.
  Refs: Requirement 049-012
- [ ] T049-17 Write the retention section of `docs/runbooks/database-recovery.md` and the bucket runbook.
  Refs: User story 049-01

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs `integration_platform_foundation` (migrations: up with data, down refused where it must be, up
again), `runtime_privileges`, the suites rerun with `SAGAT_TEST_AS_RUNTIME_ROLE=1`, design 038's budget target, the
static check of Task 049-6, `cargo clippy --workspace --all-targets --locked -- -D warnings` and the three checkers.
These are added and seen failing first: a suite that runs after the switch, with at least two dated partitions and a
clock the test moves across a month boundary (Task 049-13); a migration test on a table large enough to show an index
build, asserting that step 4 builds none (Task 049-8); a registry test with an unregistered digest column (Task 049-10);
fault tests at every retirement step and inside a concurrent detach cancelled while it waits (Task 049-14); a barrier
test in which an append and a retirement race; and the quota reconciliation after a retirement. Task 049-8 is also run
under design 051's driver at half the maximum sustained rate, and its per-second counter is the evidence of Success
criterion 049-003.
