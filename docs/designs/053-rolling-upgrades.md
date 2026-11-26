# Rolling upgrades with old and new writers on one database

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

An upgrade of the platform stops every writer today. This design makes it a rolling deployment: release N-1 and
release N run together on one PostgreSQL database while N's migrations are applied and the processes are replaced one by
one, under load. It has four parts:

1. **Migrations that expand before a release and contract after it**, with a lint that refuses a migration breaking
   the rules and a `migrate` command that never blocks the hot path for long.
2. **A handshake** by which a binary refuses a schema it cannot use, and a schema refuses a binary too old for it.
3. **New behaviour that waits for the fleet**: state that N-1 cannot process is written only after the instance
   registry has seen every live process able to process it, and stops being written before a rollback.
4. **A test** that runs N-1 and N writers on one database under load and gates every release.

The in-region topology and the platform's side of a failover are design 048; this design was split from it (Owner
decision 2026-11-24-4, the coordinating session's split of 2026-11-25).

**Terms used below.** The *instance registry* (Architecture decision 0082) records the live processes and their
*capabilities*; a process whose requirement is unmet keeps running with its *serving gate* closed (design 048 makes the
gate a set of reasons); the registry's *census* lists the processes seen within its liveness window of 30 s, and its
*activation* switches a capability on only when no live process lacks it. A *release* is a version cut by
`scripts/release-version.sh`; *N-1* is the release before *N*. Versions are compared in semantic-version order, where a
pre-release (`0.4.0-dev`) sorts below its release (`0.4.0`). The *hot tables* are those listed in
`scripts/check-migrations.toml` (the journal, runs, steps, leases, shards, the registry and the outbox at first). The
*integrity oracle* is the check of `tests/performance/scalability-ha/` that no acknowledged run was lost, no lease was
granted twice, and replaying each run's journal gives its projection; the *chain shape* is design 051's chain of three
activities, driven by design 051's driver.

## The Problem

From the code and documents on `develop` at `546d5aa5`:

- `docs/runbooks/scaling.md` (section 3) and the 0.2.0 release notes say to stop every writer, migrate, then deploy;
  `docs/runbooks/control-plane.md` says the opposite.
- No binary checks the schema it runs on, and the runtime role may not read `_sqlx_migrations`.
- sqlx runs each migration file as one simple query, one implicit transaction, with no lock timeout and no index built
  concurrently, so one migration can queue behind a long transaction and block the hot path behind it.
- The released-upgrade tests (`scripts/test-release-upgrade-profile.py`, the `released-upgrade` CI job) prove an old
  reader and an old worker against the new schema, not old and new writers together.
- `Cargo.toml` says `0.2.0` on `develop` after 0.2.0 was released, so a binary built from `develop` cannot be told from
  the release by its version.
- The registry learns a requirement within one heartbeat (10 s, jittered to 12 s; Architecture decision 0082, decision
  4), and a process that was silent at an activation can serve until its next heartbeat after it reconnects.

## Requirements

### Migrations

- **Requirement 053-001 Expand and contract.** Every migration after the lint's cut-off MUST declare its phase on the
  line after an optional `-- no-transaction`: `-- phase: expand` or `-- phase: contract`, and a contract names its
  expand and the capability that must be active before it runs (`-- contract-of: <version> <capability>`). An expand
  migration leaves the schema usable by release N-1; a contract migration removes only what N-1 no longer uses and ships
  in a release after its expand. A unique index or a unique or exclusion constraint on an existing table is a contract
  change, since N-1 does not expect its refusal, unless its columns include all those of a unique constraint the table
  already has (it then refuses nothing new). A migration on `develop` is never edited: CI compares each migration file a
  pull request touches with the same file at its merge base and refuses a change, and a test compares the applied
  migrations' checksums with `crates/infra-postgres/migration-checksums.txt`, written when a release is cut.
- **Requirement 053-002 Migrations do not block the hot path.** A transactional migration MUST take at most one lock
  that blocks inserts on a hot table, in its last statement, and every lock under `SET LOCAL lock_timeout = '2s'`; the
  lock is held through the migration's own commit, which waits for a standby (design 048), so the blocking time is the
  lock timeout plus one commit. A migration without a transaction MUST hold one concurrent statement only (an index
  built concurrently, preceded in its own earlier migration by `DROP INDEX CONCURRENTLY IF EXISTS` of the same name) and
  be idempotent; constraints are added `NOT VALID` and validated in a later migration; a migration MUST NOT rewrite a
  table, change a column's type in place, rename, add a column with a volatile default or a `NOT NULL` column without a
  default, run `SET NOT NULL` except after a validated check in a contract migration, or replace a function, trigger,
  policy or view in an expand migration (a changed function gets a new name; a security fix replaces in place under a
  waiver). A definer function pins `search_path` and is revoked from `PUBLIC`. A waiver is a header line `-- lint-allow:
  <rule> <reason>`, printed by the lint; the rule `stopped-upgrade` waives, for a migration that runs only in a stopped
  upgrade, every rule that assumes running writers (the lock rules, the concurrent-index rule, the contract rule for
  unique indexes, renames, and replaced constraints, triggers and functions), never the security rules.
- **Requirement 053-003 The lint and the runner.** `scripts/check-migrations.py` MUST refuse a migration after the
  cut-off (recorded in `scripts/check-migrations.toml`, the newest migration on `develop` when the lint merges) that
  breaks Requirement 053-001 or 053-002, by rules on its statements, with a negative fixture per rule under
  `scripts/tests/`; `.down.sql` files are checked only for the security rules. `control-plane migrate` MUST confirm the
  database as design 048's one-run commands do (Requirement 048-013: on a database that has ever been confirmed, it
  waits for a live registry row carrying the server's confirmed identity, so some process, N-1's or N's, must be running;
  Requirement 053-005 lets an N process confirm with its gate closed for the schema), then apply migrations one by one through its own loop over sqlx's
  migrator, which refuses an applied migration missing from the binary (sqlx's default, kept). It retries a migration
  that failed with `55P03` (lock not available) after 1 s, doubling to at most 30 s, at most 10 attempts, so it gives up
  after 1 + 2 + 4 + 8 + 16 + 30 × 4 = 151 s of waiting and exits non-zero naming the lock's table. After each migration
  without a transaction it checks `pg_index.indisvalid` of every index the migration names and, on an invalid one, drops
  it concurrently and retries, at most 3 times, then exits non-zero naming the index. It sets `transaction_timeout` to 0
  for its session where the server has it; refuses a contract migration unless its named capability is `active`
  (Requirement 053-006) and the census shows no process older than the release before it; and refuses to run from an
  older release than the database's newest row of `schema_compatibility` (a database without the table, fresh or 0.2.x,
  is migrated from the start).

### Versions and the handshake

- **Requirement 053-004 A version names one build line.** A binary's release is `CARGO_PKG_VERSION`. When release
  `X.Y.Z` is cut, `scripts/release-version.sh` MUST also move `develop` to `X.(Y+1).0-dev`, so that no build from
  `develop` carries a released version; a hotfix branch carries its own patch version. The script writes
  `crates/infra-postgres/releases.toml` (`version`, `last_migration`, `oldest_compatible`) and a migration that inserts
  that release's row into `schema_compatibility (release text primary key, last_migration bigint, oldest_compatible
  text, recorded_at timestamptz)`. A hotfix release adds expand migrations only, and each is merged forward to `develop`
  before the next release is cut: the script refuses to cut a release whose tree lacks a migration listed in an earlier
  release's checksum file.
- **Requirement 053-005 A binary refuses a schema it cannot use.** A process calls `sagat_schema_handshake()`
  (owner-defined, `SECURITY DEFINER`, `search_path` pinned, executable by `sagat_app` and by design 049's
  `sagat_retirer` only), which returns the applied migration versions and the effective oldest compatible release
  (Requirement 053-006), at start, at each confirmation of design 048 (Requirement 048-013), and with each registry
  heartbeat, so that a gate it closed reopens within one heartbeat once the schema is right. The process sets the gate's
  reason `SCHEMA_TOO_OLD` when an expand migration it was built with is not applied (its own contract migrations are
  never required, since a contract runs only after the fleet is on N), or `BINARY_TOO_OLD` when its release is older
  than the effective oldest compatible release; it then writes nothing but its registry rows (registration, design 048's confirmation and the heartbeat)
  and the release of its shards, and answers 503. A process closed only by `SCHEMA_TOO_OLD` still registers and
  confirms, so that `migrate` has a confirmed process to wait for when no N-1 process runs. The registry's functions in their new versions, `instance_register_v2` and
  `instance_heartbeat_v2`, return the requirement set with the epoch when a registration succeeds or the epoch differs
  from the one the process sends with its heartbeat (the gate's own, so a lost answer is asked again), and the gate
  keeps it (design 049's Task 049-5), and refuse such a process in the transaction that already reads the requirement
  epoch, so that an activation and a registration cannot race. Every entry point that opens the database calls the
  handshake: the control plane, the engine, each command of `control-plane` (the `secret-cleanup` job included) and
  `migrate`, checked by `scripts/check-architecture.py`. The handshake tests run as the runtime role. `BINARY_TOO_OLD`
  wins over `BINARY_CAPABILITY_MISSING` when both apply. A 503 shows no version to the caller; the versions go to the
  log.

### New behaviour and the fleet

- **Requirement 053-006 One rule for the oldest compatible release.** The effective oldest compatible release MUST be
  computed, never stored as a counter: the greatest of the newest `schema_compatibility` row's `oldest_compatible` and,
  for each capability whose state is `active` or `draining`, the release that introduced it (`introduced_in`).
  `platform_requirements` gains `state` (`active`, `draining`), `introduced_in`, `draining_since` and `permanent`, added
  by an expand migration; `sagat_oldest_compatible()` computes the value and both the handshake and the v2 registry
  functions call it.
- **Requirement 053-007 New behaviour waits for the fleet.** A change that writes state release N-1 cannot process MUST
  be behind a registry capability, added to the capability lists by an expand migration (the lists are closed checks,
  rewritten `NOT VALID` and validated in the next migration). `platform_requirement_activate_v2` switches it to `active`
  only when the census shows no live process older than its `introduced_in` and none without it. A process that was not
  live at the activation (silent for 30 s) can reconnect and serve until its next heartbeat, about 12 s, before the
  registry refuses it; so each capability's design MUST state either why a write of release N-1 after the activation is
  harmless, or the database-side fence (a trigger, a constraint or a default) that refuses it. In release 0.3.0 these
  are design 049's dated journal buckets (harmless: a legacy root created by a process that has not yet learned of the
  activation is valid and processed by every release) and design 052's ready queue (which states its own rule).
- **Requirement 053-008 A deactivation drains first.** `platform_requirement_deactivate_v2` MUST work in two phases. The
  first moves the capability to `draining`, records `draining_since`, appends a history row and moves the requirement
  epoch, so that every process learns of it at its next heartbeat (processes learn of a change only when the epoch
  moves); a process treats a `draining` capability as inactive and starts no new use of it. The registry's guard
  (`platform_requirements_guard`) allows only that one update, from `active` to `draining` on a row that is not
  permanent, and its deferred epoch check covers updates as well as inserts and deletes. The second phase removes the
  row and is refused until every live process has heartbeated after `draining_since` (at least 12 s, the heartbeat's
  jittered bound), the drain check its design states passes, and no contract migration naming the capability is applied.
  Only after the second phase may the binaries roll back to N-1. A capability whose design declares it permanent is
  refused by both phases and by the registry's older `platform_requirement_remove`: the guard refuses the delete or the
  move to `draining` of a permanent row. `platform_requirement_activate_v2` writes `permanent` from the list of
  permanent capabilities, and a check, `permanent = (capability IN (...))`, rewritten by the migration that adds a
  permanent capability (and written `permanent = false` while the list is empty), refuses a wrong value. In 0.3.0 the
  list holds design 049's `journal-buckets-v1`, whose drain check would be "no run of a dated bucket exists", since a
  binary of the release before it cannot read a dated run's content.
- **Requirement 053-009 Old and new writers run together.** `scripts/test-mixed-writers.py` MUST run engines and control
  planes of N-1 (built from its release's tag) and N on one database under the chain shape: apply N's migrations during
  the load, roll the processes one at a time, activate N's capabilities that are not permanent, deactivate them in both
  phases and roll one process back to N-1, then roll it forward and activate the permanent ones, and pass the integrity
  oracle, with the 99th percentile transition latency during the migrations no more than 2 s (the lock timeout) above a
  baseline run. Release N MUST NOT be published unless it passes against N-1. The test first gates a release when 0.3.0
  is N-1, since 0.2.x binaries have no handshake; until then it runs in CI with both sides built from `develop` at two
  commits, which proves the migration rules and the oracle but not the version gates, and those are proved by tests with
  fixed versions (Task 053-3). The run names its runner, a free standard runner (Owner decision 2026-11-25-1).

### Success criteria

- **Success criterion 053-001** In the mixed-writer test, the completion rate never falls to zero for more than 10 s, the
  integrity oracle passes, and the 99th percentile transition latency during the migrations is within 2 s of the
  baseline's.
- **Success criterion 053-002** The migration lint refuses each of its negative fixtures and passes every migration
  merged after its cut-off.
- **Success criterion 053-003** A binary started before its migrations answers 503 with its gate closed, and serves
  within one heartbeat (12 s) after they are applied.

### User stories

- **User story 053-01** An operator upgrades the platform during the day with runs in flight.
- **User story 053-02** An operator rolls back an upgrade that misbehaves, without losing a run.

## Goals

- An upgrade is a rolling deployment, as for any stateless service.
- A binary on the wrong schema cannot write state that another binary cannot read.

## Non-Goals

- **The 0.2.x to 0.3.0 upgrade.** It stays a stopped upgrade: 0.2.x binaries have no handshake.
- **Downgrading the schema.** A rollback replaces binaries; it never runs N-1's migrations, and the runbook forbids
  running N-1's migration job during one.
- **A rolling upgrade drill on Kubernetes.** It needs a paid runner (design 048, "Owner decisions needed"); the
  mixed-writer test on Compose is the gate.
- **Blue and green databases** (Alternatives Considered).

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6, with its reasoning.

1. **Two releases for every removal?** Yes. A removal in the same release as its replacement breaks N-1 while both run;
   the cost is a second, small migration in the next release. Delegate.
2. **Where does the oldest compatible release live?** It is computed from the newest release row and the capabilities in
   use (Requirement 053-006), so that an activation, a deactivation and a migration can never leave it out of step.
   Delegate.
3. **What does the version of a build from `develop` say?** The next minor's pre-release, moved by the release script,
   so that the version gates can tell builds apart without a second identifier. Delegate.
4. **The decision record.** [Architecture decision 0131](../adr/ADR-0131-migrations-expand-before-and-contract-after-a-release-and-a-binary-refuses-a-schema-it-cannot-use.md),
   which refines Architecture decision 0082 (two more gate reasons, a computed oldest compatible release, a deactivation
   in two phases).

## Critical User Journeys (CUJs)

### CUJ 1: A rolling upgrade

*   **Given**: release N-1 running.
*   **When**: the operator runs N's migrations and rolls the processes to N.
*   **Then**: N-1 and N processes run together; N writes no new state until the activation, which waits for the census;
    the contract migrations of N's expands ship with N+1 and wait for their capability.

### CUJ 2: A binary on the wrong schema

*   **Given**: release N deployed before its migrations were applied.
*   **When**: it starts.
*   **Then**: it closes its serving gate with `SCHEMA_TOO_OLD`, answers 503, writes nothing, and opens the gate within one
    heartbeat after the migrations are applied.

### CUJ 3: A rollback

*   **Given**: release N running with its capabilities active, misbehaving.
*   **When**: the operator deactivates N's capabilities and rolls the processes back.
*   **Then**: the first phase stops new uses; the second waits for every process and the drain check; the binaries roll
    back to N-1 on N's schema, which N-1 can use. A release that activated a permanent capability (design 049's
    `journal-buckets-v1` in 0.3.0) is fixed by rolling forward, never back below it.

## Technical Plan (Architecture)

**The order of an upgrade**, written in `docs/runbooks/scaling.md` section 3 in place of the stopped upgrade, and in
`docs/runbooks/control-plane.md` and `docs/runbooks/execution-engine.md` so that they agree:

1. Apply N's migrations (`control-plane migrate`) while N-1 runs; contract migrations among them wait for their named
   capability and the census.
2. Roll the control planes and the engines to N, one at a time (`maxUnavailable: 0`, `maxSurge: 1`).
3. Activate N's capabilities.

Rollback before step 3 rolls the binaries back onto N's schema; after it, the two-phase deactivation comes first.

When no process runs (an upgrade the operator chose to stop, or a start after a restore), the order is: start N's
processes, which register and confirm with their gate closed by `SCHEMA_TOO_OLD`; apply N's migrations; the gates open
within one heartbeat. `migrate` on a database that has been confirmed and has no live confirmed process exits with code
1 ("retry later") and says so. The 0.2.x to 0.3.0 upgrade is the exception: the migration that creates design 048's wait
runs in it, before any process can confirm, and the wait admits a database never confirmed (Requirement 048-013).

| Change | Files | Requirement |
|---|---|---|
| The lint, its fixtures, the merge-base and checksum checks | `scripts/check-migrations.py`, `scripts/check-migrations.toml`, `scripts/tests/`, `tests/integration/platform-foundation/migrations.rs`, `crates/infra-postgres/migration-checksums.txt`, CI | 053-001, 053-002, 053-003 |
| `migrate`'s loop | `apps/control-plane/src/lib.rs`, `crates/infra-postgres/src/lib.rs` | 053-003 |
| Versions, the manifest, the compatibility row | `scripts/release-version.sh`, `crates/infra-postgres/releases.toml`, the root `Cargo.toml` | 053-004 |
| The handshake, the gate reasons, the v2 registry functions | `crates/infra-postgres/src/instances.rs`, `crates/application/src/instances.rs`, every entry point of `apps/` | 053-005 |
| The computed oldest compatible release, activation and deactivation v2 | a migration, `crates/infra-postgres/src/instances.rs`, the operator commands of `control-plane` | 053-006, 053-007, 053-008 |
| The mixed-writer test | `scripts/test-mixed-writers.py`, the release workflow | 053-009 |

**The new table and columns**, each created by an expand migration with its grants and storage classification:

| Object | Columns | Runtime role |
|---|---|---|
| `schema_compatibility` | as Requirement 053-004 | none; read through `sagat_schema_handshake()` |
| `platform_requirements` (new columns) | `state text` (`active`, `draining`), `introduced_in text`, `draining_since timestamptz`, `permanent boolean not null default false` | as the table today (read) |

**Order of the build.** Design 048's Tasks 048-9 and 048-10 (the gate's reasons and the confirmation) before Tasks
053-3 and 053-4; Task 053-2 (the lint) before any other migration of designs 048, 049 and 052; Task 053-5 before
design 049's Task 049-13 (the permanent column) and Task 053-3 before its Tasks 049-5 (the requirement set returned at
registration) and 049-14 (the handshake granted to the retirement role); design 051's driver (its Task 051-2) before Task
053-6.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Old and new writers disagree on new state | Requirement 053-007; each capability's fence; the mixed-writer test on every release |
| A process that was silent at an activation writes N-1 state | The capability's database-side fence, or its stated harmlessness |
| The migration lint blocks a needed change | A waiver header naming the rule and the reason |
| A migration waits on a long transaction | The lock timeout, the bounded retry, and the exit naming the table |
| A failed concurrent index build loops | At most 3 retries, then an exit naming the index |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No transition changes; new state waits for the fleet; one transaction per transition unchanged |
| II. Modular domain, ports and adapters | Pass | The handshake and the gate reasons are in the application's instance service; statements stay in `infra-postgres` |
| III. External effects are at-least-once; sagas compensate | Pass | Unchanged |
| IV. Compatibility is versioned and pinned | Pass | Expand and contract, the compatibility table, the handshake, distinct versions, the mixed-writer test |
| V. Code, tenant boundary and secrets are isolated | Pass | The handshake is a definer function executable by the runtime role and the retirement role only; a 503 shows no version |
| VI. Observability and audit are part of the feature | Pass | `serving_gate_closed{reason}` with the two reasons; the activation and deactivation recorded in the registry's history |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Unchanged |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Negative fixtures per rule, the injected failed index build, the mixed-writer test; what it cannot prove before 0.3.0 is said |
| Delivery Constraints | Pass | Constitution 5.1.0 (Owner decision 2026-11-25-3): its throughput regression gate (point 3) is design 052's and runs in CI; the mixed-writer test is the upgrade gate of each release beside it; nothing here deletes journal rows (point 1) |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| The compatibility table, the handshake and the migration runner | A binary on the wrong schema writes state nobody can read; a contract must wait for its capability | Documentation only: the runbooks contradict each other today |
| The migration lint | The rules are easy to forget and costly when broken | Review only |
| A two-phase deactivation | Processes learn of a change within one heartbeat, not at once | One phase: a process may still write the capability's state after it is gone |

## Alternatives Considered

- **A maintenance window per upgrade (today).** Safe and simple, and it gives up the main operational property a
  durable-execution platform is chosen for.
- **Blue and green upgrades with two databases.** Logical replication of the whole schema, with its triggers and definer
  functions firing again on apply.
- **A stored oldest compatible release, raised and lowered by each activation.** Two capabilities and a deactivation can
  leave it wrong; computing it cannot.
- **The build's commit as a second identity.** Every gate would compare two identifiers; moving `develop`'s version is
  enough.

## Non-Functional Requirements

- **Non-functional requirement 053-001 Hot path.** No statement is added to a lease or a completion; the handshake runs
  at start, at each confirmation and once per heartbeat (every 10 s per process).
- **Non-functional requirement 053-002 Migration impact.** A migration blocks inserts on a hot table for at most its lock
  timeout plus one commit (Requirement 053-002), measured by the mixed-writer test.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Each task names its files and tests.

- [ ] T053-1 Revise Architecture decision 0131 to this text; add its refinement note to Architecture decision 0082; write the order of an upgrade in the three runbooks and the 0.3.0 release notes.
  Refs: Requirement 053-001, Requirement 053-007, Requirement 053-008
- [ ] T053-2 Write `scripts/check-migrations.py`, `scripts/check-migrations.toml` (the cut-off and the hot tables), its negative fixtures under `scripts/tests/` (the unique index outside a contract among them), the merge-base check of edited migrations, the checksum test in `tests/integration/platform-foundation/migrations.rs` against `crates/infra-postgres/migration-checksums.txt`, and their CI steps.
  Refs: Requirement 053-001, Requirement 053-002, Requirement 053-003, Success criterion 053-002
- [ ] T053-3 Move `develop`'s version in `scripts/release-version.sh`, write `releases.toml` and the compatibility row, the refusal of a release missing an earlier migration; add `schema_compatibility`, `sagat_schema_handshake()`, `instance_register_v2` and `instance_heartbeat_v2`, the handshake at start, at confirmation and with each heartbeat in every entry point with its architecture check, the gate reasons in the registry contract of feature 018, with the tests `a_binary_before_its_migrations_serves_within_one_heartbeat_after_them` and `a_release_below_the_oldest_compatible_is_refused_at_registration` on fixed versions, as the runtime role.
  Refs: Requirement 053-004, Requirement 053-005, Success criterion 053-003
- [ ] T053-4 Write `control-plane migrate`'s own loop (the confirmation, the bounded retry on `55P03`, the bounded invalid-index check, the contract refusals, the release check) in `apps/control-plane/src/lib.rs` and `crates/infra-postgres/src/lib.rs`, with the tests `a_failed_concurrent_index_build_is_retried_three_times_then_named`, `a_lock_held_longer_than_the_retries_ends_migrate_naming_the_table` and `migrate_with_no_process_running_proceeds_once_a_new_process_confirms_with_its_gate_closed`.
  Refs: Requirement 053-003
- [ ] T053-5 Add the capability state columns, `sagat_oldest_compatible()`, `platform_requirement_activate_v2` and the two-phase `platform_requirement_deactivate_v2` with the contract refusal, the column `permanent` with its check, the guard's single allowed update and its refusal of a permanent row's delete or drain, the epoch moved by the first phase, `activate_v2` writing `permanent` (the check written `permanent = false` while no capability is permanent), with the tests `the_oldest_compatible_release_follows_two_capabilities_and_a_deactivation`, `the_second_phase_waits_for_every_live_process_to_heartbeat` and `a_draining_capability_stops_new_uses_at_the_next_heartbeat` and `a_permanent_capability_refuses_both_deactivation_phases` (which also calls `platform_requirement_remove`) run here against a test-only capability added to the lists in a disposable database, and again by design 049's Task 049-13 against `journal-buckets-v1`, held by barriers.
  Refs: Requirement 053-006, Requirement 053-007, Requirement 053-008
- [ ] T053-6 Write `scripts/test-mixed-writers.py` and add it to the release workflow on a free standard runner; it uses design 051's driver (Task 051-2) and publishes its run with the runner named.
  Refs: Requirement 053-009, Success criterion 053-001

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs the suites of what it touches, `integration_claim_transition_efficiency` when it touches
`crates/infra-postgres/`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and the documentation and
architecture checkers of `scripts/`, the new lint among them. Each named test of Tasks 053-3 to 053-5 is seen failing
first, before the change; those of Tasks 053-3 and 053-5 also run with `SAGAT_TEST_AS_RUNTIME_ROLE=1`. Task 053-6 is the
release gate for upgrades.
