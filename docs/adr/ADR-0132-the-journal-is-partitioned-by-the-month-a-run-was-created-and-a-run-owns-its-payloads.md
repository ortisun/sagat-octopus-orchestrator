# ADR-0132: The journal is partitioned by the month a run was created, a run owns its payloads, and a month is dropped only when every run in it is archived and purgeable

**Status:** Proposed (2026-11-24)
**Decided by:** agent proposal (design 049), to be accepted under Owner decision 2026-11-24-4 once design 049's review is complete
**Refines / supersedes:** refines [ADR-0054](ADR-0054-run-content-leaves-the-hot-store-only-through-a-verified-archive-under-a-namespace-retention-policy.md) (its rejected option 3, partition drop, becomes the main path, aligned with runs; its per-run purge stays as the fallback and loses its reference check); supersedes nothing
**Enforced by:** the test that no run has rows in two buckets (Task 049-5), the static check that every statement on a partitioned journal table names its bucket (Task 049-6), the catalog checks that know partitions (Task 049-7), the relations-locked line of design 038's budget (Task 049-4), and the crash and race tests of retirement (Task 049-14)

## Context

Nothing is deleted today. Feature 020 plans a verified archive and a purge of each run in chunks; payloads are shared
by content within a tenant from 22 columns, so a purge needs a reference check and an index on each of them (feature
020, gap 1099). Architecture decision 0054 rejected partition drop because partitions did not align with runs and could
not be verified per run. Child run identifiers are hashes, not times. Every read of the journal is by run.

## Decision

1. Every run MUST carry an immutable `journal_bucket`, the UTC month it was created in (or the legacy value), and every
   row a run writes to a partitioned journal table MUST carry it, taken from the run row.
2. `run_events` and a new `run_payloads` MUST be partitioned by range of `journal_bucket`, one partition per month; the
   existing `run_events` becomes the legacy partition without a copy.
3. A run's payloads MUST be keyed by the run, `(tenant_id, journal_bucket, run_id, digest)`; a run MUST NOT reference
   another run's payload; payloads of rows that do not belong to a run stay in `payloads`.
4. Every statement on a partitioned journal table MUST name its bucket by equality, and MUST be planned with its
   parameters so that only one partition is locked, unless the measurement of design 049's Task 049-3 shows a cheaper
   way that locks no more.
5. A bucket MAY be dropped only when every run in it is purgeable under feature 020 and its archive verified; the runs
   are marked purged under their run locks first, the partitions detached concurrently, checked, then dropped.
6. New runs get a dated bucket only after the census shows no process of the previous release.
7. `runs` and `steps` are not partitioned in release 0.3.0; `steps` is measured again after design 051's queue.

## Considered options

1. **Partition by the run's creation month, payloads owned by the run (chosen).** A run's rows are in one partition; a
   drop and a single-run purge both need no reference check; reads already name the run.
2. **Per-run purge only (feature 020 as drafted).** Correct, the fallback here; millions of deletes and 22 reference
   indexes at scale.
3. **Partition by event time.** Splits long runs and cannot drop a month while any run that wrote to it lives.
4. **Partition by the run identifier.** Child identifiers are hashes: a range of identifiers is not a range of time.
5. **Keep payloads shared, partition only `run_events`.** Leaves the largest and the most referenced data with the
   reference-check purge.

## Consequences

- Good: retention costs a drop per month; a single run is purged by its key; hot partitions stay small.
- Bad: identical bytes in two runs are stored twice; the payload move takes two releases with a dual write in between;
  statements on the journal are planned every time.
- Harder: every new statement on the journal must name its bucket (checked); a long-lived run holds its month.
- Compatibility: old binaries write the legacy bucket until the switch, which waits for them to be gone.
- Rollback: before the switch, nothing is dated and the partitioned parent behaves as the old table; after it, the
  partitions stay.

## Evidence

Design 049 and its review reports; feature 020's `data-model.md` and gap 1099; `crates/application/src/execution.rs`
(`child_run_id`); `tests/performance/reports/scalability-ha.md` (the lock-manager waits of 2026-11-12). To be added when
built: the measurements of Tasks 049-2, 049-3 and 049-8.
