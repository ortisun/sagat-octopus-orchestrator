# Correctness re-review of 39157608 (drain, cursor versions, guards)

Reviewer: `correctness-reviewer` (read-only), given the diff of `39157608` and the spec, 2026-11-20. Its text, then what
was done.

Read-only review; nothing run. Read: the diff, `keys-34b8be41-correctness.md`, FR-022-022/031/053/057, D022-057 and
D022-059..063, the forced-retirement migration, the instance-registry SQL and the code paths below.

## Short answers

1. **Switched off with a key file: does it drain without admitting anything new?** In the store, yes. Admission checks
   the switch first (`crates/infra-postgres/src/execution.rs:239-246`), so a switched-off process refuses
   `CAPABILITY_DISABLED`: starts, resets (`create_from_prefix`, `start_with`, `PgRunStore::create`) and child starts
   (`passes.rs:6570`), whatever ring it holds. Publication reads the switches only (`definitions.rs:1075`). Nothing is
   listed (`served()` needs a switch). Grants, step decisions and cursors now work. New and not written down: a delayed
   gated start released while switched off takes a permit or queues (defensible as drain). **The drain cannot run in
   production while the fleet requirement is recorded**: a switched-off process is refused by the registry (Major 1).
2. **Cursor version check:** correct. The clear version selects the key and is part of the associated data
   (`keyed_digest.rs:347-355`, `387-393`); `sealed_version` reads the same four hex characters only after `open`
   succeeded; `coalesce(... = $1, false)` handles NULLs and the function always returns one row; the check runs before
   the namespace check and `admit`; a RETIRING version with no rows left answers 400 (untested, minor 4).
3. **The start gate's internal-error branch:** safe; no production path reaches it (Admission and Reset come only
   through `PgRunStore::create`, whose check refuses unless `served()`, which needs the ring; the transaction carries the
   same `Arc`; the ring is fixed at start).
4. **Tests:** the registry, cursor, label, `to_the_minute` and reset-guard tests each fail if their fix is reverted; the
   drain test would not, and covers neither the registry nor step decisions (minor 2); one assertion cannot fail
   (minor 3).
5. **Regressions:** none (config tests consistent; `groupadd --gid 10001` then `useradd --gid 10001` valid; `/data`
   owned by uid; chart unchanged).

## Major 1: with the fleet requirement recorded, a switched-off process is refused by the registry, so the drain never runs, and "Turning the switch off" stops every replica

Where: `docs/runbooks/flow-control.md:58-60`; D022-062; `apps/control-plane/src/lib.rs:552-554` and
`apps/execution-engine/src/capabilities.rs:32-34` (a switched-off process does not list `flow-control-concurrency-v1`);
`instance_register` answers `CAPABILITY_MISSING` (`20270113000000_instance_registry.up.sql:199-203`); a refused process
keeps its gate closed (`apps/control-plane/src/registry.rs:208-212`); FR-022-053.

Scenario: the fleet runs with the switch and the ring, the capability is activated; key K has a holder and two waiters;
the operator switches off with the file mounted and restarts. Each process registers without the capability, gets
`CAPABILITY_MISSING`, closes its gate (every API call 503, no pass, no shard). Nothing serves; the waiters are never
granted. The drain works only if the requirement is removed first, which by itself already refuses new gated starts
(`FLOW_CONTROL_ACTIVATION_PENDING`).

Why the tests miss it: the drain test builds a `Scheduler` on a store and never registers; the registry test checks
only the listed capabilities.

Smallest fix: state the order in the runbook and D022-062 (remove the requirement, then restart switched off), and a
test through the registry. Confidence: confirmed in code.

## minor 2: the drain test passes with or without the fix; two parts of D022-062 untested

It builds the switched-off store from a store that already holds the ring (green with 34b8be41's config); the holder's
completion runs on the switched-on world (step decisions by a switched-off process untested); no test runs
`check_flow_control_keys` for a switched-off process. Fix: build the store from a parsed configuration; a gated step a
switched-off store decides; a switched-off configuration whose ring lacks the active version refused.

## minor 3: the "nothing reset" assertion cannot fail

Resetting an ended source creates a new run; the source's journal is the same either way. Fix: assert no new run.

## minor 4: the "retiring, no rows left" case has no test; the check reads the database before `admit`

`flow_control_key_versions_in_force()` takes the active row `FOR SHARE`, so during an activation or force a paging GET
waits on the API pool, and a caller without `ops:read` triggers a round trip and gets 400 instead of 403. Fix: test the
ended rotation; run the check after `admit` (or warn in the runbook).

## minor 5: documents inconsistent

`docs/runbooks/flow-control.md:170` still says a start delay and a reset are refused `BINARY_CAPABILITY_MISSING`;
D022-057 still says a switched-off process neither reads nor holds a file, without an amendment mark; "Without the file,
waiters can only time out" leaves out delayed gated starts (their release fails every tick).

## minor 6: no task evidence names this commit or its tests

## minor 7 (predates the commit): a draining parent's gated child is neither started nor failed

With the switch off, a running parent's child of a gated definition gets `CAPABILITY_DISABLED` from `create`, which the
pass treats as an error every tick (`passes.rs:6570-6624`). Decide whether it is drain or a new start; map the refusal
to a typed step failure (extend T022-102).

## nit

`flow_control.rs:613` turns an out-of-range version into a 500; `Ok(false)` fails closed as a 400. `to_the_minute` cuts
in the value's own offset (UTC here; the contract says UTC).

## Checked with no finding

`to_the_minute` arithmetic and the unit test's constants; the label test red with the cut applied to revealers or to
the hour; `GateKey`'s `Debug`; the cursor test red with the check removed; the registry test red with 34b8be41's early
return; the admission test's two rings; `check_flow_control_keys` for every held ring.

---

## What was done (2026-11-20)

| Finding | Resolution |
|---|---|
| Major 1 | D022-062 amended (delegate): the capability is listed from the ring alone (`flow_control::listed`), the switch governs only new gated work, so a draining process stays in a fleet that requires the gate; `admission::a_draining_engine_with_the_keys_stays_in_a_fleet_that_requires_the_gate` (a real engine with the file and no switch registers listing the gate; with a file lacking the version in force it does not start); runbook rewritten |
| minor 2 | the drain test builds its store from the engine's parsed configuration (red with the early return); the switched-off ring check is in the test above; a gated step decided by a switched-off process is not added (the store path is the same as the grant's) |
| minor 3 | asserts that no reset run was created |
| minor 4 | the version check runs after `admit` (the opening stays before it); the ended-rotation case is not tested separately: it is `flow_control_key_versions_in_force()`'s own rule, proved by the rotation tests of `migrations::`, and the cursor test covers the function through a force |
| minor 5 | runbook line on delays and resets corrected; D022-057 marked amended; the switch-off paragraph names delayed starts and children |
| minor 6 | evidence added under T022-026 |
| minor 7 | a child start refused while switched off is a new start; its typed node failure added to T022-102 |
| nit | an out-of-range version is `false` (400) |
