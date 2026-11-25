# ADR-0132: The journal is partitioned by the month a run was created, a run owns its payloads, and a month is dropped only when every run in it is purged

**Status:** Proposed (2026-11-24)
**Decided by:** agent proposal (design 049), to be accepted under Owner decision 2026-11-24-4 once design 049's review is complete
**Refines / supersedes:** refines in part [ADR-0054](ADR-0054-run-content-leaves-the-hot-store-only-through-a-verified-archive-under-a-namespace-retention-policy.md): its decision 4 (one privileged purge function) gains a second path for whole months; its decision 6 (references found from the catalog) takes its references from the registry it already plans; its rejected option 3 (partition drop) is taken for whole months only, aligned with runs. Its stub, its eligibility rule and its verified archive are unchanged. Refines [ADR-0027](ADR-0027-durable-content-rate-and-retention-quotas.md) (a drop releases content bytes through the accounting function, not by row triggers). Supersedes nothing.
**Enforced by:** the bucket trigger and the suite after the switch across a month boundary (Tasks 049-5, 049-8 and 049-13); the registry test that fails on an unregistered digest column or a run-owned foreign key to `payloads` (Task 049-10); the static check that every statement on a partitioned journal table compares its bucket to a parameter or a literal (Task 049-6); the catalog checks that know partitions (Task 049-7); the relations-locked and round-trip lines of design 038's budget (Task 049-4); the crash, race and quota-reconciliation tests of retirement (Task 049-14)

## Context

Nothing is deleted today. Feature 020 plans a verified archive and a purge of each run in chunks; payloads are shared by
content within a tenant from 22 foreign-key columns and one column without a key, so a purge needs a reference check and
an index on each of them (feature 020, gap 1099). Architecture decision 0054 rejected partition drop because partitions
did not align with runs and could not be verified per run. Child run identifiers are hashes, not times. Every read of the
journal is by run. Foreign keys into a partitioned table make each detach scan the referencing tables under a lock that
blocks their writes, and a drop releases no quota because it fires no row trigger.

## Decision

1. Every run MUST carry an immutable `journal_bucket`, the UTC month of its `created_at` (or the legacy value), computed
   once by the creating transaction; every row a run writes to a partitioned journal table MUST carry it, and a trigger
   MUST refuse a row whose run does not have that bucket.
2. `run_events` and a new `run_payloads` MUST be partitioned by range of `journal_bucket`, one partition per month; the
   existing `run_events` becomes the legacy partition without a copy and without an index build under a blocking lock.
3. A run's payloads MUST be keyed by the run, `(tenant_id, journal_bucket, run_id, digest)`; a run MUST NOT reference
   another run's payload; payloads of rows that belong to no run, and the tombstones of feature 020's stubs, stay in
   `payloads`.
4. No foreign key MUST point into or out of a partitioned journal table. Every reference to run content MUST be listed in
   feature 020's registry and checked on write by a trigger.
5. Every statement on a partitioned journal table MUST compare its bucket to a parameter or a literal, and the way
   statements are planned MUST be the one design 049's Task 049-3 measures to lock no more than the pruned partition.
6. A bucket MAY be dropped only when every run in it is purgeable under feature 020 and its archive verified; the runs
   are purged under their run locks first, the bytes per tenant counted, the partitions detached concurrently, checked,
   the bytes released in one audited transaction, then dropped. Retirement is an owner-run command, never a platform
   process. It, and the cleanup of legacy payloads, wait for the amendment of principle I that feature 020 carries.
7. New runs get a dated bucket only after the registry's activation of `journal-buckets`, which refuses older processes.
8. `runs`, `steps` and the audit trail are not partitioned in release 0.3.0; `steps` is measured again after design
   051's queue; no audit record is dropped.

## Considered options

1. **Partition by the run's creation month, payloads owned by the run, triggers in place of foreign keys (chosen).** A
   run's rows are in one partition; a single run's purge and a month's drop both need no reference check; a detach and a
   drop lock nothing of the hot path's.
2. **Per-run purge only, payloads shared (feature 020 as drafted).** Correct; millions of deletes and 22 reference
   indexes at scale.
3. **The same partitioning with foreign keys.** Integrity by the database's mechanism, and a `SHARE` scan of every
   referencing table at each detach, with `ACCESS EXCLUSIVE` on `runs` at each drop.
4. **Partition by event time.** Splits long runs and cannot drop a month while any run that wrote to it lives.
5. **Partition by the run identifier.** Child identifiers are hashes: a range of identifiers is not a range of time.

## Consequences

- Good: a single run is purged by its key; retention costs a drop per month where every namespace has a policy no longer
  than the month's age; hot partitions stay small.
- Bad: identical bytes in two runs are stored, and charged to the tenant, twice; the payload move takes two releases with
  a dual write in between; a trigger call per write replaces the built-in key check; a namespace with no policy (the
  default) holds every month, so the drop is opportunistic and the per-run purge the main path.
- Harder: every new statement on the journal must name its bucket (checked); every new reference to run content must be
  registered (checked); a long-lived run holds its month.
- Compatibility: old binaries write the legacy bucket until the activation, which refuses them afterwards.
- Rollback: before the activation nothing is dated and the partitioned parent behaves as the old table; after it the
  partitions stay and the binaries cannot go back below the release that activated it.

## Evidence

Design 049 and its review reports; feature 020's `data-model.md` and gap 1099; `crates/application/src/execution.rs`
(`child_run_id`); `tests/performance/reports/scalability-ha.md` (the lock-manager waits of 2026-11-12). To be added when
built: the measurements of Tasks 049-2, 049-3 and 049-8.
