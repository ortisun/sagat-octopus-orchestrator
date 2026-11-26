# Partitioning of the journal tables, joined to the retention of feature 020

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

`run_events`, `payloads`, `runs` and `steps` grow without bound: nothing deletes a run, an event or a payload
(Architecture decision 0054: "Nothing is ever deleted today"). Feature 020 designs retention as a verified archive of
each run followed by a purge of that run in bounded chunks. This design gives that retention a cheap path where it can
have one, and makes the per-run path cheap for every run tree created after it:

1. **A journal bucket per run tree.** Every root run is given, when it is created, the UTC month of its `created_at`,
   its *journal bucket*, stored on the run row and never changed; every child run takes its root's bucket, so a tree is
   in one bucket. Runs created before the switch, and roots created by a process that has not yet learned of it, keep the
   *legacy* bucket.
2. **`run_events` is partitioned by bucket** during the stopped upgrade from 0.2.x to 0.3.0 (Architecture decision 0131,
   decision 8): the existing table becomes the legacy partition by `ATTACH PARTITION`, with no row copied, so the upgrade's
   stop is not lengthened by the table's size beyond one index build.
3. **Run trees created after the switch own their payloads** in a new `run_payloads`, partitioned by bucket and keyed by
   the tree's root: a tree's payloads are deleted by their key when the tree's last run is purged, with no check of
   references from other trees. Legacy runs keep their payloads in `payloads` as today, so nothing is copied; their
   purge keeps feature 020's reference check.
4. **No foreign key points into or out of a partitioned journal table.** The integrity those keys gave is kept by
   triggers that check the same facts on every write and raise the same SQLSTATE, so that a partition can be detached
   and dropped without scanning or locking the referencing tables. A registry lists every reference to run content,
   foreign key or not.
5. **Feature 020's per-run purge is the main path; retiring a month is the fast path** when every run of that month has
   been purged: one short transaction per partitioned table detaches and drops the month's partition, and one more
   releases the tenants' content quota, instead of deleting the month's journal and payload rows one by one.
6. **`runs`, `steps` and the audit trail are not partitioned in release 0.3.0**, for the reasons in the Technical Plan.

Why release 0.3.0: its upgrade from 0.2.x stops the writers anyway, so converting `run_events` then needs none of the
online machinery a later release would; and the owner approved the deletion this design serves. Deleting journal rows,
by month or by run, changes an obligation of principle I (the journal is append-only); that amendment is constitution
5.1.0, approved by the owner (Owner decision 2026-11-25-3, point 1: the retirement of a whole month partition once every
run in it is purged, after a verified archive) and drafted by the coordinating session. The partitioning needs none and
is built first; only the deleting tasks wait for 5.1.0 to merge.

The cost on the hot path is measured by design 038's statement budget, which gains a count of the relations each lease
and completion locks and of its round trips.

**Terms used below.** Feature 020's *run-purpose blobs* are the object-store blobs it deletes with a run (step outputs
and run inputs); its *sinks* are the destinations a run's archive is delivered to, each of which may *hold* the content
for a time; the *active region* is feature 032's writable region of a cell. The held-run reasons of retirement are
`LIVE` (a run or its tree not terminal), `NO_POLICY` (its namespace has no retention policy), `NOT_ELIGIBLE` (any other
condition of Requirement 049-009 unmet) and `NOT_VERIFIED` (its archive is not `VERIFIED`, `archive: NONE` included).
The *journal* is the append-only record of what happened in each run: `run_events` holds one row
per event with a per-run sequence (`seq`), and *payloads* hold the values (inputs, outputs, event bodies) the events and
the state rows refer to by digest. A *run tree* is a root run (`root_run_id = id`) and every run whose `root_run_id` is
that root; a *chain* is a sequence of segments, each a run: a root chain's next segment is a new root, a child chain's
next segment stays in its parent's tree (`chain_progress.rs`). A *partition* is one physical table of a table
partitioned by PostgreSQL; *pruning* is the planner's choice of the partitions a statement must open. *Feature 020*
designs retention: a namespace's policy says how long a finished run stays (*hot retention*), its archive is written to
an object store and read back to check it (*verified*), and only then is the run *purged*, leaving a small *stub* row
(`runs.purged_at` set, feature 020's data model) whose digests point to a per-tenant *tombstone* payload; *erasure* is
feature 020's purge on a subject's request, which must leave no copy. The *run lock* is the row lock on the run that
every transaction that appends to it takes first. *Design 038's budget* is the test that counts the statements,
transactions and rows of a worker's lease and of a completion and fails when they change. The *instance registry*
(Architecture decision 0082) records which processes are live and what they support; its *activation* switches a
capability on only when no live process lacks it, and every process learns of it within one heartbeat (10 s, jittered to
12 s). The *chain shape* is design 051's chain of three activities, driven by design 051's driver.

## The Problem

From the code on `develop` at `546d5aa5`:

- **Nothing shrinks.** `run_events` and `payloads` are append-only by triggers that refuse `UPDATE`, `DELETE` and
  `TRUNCATE` (`crates/infra-postgres/migrations/20261009000100_execution.up.sql`); `runs`, `steps` and the collection
  tables cannot be deleted either (`leaf_source_no_delete`, migration `20261211000000`). No table is partitioned.
  Feature 010 recorded it as gap 241; the reports measure a suspended run at 2,459 to 3,219 bytes with its indexes
  (`tests/performance/reports/durable-execution.md`, `scalability-ha.md`) and a collection at about 2,335 journal bytes
  per item (`workflow-primitives.md`).
- **Feature 020's purge is expensive where the data is.** Payloads are content-addressed per tenant, `(tenant_id,
  digest)`, and shared by every row that holds the same bytes: 22 foreign keys reference them. Twenty are in 12
  run-owned tables: `runs` (input and result), `run_events`, `steps`, `iteration_scopes` (2), `collection_iterations`
  (2), `iteration_checkpoints`, `loop_scopes` (2), `loop_iterations` (2), `loop_checkpoints` (2), `chain_segments` (2),
  `workflow_operation_states` and `operation_update_receipts` (2). Two are in shared tables:
  `schedule_revisions.input_digest` and `batch_items.payload_digest`. Three run-owned columns reference payloads with no
  foreign key (`collection_scope_items.item_digest`, migration `20261029000000`; the two result digests of
  `leaf_collection_success_certifications`, migration `20261211000000`). Deleting a run's payloads needs a check that no
  other row references them and an index on each referencing column (gap 1099 of feature 020), which adds write cost to
  the hottest tables. Architecture decision 0054 rejected partition drop because "partitions do not align with run
  lifecycles (a long run spans many), it needs the journal re-keyed (gap 241), and it cannot verify an archive per run
  before dropping".
- **What a partition key must survive.** A child run's identifier is derived from a SHA-256 of its parent and node
  (`child_run_id`, `crates/application/src/execution.rs`), so a range of run identifiers is not a range of time. An
  event's own time would split a long run across partitions. Every read of `run_events` for one run is by `(tenant_id,
  run_id)`; none reads by time; no foreign key points to `run_events`. A run's input payload is written before its run
  row (`crates/infra-postgres/src/execution.rs`). A run's `created_at` is the database clock for a run started through
  the API and for a child (`clock_timestamp()`, `execution.rs`), and the command's time for a chain's next segment
  (`chain_progress.rs`); a child may be created months after its root (pending children, `maintenance.rs`). A rerun
  takes its source's input blob, from another tree (`runs_input_blob_same_scope_fk`).
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

### The bucket

- **Requirement 049-001 The journal bucket.** Every run MUST have a `journal_bucket`: the first day of a UTC month, or
  `1970-01-01` (legacy). A root run's bucket MUST be legacy or the month of its own `created_at`; a child run's bucket,
  whatever its `created_at`, MUST equal its root's, so that a run tree is in one bucket. A child chain's next segment is
  a child and takes its root's bucket; a root chain's next segment is a new root. The creating transaction MUST compute
  the bucket once, before its first write, and pass it explicitly to every row it writes, the input payload written
  before the run row included: a root's from the `created_at` it binds (the database clock that the creating
  transaction's first statement already reads, `clock_timestamp()` in `execution.rs`, or, for a root chain's next
  segment, its command's time) and whether the process has learned of the switch
  (Requirement 049-010), which the application's registry state (`crates/application/src/instances.rs`) passes to the
  store as an explicit flag of the creation command, except that a root whose `created_at` falls in a month earlier than
  the process's current month takes the legacy bucket, so that a dated root is only ever created in the current month and
  never lands in a month a retirement may be dropping; a child's from the parent run row it already holds under the run lock. A trigger on `runs`
  (before insert) MUST refuse a root whose bucket is neither legacy nor its month, a dated root whose month ended more
  than 24 hours before `clock_timestamp()` (a margin of a day over the process's rule, so only a clock wrong by a day can
  meet it), and, for a dated root whose month has ended at all, MUST take the bucket's advisory lock in shared mode and,
  in a separate statement once it is granted (the trigger function and `journal_retire_start` are `VOLATILE`, so each
  statement takes a fresh snapshot at `READ COMMITTED`, and in this branch, off the hot path, the trigger refuses a
  transaction at any other isolation level, as `journal_retire_start` does), refuse while `journal_bucket_retiring` says a retirement of the
  month is active, so that a retirement's start waits for
  such a creation and none can follow it; and a child whose bucket is not its
  root's (one lookup of the root by key, which the existing self-referencing foreign key guarantees exists); a trigger
  MUST refuse a change of `journal_bucket`, `created_at` or `root_run_id`. A legacy root is always accepted: one created by
  a process in the 12 s before it learns of the switch is processed by every release and purged run by run like any
  legacy run.
- **Requirement 049-002 One partition per run, enforced.** Every row of a partitioned journal table MUST carry its
  run's bucket. A trigger on `run_events` MUST refuse a row whose run, `(tenant_id, run_id)`, does not exist, has another
  bucket, or is purged (`purged_at` set), raising SQLSTATE 23503 as the foreign key it replaces did. A constraint trigger
  on `run_payloads`, checked at commit because the input payload precedes the run row, MUST refuse a row whose
  `root_run_id` is not a root of the same tenant and bucket, or whose root is purged. Both are `SECURITY DEFINER`
  functions (Requirement 049-016). The messages of every trigger and function of this design (these, the `runs` triggers
  of Requirement 049-001 and the reference triggers of Requirement 049-006 included) carry the SQLSTATE and the
  constraint or function name only, never a digest, a tenant or a run identifier.

### The tables

- **Requirement 049-003 `run_events` partitioned.** `run_events` MUST be partitioned by range of `journal_bucket`, one
  partition per month and one legacy partition. Its primary key becomes `(tenant_id, run_id, seq, journal_bucket)`, so
  that lookups by run keep their index prefix. Its checks (`run_events_seq_positive`, `run_events_trace_id_format`,
  `run_events_instance_identity_consistent`), `run_events_instance_idx` and the column default `'1970-01-01'` of
  `journal_bucket` are on the parent. The append-only and no-truncate triggers stay enabled always on every partition (the
  statement-level truncate trigger is created on each, since it is not inherited). The runtime role has `SELECT` and
  `INSERT` on the parent only, and no privilege on a partition.
- **Requirement 049-004 The conversion copies nothing.** The conversion of `run_events` runs in the stopped upgrade from
  0.2.x to 0.3.0, with every writer stopped, and MUST NOT copy a row: the old table becomes the legacy partition, and the
  only work proportional to its size is one build of its new unique index and one validation of a check. Its time is
  measured and published (Success criterion 049-003). If this design does not ship in 0.3.0, its migrations MUST be
  rewritten under design 053's online rules before they ship; this design does not plan that.
- **Requirement 049-005 Run payloads for new trees, owned by the tree.** For a run with a dated bucket, every payload a
  row of that run references MUST be stored in `run_payloads`, keyed `(tenant_id, root_run_id, digest, journal_bucket)`
  and partitioned like `run_events`. The runs of one tree MAY share payloads; a run MUST NOT reference a payload of
  another tree: a reset, a rerun, a root chain's next segment or a cross-namespace operation that starts from another
  tree's value writes its own copy in its own tree. A row that belongs to no run and holds a dated run's value (feature
  022's step cache and `test_run_pins`, `schedule_revisions.input_digest`, `batch_items.payload_digest`) copies it into
  `payloads`. Every read of a run's content
  MUST go through `read_run_content` in `crates/infra-postgres/src/payloads.rs`, which branches on the run's bucket
  (`run_payloads` by its key for a dated run, `payloads` for a legacy run or a tombstone); the static check of
  Requirement 049-011 refuses a statement naming `payloads` or `run_payloads` in any other file of
  `crates/infra-postgres/src`. A legacy run keeps writing and reading its payloads in `payloads`. `run_payloads` is
  content for the quota and has its delta and flush triggers from its creation.
- **Requirement 049-006 References are checked without foreign keys.** `crates/infra-postgres/run-content-references.json`,
  created by this design and read by feature 020, MUST list every column that references run content, foreign key or
  not, with the way the trigger reaches the run that owns the row (a column of the row, or one parent row named in the
  file), and the columns that hold run content but are not references (its exclusion list: digests kept as evidence of a
  value, never read back as content); the reference-trigger migration is generated from it and a test asserts that the
  two agree. Each listed column MUST be checked on insert, and on an update that changes it, its tenant or its owning
  run (`UPDATE OF` those columns `WHEN` one of them `IS DISTINCT FROM` its old value), by a trigger enabled always: the
  trigger reads the owning run's `root_run_id`, `journal_bucket` and `purged_at` by key, and then a legacy run's
  reference must exist in `payloads`, a dated run's in `run_payloads` under the run's root and bucket, and a purged run's
  stub may reference the tenant's tombstone. The trigger's lookup MUST take `FOR KEY SHARE` on the payload row it finds,
  as the built-in key check does, so that a concurrent delete of that payload waits for it and its own re-check fails.
  The trigger raises SQLSTATE 23503. A migration test MUST fail when a digest column of a run-owned table is in neither
  the registry nor its exclusion list, when any table has a foreign key to `payloads` other than
  `schedule_revisions.input_digest` and `batch_items.payload_digest` (the two shared
  columns, or when a column that belongs to no run references `run_payloads`. A second test over `pg_constraint` MUST fail
  when any foreign key has a relation of `pg_partition_tree('run_events')` or `pg_partition_tree('run_payloads')` on
  either side.
- **Requirement 049-007 Legacy purge.** For feature 020's purge of a legacy run, the reference check of gap 1099 MUST be
  served by partial indexes whose predicate selects the rows of legacy runs, delivered with the purge they serve (gated
  as it is). Every index of a table is opened on an insert, and an indexed column updated defeats a heap-only update, so
  their cost on new writes is not nil: Task 049-3 measures it, including the heap-only update ratio of `steps` and `runs`,
  before they are built. They are built in a migration of their own: in 0.3.0's stopped upgrade if Task 049-11 lands
  before the release is cut, or after it as design 053's concurrent migrations, one index per file. The purge deletes a
  candidate payload and then re-checks its references in a later statement under a savepoint, as the built-in key check
  does.
- **Requirement 049-008 Partitions exist ahead.** A definer function, `ensure_journal_buckets()`, MUST create the
  partitions of the previous month, the current month and the next three for every partitioned table (the previous one
  so that a root whose `created_at` falls just before a month's start, by the skew of two clocks, has its partition), each a standalone table created `LIKE`
  its parent (so that a column added later is present) with only its statement-level truncate trigger, checked by a
  constraint, then attached under a `lock_timeout` of 2 s; it sets the cloned row triggers enabled always and asserts it.
  It takes a transaction-level advisory lock, and the engine's housekeeping calls it at start and daily, and hourly after
  a failure. The gauge `journal_partition_months_ahead` MUST give the fewest months that exist ahead over the
  partitioned tables, and is not exported while no table is partitioned, and the alert
  `SagatJournalPartitionsLow` MUST fire below two. A row for a bucket with no partition fails with SQLSTATE 23514 and the
  partitioned table as the error's table (which distinguishes it from the PL/pgSQL guards that also raise 23514); the
  adapter maps it to code `UNAVAILABLE` with reason `JOURNAL_BUCKET_MISSING` (retryable) for a bucket at or after the
  current month or whose retirement has not dropped it, and to code `NOT_FOUND` with reason `RUN_PURGED` (the existing
  reason) only for a month whose table `journal_retirements` records as dropped or whose retirement is done, read on this
  error path through `journal_bucket_retiring`. The function skips a month whose retirement has recorded a drop or is done, so
  a retired month is never created again, and creates any other month, an abandoned one included. The new reason is added to
  `specs/001-platform-foundation/contracts/openapi.yaml`, the error contracts and the Studio's generated types in the same
  delivery. There is no default partition.

### Retention

- **Requirement 049-009 A month is dropped only when every run in it is purged.** A bucket MAY be retired only when every
  run of it is purgeable under feature 020 (terminal, its tree terminal and, for a chain, the chain final, past its
  namespace's hot retention, every sink applied or held past its hold, not `PARTIAL`, not `COMPENSATION_FAILED`, no
  unresolved outcome or review, and no leaf-success certificate, whose sources feature 020's decision D298 keeps without
  expiry) and its archive is `VERIFIED` (a run whose policy is `archive: NONE` holds its month, since constitution 5.1.0
  admits a deletion only after a verified archive). The legacy bucket is never retired: its
  payloads are shared in `payloads`, so its runs leave only by feature 020's per-run purge. Retirement MUST, in order:
  (1) record the retirement as active in `journal_retirements` in a transaction that first takes the bucket's advisory
  lock in exclusive mode, so that it waits for the per-run purges and the late dated creations in flight, which hold it in
  shared mode (Requirements 049-014 and 049-001), and only then, in a later statement, checks the bucket's runs; it
  refuses to run unless its transaction is at `READ COMMITTED`, so that the check's snapshot follows the lock;
  (2) purge each run of the bucket in the mark-only form of Requirement 049-014, finishing first any run a per-run purge
  left in `PURGING`; (3) check that no row of the bucket's partitions belongs to a run that is not purged, and compute
  each tenant's bytes in the bucket's `run_payloads` partition (summing `octet_length(content)` as the delta trigger
  does), recorded in `journal_retirements`: the partitions are frozen by then, since a purged run refuses every append and
  a past month gains rows only from the trees of its own roots, all purged; (4) for `run_events`, then for
  `run_payloads`, one transaction each, under a `lock_timeout` of 2 s: `LOCK TABLE ONLY` the parent `IN ACCESS EXCLUSIVE
  MODE`, then lock the bucket's partition alone with `NOWAIT` (so that a vacuum of the old month, which an
  anti-wraparound vacuum never yields, fails the attempt at once instead of holding the parent for the lock timeout),
  detach it (without `CONCURRENTLY`, which needs only catalog work here,
  since no foreign key and no default partition exist, but takes `ACCESS EXCLUSIVE` on the parent), drop it, and record
  the table dropped; (5) in a following transaction, release each tenant's recorded bytes through
  `sagat_quota_release_bytes` in tenant UUID order (Architecture decision 0027), write the audit records and mark the
  retirement done; it is idempotent on the retirement's state, so a crash between transactions leaves the content
  charged until the release runs on resume, never released twice. Every retirement function, the abandon's included,
  first locks the retirement's row `FOR UPDATE` and checks its state, so no two of them interleave. A step 4 transaction
  holds only that row, which no platform transaction locks, while it waits for its parent, and once it holds the parent no platform transaction can hold the partition (every platform statement
  reaches a partition through its parent), so it cannot deadlock with the hot path; only a maintenance process (a vacuum)
  can make it wait for the partition. A lock timeout rolls it back whole and it is retried after 1 s, doubling to at most
  30 s, at most 10 attempts, then the command stops and reports; nothing is ever detached and not dropped. A retirement
  is active only by its row, never by a lock alone. Until it is done, a purge or erasure report of feature 020 for one of
  its runs says that the content awaits the month's drop, and, once a retirement is `ABANDONED`, that the content of a
  tree with a run left unpurged awaits the purge of that run. An operator may abandon a retirement at any step before step 4
  (`--abandon`), for example when a run fails the mark-only form's re-check because its namespace's retention grew or a
  review opened. The abandon first commits the state `ABANDONING`, which every other retirement function refuses (a
  resume included), and is refused once a table is recorded dropped; it then deletes, by key and in chunks, under
  feature 020's authorization row (so the quota triggers release what it deletes), the `run_events` rows of every run of
  the bucket already purged and the `run_payloads` rows of every tree of the bucket whose runs are all purged, and marks
  the retirement `ABANDONED`, after which per-run purges of the bucket resume; a crash in between is finished by running
  the abandon again. A tree with a run left unpurged keeps its payloads until feature 020 purges that run, which, being
  its tree's last, deletes them. A later `journal_retire_start` for an abandoned month reactivates its row, resetting
  its recorded step and clearing its recorded bytes in the same transaction, so that step 3 counts them again and step 5
  never releases what the abandon's deletes already released.
- **Requirement 049-014 Feature 020's purge, by key, and its mark-only form.** Feature 020's purge of a dated run MUST
  delete that run's rows of `run_events` by `(tenant_id, run_id, journal_bucket)` with no reference check, and the purge
  of the tree's last unpurged run also deletes the tree's `run_payloads` rows by `(tenant_id, root_run_id,
  journal_bucket)`; whether it is the last is decided under the root's run row, which every purge of a dated run locks
  (`FOR NO KEY UPDATE`) before its own, so that two purges of one tree serialize and the second sees the first. Erasure of
  a dated run therefore erases its whole tree: it checks the caller's `retention:manage` permission and the eligibility
  of every run of the tree (none live, none with an unresolved effect, an open review or a leaf certificate that
  decision D298 keeps), and marks every run of the tree `PURGING` (feature 020's job state, under which its guard refuses
  a rerun, a review or an append) in the same transaction, which holds the runs' rows locked in the order of their depth, then
  identifier (a parent before its child, as the platform's writers lock them); only then do its purge chunks run. It erases nothing if one run is refused, and its report names every run erased or,
  when refused, the run that blocks and the content that survives (feature 020's requirement 020-037).
  The purge of a legacy run keeps feature 020's reference check (Requirement 049-007). Every purge of a dated run but the
  mark-only form runs at `READ COMMITTED`, takes the bucket's advisory lock in shared mode for its transaction and then,
  in a separate statement once the lock is granted (so that its snapshot follows a retirement's commit), refuses with a retryable error while a
  retirement of the bucket is active (Requirement 049-009); the mark-only form runs under the retirement's own row. The *mark-only form*, used by
  retirement, is `purge_run_content_mark_only`, an owner-only function that shares feature 020's purge body (its
  authorization row and its `run.purged` audit record per run), runs under the run lock, is called only by
  `journal_retire_purge_chunk` (which `sagat_app` may not execute and which checks that its bucket's retirement row is
  active), and differs from feature 020's purge only in leaving the run's partitioned rows for the drop: it re-checks
  eligibility, writes the stub (tombstone digests, the `NOT NULL` columns kept, so an older reader reads a stub as
  feature 020 designed), deletes the run's rows of the unpartitioned run-owned tables, its run-purpose blobs that no
  other run references (feature 020's blob check, since a rerun shares its source's input blob) and its step-log
  streams (Architecture decision 0084, decision 4). No purge disables a trigger: a delete passes the append-only
  triggers only through feature 020's authorization row (`retention_purge_authorizations`, keyed by the transaction,
  with no runtime grant), which the triggers of `run_payloads` and of every partition consult as those of `run_events`
  and `payloads` do. A `run_payloads` row may be deleted only under an authorization row of a run of its tree, and only
  once every run of that tree is purged; the abandon of Requirement 049-009 writes its authorization rows through
  `authorize_abandon_deletes`, an owner-only function that only `journal_retire_abandon` calls and that checks the
  retirement's state is `ABANDONING`, and audits its deletes per tenant; the purge function `sagat_app` executes cannot
  write them. A purged run refuses any later
  append, and every read of its journal or content answers `RUN_PURGED` without serving a partition's row, so that rows
  awaiting the drop are never served: per-run reads of `run_events` that return journal rows go through the functions
  of `crates/infra-postgres/src/journal.rs`, one per read shape, each of which checks `purged_at` by a join on `runs` in
  the same statement, so that a lease or a completion gains no statement (a statement that reads only a row's time, as
  `mark_projected` does, returns no content and needs no check, and takes its bucket, which never changes, from a read
  made earlier and carried with it), as content reads go through `read_run_content`, and the
  static check of Requirement 049-011 refuses a per-run read of `run_events` in any other file. The rows that refer to content across runs belong to the run that holds the row: a chain's
  operation receipt to its `resolved_run_id`, a segment's `transfer_input_digest` to the segment's run.
- **Requirement 049-015 Features that will reference run content follow the rule.** The approved designs that plan new
  references to run content (feature 022: `test_run_pins.payload_digest` and the step cache's obligations; feature 026:
  `approval_requests.summary_digest`, `approval_decision_receipts.input_digest`; feature 030: the inputs and results of
  `service_operations`, each run tree with its own copy; feature 025: transcript bodies) MUST follow Requirements 049-005
  and 049-006; their data models are amended with this design's delivery, and so is feature 020's (the mark-only form,
  the tree as the unit of payload deletion and of erasure, the key of the purge, the registry).

### The switch, statements and the hot path

- **Requirement 049-010 The switch.** New roots get a dated bucket only once the process has learned, from the instance
  registry, that the capability `journal-buckets-v1` is active; the activation is `control-plane journal switch`, which
  first calls `ensure_journal_buckets()`. A process that has not learned of it yet creates legacy roots, which are valid
  (Requirement 049-001), so no row lock, statement or clock comparison serializes a creation with the switch. Before the
  switch, every run is legacy. The capability is never deactivated in 0.3.0: design 053's drain check for it would be "no
  run of a dated bucket exists", which holds only on a database that never used it.
- **Requirement 049-011 Every statement names its bucket.** Every statement for one run that reads or writes a
  partitioned journal table MUST compare `journal_bucket` by equality with a bind parameter or a literal; the bucket
  comes from the run row the transaction holds or has read, and every lease and completion already reads the run row.
  A statement that spans runs (a scheduler pass such as `collection_candidates` or `run_result` in `maintenance.rs`) MAY
  compare it with a joined column, opens every partition, and is allowed only off the lease and completion paths, by
  name in the check's allow-list with the pass that runs it, and only if it joins `runs` with `purged_at IS NULL`, which
  the check verifies; Task 049-3 counts its locks. A static check,
  `scripts/check-journal-buckets.py`, over the SQL literals of `crates/infra-postgres/src` and the function bodies of the
  migrations after this design's first one, MUST fail on a statement that touches a partitioned journal table without
  the comparison and is not allowed, on SQL built at run time (`format!` and the like) that names one, and on a statement
  naming `payloads` or `run_payloads` outside `payloads.rs` (Requirement 049-005). A migration test applies the same rule
  to the bodies of the functions in `pg_proc` that name a partitioned journal table.
- **Requirement 049-012 The hot path is budgeted.** Design 038's budget test MUST record, besides statements,
  transactions and rows written, the distinct relations locked during a lease and a completion (read or written) and the
  round trips, in the change that partitions each table. Partitioning MUST NOT change the statement count of a lease or a
  completion, and MUST NOT raise the relations locked by more than one partition and its indexes per partitioned table
  the operation reads or writes, plus the `runs` lookup of each reference trigger, each counted and named in the
  budget's line.
- **Requirement 049-013 The catalog checks know partitions.** The storage-classification test MUST map a partition to its
  parent; the startup privilege check and the runtime-privilege audit MUST list partitions through `pg_inherits` and
  fail on a partition with any grant to the runtime role, with a required trigger missing, duplicated or not enabled
  always, on a function of this design whose `proacl`, `prosecdef` or `proconfig` (its pinned `search_path`, `TimeZone`
  and `DateStyle`, compared as a set) differs from this design's list. The names on the list are generated from the
  functions this design's migrations create, so a function they add cannot be left off it; the expected attributes are
  a hand-written table (`crates/infra-postgres/journal-functions.toml`, default: executable by the owner only), never
  taken from the migrations' own grants, and a generated name with no entry in the table fails. The audit reads the
  effective grants, `aclexplode(coalesce(proacl, acldefault('f', proowner)))`, so that a function whose `PUBLIC` grant
  was never revoked (a null `proacl`) fails, compares grantees by identifier with `0` as `PUBLIC` (never through an inner
  join to the role catalog, which would drop `PUBLIC`) together with the privilege and `is_grantable`, which must be
  false for every grantee but the owner, and requires `proowner` to be the owner role; negative fixtures with a null
  `proacl` and with a grant `WITH GRANT OPTION` must fail it. Feature 020's purge
  function, which other migrations create, has its entry by name.
- **Requirement 049-016 Privileges.** The runtime role keeps `SELECT, INSERT` on `run_events`, `run_payloads` and
  `payloads` and gains no `UPDATE`: the reference and bucket triggers, which lock rows `FOR KEY SHARE`, are `SECURITY
  DEFINER` functions owned by the owner role with `search_path` set to `pg_catalog, pg_temp` and every name qualified,
  revoked from `PUBLIC`. Every function of this design also sets `TimeZone` to `UTC` and `DateStyle` to `ISO`, so that
  its month arithmetic and its lock keys do not depend on a session's or a role's settings. `journal_retirements` has no grant to the runtime role; a purge learns whether a retirement is
  active through the definer function `journal_bucket_retiring(bucket)`. Retirement runs as `sagat_retirer`, a login role
  with no table privilege, which may only execute the definer functions of retirement (`journal_retire_start`,
  `journal_retire_purge_chunk`, which calls `purge_run_content_mark_only`, `journal_retire_check`,
  `journal_retire_drop`, `journal_retire_release`, `journal_retire_abandon`), owned by the owner; the platform's processes
  and the chart hold no credential of either role. `sagat_quota_release_bytes` is executable by those functions' owner
  only. `journal_retire_start` itself, not only the command, refuses the legacy bucket and any month that ended less than
  24 hours ago by the database clock, since the `runs` trigger of Requirement 049-001 refuses a dated root in such a
  month after that and serializes with the start before it; `journal_retire_drop` accepts only `run_events` or `run_payloads` as its table and confirms the
  partition through `pg_partition_tree` before it drops; the functions build partition names from the date with `%I`
  quoting. The pre-drop report of sessions comes from an owner-owned definer function that returns only the process
  identifier, user, application name and transaction start, so `sagat_retirer` needs no `pg_read_all_stats`; it reports
  the transaction start as unknown for a session it cannot see, which happens unless the operator has granted
  `pg_read_all_stats` to the owner role, as the runbook says (no migration grants a predefined role, which managed
  services often refuse). `EXECUTE` is
  granted per role, and Requirement 049-013's audit compares each function's effective grants with this list: `sagat_app` may
  execute `ensure_journal_buckets`, `journal_bucket_retiring` and feature 020's purge function; `sagat_retirer` the
  retirement functions and the sessions function; `purge_run_content_mark_only`, `authorize_abandon_deletes`,
  `sagat_quota_release_bytes` and the trigger functions of Requirements 049-001, 049-002 and 049-006 are executable by
  their owner only; no one else any of them. The grants of design 053's `sagat_schema_handshake()`, which `sagat_app` and
  `sagat_retirer` both execute (Requirement 053-005), are that design's and outside this list. A tenant's audit record of a retirement names
  the platform's retirement actor; the operator's identity is kept in `journal_retirements` only.

### Success criteria

- **Success criterion 049-001** After the switch, the budget test's statement counts for a lease and both completions
  (with and without a following step) equal the counts before partitioning, and the relations locked rise by at most what
  Requirement 049-012 allows.
- **Success criterion 049-002** On a free standard runner of GitHub (Owner decision 2026-11-25-1), on a database holding
  12 monthly buckets of 20,000 runs of the chain shape each, the retirement of one bucket is measured end to end (the
  mark-only purge of its runs included) and its last transaction alone, each at the median and the maximum of five
  repetitions; during each transaction of step 4 no statement on its table waits more than that transaction's wait for
  its parent, at most the 2 s lock timeout (the partition's lock is taken `NOWAIT`, so it adds no wait), plus its
  measured catalog time (the detach and the drop), as the driver's maximum statement latency on that table shows. The report names the runner.
- **Success criterion 049-003** The conversion of a `run_events` of 5 GiB on the same runner copies no row (the legacy
  partition keeps the old table's `relfilenode`, and the relation sizes before and after are equal), and its time and the
  time of its index build are published with the runner named.
- **Success criterion 049-004** After a retirement, every affected tenant's content usage equals a fresh reconciliation of
  its rows.

### User stories

- **User story 049-01** An operator whose namespaces all keep 90 days of history sees the journal stop growing after 90
  days plus one month, without a nightly purge that fights the workload.
- **User story 049-02** A tenant whose data must be erased has its run trees purged one by one, at a cost proportional to
  its runs, not to everyone's.
- **User story 049-03** A developer adding a statement on the journal learns at once, from a check, that it must name
  its bucket.

## Goals

- A per-run purge of every run created after the switch whose cost is the run's own rows, and the tree's payloads once.
- Retention that costs three short transactions per month (one drop per partitioned table, then the release) where every namespace with runs in that month has a policy no
  longer than the month's age; elsewhere, the per-run purge.
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
- **Converting `run_events` online.** Requirement 049-004.
- **A scheduled retirement job.** Retirement is an operator's command (Requirement 049-016).

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6, with its reasoning; the acceptance of the decision
record is under Owner decision 2026-11-24-4 (pull request 274).

1. **What is the partition key?** The UTC month of the root run's `created_at`, stored on every run of the tree and
   computed once by the creating transaction. The run identifier is not time-ordered for child runs; the event's time
   splits runs. Delegate.
2. **Which width?** One calendar month. With hot retention R days and runs that end within the month they start, the
   database holds about `ceil(R / 30) + 1 + 3` buckets (the retained months, the current one, three ahead); 90 days gives
   3 + 1 + 3 = 7 and 365 days gives 13 + 1 + 3 = 17; a run that lives longer holds its month longer. A week would multiply
   the partitions, and so the planning and locking of a statement that is not pruned, by about four. Delegate.
3. **Deduplicate payloads within a run, a run tree, a bucket, or a tenant?** Within a run tree, for new runs: a tree is
   eligible for retention only as a whole (feature 020), so its runs may share payloads without a cross-run check, and a
   fan-out's children need no copies of their inputs, so runs in flight do not grow in quota use after the switch. Within
   a bucket would bring the reference check back for a single tree's purge. The cost is a copy where two trees hold the
   same bytes, which the content quota counts; Task 049-2 measures how often that happens. Delegate.
4. **Copy the legacy payloads, or leave them?** Leave them. A copy would double-charge the quota until a cleanup that
   deletes journal rows, and needs the census twice; leaving them costs feature 020's reference check for legacy runs
   only, on partial indexes whose cost on new writes Task 049-3 measures (Requirement 049-007). Delegate.
5. **Foreign keys or triggers?** Triggers that raise the same SQLSTATE. A foreign key into a partitioned table makes
   every detach scan the referencing tables under a lock that blocks their writes; one out of a partition makes its drop
   lock `runs`. The cost is a trigger call and a `runs` lookup per write instead of the built-in check, which Task 049-3
   measures. Delegate.
6. **Partition `steps`?** Not in 0.3.0. The scheduler's passes read its partial indexes without a run identifier, so
   every partition would be opened, and its rows are few per run. Delegate; a measurement after design 052's queue is a
   follow-up task.
7. **A default partition as a safety net?** No: it makes every later attach scan it, and rows that should fail would
   land there silently. Delegate.
8. **How are statements planned so that only one partition is locked?** Decided by Task 049-3, among four: statements on
   partitioned tables sent unnamed (planned with their parameters, one more round trip each); the runtime role's
   `plan_cache_mode` set to `force_custom_plan`; the same setting on the trigger functions only (`ALTER FUNCTION ... SET
   plan_cache_mode`), which reaches their cached plans without a role-wide planning cost; or the plans as they are, if
   the measured locks stay within Requirement 049-012. The decision rule: the choice with the lowest median completion
   latency among those that meet Requirement 049-012 on PostgreSQL 16 and 18. This design is amended with the choice
   before Task 049-8 starts. Delegate.
9. **Concurrent or plain detach?** Plain, in the transaction that drops: with no foreign key and no default partition it
   is catalog work, it waits at most the lock timeout, and no table is ever left detached and not dropped, so no state
   between the two needs a recovery path. A concurrent detach cannot run in a transaction or a function, so it would need
   the owner's credential in the command and a resumable state between detach and drop. Delegate.
10. **The decision record.**
    [Architecture decision 0132](../adr/ADR-0132-the-journal-is-partitioned-by-the-month-a-run-was-created-and-a-run-owns-its-payloads.md)
    records the key, the run-owned payloads of new trees, the triggers in place of foreign keys and the rule for dropping
    a month; it refines Architecture decision 0054 in part (its decisions 4 and 6, and its rejected option 3 for whole
    months only).

## Owner decisions needed

- **The amendment of principle I: decided.** Retirement deletes journal rows, as feature 020's purge does. The
  coordinating session kept it out of 5.0.0 (Owner decision 2026-11-24-511), and the owner approved it for constitution
  5.1.0 (Owner decision 2026-11-25-3, point 1): two audited ways to delete journal rows, only after a verified archive,
  feature 020's per-run purge under a namespace retention policy and the retirement of a whole month partition of this
  design once every run in it is purged. The coordinating session drafts the text and three reviewers read it; nothing is
  left for the owner. Its merge gates Tasks 049-11 and 049-14; Task 049-18 checks the merged text against this design.
- **Scope.** Owner decision 2026-11-24-1 (pull request 273) names "partitioning of the journal tables" among the new
  designs of release 0.3.0; the delegation covers its open questions.

## Critical User Journeys (CUJs)

### CUJ 1: A month is retired

*   **Given**: every namespace with runs in bucket 2026-12 has 90 days of hot retention and archives required; the
    bucket's runs all ended before 2027-01-01 and were archived and verified.
*   **When**: the operator runs `control-plane journal retire --bucket 2026-12` after 2027-04-01.
*   **Then**: it records the retirement, purges each run of the bucket in the mark-only form, checks the partitions and
    counts their bytes, then detaches and drops the December partition of `run_events` in one transaction and that of
    `run_payloads` in another, and releases the bytes in a third; a statement on either table never waits more than a
    lock timeout plus that table's drop's catalog time.

### CUJ 2: A run that lives a year

*   **Given**: one run of bucket 2026-12 still waiting for an approval in 2027-06.
*   **When**: the operator's retirement reaches December's bucket.
*   **Then**: it stops before recording anything and prints one held run of reason `LIVE`; feature 020
    purges every other run of the bucket one by one, by its key; when the run ends and becomes purgeable, the bucket
    can be retired.

### CUJ 3: The upgrade

*   **Given**: a database of release 0.2.x with 100 GiB of `run_events`.
*   **When**: the operator stops the writers and runs 0.3.0's migrations, as the stopped upgrade requires.
*   **Then**: `run_events` becomes a partitioned table whose legacy partition is the old table, after one index build and
    with no row copied; the foreign keys become triggers that raise the same SQLSTATE; new runs stay legacy until the
    operator runs `control-plane journal switch`.

### CUJ 4: A statement without its bucket

*   **Given**: a change that adds `SELECT ... FROM run_events WHERE tenant_id = $1 AND run_id = $2`.
*   **When**: its pull request runs the static check.
*   **Then**: the check fails and names the statement; the author adds `AND journal_bucket = $3`.

## Technical Plan (Architecture)

### Layout

| Table | Partitioned | Key | Primary key |
|---|---|---|---|
| `run_events` | yes, by range of `journal_bucket` | the run tree's month | `(tenant_id, run_id, seq, journal_bucket)` |
| `run_payloads` (new) | yes, by range of `journal_bucket` | the run tree's month | `(tenant_id, root_run_id, digest, journal_bucket)` |
| `payloads` | no | | `(tenant_id, digest)`: legacy runs' payloads, the two shared columns, values copied by rows of no run, the tombstones |
| `runs` | no | | `id`, with `journal_bucket` (and feature 020's stub columns) |
| `steps` and the other run-owned tables | no | | unchanged; their reference triggers read the bucket from `runs` |
| `audit_records` | no | | unchanged |
| `journal_retirements` (new) | no | | `(bucket)`: state, per-tenant bytes, actor (database user and command identity), times |

Partition names are `<table>_b<YYYYMM>` and `<table>_legacy`. Bucket code lives in a new
`crates/infra-postgres/src/journal_buckets.rs`, not in `partitioning.rs`, which holds the scheduler's shards.

### The migrations of the stopped upgrade

Every migration of this design that runs in 0.3.0's stopped upgrade is transactional and carries the waiver
`-- lint-allow: stopped-upgrade <reason>` of design 053 (Architecture decision 0131, decision 2) for each rule it
breaks: the index builds that are not concurrent (the new unique index of `run_events` and the index on
`runs (journal_bucket)`), the several locks that block inserts in one migration, a unique index on an existing table
(design 053's contract rule), the constraint drops, the rename, the replaced primary key and the dropped triggers, all of
which break design 053's rules only because they assume writers that are running. Design 053's lint (its Task 053-2) is a prerequisite of
Task 049-8, so that each waiver is checked. Each migration has a `down` that restores the previous schema (the foreign
keys recreated `NOT VALID` and validated), refused once a dated row exists. In order:

1. **Task 049-8, `run_events`.** Add `journal_bucket date NOT NULL DEFAULT '1970-01-01'` to `runs` and `run_events` (a
   constant default: no rewrite), and an index on `runs (journal_bucket)`; build `CREATE UNIQUE INDEX
   run_events_bucket_key ON run_events (tenant_id, run_id, seq, journal_bucket)`; replace the primary key with `ALTER
   TABLE run_events DROP CONSTRAINT run_events_pkey, ADD CONSTRAINT run_events_legacy_pkey PRIMARY KEY USING INDEX
   run_events_bucket_key` (no scan: `journal_bucket` is `NOT NULL`); add `run_events_legacy_bucket CHECK (journal_bucket =
   '1970-01-01')` and validate it; drop `run_events`' foreign keys to `runs` and to `payloads`, replaced in the same migration by the bucket trigger of
   Requirement 049-002 and the reference trigger of `run_events.payload_digest`, written by hand with its legacy branch
   only (no run is dated before the switch, which comes after step 3) and regenerated from the registry in step 3; rename `run_events` to
   `run_events_legacy`, its triggers to `*_legacy` and its indexes (`run_events_instance_idx` to
   `run_events_legacy_instance_idx`; an attach matches indexes by definition, not name); create the partitioned
   `run_events` with the same columns, the default, the primary key of Requirement 049-003, its three checks,
   `run_events_instance_idx` and its triggers; attach `run_events_legacy` `FOR VALUES FROM (MINVALUE) TO ('1970-01-02')`
   (no scan: the validated check proves the range; no index build: the indexes exist with matching definitions); drop
   the legacy table's renamed row triggers, now duplicated by the cloned ones (its statement-level truncate trigger is
   kept, since it is not cloned), and set the cloned triggers enabled always; revoke the runtime role's grants on
   `run_events_legacy` and grant `SELECT, INSERT` on the parent; call `ensure_journal_buckets()` (Task 049-9, merged
   first), which creates the partitions of the previous, current and next three months.
2. **Task 049-5, `runs`.** The bucket and immutability triggers of Requirement 049-001. A refusal of a bucket that does
   not match its rule raises SQLSTATE 23514 with the constraint name `runs_journal_bucket`, which the adapter maps to an
   internal error, since only a defect in the platform's bucket computation can cause it; the refusals of the
   ended-month branch (a retirement active, an isolation other than `READ COMMITTED`) raise it with
   `runs_journal_bucket_retiring` and `runs_journal_bucket_isolation`, mapped to `UNAVAILABLE` (retryable: a retry
   succeeds) and to an internal error respectively.
3. **Task 049-10, `run_payloads` and the references.** Create `run_payloads`, partitioned like `run_events`, with the
   append-only, no-truncate, quota delta and flush triggers, the constraint trigger of Requirement 049-002, grants `SELECT,
   INSERT` on the parent only, and its storage classification as content; create the reference triggers of Requirement
   049-006 from the registry, setting the clone of each on every existing partition (`run_events_legacy` and the months
   of step 1) enabled always; then drop the 19 remaining foreign keys to `payloads` of run-owned tables that they replace
   (the twentieth, `run_events`', went in step 1); then call `ensure_journal_buckets()` so that `run_payloads` has its
   partitions in a database that is only migrated. Until this step the reference trigger of `run_events.payload_digest`
   checks `payloads` for every run.
4. **Task 049-13.** Add `journal-buckets-v1` to the registry's closed capability lists, as design 053 adds a capability.

Before the switch nothing writes `run_payloads`; after it, dated trees write it and only it.

### The switch

`control-plane journal switch`, as the operator: `ensure_journal_buckets()`, then the registry's activation of
`journal-buckets-v1` (which refuses while a live process lacks it). Each process learns of it at its next heartbeat and
from then creates dated roots. The activation is recorded in the registry's history; no table of this design records it.

### Locks and plans

A statement planned with the bucket's value prunes to one partition and locks it and its indexes. How plans are made is
Open Question 8, decided by Task 049-3 with the rule stated there, before Task 049-8. The relations a transaction locks,
the triggers' statements included, are counted by the budget (Requirement 049-012), through a probe behind the
`test-probes` feature of `crates/infra-postgres`, off in every build but the budget's (a CI step refuses it in `cargo
tree -e features` of each application's binary, since a workspace build could unify it on): at the commit of the two
operations the budget measures (the claim's transaction in `crates/infra-postgres/src/dispatch.rs` and the completion's
in `execution.rs`), the hook runs `SET CONSTRAINTS ALL IMMEDIATE`, then waits on a barrier while the test reads
`pg_locks` for that backend from a second connection, then commits. Round trips are counted as the budget already counts
statements, from `pg_stat_statements` (`work_budget.rs`), plus the transaction's `BEGIN` and `COMMIT`; the probe's own
statements carry the comment `/* budget-probe */`, which the budget's count excludes.

### Retirement

`control-plane journal retire --bucket YYYY-MM [--abandon]`, run by an operator, resuming a retirement from its recorded
step, as `sagat_retirer` (Requirement
049-016), never by the platform's processes. Each step is a definer function, so the command holds no privilege beyond
calling them:

1. `journal_retire_start(bucket)`: refuse a transaction not at `READ COMMITTED`, the legacy bucket and a month that
   ended less than 24 hours ago; take the
   bucket's advisory lock in exclusive mode; then, in a later statement, check every run of the bucket (on `runs
   (journal_bucket)`), and stop if any is not purgeable, printing the held runs counted by reason (`NO_POLICY`,
   `NOT_ELIGIBLE`, `NOT_VERIFIED`, `LIVE`); otherwise insert the active row and commit.
2. `journal_retire_purge_chunk(bucket)`, called until it reports none left: purge runs in the mark-only form, in chunks,
   each under its run lock, through feature 020's purge function; it stops and reports a run that fails the re-check.
3. `journal_retire_check(bucket)`: verify that no row of the bucket's partitions belongs to a run that is not purged,
   and record each tenant's bytes of the bucket's `run_payloads` partition.
4. `journal_retire_drop(bucket, table)`, for `run_events` then `run_payloads`: the transactions of Requirement 049-009
   step 4: `SET LOCAL lock_timeout = '2s'`, `LOCK TABLE ONLY` the parent, lock the bucket's partition `NOWAIT`, detach and drop it,
   and record the table dropped. It holds no tenant's usage row, so its exclusive locks last only for catalog work.
5. `journal_retire_release(bucket)`: release each tenant's recorded bytes in tenant UUID order, write one audit record
   per tenant with that tenant's counts only and the platform's retirement actor, record the retirement's own audit in `journal_retirements`
   (an audit record needs a tenant), and mark it done; a second call on a done retirement does nothing.

Before step 4 the command lists, through the sessions function of Requirement 049-016, the sessions that hold a lock on
either parent and are older than 60 s, printing their process identifier, user, application name and transaction start
only, never their query. A wait on the bucket's advisory lock is printed with its duration and the holder's process
identifier and role, since any role can take an advisory lock; the command is run by an operator, so it prints what a
long-running process would export, and records the waits in the retirement's row once step 1 has created it; the held
runs of a refused step 1 are printed only, since no row exists then. The bucket's advisory lock key is
`hashtextextended('sagat.journal_bucket:' || to_char(bucket, 'YYYY-MM-DD'), 0)` and that of `ensure_journal_buckets()`
`hashtextextended('sagat.ensure_journal_buckets', 0)`.

### What is not partitioned, and why

- **`runs`.** A purged run keeps its stub (feature 020, owner question 1082), and its queries by status and by shard read
  across runs of every age.
- **`steps`.** Open Question 6.
- **`audit_records`.** No audit record may be dropped, and its key `(id)` would have to become `(id, created_at)`,
  losing the uniqueness of `id` that `crates/infra-postgres/src/chain_progress.rs` relies on.
- **The outbox, the inbox, the deduplication table and the webhook deliveries.** They are deleted by age already.

### Interactions

- **Feature 020** builds the purge, the archive, erasure and the purge authorization; this design defines the registry
  file, the key of the purge of a dated run, the tree as the unit of payload deletion, and the mark-only form, and amends
  feature 020's data model and tasks (Task 049-15), which lands before Task 049-10, so that feature 020's Task T020-046
  finds references in the registry rather than in catalog foreign keys. Until feature 020's Task T020-045 adds
  `runs.purged_at`, the triggers have no purged branch; Task T020-045 adds it to them. Tasks 049-11 and 049-14 run after
  feature 020's Tasks T020-045 and T020-046. Feature 020's own task for the amendment of principle I (T020-061) is
  answered by constitution 5.1.0.
- **Feature 032** adds `run_events.region_epoch` (Architecture decision 0067, decision 9). A column added to the
  partitioned parent is added to every partition; whichever design lands second writes its migration against the
  other's schema; `ensure_journal_buckets()` runs only in the active region.
- **Design 053.** Its lint checks the waivers here; after 0.3.0 every migration of this design (the legacy indexes of
  Task 049-11 among them) follows its online rules.
- **Designs 051 and 052.** Design 051's shapes (its Task 051-1), driver (Task 051-2) and history loader (Task 051-3) are
  used by Tasks 049-2, 049-8 and 049-14; design 052's queue is not partitioned.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A statement forgets the bucket and opens every partition | The static check; the lock count in the budget |
| A run's rows land in two buckets | The bucket is computed once and bound; the trigger on `runs` makes a child take its root's bucket; the bucket triggers refuse a mismatch; the suite after the switch drives both clocks across a month boundary |
| A reference points to a payload that is not there | The reference triggers on every registered column; the registry tests |
| No partition for next month | Created three months ahead daily; an alert below two; a missing one fails loudly and retryably |
| A trigger per write costs more than the key check it replaces | Task 049-3 measures both; the budget records locks and round trips |
| A long run holds a month | The rest of the month is purged by run; held runs are a metric with a reason |
| Retirement races an append or a per-run purge | Runs are purged under their run lock before the drop; a purged run refuses appends; the bucket's advisory lock and the active row exclude per-run purges; the check before the drop |
| A drop does not release the tenant's quota | Bytes counted from the frozen partition by an owner-only function and released by the idempotent transaction that follows the drops; Success criterion 049-004 |
| A credential for retirement is stolen | `sagat_retirer` can only call the retirement functions; no platform pod or chart holds it |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass with conditions | The partitioning keeps the journal append-only with a monotonic per-run sequence and a tree's rows together; retirement deletes journal rows and waits for the merge of constitution 5.1.0, whose principle I admits it (Owner decision 2026-11-25-3) |
| II. Modular domain, ports and adapters | Pass | `journal_bucket` stays in `infra-postgres`'s row types and never enters the domain's run type; the application, scheduler and control-plane changes (Tasks 049-5, 049-9, 049-13 and 049-14) only call the store; the gate's `is_active` reads the requirement set the gate keeps from every successful registration and every epoch move (the heartbeat returns the set when the epoch the process sends differs, design 053) |
| III. External effects are at-least-once; sagas compensate | n/a | No effect changes |
| IV. Compatibility is versioned and pinned | Pass | The conversion in the stopped upgrade; triggers that raise the same SQLSTATE as the keys they replace; late legacy roots valid; the switch is a registry activation |
| V. Code, tenant boundary and secrets are isolated | Pass | Every key starts with `tenant_id`; no privilege on a partition; definer functions with a pinned path and no `PUBLIC` grant; retirement's role can only call its functions; messages carry no value |
| VI. Observability and audit are part of the feature | Pass | Months ahead, held runs by reason, lock waits, retirement steps; each retirement audited per tenant |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | Not touched |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Crash at each retirement step, a lock timeout in the drop, a missing partition, an append and a per-run purge racing a retirement, a suite after the switch across a month boundary; every measurement names its runner |
| Delivery Constraints | Pass | PostgreSQL stays the only authority; nothing is dropped that feature 020 has not archived and verified |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| `journal_bucket` on `runs` and the partitioned tables | The key must be on the run, known to every writer, and the same for a whole tree | The run identifier: child identifiers are hashes; the event's time: splits runs |
| `run_payloads` for new trees | A tree must own its payloads for its purge, or a month's drop, to need no reference check | Keeping `payloads` shared for every run: feature 020's 22 reference indexes on the hottest tables |
| Triggers in place of foreign keys | Foreign keys make detaches scan and lock the referencing tables, and drops lock `runs` | Foreign keys: the drop path would block the hot path |
| A deletion of journal rows by month | Retention at the size of a month's data | Waiting for the amendment before the partitioning: only retirement waits |

## Alternatives Considered

- **Delete per run only** (feature 020 as drafted). Correct, and the main path here. A month's drop removes the rows of
  its two largest tables, `run_events` and `run_payloads`, in one transaction, while the mark-only form still deletes
  each run's rows of the unpartitioned tables; it applies only to months whose every namespace has a policy, which the
  default (no policy) never is. Task 049-2 measures the share of a run's bytes in the two tables, which is what the drop
  saves.
- **Copy the legacy payloads into `run_payloads`.** One regime instead of two, at the price of a double charge until a
  cleanup that deletes journal rows, a move job and two censuses.
- **Partition by event time.** Splits a long run's events and cannot drop a month while any run that wrote to it lives.
- **Partition by hash of the tenant, or by retention class.** The first drops nothing and does not follow retention,
  which is per namespace; the second moves a namespace's runs when its policy changes.
- **Foreign keys to `run_payloads`.** Integrity by the database's own mechanism, at the price of a `SHARE` scan of every
  referencing table at each detach.
- **The `pg_partman` extension.** An extension the managed PostgreSQL targets may not offer, for a creation rule of a
  few statements.
- **A lock on a switch row in every creation** (this design's earlier draft). Every creation would share-lock one row,
  making a multixact each time, and the bucket is needed before the first write; learning the switch from the registry
  and accepting late legacy roots needs neither.

## Non-Functional Requirements

- **Non-functional requirement 049-001 Hot path.** As Requirement 049-012 and Success criterion 049-001.
- **Non-functional requirement 049-002 Upgrade time.** As Requirement 049-004 and Success criterion 049-003.
- **Non-functional requirement 049-003 Security.** As Requirement 049-016.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Tasks 049-11 and 049-14 wait for constitution 5.1.0 to merge (their Owner-gate lines); the others are release 0.3.0 and
wait for nothing but their order. Tasks 049-3 and 049-4 come first (the plan choice and the budget's new lines that
Task 049-8 is measured against); Task 049-9 merges first among the schema tasks, then 049-8 with 049-7, 049-5, 049-15, 049-10,
049-12 and 049-13, with design 053's Task 053-2 before Task 049-9's migration, its Task 053-3 before Tasks
049-5 and 049-14, and its Task 053-5 before Task 049-13; Task 049-6 merges before 049-8 with a baseline per rule, each part emptied by the task that
moves its statements (the bucket rule and the `journal.rs` rule by Task 049-8, the `payloads.rs` rule by Task 049-10).
Each task's files are named; the test suites are those of the Verification Plan.

- [ ] T049-1 Revise and accept Architecture decision 0132 to this text, with its checks in "Enforced by"; write the refinement notes in Architecture decisions 0027, 0054 and 0131, a dated section of `specs/020-visibility-analytics-archival/research.md`, the layout in `docs/architecture/storage-and-events.md`, and the new checks' rows in `docs/engineering/architecture-principles.md`.
  Refs: Requirement 049-001, Requirement 049-005, Requirement 049-014
- [ ] T049-2 Measure, with design 051's shapes and history loader (its Tasks 051-1 and 051-3), the share of payload bytes referenced by more than one tree and the share of a run's bytes in `run_events` and payloads, and record them in `tests/performance/reports/journal-partitioning.md`.
  Refs: Requirement 049-005
- [ ] T049-3 Add `SAGAT_TEST_POSTGRES_IMAGE`, an override of the database image of the test support for an image built in this repository from the pinned PostgreSQL image with libfaketime built from a fixed source commit checked by its checksum (`FAKETIME_TIMESTAMP_FILE` with `FAKETIME_NO_CACHE`, a clock file the test rewrites), started as a server no other test shares (design 041's `isolated_server`), so that the tests of Tasks 049-5 and 049-14 move the database clock across a month boundary and into the 24-hour window; use design 041's `SAGAT_TEST_POSTGRES_TAG` for the version, in the database image of `tests/support/mod.rs` and `tests/support/bounded.rs` (default `16-alpine`); then measure, in a new `perf_journal_partitioning` target, locks, round trips, planning time and completion latency with 1, 6, 16 and 60 partitions for the four planning choices, for triggers against key checks, and for the statements that span runs, on PostgreSQL 16 and 18; choose by Open Question 8's rule and amend this design.
  Refs: Requirement 049-006, Requirement 049-011, Requirement 049-012
- [ ] T049-4 Add the `test-probes` feature of `crates/infra-postgres` with the probe of "Locks and plans" and the step of `.github/workflows/ci.yml` that refuses it in `cargo tree -e features` of each application, and to `tests/integration/claim-transition-efficiency/work_budget.rs` the relations locked and the round trips, at today's values.
  Refs: Requirement 049-012
- [ ] T049-5 Add `ServingGate::is_active(Capability)` in `crates/application/src/instances.rs`, reading a requirement set the gate keeps from every successful registration and every epoch move (the heartbeat returns the set only when the epoch moves, design 053), tested here on a test-only capability, and the flag `journal_buckets_active` on each creation command below, set from it (always false until Task 049-13 adds the capability; the tests set the flag on the store's creation call directly). Write the `runs` triggers of Requirement 049-001; compute the bucket before the first write of each creating transaction (`crates/infra-postgres/src/execution.rs` for roots and children, `chain_progress.rs` for both kinds of next segment, `maintenance.rs` for pending children) and bind it to every row; with the tests `a_child_created_in_a_later_month_takes_its_roots_bucket` and `a_child_chains_next_segment_stays_in_its_tree`, driving the database clock, the command time and an injectable process clock across a month boundary, with `a_root_dated_before_the_process_month_takes_the_legacy_bucket`. Switch the budget's runs (`work_budget.rs`) to dated runs through the store's flag, naming each locked partition as the partition of the run's own bucket, never by the current month.
  Refs: Requirement 049-001, Requirement 049-010
- [ ] T049-6 Write `scripts/check-journal-buckets.py` (over `crates/infra-postgres/src` and the function bodies of the migrations after this design's first one, with the dynamic SQL of `ensure_journal_buckets` and `journal_retire_drop` and the reference triggers on its allow-list), the migration test over `pg_proc` of Requirement 049-011, a test of the registry's generator asserting that every generated reference lookup compares `tenant_id`, `root_run_id` and `journal_bucket` and binds row values with `USING`, never by formatting them into the statement, its negative fixtures under `scripts/tests/`, its allow-list of statements that span runs (and of the readiness check of `execution.rs` that names the tables), counting the literals parsed as statements as `tests/integration/platform-foundation/runtime_privileges/rust_source.rs` does and also any other resolved literal that names `run_events`, `run_payloads` or `payloads` as a whole identifier outside its owning file and the allow-list, and refusing SQL assembled with `push_str` or a query builder in `crates/infra-postgres/src`, its rule that only `journal.rs` reads `run_events` for one run and only `payloads.rs` names a payload table, and its CI step in `.github/workflows/ci.yml` beside the architecture check, with a baseline per rule of today's statements that break it, which the tasks of the merge order empty (the `format!` of `crates/infra-postgres/src/telemetry.rs` among them).
  Refs: Requirement 049-011
- [ ] T049-7 Map partitions to their parent in the classification test (`tests/integration/security-multitenancy/durable_quotas.rs`, its `manifest_covers_every_variable_size_column_and_every_accounted_table_has_always_triggers`), and list partitions through `pg_inherits` in `crates/infra-postgres/src/privilege.rs` and `tests/integration/platform-foundation/runtime_privileges.rs`, with the trigger and function checks of Requirement 049-013 (`proacl`, `prosecdef` and `proconfig`, the generator of the names and `crates/infra-postgres/journal-functions.toml`); it merges with Task 049-8, whose partitions the classification test would otherwise refuse. Task 049-10 adds `run_payloads` to the append-only set.
  Refs: Requirement 049-013, Requirement 049-016
- [ ] T049-8 Write step 1 of "The migrations of the stopped upgrade" with its waivers (after design 053's Task 053-2), rewrite the references to `run_events` in `crates/infra-postgres/src` to name the bucket from the run row and move the per-run reads into one function per read shape (`read_run_events` among them, and the functions that hold a per-run read inside a statement on another table, moved whole; in `telemetry.rs`, `execution.rs`, `approval_progress.rs`, `chain_progress.rs`, `declared_operations.rs`, `execution/database.rs`, `execution/path.rs`, `execution/resets.rs`, `leaf_certification.rs`, `loop_progress.rs`, `payload_disclosure.rs`, `primitives.rs`, `scripts.rs` and any other the static check finds) in a new `crates/infra-postgres/src/journal.rs` (its `purged_at` check arrives with feature 020's Task T020-045), add the mapping of `JOURNAL_BUCKET_MISSING` and `RUN_PURGED` with the reason in the contracts, a test of the first (the test of the second is added with feature 020's Task T020-045, which brings `runs.purged_at`), update `tests/integration/claim-transition-efficiency/work_budget.rs` for the legacy runs it measures at this task, and run the migration tests and the measurement of Success criterion 049-003.
  Refs: Requirement 049-002, Requirement 049-003, Requirement 049-004, Requirement 049-008, Success criterion 049-003
- [ ] T049-9 Add `journal_retirements` (key `bucket date`; `state text` checked among `ACTIVE`, `ABANDONING`, `ABANDONED`, `DONE`; `run_events_dropped` and `run_payloads_dropped` booleans; the other columns from Task 049-14), `journal_bucket_retiring(bucket date) returns text` (the state, `DROPPED` once either table is recorded dropped, or null), and `ensure_journal_buckets()` in `crates/infra-postgres/src/journal_buckets.rs` and their migration, with tests on a disposable partitioned table (five months created `LIKE` the parent, the truncate trigger present, the row triggers enabled always, a `DONE` or dropped month skipped, an `ABANDONED` month created, two concurrent calls), enumerating the partitioned tables through `pg_partitioned_table`; its call from the engine's housekeeping in `crates/scheduler/src/passes.rs`, at start, daily and hourly after a failure; the gauge and the alert in `infra/observability/alerts.yaml` with its rule test added to CI's `promtool` list.
  Refs: Requirement 049-008
- [ ] T049-10 Write `crates/infra-postgres/run-content-references.json` and the reference-trigger migration generated from it, the three registry tests, the barrier tests of a reference racing a delete in both orders, `run_payloads` and its triggers (step 3), `read_run_content` and `write_run_content` in a new `crates/infra-postgres/src/payloads.rs`, and make every payload write (`execution.rs`, `chain_progress.rs`, `declared_operations.rs`, `loop_progress.rs`, `primitives.rs`, `execution/path.rs`, `batch.rs`, `schedules.rs`) and reads (`execution/path_read.rs`, `execution/resets.rs`, `scope_frontier.rs`, `payload_inspection.rs`, `payload_disclosure.rs`, `collection_outputs.rs`, `database_results.rs`, `ad_hoc.rs`, `maintenance.rs` and the others the static check finds) use them. Add to the budget the `run_payloads` partition and the reference triggers' `runs` lookups, and check Success criterion 049-001 against the counts before partitioning.
  Refs: Requirement 049-005, Requirement 049-006, Requirement 049-016, Success criterion 049-001
- [ ] T049-11 After Task 049-3's measurement, build the partial legacy reference indexes (Requirement 049-007) that feature 020's Task T020-046 uses for its reference check of a legacy run (which, with the re-check after the delete, is that task's, not this one's), with a test that the check's plan uses them.
  Refs: Requirement 049-007
  Owner-gate: pending, the merge of constitution 5.1.0 (approved by Owner decision 2026-11-25-3, point 1)
- [ ] T049-12 With dated runs made by setting the store's creation flag directly, as Task 049-5's tests do, write the per-tree copy rules for resets, reruns, root chains' next segments, cross-namespace operations and rows of no run, with a test per path that the new tree references only its own payloads and a child chain's next segment shares its tree's.
  Refs: Requirement 049-005, Requirement 049-014
- [ ] T049-13 Add `journal-buckets-v1` to the `Capability` enum of `crates/application/src/instances.rs`, marked permanent in the check of `platform_requirements.permanent` (design 053's Task 053-5, which merges first) and by `Capability::is_permanent()`, with a test that the two lists agree and the run of design 053's `a_permanent_capability_refuses_both_deactivation_phases` on it, and the schema's closed lists (step 4), and the test `a_process_registered_after_the_switch_creates_dated_roots`, and write `control-plane journal switch` in `apps/control-plane/src/cli.rs` on the registry's activation, with a suite that runs after it across a month boundary with two dated partitions; tests before the command exists activate the capability through the registry's test fixture.
  Refs: Requirement 049-002, Requirement 049-010
- [ ] T049-14 Write `control-plane journal retire`, `sagat_retirer` with its grant of design 053's handshake (in the migration that creates the role, tested by running the command's handshake as `sagat_retirer`), the columns of `journal_retirements` that retirement needs, the six retirement functions, the sessions function, `sagat_quota_release_bytes`, `purge_run_content_mark_only`, `authorize_abandon_deletes` and the bucket lock in its purge, with the crash test at every step (between the drop and the release among them), a lock timeout in the drop's transaction, the races with an append, with a per-run purge held by a barrier at the retirement's start, and of two purges of one tree, the abandon after a run fails its re-check with its sweep, a crash during an abandon followed by a resume (refused) and by the abandon again, an abandon racing a purge chunk, an abandon after step 3 then a reactivation and a retirement, a partition held by another session behind a barrier (the attempt fails at once and is retried), and the quota reconciliation test after each; the refusals of `journal_retire_start` at `REPEATABLE READ`, at `SERIALIZABLE` and for the current month and of `journal_retire_drop` for any other table, called directly as `sagat_retirer`; an erasure refused whole when one run of its tree blocks; a dated root of a month that ended less than 24 hours ago refused at `REPEATABLE READ` and at `SERIALIZABLE` with `runs_journal_bucket_isolation` and accepted at `READ COMMITTED`; barrier tests of a dated root of an ended month racing `journal_retire_start` and of an erasure's lock order against a writer of the tree.
  Refs: Requirement 049-009, Requirement 049-014, Requirement 049-016, Success criterion 049-002, Success criterion 049-004
  Owner-gate: pending, the merge of constitution 5.1.0 (approved by Owner decision 2026-11-25-3, point 1)
- [ ] T049-15 Amend the data models and tasks of features 020, 022, 025, 026 and 030 so that their run-content references follow Requirements 049-005 and 049-006, and feature 020's purge and erasure have the key, the tree as unit and the mark-only form of Requirement 049-014, its Task T020-045 adds the purged branch to this design's triggers, its Task T020-046 reads the registry and merges without the partial legacy indexes, which Task 049-11 adds afterwards with a test that the check's plan uses them, and its Task T020-061 points to constitution 5.1.0 instead of opening an amendment of its own.
  Refs: Requirement 049-014, Requirement 049-015
- [ ] T049-17 Write the retention section of `docs/runbooks/database-recovery.md` and the bucket runbook `docs/runbooks/journal-buckets.md` (the switch, which 0.3.0 leaves off until an operator runs it, and retirement), and the release-notes line.
  Refs: User story 049-01
- [ ] T049-18 When constitution 5.1.0 merges, check that its principle I admits the retirement of Requirement 049-009 and the purge key of Requirement 049-014 as written, record the merge commit here, and raise any difference with the coordinating session before Tasks 049-11 and 049-14 start.
  Refs: Requirement 049-009, Requirement 049-014

#### Follow-up tasks

- [ ] T049-16 Measure the scheduler's passes with `steps` partitioned by bucket after design 052's queue, and record the decision in `tests/performance/reports/journal-partitioning.md`.
  Refs: Requirement 049-012

### Verification Plan

Each pull request runs `integration_platform_foundation` (its migration tests: up with data, down refused where the
migration destroys data, up again; and its `runtime_privileges` module), `integration_security_multitenancy` (the
classification test), the suites rerun with `SAGAT_TEST_AS_RUNTIME_ROLE=1`, design 038's budget target, the static check
of Task 049-6, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and the documentation and architecture
checkers of `scripts/`. These are added and seen failing first: the suite after the switch across a month boundary
(Task 049-13); the child-bucket tests (Task 049-5); a migration test asserting, by the identity (`relfilenode`) of the
legacy table and of each of its indexes before and after, that step 1 copies no row and builds only the new unique
index, and that the parent carries the three checks, the default and the instance index (Task 049-8); the registry tests
with an unregistered digest column and with a column of no run referencing `run_payloads` (Task 049-10), and the
confirmation of what the two certificate digests of `leaf_collection_success_certifications` hold before they are
registered; the per-tree copy tests (Task 049-12); a runtime-role test that `DETACH`, `DROP` and the retirement functions
fail for `sagat_app` (Task 049-14); fault tests at every retirement step and on a lock timeout in the drop's transaction,
the race tests, the read of a purged run before the drop, and the quota reconciliation after a retirement (Task 049-14).
Tasks 049-8 and 049-14 are run under design 051's driver on a free standard runner; its maximum statement latency per
table is the evidence of Success criterion 049-002, and the relation identities and sizes of Success criterion 049-003.
