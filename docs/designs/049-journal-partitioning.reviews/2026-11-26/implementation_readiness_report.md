# egm-implementation-readiness on design 049, full round of 2026-11-26

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 66634fb997c95e6725e2e8773f120c112759b347
Note: the reviewer's output, kept whole. The reviewer read files only and ran nothing.

## Blocker

1. T049-5 is ordered before T049-8, but the `journal_bucket` columns, `journal_bucket_switch` and the switch/immutability
   trigger on `runs` are created in T049-8 step 1, and both tasks claim that trigger. T049-5 cannot merge alone. Fix: move
   the schema for Requirements 049-001 and 049-010 into a T049-5 migration and remove it from step 1, or put T049-5 after
   T049-8.
2. T049-8 step 1 writes the reference triggers and drops the 20 payload foreign keys. Those triggers must look up
   `run_payloads`, which T049-10 creates in "the next migration", and the column list they use is owned by T049-10. Fix:
   move the reference triggers, the foreign-key drops and the referencing-table columns into T049-10, after
   `run_payloads` exists.
3. The migrations cannot run as described. `MIGRATOR` (`crates/infra-postgres/src/lib.rs:111`) is plain sqlx. It sends a
   `-- no-transaction` file as one string with several statements, so `CREATE INDEX CONCURRENTLY` is refused and nothing
   retries statement by statement. No migration in the repository uses no-transaction yet. Design 053 requires its Task
   053-2 before any 049 migration, but 049 says "retry by hand". Fix: list the migration files, one concurrent index per
   file, and make T053-2 and T053-4 hard prerequisites or write the manual retry steps.

## Major

1. The partial legacy indexes are built in two places: T049-8 step 2 and the gated T049-11 (after T049-3). Fix: keep them
   only in T049-11.
2. Requirement 049-001 computes the bucket before the input payload insert, but Requirement 049-010 folds the switch read
   into the later run insert. T049-5 cannot do both. Fix: name the statement that reads `journal_bucket_switch FOR
   SHARE`, for example folded into the payload insert, and give its SQL.
3. If T049-6's CI step lands first, it fails on the 43 existing unbucketed references, including the `format!` at
   `telemetry.rs:361`. Fix: ship a baseline allow-list that T049-8 empties, or turn on the CI step in T049-8.
4. T049-4 relies on a pg_locks hook and a "connection wrapper" that do not exist. The budget is counted through
   `pg_stat_statements` (`work_budget.rs:36-43`), and the lease and completion run on the store's own pool and
   transactions. Fix: name the seam (file, function, feature flag) that samples locks before commit and counts round trips.
5. The "one adapter function" for payload reads has no name, file or task. About 18 read sites are not listed
   (`path_read.rs`, `resets.rs`, `scope_frontier.rs`, `payload_inspection.rs`, `payload_disclosure.rs`,
   `collection_outputs.rs`, `database_results.rs`, `ad_hoc.rs`, `maintenance.rs`). T049-10 lists insert sites only, and
   wrongly includes `approval_progress.rs`, which has no payload insert. Fix: add a task for the read adapter plus the
   static rule for payload reads.
6. T049-8 says "the two typed errors", but Requirement 049-008 defines three reasons (`JOURNAL_BUCKET_MISSING`,
   `RUN_PURGED`, `JOURNAL_SWITCHED`). `JOURNAL_SWITCHED` comes from T049-5's trigger. Fix: assign each mapping to a task
   and name `specs/001-platform-foundation/contracts/openapi.yaml`, `frontend/apps/studio/src/api/schema.d.ts` and the
   error contracts.
7. The feature 020 code that this design needs is not built: the 049-002 and 049-006 triggers check for a stub or
   tombstone, which T020-045 creates; T049-11 changes 020's purge (T020-046) but is not ordered after it; no task adds the
   bucket try-lock or `CONTENT_PENDING_DROP` handling to 020's purge code. Fix: say how a stub is detected before 020
   exists, and order T049-11 after T020-046.
8. Prerequisites from other designs are missing: T049-2 needs design 051's shapes (T051-1) and history loader (T051-3);
   T049-8 and T049-14 need the driver (T051-2); T049-3 needs PostgreSQL 18, but the tests start only `postgres:16-alpine`
   (`tests/support/mod.rs:168`, `bounded.rs:88`). Fix: cite those tasks and add a task to parameterise the database image.
9. The capability `journal-buckets-v1` is not in the `Capability` enum (`crates/application/src/instances.rs:108-126`) or
   in the schema's closed list, which a test keeps equal to the enum. Fix: add the enum value and its migration to T049-13.

## Nit

1. Release scope is inconsistent: the plan says only T049-14 waits for the amendment, but T049-11 is gated too; T049-18
   is in no release list; ADR-0132 decision 6 says the wait is for "the amendment feature 020 carries", not constitution
   5.1.0.
2. T049-1 says "Write" ADR-0132, but it already exists as Proposed; say "revise and accept". Its "Enforced by" line omits
   T049-9.
3. Step 5 creates the current month and the next two; Requirement 049-008 says the next three. Align them.
4. T049-9 should enumerate tables through `pg_partitioned_table`, since `run_payloads` only arrives in T049-10, and should
   say what drives the hourly retry.
5. Paths are missing for the CI file (`.github/workflows/ci.yml:347`), the chart job (T049-14), the bucket runbook
   (T049-17) and the T049-16 report; the metric for late-legacy refusals is unnamed.
6. `crates/infra-postgres/src/partitioning.rs` already exists for scheduler shards; tell builders not to put bucket code
   there.
7. T049-12 needs dated runs before T049-13's switch command exists; name the test fixture that sets
   `journal_bucket_switch`.

Counts: Blocker 3, Major 9, Nit 7
