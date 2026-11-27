# Approval confirmation of design 053: correctness (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 3cf5ee9737c4611e9126403c7f5dbfecc0b4f861
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`correctness-full.md`, at 7771688e),
then `reread-1-correctness.md` (at d9053977), whose seven Majors and eight Nits are all confirmed resolved here; the
later changes (the widening's one lock, `connect_command`, the descriptors, phase 2's table-then-row order and `40P01`
retry, the fence's serialization path, the lint's refusal of partitioned-index migrations, `journal-buckets-v1`
unfenced) checked consistent. Not a full review. The reviewer read files only and ran nothing; items marked plausible
rest on its knowledge of PostgreSQL.

No Blocker.

## Major

1. (Plausible) Concurrent `migrate` runs deadlock on a concurrent index build: runners blocked in
   `pg_advisory_lock` hold snapshots, which the build's last phase waits for, while they wait for the build's runner;
   the detector kills one, so a waiting runner exits with a deadlock error or the build is retried into the same
   deadlock until it gives up. `concurrent_runners_do_not_corrupt_or_fail` would then fail. Fix: take the migration
   lock by polling `pg_try_advisory_lock` (each attempt a short statement, no snapshot held while waiting) about every
   1 s for up to 60 s, then exit 1; the test `a_concurrent_runner_waiting_for_the_lock_does_not_deadlock_a_concurrent_build`.

## Nit

1. ADR-0131's Consequences say "no write lands after it"; for unfenced `journal-buckets-v1` a dated root can. Say "no
   fenced write".
2. A drain check that times out (`57014`) is retried as "retry later" for ever with no named check. Report it as
   `DRAIN_CHECK_REFUSED` (timed out), exit 2, naming the check, or require each check's plan to be an index or a
   bounded scan.
3. Exception (b) of Requirement 053-001 can add a refusal (`NULLS NOT DISTINCT`, another collation, operator class or
   an expression). Require plain columns, the existing constraint's collations and operator classes, and `NULLS
   DISTINCT` unless the existing one is not.

Counts: Blocker 0, Major 1, Nit 3
