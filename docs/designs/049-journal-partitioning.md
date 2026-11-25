# Partitioning of the journal tables, joined to the retention of feature 020

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

`run_events`, `payloads`, `runs`, `steps` and the audit trail grow without bound: nothing deletes a run, an event or a
payload (Architecture decision 0054: "Nothing is ever deleted today"). Feature 020 designs retention as a verified
archive followed by a purge of each run in bounded chunks. This design gives that retention a cheap path and gives the
hot path smaller tables:

1. **A journal bucket per run.** Every run is given, when it is created, the month (UTC) it was created in, its
   *journal bucket*, stored on the run row and never changed. Everything the run writes to the journal tables goes to
   that bucket's partition, so a run's events and payloads are always in one partition, however long the run lives.
2. **`run_events` is partitioned by bucket**, online: the existing table becomes the *legacy* partition by
   `ATTACH PARTITION`, with no copy.
3. **Payloads that belong to a run move to `run_payloads`**, partitioned by bucket and keyed by the run, so that a
   run's payloads die with it: a purge needs no check of references from other runs, which removes gap 1099 of feature
   020. Payloads that do not belong to a run (schedule and batch inputs) stay in `payloads`.
4. **A bucket is dropped, not deleted,** when every run in it has been archived and verified by feature 020 and is
   eligible under its namespace's policy: one `DETACH PARTITION CONCURRENTLY` and one `DROP TABLE` instead of millions of
   deletes. A run that is still live holds its bucket; the runs around it are purged one by one by feature 020's path,
   which becomes simple because a run's rows are selected by its own key.
5. **The audit trail is partitioned by month** of the record, with no drop by default.
6. **`runs` and `steps` are not partitioned in release 0.3.0**, for the reasons measured or stated in the Technical Plan;
   `steps` is measured again once design 051's queue removes the claim's scan of it.

The cost on the hot path is measured by design 038's statement budget, which gains a count of the relations each lease
and completion locks.

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
  `(tenant_id, digest)`, and shared by every row that holds the same bytes, from 22 columns of 15 tables. Deleting a
  run's payloads needs a check that no other row references them, and an index on each of those 22 columns (gap 1099 of
  feature 020), which adds write cost to the hottest tables. Architecture decision 0054 rejected partition drop in its
  version 1 because "partitions do not align with run lifecycles (a long run spans many), it needs the journal re-keyed
  (gap 241), and it cannot verify an archive per run before dropping".
- **What a partition key must survive.** Run identifiers are not all time-ordered: a run started through the API gets a
  UUID version 7, but a child run's identifier is derived from a SHA-256 of its parent and node
  (`child_run_id`, `crates/application/src/execution.rs`), so a range of run identifiers is not a range of time. An
  event's own time would split a long run across partitions. Every read of `run_events` is by `(tenant_id, run_id)`
  (point reads, pages by `seq`, the latest event of a node); none reads by time or by a global cursor; no foreign key
  points to `run_events`.
- **Partitions cost locks.** With four and eight API replicas the database already spends 4% to 11% of active backends
  on `LWLock:LockManager` (`tests/performance/reports/scalability-ha.md`, the section of 2026-11-12). A statement that
  the planner cannot prune at plan time opens, and locks, every partition of the table. Partitioning that ignores this
  would trade disk for throughput.
- **The checks that read the catalog** (the storage-classification test, `crates/infra-postgres/src/privilege.rs`, the
  runtime-privilege audit) treat every relation of kind `r` as a table; a partition is one, so they would fail or
  mislead on a partitioned schema.

## Requirements

- **Requirement 049-001 The journal bucket.** Every run MUST have a `journal_bucket` (a date: the first day, UTC, of the
  month the run was created in, or the legacy value `1970-01-01` for a run created before the switch of Requirement
  049-010). It MUST be set when the run row is inserted and MUST NOT change afterwards; a trigger MUST refuse a change.
- **Requirement 049-002 One partition per run.** Every row of a partitioned journal table that belongs to a run MUST be
  written with that run's `journal_bucket`, taken from the run row read in the same transaction, never computed again.
  A test MUST fail when any run has rows in two buckets of any partitioned table.
- **Requirement 049-003 `run_events` partitioned.** `run_events` MUST be partitioned by range of `journal_bucket`, one
  partition per month and one legacy partition. Its primary key becomes `(tenant_id, journal_bucket, run_id, seq)`; the
  append-only and no-truncate triggers stay `ENABLE ALWAYS` on every partition; the runtime role keeps `SELECT` and
  `INSERT` on the parent only and has no privilege on a partition.
- **Requirement 049-004 Online.** The partitioning of `run_events` MUST NOT copy its rows and MUST NOT hold a lock that
  blocks inserts for more than the `lock_timeout` of design 048 (2 s) at any step: the existing table becomes the legacy
  partition after a `CHECK` that proves its range is added `NOT VALID` and validated in a later migration.
- **Requirement 049-005 Run payloads.** A payload referenced by a row that belongs to a run MUST be stored in
  `run_payloads`, keyed `(tenant_id, journal_bucket, run_id, digest)` and partitioned like `run_events`. A run MAY share
  a payload between its own rows (deduplication within the run) and MUST NOT reference another run's payload: a child,
  a reset or a rerun that starts from another run's value writes its own copy. Payloads referenced only by rows that do
  not belong to a run (`schedule_revisions.input_digest`, `batch_jobs.payload_digest`) stay in `payloads`.
- **Requirement 049-006 References keep their foreign keys.** Each of the 20 columns that reference a run's payload MUST
  reference `run_payloads` by a foreign key that includes the run and the bucket; its table gains `journal_bucket`
  (`NOT NULL`, default the legacy value, which needs no rewrite). The two shared columns keep their foreign key to
  `payloads`.
- **Requirement 049-007 The move of existing payloads.** Payloads of runs created before the switch MUST be copied into
  `run_payloads` (legacy bucket) by a resumable background job run as the owner role, in chunks of one run, without
  blocking writers. Until the job reports every legacy run copied, every new payload write MUST write both tables; the
  foreign keys move to `run_payloads` only after the job has finished and the instance registry's census shows no
  process of the release before this one (design 048, Requirement 048-034). Rows of `payloads` that no shared column
  references are then deleted by a one-off owner job.
- **Requirement 049-008 Partitions exist ahead.** A definer function, `ensure_journal_buckets()`, owned by the owner
  role, with no argument and a pinned `search_path`, MUST create the partitions of the current month and the next two
  for every partitioned table (as a standalone table attached with `ATTACH PARTITION`, under `lock_timeout`). The
  engine's housekeeping calls it at start and daily. A metric MUST give the months of partitions that exist ahead, and
  an alert MUST fire when fewer than two do. A row for a bucket without a partition fails its transaction with a typed,
  retryable error; nothing writes to a default partition, and there is none.
- **Requirement 049-009 A bucket is dropped only when every run in it may be.** A bucket MAY be retired only when every
  run whose `journal_bucket` is that bucket is purgeable under feature 020 (terminal, its tree terminal, past its
  namespace's hot retention, every sink applied or held past its hold, not `PARTIAL`, not `COMPENSATION_FAILED`, no
  unresolved outcome or review), and its archive is `VERIFIED` (or its policy is `archive: NONE`). Retirement MUST mark
  each such run purged under its run lock first (which refuses any later append), then detach the bucket's partitions
  of every partitioned table concurrently, then check that the detached tables hold no row of a run that is not marked,
  then drop them. A refusal at any step MUST leave every partition attached and every row in place.
- **Requirement 049-010 The switch.** New runs get a dated bucket only after the census shows no process of the
  release before this one: the switch is one owner command that changes the default of `runs.journal_bucket` from the
  legacy value to the month of `clock_timestamp()` in UTC. Before it, every run is legacy and every process writes the
  legacy bucket.
- **Requirement 049-011 Every statement names its bucket.** Every statement that reads or writes a partitioned journal
  table MUST carry the bucket as an equality on `journal_bucket`, so that the planner prunes to one partition. A static
  check over the store sources, like the runtime-privilege audit, MUST fail on a statement that touches a partitioned
  journal table without it.
- **Requirement 049-012 The hot path is budgeted.** Design 038's budget test MUST record, besides statements,
  transactions and rows written, the number of distinct relations a lease and a completion lock, and MUST record it in
  the change that partitions each table. Partitioning MUST NOT change the statement count of a lease or a completion
  once the switch is done, and MUST NOT raise the relations locked by more than the target partition and its indexes
  per partitioned table the operation touches. During the dual write of Requirement 049-007, the budget records the one
  extra statement per payload write as a temporary line that the contract step removes.
- **Requirement 049-013 The audit trail.** `audit_records` MUST be partitioned by month of `created_at`, online by the
  same attach-the-legacy method, with partitions ahead by `ensure_journal_buckets()`. No partition of it is dropped
  unless the operator sets an audit retention in configuration (none by default), and then only partitions entirely
  older than it.
- **Requirement 049-014 The catalog checks know partitions.** The storage-classification test, the startup privilege
  check and the runtime-privilege audit MUST treat a partition as its parent: classification and grants are of the
  parent; a partition with any grant to the runtime role, or with a trigger missing, MUST fail them.
- **Requirement 049-015 Feature 020 uses the bucket.** Feature 020's purge of one run MUST delete that run's rows of the
  partitioned tables by `(tenant_id, journal_bucket, run_id)`, with no check of other references. Its stub of a purged
  run keeps the run row with its digests set to null and `purged_at` set, instead of a tombstone payload (which a
  foreign key to `run_payloads` could not reference).

### Success criteria

- **Success criterion 049-001** After the switch, the budget test's statement counts for a lease and both completions
  equal the counts before partitioning, and the relations locked rise by at most two per partitioned table touched.
- **Success criterion 049-002** On a database holding 12 monthly buckets of 1,000,000 runs of the chain shape each, the
  retirement of one bucket, from the start of the detach to the end of the drop, never takes more than 120 s and is
  reported at the median, and the chain shape's 99th percentile latency, measured by design 051's driver during the
  retirement, rises by less than 10%. The bound: a concurrent detach waits twice for the transactions older than it, and
  no platform transaction outlives `idle_in_transaction_session_timeout` (60 s, `crates/infra-postgres/src/pool.rs`), so
  2 × 60 s = 120 s; a drop of a detached table waits for nothing of the platform's.
- **Success criterion 049-003** The partitioning of a `run_events` of 100 GiB, under the chain shape at half its
  maximum sustained rate, never blocks an insert for more than 2 s, as measured by the driver's per-second counter.
- **Success criterion 049-004** The payload move of Requirement 049-007 can be stopped and restarted at any point and
  ends with every legacy run's payloads present in `run_payloads`, checked by a digest of each run's referenced
  payloads before and after.

### User stories

- **User story 049-01** An operator who keeps 90 days of history sees the database stop growing after 90 days plus one
  month, without a nightly purge that fights the workload.
- **User story 049-02** A tenant whose data must be erased has their runs purged one by one, at a cost proportional to
  their runs, not to everyone's.
- **User story 049-03** A developer adding a statement on the journal learns at once, from a check, that it must name
  its bucket.

## Goals

- Retention that costs a drop per month, not a delete per row.
- A journal whose hot partitions stay the size of a few months, so their indexes stay in memory.
- No change in what a transition costs in statements, and a measured, bounded change in what it locks.

## Non-Goals

- **Retention policy and archival themselves** are feature 020's (its requirements 020-030 to 020-038); this design
  gives them a storage layout and changes only the stub (Requirement 049-015).
- **Partitioning `runs` and `steps` in 0.3.0** (Technical Plan, "What is not partitioned").
- **Moving a live run's rows to another bucket** so that its old bucket can drop. A long-lived run holds its bucket; the
  rest of the bucket is purged run by run.
- **Partitioning the outbox, the inbox, the command deduplication and the webhook deliveries.** They already delete by
  age in small, indexed batches.
- **Changing feature 041's step logs.** Its tables are its own (Architecture decision 0084).

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6 (and of Owner decision 2026-11-24-4 where it
accepts or rejects), with its reasoning.

1. **What is the partition key?** The run's creation month, stored on the run (`journal_bucket`). The run identifier is
   not time-ordered for child runs; the event's time splits runs; a key stored on the run keeps a run in one partition
   and is known to every writer, which holds the run row. Delegate (Owner decision 2026-11-20-6).
2. **Which width?** One calendar month. With hot retention R days, the database holds about `ceil(R / 30) + 3` buckets
   (the retained months, the current one, two ahead); 90 days gives 3 + 3 = 6 and 365 days gives 13 + 3 = 16. A week
   would multiply the partitions, and so the planning and locking per unpruned statement, by four for the same
   retention. Delegate (Owner decision 2026-11-20-6).
3. **Deduplicate payloads within a run, a bucket, or a tenant?** Within a run. Within a bucket would bring back the
   reference check for a single run's purge (erasure, a straggler), which is what makes feature 020's purge expensive.
   The cost is a copy where two runs hold the same bytes; Task 049-2 measures how often that happens in the benchmark
   shapes. Delegate (Owner decision 2026-11-20-6).
4. **Partition `steps`?** Not in 0.3.0. Its partial indexes are read by the scheduler's passes and today by the claim's
   candidate scan without a run identifier, so every partition would be opened by them, and its rows are few per run
   (three for a chain), so the purge by run costs little. Design 051's queue removes the claim's scan; Task 049-16
   measures the passes with partitioned `steps` and records the decision. Delegate (Owner decision 2026-11-20-6).
5. **A default partition as a safety net?** No. A default partition makes every later attach scan it, and rows that
   should fail would land there silently. A missing partition fails loudly and the alert fires two months before.
   Delegate (Owner decision 2026-11-20-6).
6. **The decision record.**
   [Architecture decision 0132](../adr/ADR-0132-the-journal-is-partitioned-by-the-month-a-run-was-created-and-a-run-owns-its-payloads.md)
   records the key, the ownership of payloads and the rule for dropping a bucket, and refines Architecture decision
   0054 (its rejected option 3 becomes the main path, for the reasons above).

## Owner decisions needed

None. The change to feature 020's stub (Requirement 049-015) is a design change inside two 0.3.0 features, decided
under Owner decision 2026-11-20-6.

## Critical User Journeys (CUJs)

### CUJ 1: A month is retired

*   **Given**: namespaces with 90 days of hot retention and archives required; bucket 2026-12 whose runs all ended
    before 2027-01-01 and were archived and verified.
*   **When**: the retention job of feature 020 reaches 2027-04-01.
*   **Then**: it marks the bucket's runs purged, detaches the December partitions of `run_events` and `run_payloads`
    concurrently, checks them, and drops them; inserts never wait more than the lock timeout.

### CUJ 2: A run that lives a year

*   **Given**: one run of bucket 2026-12 still waiting for an approval in 2027-06.
*   **When**: the retention job reaches December's bucket.
*   **Then**: the bucket stays; feature 020 purges every other run of it one by one, by its key; the metric
    `journal_bucket_held_runs` shows one; when the run ends and becomes purgeable, the bucket is retired.

### CUJ 3: The upgrade

*   **Given**: a database of the previous release with 100 GiB of `run_events`.
*   **When**: the release's expand migrations run while it serves.
*   **Then**: `run_events` becomes a partitioned table whose legacy partition is the old table, without a copy; new runs
    stay legacy until the operator runs the switch after the census; the payload move runs in the background.

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
| `payloads` | no | | `(tenant_id, digest)`, for the two shared columns and legacy rows until the move ends |
| `audit_records` | yes, by range of `created_at` | the record's month | `(id, created_at)` |
| `runs` | no | | `id`, with `journal_bucket` and the stub columns |
| `steps` and the other run-owned tables | no | | unchanged, each with `journal_bucket` for its foreign key to `run_payloads` |
| `ready_steps` (design 051) | no | | small and short-lived |

Partition names are `<table>_b<YYYYMM>` and `<table>_legacy`.

### `run_events`, online

The migrations, each in its own file, all with `lock_timeout` 2 s (design 048):

1. *Expand.* `ALTER TABLE run_events ADD COLUMN journal_bucket date NOT NULL DEFAULT '1970-01-01'` (a constant
   default: no rewrite) and `ALTER TABLE runs ADD COLUMN journal_bucket date NOT NULL DEFAULT '1970-01-01'`, plus the
   trigger that refuses a change of `runs.journal_bucket`.
2. *Expand.* `ALTER TABLE run_events ADD CONSTRAINT run_events_legacy_range CHECK (journal_bucket = '1970-01-01') NOT VALID`.
3. *Expand.* `VALIDATE CONSTRAINT run_events_legacy_range` (takes `SHARE UPDATE EXCLUSIVE`, which does not block
   inserts or reads).
4. *Expand, one transaction.* Rename `run_events` to `run_events_legacy`; create the partitioned `run_events` with the
   same columns; attach `run_events_legacy` `FOR VALUES FROM (MINVALUE) TO ('1970-01-02')` (no scan: the validated
   check proves the range); create the partitions of the current month and the next two; move the triggers, the
   foreign keys and the grants to the parent; attach the legacy table's indexes to the parent's. The rename takes an
   `ACCESS EXCLUSIVE` lock for the duration of this short transaction; with `lock_timeout` the migration fails fast
   instead of queueing the hot path behind it, and is run again.

The primary key changes from `(tenant_id, run_id, seq)` to `(tenant_id, journal_bucket, run_id, seq)`. On the legacy
partition the old index is attached as the partition's index only if it has the same columns; it does not, so step 4
is preceded by a `CREATE UNIQUE INDEX CONCURRENTLY` of the new key on the old table (its own migration without a
transaction), which step 4 attaches. Uniqueness of `(tenant_id, run_id, seq)` still holds: a run's rows are all in one
bucket (Requirement 049-002), and the run's version compare-and-set refuses a second append of the same sequence, as
today.

Old binaries keep working: they insert into `run_events` by name without `journal_bucket`, which takes the legacy
default, and every run they see is legacy until the switch, which waits for them to be gone.

### `run_payloads` and the move

1. *Expand.* Create `run_payloads` partitioned like `run_events`, append-only and no-truncate triggers, quota delta and
   flush triggers like `payloads`' (it holds the same class of content), grants `SELECT, INSERT` on the parent only, its
   storage classification. Add `journal_bucket` to the 15 run-owned tables that reference payloads.
2. *Dual write.* Release N's adapter writes each run payload to both `payloads` (so that the existing foreign keys and
   older readers stay satisfied) and `run_payloads`. One extra statement per payload write, recorded as a temporary line
   of the budget.
3. *Move.* `control-plane journal move-payloads`, as the owner role, walks legacy runs by identifier, copies in one
   transaction per run the payloads its rows reference into `run_payloads` (`INSERT ... SELECT ... ON CONFLICT DO
   NOTHING`), and records its position, so that it can stop and resume. It runs at a configurable rate and pauses when
   its own statement takes more than a configurable time; both defaults are set from Task 049-2's measurement of the
   payload volume, so that the move of the measured volume ends within one release.
4. *Contract, the next release.* When the move has finished and the census shows no older process: add the new foreign
   keys `NOT VALID`, validate them, switch the readers (a registry capability), stop the dual write, drop the old
   foreign keys of the 20 run-owned columns. Then the one-off job deletes the rows of `payloads` that neither shared
   column references.

The 20 run-owned columns are `runs.input_digest`, `runs.result_digest`, `run_events.payload_digest`,
`steps.output_digest`, two of `iteration_scopes`, two of `collection_iterations`, `iteration_checkpoints`,
`workflow_operation_states.state_digest`, `operation_update_receipts.result_digest` and `arguments_digest`, two of
`loop_scopes`, two of `loop_iterations`, two of `loop_checkpoints` and two of `chain_segments`; the two shared ones are
`schedule_revisions.input_digest` and `batch_jobs.payload_digest`. 2 + 1 + 1 + 2 + 2 + 1 + 1 + 2 + 2 + 2 + 2 + 2 = 20
and 20 + 2 = 22, the count of feature 020's gap 1099.

### Locks and plans

A statement planned with the bucket's value prunes to one partition at plan time and locks only it and its indexes. A
generic prepared plan prunes at execution and, on the PostgreSQL versions the platform supports, may lock every
partition first. Statements on partitioned journal tables are therefore sent as unnamed statements (sqlx's
`persistent(false)`), planned with their parameters each time. Task 049-3 measures, on PostgreSQL 16 and 18, the
planning time this adds and the relations locked with 1, 6, 16 and 60 partitions, prepared and unprepared, and the
budget test then records the relations locked (Requirement 049-012). If unprepared planning costs more than the locks
it saves, the measurement decides, and this paragraph is amended before the build goes on.

### Retirement

`control-plane journal retire --bucket YYYY-MM`, as the owner role, outside a transaction (so that it can detach
concurrently), called by feature 020's retention job:

1. Read feature 020's state for every run of the bucket (an index on `runs (journal_bucket)` serves it); stop if any is
   not purgeable and archived.
2. Mark each run purged through feature 020's purge function, under its run lock, in chunks; a purged run refuses any
   later append (the run's state is terminal and purged).
3. `ALTER TABLE ... DETACH PARTITION ... CONCURRENTLY` for each partitioned table's partition of the bucket.
4. Check that no row of the detached tables belongs to a run that is not purged; if one does, attach the partition
   again and stop.
5. `DROP TABLE` the detached partitions.

A crash between steps leaves a state the command resumes from: a partition detached and not dropped is found by name.

### What is not partitioned, and why

- **`runs`.** A purged run keeps its stub (feature 020, owner question 1082: kept in version 1), so its rows do not
  leave with a bucket. Its queries by status and by shard read across runs of every age, which would open every
  partition.
- **`steps`.** See Open Question 4. Its rows of a purged run are deleted by feature 020's purge by
  `(tenant_id, run_id)`, a few rows per run.
- **The outbox, the inbox, the deduplication table and the webhook deliveries.** They are deleted by age already, in
  small batches on their own indexes; a drop would need every undelivered row out of the partition first.

### Interactions

- **Feature 032** adds `run_events.region_epoch` (Architecture decision 0067, decision 9). A column added to the
  partitioned parent is added to every partition; whichever design lands second writes its migration against the
  other's schema, and the order is recorded in both task lists.
- **Design 048**'s rules apply to every migration here (phase marks, lock timeouts, concurrent index builds, the
  compatibility table).
- **Design 051**'s queue is not partitioned.
- **Design 041**'s step logs are not touched.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A statement forgets the bucket and scans every partition | The static check of Requirement 049-011; the lock count in the budget |
| A run's rows land in two buckets | The bucket comes only from the run row; the immutability trigger; the test of Requirement 049-002; retirement's check before the drop |
| No partition for next month | Created three months ahead daily; an alert at fewer than two; a missing one fails loudly and retryably |
| The rename of step 4 queues behind a long transaction | `lock_timeout` 2 s; the migration is run again; the operator runbook says when (low load) |
| The payload move doubles payload bytes for a while | The move copies only what legacy rows reference; the one-off job deletes the shared table's unreferenced rows after the contract; the report states the peak |
| Unprepared statements cost planning time | Task 049-3 measures it before the build relies on it |
| A long run holds a month for ever | The rest of the month is purged by run; the held runs are a metric; the bucket is retired when they end |
| Retirement races an append | Runs are marked purged under their run lock before the detach; a purged run refuses appends; the check after the detach reattaches on any surprise |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | The journal stays append-only with a monotonic per-run sequence; a run's rows stay together; retirement happens only after feature 020's verified archive |
| II. Modular domain, ports and adapters | Pass | All changes are in `infra-postgres`, its migrations and the control plane's commands; the bucket is a storage fact, not a domain field |
| III. External effects are at-least-once; sagas compensate | n/a | No effect changes |
| IV. Compatibility is versioned and pinned | Pass with conditions | Expand and contract by design 048's rules; old binaries write the legacy bucket; the switch waits for the census (Complexity Tracking) |
| V. Code, tenant boundary and secrets are isolated | Pass | Every key starts with `tenant_id`; the runtime role has no privilege on a partition; the definer function takes no argument |
| VI. Observability and audit are part of the feature | Pass | Months ahead, held runs, retirement steps and the move's progress as metrics with closed labels; each retirement audited |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | Not touched |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Crash at each retirement step, a missing partition, the move stopped and resumed, an append racing a retirement |
| Delivery Constraints | Pass | PostgreSQL stays the only authority; nothing is dropped that feature 020 has not archived and verified |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| `journal_bucket` on runs and on the run-owned tables | The key must be on the run, known to every writer, and the same for all of a run's rows | The run identifier: child identifiers are hashes; the event's time: splits runs |
| `run_payloads` and a move across two releases | A run must own its payloads for a drop or a purge to need no reference check | Keeping `payloads` shared: feature 020's 22 reference indexes on the hottest tables |
| Unprepared statements on partitioned tables | A generic plan may lock every partition | Prepared statements: lock count grows with retention |
| A dual write during the move | The existing foreign keys and older readers need `payloads` until the contract | A stopped upgrade that copies everything: downtime proportional to the data |

## Alternatives Considered

- **Delete per run only** (feature 020 as drafted). Correct, and the fallback here; at scale it is millions of deletes,
  a bloat that vacuum must catch up with, and 22 reference indexes.
- **Partition by event time.** Splits a long run's events, loses the global uniqueness of a run's sequence, and cannot
  drop a month while any run that wrote to it lives.
- **Partition by hash of the tenant.** Spreads vacuum and indexes, but drops nothing and does not follow retention,
  which is per namespace.
- **A separate database per period** (archive databases). Reads of an old run would cross databases; feature 020's
  archive already serves the old runs.
- **The `pg_partman` extension.** It manages time partitions well, but it is an extension the platform's managed
  PostgreSQL targets may not offer, and the creation rule here is three statements; the definer function is enough.

## Non-Functional Requirements

- **Non-functional requirement 049-001 Hot path.** As Requirement 049-012 and Success criterion 049-001.
- **Non-functional requirement 049-002 Online.** As Requirement 049-004 and Success criterion 049-003.
- **Non-functional requirement 049-003 Security.** The definer functions take no argument from a caller, pin their
  `search_path` and are executable only by the roles named in their migration.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T049-1 Write Architecture decision 0132, the note in feature 020's research register of the stub change, and the layout in `docs/architecture/storage-and-events.md`.
  Refs: Requirement 049-001, Requirement 049-005, Requirement 049-015
- [ ] T049-2 Measure on the benchmark shapes how many payload bytes are shared between runs today, and record it.
  Refs: Requirement 049-005
- [ ] T049-3 Measure planning time and relations locked with 1, 6, 16 and 60 partitions, prepared and unprepared, on PostgreSQL 16 and 18, and record the choice.
  Refs: Requirement 049-011, Requirement 049-012
- [ ] T049-4 Add the relations-locked count to design 038's budget test, at today's values.
  Refs: Requirement 049-012
- [ ] T049-5 Add `journal_bucket` to `runs` with its immutability trigger, and make every writer read it from the run row.
  Refs: Requirement 049-001, Requirement 049-002
- [ ] T049-6 Write the static check that every statement on a partitioned journal table names its bucket, with negative fixtures.
  Refs: Requirement 049-011
- [ ] T049-7 Teach the storage-classification test, the startup privilege check and the runtime-privilege audit about partitions.
  Refs: Requirement 049-014
- [ ] T049-8 Partition `run_events` online by the four migrations, with the migration tests and a measurement under load.
  Refs: Requirement 049-003, Requirement 049-004, Success criterion 049-003
- [ ] T049-9 Add `ensure_journal_buckets()`, its call from housekeeping, the months-ahead metric and alert, and the missing-partition error.
  Refs: Requirement 049-008
- [ ] T049-10 Create `run_payloads`, add `journal_bucket` to the run-owned referencing tables, and dual-write.
  Refs: Requirement 049-005, Requirement 049-006, Requirement 049-007
- [ ] T049-11 Write `control-plane journal move-payloads` with resume, rate and the digest check.
  Refs: Requirement 049-007, Success criterion 049-004
- [ ] T049-12 Write the contract migrations (foreign keys to `run_payloads`, readers switched, dual write stopped) and the one-off cleanup, for the next release.
  Refs: Requirement 049-006, Requirement 049-007
- [ ] T049-13 Write the switch command for dated buckets, gated on the census.
  Refs: Requirement 049-010
- [ ] T049-14 Write `control-plane journal retire` with the crash test at every step and the race with an append.
  Refs: Requirement 049-009, Success criterion 049-002
- [ ] T049-15 Partition `audit_records` online, with the optional audit retention.
  Refs: Requirement 049-013
- [ ] T049-16 Measure the scheduler's passes with `steps` partitioned by bucket after design 051's queue, and record the decision.
  Refs: Requirement 049-012
- [ ] T049-17 Write the retention section of `docs/runbooks/database-recovery.md` and the bucket runbook.
  Refs: User story 049-01

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs `integration_platform_foundation` (migrations: up with data, down refused where it must be, up
again), `runtime_privileges`, the suites rerun with `SAGAT_TEST_AS_RUNTIME_ROLE=1`, design 038's budget target, the
static check of Task 049-6, `cargo clippy --workspace --all-targets --locked -- -D warnings` and the three checkers. Task
049-8 is also run under design 051's driver at half the maximum sustained rate, and its per-second counter is the
evidence of Success criterion 049-003. Task 049-14 has a fault test per step (a crash after the marks, after one
detach, after the check) and a barrier test in which an append and a retirement race; each is seen failing first.
