# Architecture review of design 053, full round of 2026-11-26

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 7771688e6ab65562395c6c2faa8c548e66a50bdd
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only,
ran nothing, and could not list other branches.

## Blocker

1. Phase 2 does not prove "a deactivation cannot pass while a process still uses the capability": a transaction that
   read `requires()` before the beat can commit after phase 2 (a READ COMMITTED drain check cannot see it, also for a
   first dated root in flight); a process the full table did not take has no row yet keeps requirements, and a control
   plane in that state creates runs; one-run commands never register. Fix: the database-side fence is mandatory for
   state N-1 cannot process, and takes the requirement row `FOR KEY SHARE` inside the writing transaction, so phase 2's
   delete waits for writers in flight and later ones are refused; `requires()` false while not counted; commands write
   through the fence; a test of a use that started before phase 1; ADR-0131 restated.

## Major

1. The design's own lint refuses the widening of the closed capability checks Requirement 053-006 requires (the pattern
   of `20270211000100_lease_withdrawal_v2.up.sql` changes three registry tables). Fix: a named rule accepting a
   same-name superset; one table per migration or a precise hot list; fixtures.
2. `draining` is undefined for every reader: the guard refuses every update (ADR-0082 decision 1); activation answers
   `ALREADY_RECORDED` for a draining row; `requirement_recorded` counts it, so `lease-withdrawal-v2` could activate while
   v1 drains; `requirements()` carries no state; the history check. Fix: a table per reader; the guard's update rule;
   refuse draining what an active capability follows; a test per row.
3. `docs/runbooks/instance-registry.md` and `docs/runbooks/lease-withdrawal.md` tell operators to call
   `platform_requirement_remove` directly; no drain, undrain or remove command exists. Fix: both runbooks in Task
   053-1; name the commands or keep the functions as the interface.
4. `migrate`'s own loop drops sqlx's advisory lock (`concurrent_runners_do_not_corrupt_or_fail`) and the refusal of a
   dirty version; the procedure writes `_sqlx_migrations` by hand. Fix: hold `Migrate::lock`, refuse dirty versions,
   keep the test, write through sqlx's `Migrate` API.
5. The capability-to-migration map has no owner and no check, and versions would leak into the application. Fix: the
   map beside `MIGRATOR`; the port returns a typed verdict; a test per capability.
6. "Functions called before the migrations exist in N-1's schema" has no check against a real older release. Fix: the
   mixed-writer test starts one N process on N-1's schema before `migrate`.
7. The entry-point check has no target. Fix: by type: the one production constructor of the serving `DbHandle` runs the
   check; the architecture script refuses `connect` and `open_pool` in `apps/` elsewhere; a negative fixture.
8. Exit code 1 conflicts with `input-schema-inventory` (decision D043-072); use 2 there and test it.
9. A patch release is never tested against the patch it replaces. Fix: also against the previous patch of its minor.
10. Design 052's ready queue "states its own rule" but design 052 says nothing of it. Fix: state the rule here or in
    052 and cite it.
11. ADR-0131 does not record its divergence from accepted ADR-0132 (`Capability::is_permanent()`). Fix: "amends
    ADR-0132's Enforced by", citing Task 049-20.
12. Owner decision 2026-11-24-1 puts rolling N and N-1 upgrades in 0.3.0; the design first gates at 0.4.0, unrecorded.
    Fix: an Open Question, a delegate row, the release notes.

## Nit

1. Design 048's Requirement 048-014 says two gate reasons.
2. A removal takes two heartbeats (about 24 s), not one.
3. ADR-0131's options omit the alternatives the design weighs; "Enforced by" is not marked pending and omits the
   existing tests that change.
4. ADR-0131 calls the change additive, though the guard and `platform_requirement_remove` are replaced (in the stopped
   upgrade); "from release 0.3.0 on" is ambiguous.
5. Put `migrate`'s loop in `crates/infra-postgres` (`migrate_online`), the one allowed site of design 048's `BEGIN` and
   `SET` rule.
6. A test that `sagat_schema_missing` agrees with sqlx's `Migrator`.
7. `instance_heartbeat_applied` should refuse an `applied_epoch` above the current epoch.
8. Bound the partitioned-index procedure's re-listing.
9. No alert for a gate held by `SCHEMA_TOO_OLD`; no test that a 503 shows no version; whether the reason shows after
   authentication.
10. `lease-withdrawal-v1`, `lease-withdrawal-v2` and `schedules-v1` after the handover are also non-removable, with
    phase 1 refused; state the difference from `journal-buckets-v1`.
11. Rows in `docs/engineering/architecture-principles.md` for the new checks (AGENTS.md rule 21).
12. `cleanup-secrets`; the grant to `sagat_retirer` is design 049's Task 049-14.

Counts: Blocker 1, Major 12, Nit 12
