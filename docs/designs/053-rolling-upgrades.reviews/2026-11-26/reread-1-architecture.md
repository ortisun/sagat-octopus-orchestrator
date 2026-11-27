# Targeted re-read 1 of design 053: architecture (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 655ecb38b3bb2d4c6a17054fa07d706000920547
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`architecture-full.md`, not a full review. The reviewer read files only, ran nothing, and could not list branches.

Full round: 23 of 25 findings resolved; Major 7 and Nit 6 partly (carried below as Major B and Nit 5).

## Major

1. (A) The capability-list widening locks the three tables one after another, each with its own 2 s lock timeout and
   no statement timeout, so `platform_requirements`, now read by run starts' fence, can stay locked about 4 s plus a
   commit, against the stated bound. Fix: one `LOCK TABLE` of the three under the 2 s statement timeout, required by
   the rule, with a fixture; ADR-0131 decision 2.
2. (B) The entry-point check refuses every one-run command (each opens `open_pool`) and gives them no target. Fix:
   `connect_command(capabilities)` running `command_await_durability` first; only the two constructors and `migrate`
   allowed; a fixture.
3. (C) The per-capability fence and drain check have no owner or check, and placing the check beside `ACTIVATABLE`
   puts SQL in `apps/`. Fix: a descriptor per capability in `infra-postgres` that `remove` reads; a test over every
   capability.

## Nit

1. Phase 2 holds new writes of `runs` for its 1 s lock wait, the 2 s check and the commit; state that bound.
2. ADR-0131 decision 2 says "a released binary relies on" where the lint says "an earlier migration created".
3. The dirty version comes from `Migrate::dirty_version`, not `list_applied_migrations`.
4. N-1 is undefined for `X.0.0`.
5. A test that the schema check of the migrator's own versions is empty after `migrate()` and the pending set before.

Counts: Blocker 0, Major 3, Nit 5

All eight were fixed at the next commit. Nothing else in ADR-0131 stops its acceptance.
