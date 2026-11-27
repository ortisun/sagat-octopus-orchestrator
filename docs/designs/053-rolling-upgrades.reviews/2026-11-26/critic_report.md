# egm-design (critic) on design 053, full round of 2026-11-26

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 212e4262a9a4834529a22e88bba39d184e93b191
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only,
ran nothing, and took PostgreSQL behaviour from its knowledge.

## Blocker

1. Phase 2 can pass while a process still uses the capability: a beat updates `last_seen` before it reads the epoch
   (`20270211000300`), and its `FOR SHARE SKIP LOCKED` read returns the old epoch while phase 1 holds the row, so a
   process whose beat overlapped phase 1 counts as "heartbeated after `draining_since`" without learning of the drain.
   Fix: the heartbeat stores the epoch the process has applied (`serving_instances.applied_epoch`); phase 2 requires it
   at or above phase 1's epoch for every live row, under the epoch lock; a barrier test.

## Major

1. Deactivation has no fence: a process not live at phase 2, one refused a row (`REGISTRY_FULL`), or a transaction begun
   before the drain can still write the capability's state. Fix: Requirement 053-007's fence-or-harmless rule extended
   to deactivation.
2. The two-phase machine leaks: (a) `platform_requirement_remove` and `PgInstanceStore::remove` still delete an active
   row in one phase; (b) the handover and replay-floor guards fire only `BEFORE DELETE`, so draining `schedules-v1`
   stops every schedule (decision D2048's outage); (c) no way back from `draining`. Fix: delete only a `draining` row;
   run those checks on the update; allow `draining` back to `active`.
3. A contract and phase 2 race, linked only by a file comment `_sqlx_migrations` does not hold. Fix: `migrate` records
   `(capability, version)` before a contract under the requirement row's `FOR SHARE`; phase 2 reads it under the epoch
   lock.
4. "Lock timeout plus one commit" is false: `lock_timeout` bounds each wait, and `ALTER TABLE` on a partitioned journal
   table locks each partition in turn; the statement's run time is unbounded. Fix: `statement_timeout` 2 s before the
   blocking statement, `57014` retried; no non-concurrent index, unique or primary key on a hot table; the test database
   holds the steady-state partitions.
5. No legal way to index a partitioned journal table (`CREATE INDEX CONCURRENTLY` is refused on a partitioned table;
   partitions are created at run time), yet design 049 defers its indexes to this design. Fix: a partitioned-index
   procedure in `migrate` (`ON ONLY` parent, concurrent per partition, `ATTACH PARTITION`, `indisvalid`), and a lint rule.
6. Version order breaks for `develop` builds (`X.Y.0-dev` sorts below `introduced_in = X.Y.0`), and nothing defines the
   comparator or a malformed `binary_version`. Fix: `introduced_in` recorded with the `-dev` suffix by the expand
   migration; `sagat_version_cmp()`; validation at registration.
7. An N process cannot register before N's migrations (the closed `capabilities` check refuses N's new names, `23514`),
   so the "no process runs" order deadlocks. Fix: register only the capabilities whose migration is applied, re-register
   after; N-1's schema has every function an N binary calls before its migrations; the test adds a capability in N.
8. Hotfix ordering: a 0.3.1 cut after 0.4.0 cannot upgrade to 0.4.0 (`VersionMissing`); "earlier" and "newest" are
   ambiguous. Fix: order by version; check every lower release's checksums; the runbook's hotfix path; the gate against
   the newest patch of the previous minor.
9. Expand migrations may add refusals N-1 hits (`NOT VALID` checks and keys, raising triggers), and unconditional fences
   stay after a rollback. Fix: those are contract changes; a fence is conditional on the capability row.
10. The release gate cannot see stuck runs or the state of other capabilities. Fix: each capability adds a scenario;
    the oracle requires every run to reach a terminal state within a stated time.

## Nit

1. `instance_heartbeat_v2` must succeed `instance_heartbeat_reporting_change` (decision D006-098); "cannot race" holds
   for registration only.
2. This design's own replacements (the guard, the deferred trigger) need `stopped-upgrade` and must land in 0.3.0; the
   lint must refuse that waiver after 0.3.0.
3. `state text NOT NULL DEFAULT 'active'`; a null `introduced_in` ignored.
4. The unique-superset exemption needs a non-partial, non-deferrable existing constraint.
5. The earlier `DROP INDEX CONCURRENTLY IF EXISTS` never re-runs; `SET LOCAL` has no effect without a transaction; state
   the session `lock_timeout` for a concurrent build.
6. The retry bound excludes the attempts' own waits: about 171 s.
7. The handshake returns every applied version on every beat; return the highest and a digest.
8. `BINARY_TOO_OLD` closes every kind; the census must cover all kinds.
9. A `BINARY_TOO_OLD` process "writes its registry rows" but is refused; ADR-0131 decision 5 says "write nothing".
10. The Constitution Check cites 5.1.0, not merged.
11. `release-version.sh` is read-only; the `develop` move needs its own command; SDK and Studio versions (PEP 440); refuse
    an `oldest_compatible` above the previous minor.
12. `migrate` cannot call the handshake (its schema is too old, or the function absent); it uses the release check.
13. Keep the permanent list in a table or an argument, not in the function's body.

Counts: Blocker 1, Major 10, Nit 13
