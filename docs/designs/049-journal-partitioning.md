# Partitioning of the journal tables, joined to the retention of feature 020

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

`run_events`, `payloads`, `runs` and `steps` grow without bound: nothing deletes a run, an event or a payload
(Architecture decision 0054: "Nothing is ever deleted today"). Feature 020 designs retention as a verified archive of
each run followed by a purge of that run in bounded chunks. This design gives that retention a cheap path where it can
have one, and makes the per-run path cheap for every run created after it:

1. **A journal bucket per run.** Every run is given, when it is created, the UTC month of its `created_at`, its
   *journal bucket*, stored on the run row and never changed. Runs created before the switch keep the *legacy* bucket.
2. **`run_events` is partitioned by bucket**, online: the existing table becomes the legacy partition by
   `ATTACH PARTITION`, with no copy and no index build under a lock that blocks the hot path.
3. **Run trees created after the switch own their payloads** in a new `run_payloads`, partitioned by bucket and keyed by
   the tree's root (a child run takes its root's bucket): purging a tree deletes rows selected by its key, with no check
   of references from other trees. Legacy runs keep their payloads in `payloads` as today, so nothing is copied; their
   purge keeps feature 020's reference check, served by partial indexes of legacy rows delivered with that purge.
4. **No foreign key points into or out of a partitioned journal table.** The integrity those keys gave is kept by
   triggers that check the same facts on every write, raise the same SQLSTATE, and take no lock when a partition is
   detached or dropped. A registry lists every reference to run content, foreign key or not.
5. **Feature 020's per-run purge is the main path; dropping a month is the fast path** when every run of that month is
   purgeable: one `DETACH PARTITION CONCURRENTLY` and one `DROP TABLE` per partitioned table instead of deleting the
   month's journal and payload rows, with the tenants' content quota released from the bytes counted in the detached
   tables.
6. **`runs`, `steps` and the audit trail are not partitioned in release 0.3.0**, for the reasons in the Technical Plan.

Deleting journal rows, by month or by run, changes an obligation of principle I (the journal is append-only); that
amendment is constitution 5.1.0, pending (Owner decision 2026-11-24-511). The partitioning needs none and is built first;
only the deleting tasks wait for it.

The cost on the hot path is measured by design 038's statement budget, which gains a count of the relations each lease
and completion locks and of its round trips.

**Terms used below.** The *journal* is the append-only record of what happened in each run: `run_events` holds one row
per event with a per-run sequence (`seq`), and *payloads* hold the values (inputs, outputs, event bodies) the events and
the state rows refer to by digest. A *partition* is one physical table of a table partitioned by PostgreSQL; *pruning*
is the planner's choice of the partitions a statement must open. *Feature 020* designs retention: a namespace's policy
says how long a finished run stays (*hot retention*), its archive is written to an object store and read back to check
it (*verified*), and only then is the run *purged*, leaving a small *stub* row whose digests point to a per-tenant
*tombstone* payload. The *run lock* is the row lock on the run that every transaction that appends to it takes first.
*Design 038's budget* is the test that counts the statements, transactions and rows of a worker's lease and of a
completion and fails when they change. The *instance registry* (Architecture decision 0082) records which processes are
live and what they support; its *activation* switches a capability on only when no live process lacks it, and refuses
such a process afterwards.

## The Problem

From the code on `develop` at `546d5aa5`:

- **Nothing shrinks.** `run_events` and `payloads` are append-only by triggers that refuse `UPDATE`, `DELETE` and
  `TRUNCATE` (`crates/infra-postgres/migrations/20261009000100_execution.up.sql`); `runs`, `steps` and the collection
  tables cannot be deleted either (`leaf_source_no_delete`, migration `20261211000000`). No table is partitioned.
  Feature 010 recorded it as gap 241; the reports measure a suspended run at 2,459 to 3,219 bytes with its indexes
  (`tests/performance/reports/durable-execution.md`, `scalability-ha.md`) and a collection at about 2,335 journal bytes
  per item (`workflow-primitives.md`).
- **Feature 020's purge is expensive where the data is.** Payloads are content-addressed per tenant, `(tenant_id,
  digest)`, and shared by every row that holds the same bytes: 22 foreign-key columns in 14 tables reference them (20
  in 12 run-owned tables, 2 in shared ones), and three run-owned columns reference them with no foreign key
  (`collection_scope_items.item_digest`, migration `20261029000000`; the two result digests of
  `leaf_collection_success_certifications`, migration `20261211000000`). Deleting a run's payloads needs a check that no
  other row references them and an index on each referencing column (gap 1099 of feature 020), which adds write cost to
  the hottest tables. Architecture decision 0054 rejected partition drop because "partitions do not align with run
  lifecycles (a long run spans many), it needs the journal re-keyed (gap 241), and it cannot verify an archive per run
  before dropping".
- **What a partition key must survive.** A child run's identifier is derived from a SHA-256 of its parent and node
  (`child_run_id`, `crates/application/src/execution.rs`), so a range of run identifiers is not a range of time. An
  event's own time would split a long run across partitions. Every read of `run_events` is by `(tenant_id, run_id)`;
  none reads by time; no foreign key points to `run_events`. A run's input payload is written before its run row
  (`crates/infra-postgres/src/execution.rs`), and `runs.created_at` is set by the application.
- **Foreign keys and partitions do not mix cheaply.** A detach of a partition of a referenced table checks every
  referencing table under a `SHARE` lock, which blocks writes to `runs` and `steps` while it scans; dropping a table that
  has a foreign key to `runs` locks `runs`; before PostgreSQL 18 a foreign key on a partitioned referencing table cannot
  be added `NOT VALID`.
- **Partitions cost locks.** With four and eight API replicas `LWLock:LockManager` already reaches 4% to 11% of active
  backends (`tests/performance/reports/scalability-ha.md`, section of 2026-11-12). A statement not pruned at plan time
  opens and locks every partition, and so does a generic cached plan on the supported PostgreSQL versions, the cached
  plans of PL/pgSQL triggers included.
- **Dropping fires no row trigger.** The content quota (Architecture decision 0027) is kept by row triggers through
  `sagat_quota_add_delta` and a deferred flush; a `DROP TABLE` changes no row, so it releases nothing.
- **The catalog checks** treat partitions inconsistently: the classification test enumerates relations of kind `r` and
  would list each partition; the startup privilege check (`crates/infra-postgres/src/privilege.rs`) and the
  runtime-privilege audit look tables up by name and would ignore partitions.

## Requirements

- **Requirement 049-001 The journal bucket.** Every run MUST have a `journal_bucket`: the first day of the UTC month of
  its `created_at`, or `1970-01-01` (legacy); a child run MUST take its root's bucket, so that a run tree is in one
  bucket (feature 020 already makes a tree eligible only as a whole). The creating transaction MUST compute it once from
  the `created_at` it binds (or read it from the root it holds) and pass it explicitly to every row it writes, the input
  payload written before the run row included. A check on
  `runs` MUST refuse a bucket that is neither legacy nor the month of `created_at`; a trigger MUST refuse a change of
  either, and MUST refuse a legacy bucket for a run whose `created_at` is later than the switch time plus the instance
  registry's liveness window of 30 s (Requirement 049-010), so that a process that has not yet learned of the switch is
  not refused while it is still live.
- **Requirement 049-002 One partition per run, enforced.** Every row of a partitioned journal table MUST carry its run's
  bucket. A trigger on each partitioned table MUST refuse a row whose `(tenant_id, run_id, journal_bucket)` is not a
  run's, or whose run is purged (has a stub), raising SQLSTATE 23503 as the foreign key it replaces did; on `run_payloads` it is a constraint trigger checked
  at commit, because the input payload precedes the run row. A suite that runs after the switch, with at least two dated
  partitions and a clock the test moves across a month boundary, MUST cover every insert site.
- **Requirement 049-003 `run_events` partitioned.** `run_events` MUST be partitioned by range of `journal_bucket`, one
  partition per month and one legacy partition. Its primary key becomes `(tenant_id, run_id, seq, journal_bucket)`, so
  that lookups by run keep their index prefix. Its checks (`run_events_seq_positive`, `run_events_trace_id_format`,
  `run_events_instance_identity_consistent`) and `run_events_instance_idx` are on the parent. The append-only and
  no-truncate triggers stay enabled always on every partition (the statement-level truncate trigger is created on each,
  since it is not inherited). The runtime role has `SELECT` and `INSERT` on the parent only, and no privilege on a
  partition.
- **Requirement 049-004 Online.** The partitioning of `run_events` MUST NOT copy its rows and MUST NOT build an index or
  scan the table while it holds a lock that blocks inserts. A statement that takes such a lock MUST wait for it under a
  `lock_timeout` of 2 s (design 048) and be retried, and run only catalog work while it holds it. The concurrent
  operations (`CREATE INDEX CONCURRENTLY`, `DETACH ... CONCURRENTLY`) run with no `lock_timeout`, because they wait for
  older snapshots through lock waits by design.
- **Requirement 049-005 Run payloads for new runs, owned by the run tree.** For a run with a dated bucket, every payload
  a row of that run references MUST be stored in `run_payloads`, keyed `(tenant_id, root_run_id, digest, journal_bucket)`
  and partitioned like `run_events`. The runs of one tree (a root and its children) MAY share payloads; a run MUST NOT
  reference a payload of another tree: a reset, a rerun, a chain's next segment (a new root) or a cross-namespace
  operation that starts from another tree's value writes its own copy. Every read of a run's content MUST go through
  one adapter function that branches on the run's bucket (`run_payloads` for a dated run, `payloads` for a legacy run or a
  tombstone); the static check of Requirement 049-011 refuses any other statement that reads `payloads` or
  `run_payloads` with a run's digest, and the suite after the switch asserts content on every read path. A legacy run keeps writing and reading its payloads in `payloads`. Payloads of rows that belong to no run (the
  two shared columns, the tombstones) stay in `payloads`. `run_payloads` is content for the quota and has its delta and
  flush triggers from its creation.
- **Requirement 049-006 References are checked without foreign keys.** Every column that references run content MUST
  be listed, with the run that owns the row, in the migration that creates the reference triggers, foreign key or not
  (when feature 020 builds its registry, `crates/infra-postgres/run-content-references.json`, which it owns, a test
  asserts that the two lists agree);
  each listed column MUST be checked on insert and on update of that column by a trigger, enabled always: a legacy run's
  reference must exist in `payloads`, a dated run's in `run_payloads` for the same tenant, run and bucket, a purged
  run's stub may reference the tenant's tombstone. The trigger's lookup MUST take `FOR KEY SHARE` on the row it finds,
  as the built-in key check does, so that a concurrent delete of that payload waits for it and its own re-check fails;
  and it MUST refuse a row whose `journal_bucket` differs from its run's. The trigger raises SQLSTATE 23503. Its table gains `journal_bucket`
  (`NOT NULL`, constant default the legacy value, so no rewrite). A migration test MUST fail when a digest column of a
  run-owned table is in neither the registry nor its exclusion list, or when a run-owned table has a foreign key to
  `payloads` other than the two shared columns. A second test over `pg_constraint` MUST fail when any foreign key has
  a relation of `pg_partition_tree('run_events')` or `pg_partition_tree('run_payloads')` on either side.
- **Requirement 049-007 Legacy purge.** For feature 020's purge of a legacy run, the reference check of gap 1099 MUST
  be served by partial indexes whose predicate is `journal_bucket = '1970-01-01'`, built concurrently, and delivered with
  the purge they serve (gated as it is). Their cost on new writes is not nil (every index of a table is opened on an
  insert, and an indexed column updated defeats a heap-only update), so Task 049-3 measures it, including the heap-only
  update ratio of `steps` and `runs`, before they are built. The purge deletes a candidate payload and then re-checks its
  references in a later statement under a savepoint, as the built-in key check does.
- **Requirement 049-008 Partitions exist ahead.** A definer function, `ensure_journal_buckets()`, owned by the owner
  role, executable by the runtime role, with no argument, a pinned `search_path` and a transaction-level advisory lock,
  MUST create the partitions of the current month and the next three for every partitioned table (each a standalone table
  with only its statement-level truncate trigger, checked by a constraint, then attached, which clones the row triggers,
  under the 2 s `lock_timeout`), retried hourly after a failure. The engine's housekeeping calls it at start and daily.
  The gauge `journal_partition_months_ahead` MUST give the months that exist ahead, and the alert
  `SagatJournalPartitionsLow` MUST fire below two. Requirement 049-013's audit fails on a duplicated trigger. A row for a bucket with no
  partition fails with SQLSTATE 23514, no constraint name and the partitioned table as the error's table (which
  distinguishes it from the PL/pgSQL guards that also raise 23514); the adapter maps it to code `UNAVAILABLE` with reason
  `JOURNAL_BUCKET_MISSING` (retryable) for a bucket at or after the current month, and to code `NOT_FOUND` with reason
  `RUN_PURGED` (the existing reason) for a retired one; a legacy run refused after the switch is `UNAVAILABLE` with reason
  `JOURNAL_SWITCHED` (retryable on another process). The reasons are added to the error contracts and OpenAPI in the same
  delivery. There is no default partition.
- **Requirement 049-009 A month is dropped only when every run in it is purged.** A bucket MAY be retired only when every
  run of it is purgeable under feature 020 (terminal, its tree terminal and, for a chain, the chain final, past its
  namespace's hot retention, every sink applied or held past its hold, not `PARTIAL`, not `COMPENSATION_FAILED`, no
  unresolved outcome or review, and no leaf-success certificate, whose sources decision D298 keeps without expiry) and
  its archive is `VERIFIED` (or its policy is `archive: NONE`). The legacy bucket is never retired: its payloads are
  shared in `payloads`, so its runs leave only by feature 020's per-run purge. Retirement MUST take a global retirement
  lock (one detach may be pending per table), then the bucket's lock, which feature 020's per-run purge takes with a
  try-lock before its run lock (so that the order is bucket, then run, and neither waits for the other), and MUST, in
  order: purge each run in the mark-only
  form of Requirement 049-014; detach the bucket's partitions concurrently (finishing a detach left pending with
  `DETACH ... FINALIZE`); check that the detached tables hold no row of a run that is not purged, reattaching and
  restarting from the first step if one does; count the bytes of the detached `run_payloads` table per tenant; release
  them and drop the detached tables in one transaction, with the retirement's audit record and one audit record per
  tenant, through `sagat_quota_release_bytes`, an owner-only definer function that locks each tenant's usage row in UUID
  order (Architecture decision 0027) and refuses an underflow, summing `octet_length(content)` as the delta trigger
  does. On resume, each table's state (attached, detach pending, detached, dropped) is read from the catalog, not only from
  the last record, and each table is finished from its own state. A retirement abandoned after its purges leaves its runs
  in the state `CONTENT_PENDING_DROP`, recorded per run; a per-run purge of feature 020 finishes such a run by its key
  whenever no retirement of its bucket is active, and refuses with a retryable error while one is (a detach pending
  included). Erasure never uses the mark-only form. Each step is
  recorded in `journal_retirements`, so a crash at any point resumes from the last recorded step.
- **Requirement 049-010 The switch.** New runs get a dated bucket only after the registry's activation of the
  capability `journal-buckets-v1`, which updates the one row of `journal_bucket_switch` (it always exists, empty before
  the switch). The creating transaction of a run reads that row `FOR SHARE` (folded into the run insert, so no statement
  is added), so a creation and the switch serialize and no creation in flight is refused. The bucket, switch, reference
  and late-legacy triggers are all enabled always; the late-legacy refusal raises its own SQLSTATE and constraint name.
  Before the switch, every run is legacy.
- **Requirement 049-011 Every statement names its bucket.** Every statement that reads or writes a partitioned journal
  table MUST compare `journal_bucket` by equality with a bind parameter or a literal. The writer takes the bucket from
  the run row it holds; a reader that does not hold the run row reads it (one statement, counted where it is on the hot
  path). A static check, `scripts/check-journal-buckets.py`, over the SQL literals of `crates/infra-postgres/src` and the
  function bodies of the migrations after this design's first one, MUST fail on a statement that touches a partitioned
  journal table without it, and on SQL built at run time (`format!` and the like) that names one. A migration test applies
  the same rule to the bodies of every function in `pg_proc`. The suite after the switch MUST assert that no statement for
  a dated run reads `payloads` except for a tombstone.
- **Requirement 049-012 The hot path is budgeted.** Design 038's budget test MUST record, besides statements,
  transactions and rows written, the distinct relations locked during a lease and a completion and the round trips, in
  the change that partitions each table. Partitioning MUST NOT change the statement count of a lease or a completion, and
  MUST NOT raise the relations locked by more than one partition and its indexes per partitioned table the operation
  writes, besides the relations the design adds on the run-creating path (`journal_bucket_switch`), each counted and
  named in the budget's line.
- **Requirement 049-013 The catalog checks know partitions.** The storage-classification test MUST map a partition to its
  parent; the startup privilege check and the runtime-privilege audit MUST list partitions through `pg_inherits` and
  fail on a partition with any grant to the runtime role, or with a required trigger missing or not enabled always.
- **Requirement 049-014 Feature 020's purge, by key, and its mark-only form.** Feature 020's purge of a dated run MUST
  delete that run's rows of the partitioned tables by `(tenant_id, run_id, journal_bucket)`, with no reference check, and
  its run-purpose blobs as feature 020 designs; the purge of a legacy run keeps feature 020's reference check
  (Requirement 049-007). The *mark-only form*, used by retirement, does the same under the run lock except that it leaves
  the run's partitioned rows for the drop: it re-checks eligibility, writes the stub (tombstone digests, the `NOT NULL`
  columns kept, so an older reader reads a stub as feature 020 designed), deletes the run's rows of the unpartitioned
  run-owned tables, its run-purpose blobs and its step-log streams (Architecture decision 0084, decision 4). A purged run refuses any later append. The rows that refer to content
  across runs belong to the run that holds the row: a chain's operation receipt to its `resolved_run_id`, a segment's
  `transfer_input_digest` to the segment's run (the next run holds its own copy).
- **Requirement 049-015 Features that will reference run content follow the rule.** The approved designs that plan new
  references to run content (feature 022: `test_run_pins.payload_digest` and the step cache's obligations; feature 026:
  `approval_requests.summary_digest`, `approval_decision_receipts.input_digest`; feature 030: the inputs and results of
  `service_operations`, each run tree with its own copy; feature 025: transcript bodies) MUST follow Requirements 049-005 and
  049-006; their data models are amended with this design's delivery, and so is feature 020's (the mark-only form, the
  registry's owner column, the key of the purge).

### Success criteria

- **Success criterion 049-001** After the switch, the budget test's statement counts for a lease and both completions
  equal the counts before partitioning, and the relations locked rise by at most one partition and its indexes per
  partitioned table written.
- **Success criterion 049-002** On a database holding 12 monthly buckets of 1,000,000 runs of the chain shape each, the
  retirement of one bucket is measured end to end (the mark-only purge of its runs included) and from the start of the
  detach to the end of the drop, each at the median and the maximum of five repetitions, and the chain shape's 99th
  percentile latency, measured by design 051's driver during the retirement, rises by less than 10%. The target for the
  detach-to-drop part is at most 120 s when no transaction older than 60 s is open: a concurrent detach waits twice for
  the transactions older than it, so 2 × 60 s = 120 s. Before each detach the retirement command lists the transactions
  older than 60 s and waits for them, or stops and reports them, so the bound is checked, not assumed.
- **Success criterion 049-003** The partitioning of a `run_events` of 100 GiB, under the chain shape at half its maximum
  sustained rate, blocks no insert for more than 2 s of wait plus the measured hold of its catalog statements, as shown by
  the driver's per-second counter and the migration's timing of each lock.
- **Success criterion 049-004** After a retirement, every affected tenant's content usage equals a fresh reconciliation of
  its rows.

### User stories

- **User story 049-01** An operator whose namespaces all keep 90 days of history sees the journal stop growing after 90
  days plus one month, without a nightly purge that fights the workload.
- **User story 049-02** A tenant whose data must be erased has its runs purged one by one, at a cost proportional to its
  runs, not to everyone's.
- **User story 049-03** A developer adding a statement on the journal learns at once, from a check, that it must name
  its bucket.

## Goals

- A per-run purge of every run created after the switch whose cost is the run's own rows.
- Retention that costs a drop per month where every namespace in the database has a policy no longer than the month's
  age; elsewhere, the per-run purge.
- No change in what a transition costs in statements, and a measured, bounded change in what it locks.

## Non-Goals

- **Retention policy and archival themselves** are feature 020's (its requirements 020-030 to 020-038).
- **Moving the payloads of legacy runs.** They stay in `payloads` and leave with feature 020's per-run purge.
- **Partitioning `runs`, `steps` and the audit trail in 0.3.0** (Technical Plan, "What is not partitioned").
- **Dropping audit records.** No audit retention exists; deleting audit evidence would be a new decision of the owner's.
- **Moving a live run's rows to another bucket** so that its old month can drop.
- **Partitioning the outbox, the inbox, the command deduplication and the webhook deliveries.** They already delete by
  age in small, indexed batches.
- **Changing feature 041's step logs** (Architecture decision 0084).

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6, with its reasoning; the acceptance of the decision
record is under Owner decision 2026-11-24-4 (pull request 274).

1. **What is the partition key?** The UTC month of the run's `created_at`, stored on the run and computed once by the
   creating transaction. The run identifier is not time-ordered for child runs; the event's time splits runs. Delegate.
2. **Which width?** One calendar month. With hot retention R days, the database holds about `ceil(R / 30) + 3` buckets
   (the retained months, the current one, two ahead); 90 days gives 3 + 3 = 6 and 365 days gives 13 + 3 = 16. A week
   would multiply the partitions, and so the planning and locking of a statement that is not pruned, by four. Delegate.
3. **Deduplicate payloads within a run, a run tree, a bucket, or a tenant?** Within a run tree, for new runs: a tree is
   eligible for retention only as a whole (feature 020), so its runs may share payloads without a cross-run check, and a
   fan-out's children need no copies of their inputs, so runs in flight do not grow in quota use after the switch. Within a bucket would bring
   the reference check back for a single run's purge. The cost is a copy where two runs hold the same bytes, which the
   content quota counts; Task 049-2 measures how often that happens. Delegate.
4. **Copy the legacy payloads, or leave them?** Leave them. A copy would double-charge the quota until a cleanup that
   deletes journal rows, and needs the census twice; leaving them costs feature 020's reference check for legacy runs
   only, on partial indexes new writes never touch (Requirement 049-007). Delegate.
5. **Foreign keys or triggers?** Triggers that raise the same SQLSTATE. A foreign key into a partitioned table makes
   every detach scan the referencing tables under a lock that blocks their writes; one out of a partition makes its drop
   lock `runs`. The cost is a trigger call per write instead of the built-in check, which Task 049-3 measures. Delegate.
6. **Partition `steps`?** Not in 0.3.0. The scheduler's passes read its partial indexes without a run identifier, so
   every partition would be opened, and its rows are few per run. Task 049-16 measures it after design 052's queue removes
   the claim's scan. Delegate.
7. **A default partition as a safety net?** No: it makes every later attach scan it, and rows that should fail would
   land there silently. Delegate.
8. **How are statements planned so that only one partition is locked?** Decided by Task 049-3, among three: statements
   on partitioned tables sent unnamed (planned with their parameters, one more round trip each); the runtime role's
   `plan_cache_mode` set to `force_custom_plan`; or the triggers' and statements' plans as they are, if the measured
   locks stay within Requirement 049-012. The decision rule: the choice with the lowest median completion latency among
   those that meet Requirement 049-012 on PostgreSQL 16 and 18. This design is amended with the choice before Task 049-8
   starts. Delegate.
9. **The decision record.**
   [Architecture decision 0132](../adr/ADR-0132-the-journal-is-partitioned-by-the-month-a-run-was-created-and-a-run-owns-its-payloads.md)
   records the key, the run-owned payloads of new runs, the triggers in place of foreign keys and the rule for dropping a
   month; it refines Architecture decision 0054 in part (its decisions 4 and 6, and its rejected option 3 for whole
   months only).

## Owner decisions needed

- **The amendment of principle I, pending constitution 5.1.0.** Retirement deletes journal rows, as feature 020's purge
  does. Decided by the coordinating session (Owner decision 2026-11-24-511): it is not part of 5.0.0; it is a 5.1.0
  amendment written once this design and feature 020's retention design are approved, so that its text follows the
  approved mechanisms and names both audited privileged paths (feature 020's per-run purge and the retirement of a whole
  month; Architecture decision 0054's proposed text says "one audited privileged path"), and has its own review. Task
  049-18 writes this design's part of it. Gates Tasks 049-11 and 049-14.
- **Scope.** Owner decision 2026-11-24-1 (pull request 273) names "partitioning of the journal tables" among the new
  designs of release 0.3.0; the delegation covers its open questions. The rows of Owner decisions 2026-11-24-1, -2 and -4
  are in pull requests 273 and 274, which merge before this design.

## Critical User Journeys (CUJs)

### CUJ 1: A month is retired

*   **Given**: every namespace with 90 days of hot retention and archives required; bucket 2026-12, whose runs all ended
    before 2027-01-01 and were archived and verified.
*   **When**: the operator's retirement job runs after 2027-04-01.
*   **Then**: it purges each run of the bucket in the mark-only form, detaches the December partitions of `run_events` and
    `run_payloads` concurrently, checks them, counts and releases their bytes, and drops them; the hot path never waits
    more than a lock timeout.

### CUJ 2: A run that lives a year

*   **Given**: one run of bucket 2026-12 still waiting for an approval in 2027-06.
*   **When**: the retirement job reaches December's bucket.
*   **Then**: the bucket stays; feature 020 purges every other run of it one by one, by its key;
    `journal_bucket_held_runs{reason="LIVE"}` shows one; when the run ends and becomes purgeable, the bucket is retired.

### CUJ 3: The upgrade

*   **Given**: a database of the previous release with 100 GiB of `run_events`.
*   **When**: the release's expand migrations run while it serves.
*   **Then**: `run_events` becomes a partitioned table whose legacy partition is the old table, without a copy or an index
    build under a blocking lock; the foreign keys become triggers that raise the same SQLSTATE; new runs stay legacy until
    the operator activates `journal-buckets-v1`.

### CUJ 4: A statement without its bucket

*   **Given**: a change that adds `SELECT ... FROM run_events WHERE tenant_id = $1 AND run_id = $2`.
*   **When**: its pull request runs the static check.
*   **Then**: the check fails and names the statement; the author adds `AND journal_bucket = $3`.

## Technical Plan (Architecture)

### Layout

| Table | Partitioned | Key | Primary key |
|---|---|---|---|
| `run_events` | yes, by range of `journal_bucket` | the run's month | `(tenant_id, run_id, seq, journal_bucket)` |
| `run_payloads` (new) | yes, by range of `journal_bucket` | the run tree's month | `(tenant_id, root_run_id, digest, journal_bucket)` |
| `payloads` | no | | `(tenant_id, digest)`: legacy runs' payloads, the two shared columns, the tombstones |
| `runs` | no | | `id`, with `journal_bucket` and feature 020's stub columns |
| `steps` and the other run-owned tables | no | | unchanged, each with `journal_bucket` for its reference check |
| `audit_records` | no | | unchanged |

Partition names are `<table>_b<YYYYMM>` and `<table>_legacy`.

### `run_events`, online

The migrations, marked `expand` (design 048). Each statement that takes a lock blocking inserts runs in a transaction
of its own (files run without a transaction, `-- no-transaction` first, retried statement by statement), so that no
transaction accumulates exclusive locks on several tables, and waits under `lock_timeout` 2 s. Each step has a `down`
that restores the previous schema (the foreign keys recreated `NOT VALID` and validated), refused once a dated row
exists:

1. Add `journal_bucket date NOT NULL DEFAULT '1970-01-01'` to `runs`, `run_events` and the run-owned referencing
   tables (`steps`, `iteration_scopes`, `collection_iterations`, `iteration_checkpoints`, `collection_scope_items`,
   `workflow_operation_states`, `operation_update_receipts`, `loop_scopes`, `loop_iterations`, `loop_checkpoints`,
   `chain_segments`, `leaf_collection_success_certifications`) (a constant default: no rewrite); `journal_bucket_switch` (one row, empty); the check and the immutability and
   switch trigger of Requirement 049-001 on `runs` (the check `NOT VALID`); the bucket trigger of Requirement 049-002 on
   `run_events`; the reference triggers of Requirement 049-006 (`UPDATE OF` their columns, enabled always); then drop,
   one per statement, the foreign keys they replace (the 20 to `payloads` and `run_events`' to `runs`). The triggers
   raise SQLSTATE 23503, so the previous release sees the same error as before. Dropping a constraint in an expand
   migration is an exception to design 048's lint (Architecture decision 0131, decision 2), recorded in the migration's
   header in the allow-list form of design 048's Task 048-12, which Architecture decision 0132 names as a refinement of
   0131.
2. Without a transaction: `DROP INDEX CONCURRENTLY IF EXISTS run_events_bucket_key`, then `CREATE UNIQUE INDEX
   CONCURRENTLY run_events_bucket_key ON run_events (tenant_id, run_id, seq, journal_bucket)`, with no `lock_timeout`;
   likewise the partial legacy indexes of Requirement 049-007 and `runs (journal_bucket)`.
3. Replace the primary key: `ALTER TABLE run_events DROP CONSTRAINT run_events_pkey, ADD CONSTRAINT
   run_events_legacy_pkey PRIMARY KEY USING INDEX run_events_bucket_key` (no scan: `journal_bucket` is `NOT NULL`); add
   `CHECK (journal_bucket = '1970-01-01') NOT VALID`.
4. Validate the two checks (`SHARE UPDATE EXCLUSIVE`, which blocks no insert or read).
5. One short transaction: rename `run_events` to `run_events_legacy`, its triggers to `*_legacy` and its indexes
   (`run_events_instance_idx` to `run_events_legacy_instance_idx`; an attach matches indexes by definition, not name); create the
   partitioned `run_events` with the same columns, the primary key of Requirement 049-003, its three checks,
   `run_events_instance_idx` and its triggers; attach `run_events_legacy` `FOR VALUES FROM (MINVALUE) TO ('1970-01-02')`
   (no scan: the validated check proves the range; no index build: the primary key and the instance index already exist
   on it with matching definitions); drop the legacy table's renamed row triggers, now duplicated by the cloned ones
   (its statement-level truncate trigger is kept, since it is not cloned), and set the cloned triggers enabled always; revoke
   the runtime role's grants on `run_events_legacy` and grant `SELECT, INSERT` on the parent; create the partitions of
   the current month and the next two. Only catalog statements run under the `ACCESS EXCLUSIVE` lock of the rename.

A step 2 index stopped by a timeout or a crash is invalid; step 2 drops and rebuilds it. A failed step 5 rolls back
whole. The rename is the second recorded exception to design 048's lint. The upgrade from 0.2.x to 0.3.0 is a stopped
upgrade (Architecture decision 0131, decision 6), so in it these steps run with writers stopped and Requirement 049-004
protects nothing but the time they take, which is reported; from 0.3.x on they run while the platform serves. While the
previous release still runs, its statements carry no bucket and lock every partition; Task 049-3 records that cost for
the upgrade window.

### `run_payloads`

Created in step 1 of the next migration: partitioned like `run_events`, the append-only, no-truncate, quota delta and
flush triggers, the deferred bucket trigger, grants `SELECT, INSERT` on the parent only, its storage classification as
content. Before the switch nothing writes it; after it, dated runs write it and only it.

### The switch

`control-plane journal switch`, as the owner role, after the release is everywhere: the registry's activation of
`journal-buckets-v1` (which refuses while a live process lacks it), and in the same transaction the switch time in
`journal_bucket_switch`. From then the creating transaction of a run computes a dated bucket, and the trigger on `runs`
refuses a legacy bucket for a run created later than the switch time plus 30 s, so a stale process cannot keep creating legacy
runs once it would have learned of the switch; the refusals are counted, and a barrier test holds the activation against a
run creation.

### Locks and plans

A statement planned with the bucket's value prunes to one partition and locks it and its indexes. How plans are made is
Open Question 8, decided by Task 049-3 with the rule stated there, before Task 049-8. The relations a transaction locks,
the triggers' statements included, are counted by the budget (Requirement 049-012).

### Retirement

`control-plane journal retire --bucket YYYY-MM`, as the owner role, run by the operator, never by the platform's
processes; the chart offers it as an opt-in scheduled job with its own service account and an owner credential in a
Secret that only that job mounts. It takes
an advisory lock that feature 020's per-run purge also takes for the bucket, reads feature 020's job state, sets
`statement_timeout = 0` and lifts `transaction_timeout` for its session, and records every step in
`journal_retirements` (a platform-level table with its audit, as `platform_requirement_history` is for the registry):

1. Check every run of the bucket (on `runs (journal_bucket)`); stop if any is not purgeable, counting the held runs by
   reason (`NO_POLICY`, `NOT_ELIGIBLE`, `NOT_VERIFIED`, `LIVE`) in `journal_bucket_held_runs`.
2. Purge each run in the mark-only form, in chunks, each under its run lock.
3. `ALTER TABLE ... DETACH PARTITION ... CONCURRENTLY` for each partitioned table's partition of the bucket; on resume, a
   detach left pending is finished with `DETACH PARTITION ... FINALIZE`.
4. Check that no row of the detached tables belongs to a run that is not purged; if one does, attach the partitions
   again and restart at step 1.
5. Count the bytes of the detached `run_payloads` table per tenant, which nothing can change any more, and record them.
6. In one transaction: release them through `sagat_quota_release_bytes` in tenant UUID order, write the retirement's
   audit record and one per tenant, `DROP TABLE` the detached partitions (they have no foreign key, so the drop locks
   nothing of the platform's), and mark the step done.

The legacy partition is never retired by this command: its runs leave by feature 020's per-run purge, with the reference
check that shared payloads need, and the partition is detached and dropped once no legacy run remains.

### What is not partitioned, and why

- **`runs`.** A purged run keeps its stub (feature 020, owner question 1082), and its queries by status and by shard read
  across runs of every age.
- **`steps`.** Open Question 6.
- **`audit_records`.** No audit record may be dropped, and its key `(id)` would have to become `(id, created_at)`,
  losing the uniqueness of `id` that `crates/infra-postgres/src/chain_progress.rs` relies on.
- **The outbox, the inbox, the deduplication table and the webhook deliveries.** They are deleted by age already.

### Interactions

- **Feature 020** builds the purge, the archive and the purge authorization; this design defines the registry file, the
  key of the purge of a dated run and the mark-only form, and amends feature 020's data model and tasks (Task 049-15).
  Its Task 049-14 runs after feature 020's Tasks T020-045 and T020-046.
- **Feature 032** adds `run_events.region_epoch` (Architecture decision 0067, decision 9). A column added to the
  partitioned parent is added to every partition; whichever design lands second writes its migration against the
  other's schema; `ensure_journal_buckets()` runs only in the active region.
- **Design 048**'s rules apply to every migration here, with the two recorded exceptions (the constraint drops of step
  1, the rename of step 5). The lint (design 048's Task 048-12) and the retrying `migrate` are prerequisites of Task
  049-8; until they exist, Task 049-8 retries by hand and the exceptions are written in the migrations' headers.
- **Designs 051 and 052.** Design 051's driver measures Tasks 049-8 and 049-14; design 052's queue is not partitioned.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A statement forgets the bucket and opens every partition | The static check; the lock count in the budget |
| A run's rows land in two buckets | The bucket is computed once and bound; the bucket triggers refuse a mismatch; the switch trigger refuses late legacy runs; the suite after the switch with a controlled clock |
| A reference points to a payload that is not there | The reference triggers on every registered column; the registry test |
| No partition for next month | Created three months ahead daily; an alert below two; a missing one fails loudly and retryably |
| A trigger per write costs more than the key check it replaces | Task 049-3 measures both; the budget records latency and round trips |
| The rename queues behind a long transaction | `lock_timeout` 2 s and a retry; only catalog statements under the lock |
| A long run holds a month | The rest of the month is purged by run; held runs are a metric with a reason |
| Retirement races an append or a per-run purge | Runs are purged under their run lock before the detach; a purged run refuses appends; the bucket lock excludes per-run purges; the check after the detach reattaches on any surprise |
| A drop does not release the tenant's quota | Bytes counted from the detached table and released through the definer function in an audited transaction; Success criterion 049-004 |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass with conditions | The partitioning keeps the journal append-only with a monotonic per-run sequence and a run's rows together; retirement deletes journal rows and waits for the constitution 5.1.0 amendment of principle I (Owner decision 2026-11-24-511) |
| II. Modular domain, ports and adapters | Pass | `journal_bucket` stays in `infra-postgres`'s row types and never enters the domain's run type; the application, scheduler and control-plane changes (Tasks 049-9 and 049-13) only call the store |
| III. External effects are at-least-once; sagas compensate | n/a | No effect changes |
| IV. Compatibility is versioned and pinned | Pass | Expand migrations whose triggers raise the same SQLSTATE as the keys they replace; old binaries write the legacy bucket; the switch is a registry activation |
| V. Code, tenant boundary and secrets are isolated | Pass | Every key starts with `tenant_id`; no privilege on a partition; the definer functions take no caller-supplied scope beyond the tenant they release; retirement's owner credential is in no platform pod |
| VI. Observability and audit are part of the feature | Pass | Months ahead, held runs by reason, legacy refusals after the switch, retirement steps; each retirement audited |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | Not touched |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Crash at each retirement step and inside the detach, a missing partition, an append and a per-run purge racing a retirement, a suite after the switch across a month boundary |
| Delivery Constraints | Pass | PostgreSQL stays the only authority; nothing is dropped that feature 020 has not archived and verified |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| `journal_bucket` on runs and the run-owned tables | The key must be on the run, known to every writer, and the same for all of a run's rows | The run identifier: child identifiers are hashes; the event's time: splits runs |
| `run_payloads` for new runs | A run must own its payloads for its purge, or a month's drop, to need no reference check | Keeping `payloads` shared for every run: feature 020's 22 reference indexes on the hottest tables |
| Triggers in place of foreign keys | Foreign keys make detaches scan and lock the referencing tables, and drops lock `runs` | Foreign keys: the drop path would block the hot path |
| A deletion of journal rows by month | Retention at the size of a month's data | Waiting for the amendment before the partitioning: only retirement waits |

## Alternatives Considered

- **Delete per run only** (feature 020 as drafted). Correct, and the main path here; at scale the drop path avoids
  millions of deletes and their bloat.
- **Copy the legacy payloads into `run_payloads`.** One regime instead of two, at the price of a double charge until a
  cleanup that deletes journal rows, a move job and two censuses.
- **Partition by event time.** Splits a long run's events and cannot drop a month while any run that wrote to it lives.
- **Partition by hash of the tenant.** Drops nothing and does not follow retention, which is per namespace.
- **Foreign keys to `run_payloads`.** Integrity by the database's own mechanism, at the price of a `SHARE` scan of every
  referencing table at each detach.
- **The `pg_partman` extension.** An extension the managed PostgreSQL targets may not offer, for a creation rule of a
  few statements.

## Non-Functional Requirements

- **Non-functional requirement 049-001 Hot path.** As Requirement 049-012 and Success criterion 049-001.
- **Non-functional requirement 049-002 Online.** As Requirement 049-004 and Success criterion 049-003.
- **Non-functional requirement 049-003 Security.** The definer functions pin their `search_path` and are executable only
  by the roles named in their migration; `sagat_quota_release_bytes` is the owner's only; the retirement command's owner
  credential lives in an operator Secret that no platform pod mounts.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Tasks 049-1 to 049-13 and 049-15 to 049-17 are release 0.3.0; Task 049-14 waits for the amendment. Each task's files
are named; the test suites are those of the Verification Plan.

- [ ] T049-1 Write Architecture decision 0132, the refinement notes in Architecture decisions 0027, 0054 and 0131, a dated section of `specs/020-visibility-analytics-archival/research.md`, the layout in `docs/architecture/storage-and-events.md`, and the new checks' rows in `docs/engineering/architecture-principles.md`.
  Refs: Requirement 049-001, Requirement 049-005, Requirement 049-014
- [ ] T049-2 Measure, with design 051's history loader, the share of payload bytes referenced by more than one run in the nine shapes, and record it in `tests/performance/reports/journal-partitioning.md`.
  Refs: Requirement 049-005
- [ ] T049-3 Measure, in a new `perf_journal_partitioning` target, locks, round trips, planning time and completion latency with 1, 6, 16 and 60 partitions for the three planning choices and for triggers against key checks, and the locks of attach and detach, on PostgreSQL 16 and 18; choose by Open Question 8's rule and amend this design.
  Refs: Requirement 049-006, Requirement 049-011, Requirement 049-012
- [ ] T049-4 Add to `tests/integration/claim-transition-efficiency/work_budget.rs` the relations locked (read from `pg_locks` for the test's backend by a hook of the test support after `SET CONSTRAINTS ALL IMMEDIATE`, so that deferred triggers are counted) and the round trips (counted by the test's connection wrapper), at today's values.
  Refs: Requirement 049-012
- [ ] T049-5 Compute the bucket once in the creating transaction of a run from a clock the tests control (`crates/infra-postgres/src/execution.rs` near the run insert, `chain_progress.rs` at its two run inserts) and bind it to every row it writes, with the switch trigger and the late-legacy refusal metric.
  Refs: Requirement 049-001, Requirement 049-010
- [ ] T049-6 Write `scripts/check-journal-buckets.py`, its negative fixtures under `scripts/tests/`, and its CI step beside the architecture check.
  Refs: Requirement 049-011
- [ ] T049-7 Map partitions to their parent in the classification test (`tests/integration/security-multitenancy/durable_quotas.rs`), and list partitions through `pg_inherits` in `crates/infra-postgres/src/privilege.rs` and `tests/integration/platform-foundation/runtime_privileges.rs`, adding `run_payloads` to the append-only set.
  Refs: Requirement 049-013
- [ ] T049-8 Write the five migrations of "`run_events`, online", rewrite the 43 references to `run_events` in the 18 files of `crates/infra-postgres/src` to name the bucket from the run row, add the error mapping of the two typed errors, and run the migration tests and a measurement under load.
  Refs: Requirement 049-002, Requirement 049-003, Requirement 049-004, Requirement 049-008, Success criterion 049-003
- [ ] T049-9 Add `ensure_journal_buckets()`, its call from `crates/scheduler/src/passes.rs` (housekeeping), the gauge and the alert in `infra/observability/alerts.yaml` with its rule test added to CI's `promtool` list.
  Refs: Requirement 049-008
- [ ] T049-10 Write the list of run-content columns with their owning run in the reference-trigger migration, the two catalog tests, the barrier tests of a reference racing a delete in both orders, `run_payloads` and its triggers, and make dated runs' payload writes and reads (the payload insert sites of `execution.rs`, `chain_progress.rs`, `declared_operations.rs`, `loop_progress.rs`, `primitives.rs`, `execution/path.rs`, `approval_progress.rs`, `batch.rs`, `schedules.rs`) use it.
  Refs: Requirement 049-005, Requirement 049-006
- [ ] T049-11 After Task 049-3's measurement, build the partial legacy reference indexes concurrently and give feature 020's legacy purge its reference check on them, with the re-check after the delete.
  Refs: Requirement 049-007
  Owner-gate: pending, the constitution 5.1.0 amendment of principle I (Owner decision 2026-11-24-511; Owner decisions needed)
- [ ] T049-12 Write the per-run copy rules for children, resets, reruns, the chain's next segment and cross-namespace operations, with a test per path that the new run references only its own payloads.
  Refs: Requirement 049-005, Requirement 049-014
- [ ] T049-13 Write `control-plane journal switch` in `apps/control-plane/src/cli.rs` on the registry's activation (`crates/application/src/instances.rs`), with a suite that runs after it across a month boundary with two dated partitions.
  Refs: Requirement 049-002, Requirement 049-010
- [ ] T049-14 Write `control-plane journal retire`, `journal_retirements`, `sagat_quota_release_bytes`, the mark-only form and the chart's scheduled job, with the crash test at every step and inside a detach cancelled while it waits, the races with an append and with a per-run purge, and the quota reconciliation test.
  Refs: Requirement 049-009, Requirement 049-014, Success criterion 049-002, Success criterion 049-004
  Owner-gate: pending, the constitution 5.1.0 amendment of principle I (Owner decision 2026-11-24-511; Owner decisions needed)
- [ ] T049-15 Amend the data models and tasks of features 020, 022, 025, 026 and 030 so that their run-content references follow Requirements 049-005 and 049-006, and feature 020's purge has the key and the mark-only form of Requirement 049-014.
  Refs: Requirement 049-014, Requirement 049-015
- [ ] T049-16 Measure the scheduler's passes with `steps` partitioned by bucket after design 052's queue, and record the decision.
  Refs: Requirement 049-012
- [ ] T049-17 Write the retention section of `docs/runbooks/database-recovery.md` and the bucket runbook.
  Refs: User story 049-01
- [ ] T049-18 Once this design and feature 020's retention design are approved, draft with feature 020 the text of the constitution 5.1.0 amendment of principle I that admits the two audited deletion paths, for its own review.
  Refs: Requirement 049-009, Requirement 049-014

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs `integration_platform_foundation` (its migration tests: up with data, down refused where the
migration destroys data, up again; and its `runtime_privileges` module), `integration_security_multitenancy` (the
classification test), the suites rerun with `SAGAT_TEST_AS_RUNTIME_ROLE=1`, design 038's budget target, the static check
of Task 049-6, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and the documentation and architecture
checkers of `scripts/`. These are added and seen failing first: the suite after the switch across a month boundary
(Task 049-13); a migration test asserting, by the identity (`relfilenode`) of every index of the legacy table before and after,
that step 5 builds none and
that the parent carries the three checks and the instance index (Task 049-8); the registry test with an unregistered
digest column (Task 049-10), and the confirmation of what the two certificate digests of
`leaf_collection_success_certifications` hold before they are registered; the per-run copy tests (Task 049-12); fault tests at every retirement step and inside a
concurrent detach cancelled while it waits, the race tests, and the quota reconciliation after a retirement (Task
049-14). Task 049-8 is also run under design 051's driver at half the maximum sustained rate, and its per-second counter
is the evidence of Success criterion 049-003.
