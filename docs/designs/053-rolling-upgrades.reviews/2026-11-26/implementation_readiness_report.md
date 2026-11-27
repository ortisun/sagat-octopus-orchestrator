# egm-implementation-readiness on design 053, full round of 2026-11-26

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 7771688e6ab65562395c6c2faa8c548e66a50bdd
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing.

## Blocker

1. Task 053-2 merges before its own boundary exists: the online rules' start is 0.3.0's last migration, not yet written
   when the lint merges. Fix: the key absent means "the online rules apply to no migration"; the 0.3.0 release pull
   request sets it; the release workflow refuses a tag at or above 0.3.0 without it; a fixture for the absent key.

## Major

1. The applied-epoch check cannot be built: no column holds phase 1's epoch; which epoch a process sends is unsaid (a
   re-registration moves `gate.epoch()` without re-reading the set); `instance_register` leaves `applied_epoch` null;
   the kinds checked and the refusal's outcome are undefined. Fix: `draining_epoch`; send `kept_epoch()`; registration
   sets it; null is not applied; live rows of the capability's kinds; outcome `NOT_APPLIED` and
   `ChangeRefused::NotApplied`.
2. Task 053-5 collides with the registry triggers: `platform_requirements_guard` refuses every update; the handover and
   floor guards `RETURN OLD`, which would cancel an update; the history check allows `ACTIVATED` and `REMOVED` only; the
   drain outcomes are unnamed; about 36 call sites remove active rows. Fix: the guard admits the state change; the two
   guards `BEFORE DELETE OR UPDATE OF state` returning `NEW` on update; `DRAINED` and `UNDRAINED`; outcomes named; a
   `drain_then_remove` fixture and the call sites listed.
3. What `draining` means for each reader of `platform_requirements` (registration, `missing_for`, `requirements()`,
   `ServingGate::requires`, activation, the handover, `follows`) is undefined. Fix: one rule table.
4. The phase-2 tests assert only refusals; an always-refusing phase 2 passes. Fix: a success test and a test that the
   drain check runs.
5. The operator commands for drain, undrain and remove, their exit codes and role, and the per-capability verify table
   are not designed.
6. No capability-to-migration mapping exists, and every real capability predates the schema check, so the named tests
   cannot be set up. Fix: `Capability::listed_by()`, an injectable expected-version list, a test-only migration
   directory with a test-only capability.
7. The lint forbids the widening of the closed capability lists that Requirement 053-006 requires. Fix: an exception
   for a superset rewrite, and a fixture of a narrowing.
8. The lint's rules cannot be coded from the text: no rule identifiers, the hot tables by description, no method for
   "a released binary relies on", no lock-level table, no source for partitioned tables, a fixture list ending "among
   them".
9. `migrate`'s loop: sqlx's `validate_applied_migrations` is private; a retried no-transaction migration is already
   recorded; timeouts name no table; indexes named need parsing; the backoff makes a 3-minute test;
   `SAGAT_MIGRATE_ON_START` would wait for itself; `sagat_infra_postgres::migrate` in about 60 tests. Fix: `run_to` per
   version; delete the row before a retry; `pg_locks` for the table; an `-- index:` header; an injectable backoff;
   `SAGAT_MIGRATE_ON_START` skips the wait (development only); `migrate_online()` beside `migrate()`.
10. One-run commands: no kind or capability list for `missing_for`, overlap with `command_await_durability`, the
    architecture rule undefined, exit code 1 against `input-schema-inventory`.
11. The build order omits Task 053-3 before Task 053-4.
12. Task 053-6's harness: `test-release-upgrade-profile.py` is fixed to v0.2.0 and builds a test binary; the topology,
    rolling, baseline, metric and partition count are unsaid; the zero-completion floor is missing; the CI job and the
    N-1 of a patch release are unsaid.
13. The scripted rollback is impossible when a capability cannot be removed (049's `journal-buckets-v1` in merge-base
    mode). Fix: roll back only when every activated capability passed phase 2; otherwise drain, undrain and record it.

## Nit

1. `cleanup-secrets`, not "the `secret-cleanup` job".
2. Design 048's Requirement 048-014 says two gate reasons; Task 053-1 should amend it.
3. Name feature 018's `contracts/errors.md` and `data-model.md`, `openapi.yaml` and `sdks/SPEC.md`.
4. `docs/releases/0.3.0.md` does not exist; Task 053-1 creates it.
5. Say what a binary does when `sagat_schema_missing` is absent (`42883`).
6. Pin a `pglast` whose `libpg_query` matches the server's major version; `pip install --require-hashes`.
7. Task 053-4 has only failure tests; add a lock released within the retries.
8. Define the order of "a tag of a lower version".

Counts: Blocker 1, Major 13, Nit 8
