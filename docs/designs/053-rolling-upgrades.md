# Rolling upgrades with old and new writers on one database

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

An upgrade of the platform stops every writer today. This design makes it a rolling deployment: release N-1 and
release N run together on one PostgreSQL database while N's migrations are applied and the processes are replaced one by
one, under load. It has four parts:

1. **Migrations that only expand while two releases run**, with a lint that refuses a migration breaking the rules and
   a `migrate` command that never blocks the hot path for long.
2. **A schema check** by which a binary keeps its serving gate closed until the migrations it was built with are
   applied.
3. **New behaviour behind the instance registry's capabilities**: state that N-1 cannot process is written only after
   the registry has seen every live process able to process it, and stops being written, provably, before a rollback.
   The capabilities are the one mechanism that keeps an older binary away from newer state; there is no second,
   version-based gate.
4. **A test** that runs N-1 and N writers on one database under load and gates every release.

The in-region topology and the platform's side of a failover are design 048; this design was split from it (Owner
decision 2026-11-24-4, the coordinating session's split of 2026-11-25).

**Terms used below.** The *instance registry* (Architecture decision 0082) records the live processes and their
*capabilities*; a process whose requirement is unmet keeps running with its *serving gate* closed (design 048 makes the
gate a set of reasons); the registry's *census* lists the processes seen within its liveness window of 30 s, and its
*activation* switches a capability on only when no live process lacks it; the *requirement epoch* is a counter that
every change of the requirements moves, and a process reads the requirements again whenever it sees the epoch move. A
*release* is a version cut by the release workflow; *N-1* is the release before *N*. The *hot tables* are those listed
in `scripts/check-migrations.toml` (the journal, runs, steps, leases, shards, the registry and the outbox at first). The
*integrity oracle* is the check of `tests/performance/scalability-ha/` that no acknowledged run was lost, no lease was
granted twice, and replaying each run's journal gives its projection; the *chain shape* is design 051's chain of three
activities, driven by design 051's driver.

## The Problem

From the code and documents on `develop` at `546d5aa5`:

- `docs/runbooks/scaling.md` (section 3) and the 0.2.0 release notes say to stop every writer, migrate, then deploy;
  `docs/runbooks/control-plane.md` says the opposite.
- No binary checks the schema it runs on, and the runtime role may not read `_sqlx_migrations`.
- sqlx runs each migration file as one simple query, one implicit transaction, with no lock or statement timeout and no
  index built concurrently, so one migration can queue behind a long transaction and block the hot path behind it.
- The released-upgrade tests (`scripts/test-release-upgrade-profile.py`, the `released-upgrade` CI job) prove an old
  reader and an old worker against the new schema, not old and new writers together.
- The registry learns a requirement within one heartbeat (10 s, jittered to 12 s; Architecture decision 0082, decision
  4), and a process that was silent at an activation can serve until its next heartbeat after it reconnects. A removal
  of a requirement is one statement: a process may still be writing the capability's state when it is gone.

## Requirements

### Migrations

- **Requirement 053-001 Only expand while two releases run.** Every migration after the online rules' start
  (Requirement 053-003) MUST be an
  *expand* migration: one that leaves the schema usable by every older binary the registry still admits, adding and
  never removing or narrowing. A migration on an existing table MUST NOT add a refusal an older binary can hit: no new
  `CHECK`, foreign key, `NOT NULL`, unique index or constraint, exclusion constraint or trigger that raises, unless the
  new refusal is conditional on a capability's row being `active` or `draining` (a fence of Requirement 053-007, a
  trigger that reads `platform_requirements`), or a unique one covers all the columns of a unique constraint the table
  already has that is neither partial nor deferrable (it then refuses nothing new). Removing what an older release
  used (a *contract* migration) is out of scope until a design needs the first one; that design adds the contract rules
  (Non-Goals) and, until it does, the lint refuses the header `-- phase: contract`. A migration on `develop` is never
  edited: CI compares each migration file a pull request touches with the same file at its merge base and refuses a
  change; sqlx's own checksums refuse an applied migration whose file changed.
- **Requirement 053-002 Migrations do not block the hot path.** A transactional migration MUST take at most one lock
  that blocks inserts on a hot table, in its last statement, which runs under `SET LOCAL lock_timeout = '2s'` and
  `SET LOCAL statement_timeout = '2s'` (every lock wait and the statement's own run, partitions included, count against
  the second); the lock is held through the migration's own commit, which waits for a standby (design 048), so a
  migration blocks inserts on a hot table for at most 2 s plus one commit. On a hot table a migration MUST NOT build an
  index, a unique constraint or a primary key except concurrently. A migration without a transaction MUST hold one
  concurrent statement only (an index built concurrently, `IF NOT EXISTS`), run in a session whose `lock_timeout` is 2 s,
  and be idempotent. Constraints are added `NOT VALID` and validated in a later migration (the existing static test
  `a_constraint_added_not_valid_is_validated_by_a_later_migration` stays the one owner of that rule). A migration MUST
  NOT rewrite a table, change a column's type in place, rename, add a column with a volatile default or a `NOT NULL`
  column without a default, or replace a function, trigger, policy or view that a released binary calls or relies on
  (a changed function gets a new name; a security fix replaces in place under a waiver). A definer function pins
  `search_path` and is revoked from `PUBLIC`. A waiver is a header line `-- lint-allow: <rule> <reason>`, printed by the
  lint.
- **Requirement 053-003 The lint.** `scripts/check-migrations.py` MUST refuse a migration after the online rules'
  start that breaks Requirement 053-001 or 053-002, by rules over the parse tree of each statement given by `pglast` (the Python binding
  of PostgreSQL's own parser, `libpg_query`, pinned with its hash in `scripts/requirements-lint.txt`), which reads
  dollar-quoted bodies and `DO` blocks as PostgreSQL does, with a negative fixture per rule under `scripts/tests/`. `scripts/check-migrations.toml` records two
  boundaries. The *cut-off* is the newest migration on `develop` when the lint merges: the security rules (a definer's
  pinned `search_path`, the revocation from `PUBLIC`) apply to every migration after it, `.up.sql` and `.down.sql`
  alike, and design 048's protected-names rule uses it. The *online rules' start* is the last migration of release 0.3.0:
  every migration of 0.3.0 runs in the stopped upgrade from 0.2.x, where the rules that assume running writers protect
  nothing, so they apply only after it, and a waiver header before it is accepted and printed. After the online rules'
  start `.down.sql` files are checked only for the security rules. An index on a partitioned table is refused unless the migration uses the partitioned-index
  procedure of Requirement 053-004, and until that procedure is built every such index is refused.
- **Requirement 053-004 The runner.** `control-plane migrate` MUST confirm the database as design 048's one-run commands
  do (Requirement 048-013: on a database that has ever been confirmed, it waits for a live registry row carrying the
  server's confirmed identity, so some process, N-1's or N's, must be running; Requirement 053-005 lets an N process
  confirm before N's migrations), then apply migrations one by one through its own loop over sqlx's migrator, which
  refuses an applied migration missing from the binary (sqlx's default, kept: a `migrate` older than the database stops
  there). It retries a migration that failed with `55P03` (lock not available) or `57014` (statement timeout) after 1 s,
  doubling to at most 30 s, at most 10 attempts, then exits non-zero naming the lock's table; the nine waits between the ten attempts sum to
  1 + 2 + 4 + 8 + 16 + 30 × 4 = 151 s and the attempts to at most 10 × 2 = 20 s, so it gives up within about 171 s.
  After each migration without a transaction it checks `pg_index.indisvalid` of every index the migration names and,
  on an invalid one (a concurrent build that failed, its wait for older transactions included), drops it concurrently
  and retries, at most 3 times, then exits non-zero naming the index. It sets `transaction_timeout` to 0 for its
  session where the server has it. `migrate` does not call the schema check of Requirement 053-005: its own release is
  always the newest, and sqlx's refusal covers an older one. **The partitioned-index procedure**, built by the first
  design that adds an index to a partitioned hot table after 0.3.0 (design 049 defers its later indexes to it), is
  defined here so that no design invents its own: a migration whose header is `-- partitioned-index` and whose one
  statement is `CREATE INDEX <name> ON <parent> (...)` is run by the loop as `CREATE INDEX IF NOT EXISTS <name> ON ONLY
  <parent>` under the lock and statement timeouts, then, for each partition `pg_inherits` lists, `CREATE INDEX
  CONCURRENTLY IF NOT EXISTS` and `ALTER INDEX <name> ATTACH PARTITION`, listing the partitions again until the parent's
  index is valid (a partition created meanwhile by `ensure_journal_buckets()` receives the index at its creation), and
  only then records the migration as applied.

### The schema check

- **Requirement 053-005 A binary keeps its gate closed on a schema it cannot use.** A process calls
  `sagat_schema_missing(versions bigint[])` (owner-defined, `SECURITY DEFINER`, `search_path` pinned, executable by
  `sagat_app` and by design 049's `sagat_retirer` only), which returns the versions among those it is given that are not
  applied with `success = true` in `_sqlx_migrations`; the process gives the versions of the migrations it was built
  with. It calls it at start, at each confirmation of design 048 (Requirement 048-013), and with each heartbeat only
  while the answer was not empty, so that a gate it closed reopens within one heartbeat once the schema is right. While
  the answer is not empty the process sets the gate's reason `SCHEMA_TOO_OLD`, writes nothing but its registry rows
  (registration, design 048's confirmation and the heartbeat) and the release of its shards, and answers 503. Such a
  process registers only the capabilities whose migration the answer shows applied, so that the registry's closed list
  of capability names, which its own release's migrations extend, accepts it; it registers again with its full list
  once the answer is empty. Every function a binary calls before its migrations are applied (the registration, the
  heartbeat, design 048's confirmation and this check) MUST exist with the same name and signature in the schema of the
  release before it: a release that needs a new one adds it under a new name and the binary calls the old one until the
  check's answer is empty. Every entry point that opens the database except `migrate` calls the check: the control
  plane, the engine and each command of `control-plane` (the `secret-cleanup` job included), checked by
  `scripts/check-architecture.py`; a one-run command whose answer is not empty exits with code 1 ("retry later"), and a
  one-run command also checks, with the registry's `missing_for`, that it has every capability the registry requires of
  its kind. The check's tests run as the runtime role. A 503 shows no version to the caller; the versions go to the log.

### New behaviour and the fleet

- **Requirement 053-006 New behaviour waits for the fleet.** A change that writes state release N-1 cannot process MUST
  be behind a registry capability, added to the capability lists by an expand migration (the lists are closed checks,
  rewritten `NOT VALID` and validated in the next migration), and required of every kind of process that could meet the
  state. `platform_requirement_activate` switches it on only when the census shows no live process without it; a
  process refused a registry row because the table is full serves no capability call (decision D006-100). A process
  that was not live at the activation (silent for 30 s) can reconnect and serve until its next heartbeat, about 12 s,
  before the registry refuses it, and a process not live at the second phase of a deactivation (Requirement 053-007)
  can, after it, still hold the capability as active until its next heartbeat; so each capability's design MUST state,
  for both, either why such a late write is harmless or the database-side fence that refuses it, conditional on the
  capability's row (`active` or `draining`, read by a trigger), never a bare check that would still refuse N-1 after a
  rollback. In release 0.3.0 these are design 049's dated journal buckets (harmless: a legacy root created by a process
  that has not yet learned of the activation, or of a drain, is valid and processed by every release) and design 052's
  ready queue (which states its own rule).
- **Requirement 053-007 A deactivation drains first.** Deactivating a capability MUST take two phases.
  - **Phase 1** (`platform_requirement_drain`) moves the row to `draining`, records `draining_since`, appends a history
    row and moves the requirement epoch, so that every process learns of it at its next heartbeat; a process treats a
    `draining` capability as inactive and starts no new use of it. The checks that refuse a removal today
    (`platform_requirements_handover_guard` for `schedules-v1`, `platform_requirements_replay_floor_guard`) run on the
    move to `draining` as well as on the delete. `platform_requirement_undrain` moves a `draining` row back to `active`,
    with the epoch moved and a history row, for a deactivation that must be abandoned.
  - **Phase 2** (`platform_requirement_remove`, the existing function and `PgInstanceStore::remove`) deletes the row,
    and the registry's guard (`platform_requirements_guard`) refuses the delete of a row that is not `draining`. It is
    refused, under the epoch lock it already takes, until every live registry row has *applied* the epoch phase 1 wrote:
    the heartbeat `instance_heartbeat_applied(instance, token, applied_epoch)`, which succeeds
    `instance_heartbeat_reporting_change` (decision D006-098) and does what it does, records in the new column
    `serving_instances.applied_epoch` the epoch whose requirement set the process has already applied, which it sends;
    a beat that overlapped phase 1 and read the old epoch therefore cannot pass phase 2. Phase 2 also runs the
    capability's own drain check, the `verify` step the removal already takes. Only after phase 2 may the binaries roll
    back to N-1.
  - **A capability that cannot be removed** is one whose drain check cannot pass once the capability has been used. In
    0.3.0 that is design 049's `journal-buckets-v1`, whose drain check is "no run of a dated bucket exists", since a
    binary of the release before it cannot read a dated run's content; its phase 1 is allowed (new roots are then
    legacy, which every release processes) and can be undone.
  - The guard's deferred epoch check covers updates as well as inserts and deletes. `state` is `text NOT NULL DEFAULT
    'active'`, so the existing rows and the existing `platform_requirement_activate` need no change.
- **Requirement 053-008 Old and new writers run together.** `scripts/test-mixed-writers.py` MUST run engines and control
  planes of N-1 and N on one database under the chain shape: apply N's migrations during the load, roll the processes
  one at a time, activate N's capabilities, drain and remove the ones whose drain check can pass and roll one process
  back to N-1, then roll it forward and activate them again. It passes when the integrity oracle passes, every run
  started during the test reaches a terminal state within 120 s after the load ends (a run N-1 ignores stays stuck, which
  the oracle alone does not see), and the 99th percentile transition latency during the migrations is no more than 2 s
  above a baseline run. Each capability's design adds a scenario to it: state written by N, then processed by N-1 after
  the rollback, and the reverse. Release N MUST NOT be published unless it passes against the newest patch release of
  the minor before it. The test reuses the released-tree build and checks of `scripts/test-release-upgrade-profile.py`
  (and the `released-upgrade` job's full history checkout). It first gates a release when 0.3.0 is N-1, since 0.2.x
  binaries have no schema check; until then it runs in CI with N-1 built at the pull request's merge base and N at its
  head, which proves the migration rules and the oracle, and the schema check is proved by tests on a fixed pair of
  migration sets (Task 053-3). The run names its runner, a free standard runner (Owner decision 2026-11-25-1).
- **Requirement 053-009 A hotfix upgrades forward.** A hotfix release adds expand migrations only, and each is merged
  forward to `develop` before the next minor release is cut: the release workflow refuses to cut a release whose tree
  lacks a migration file present in the tree of any earlier release tag (`git ls-tree` of each `v*` tag of a lower
  version), since sqlx refuses to upgrade a database holding a migration the binary lacks. The runbook states that a
  hotfix `X.Y.Z` upgrades only to a release cut after it.

### Success criteria

- **Success criterion 053-001** In the mixed-writer test, the completion rate never falls to zero for more than 10 s,
  the integrity oracle passes, every run reaches a terminal state within 120 s after the load, and the 99th percentile
  transition latency during the migrations is within 2 s of the baseline's.
- **Success criterion 053-002** The migration lint refuses each of its negative fixtures and passes every migration
  merged after its cut-off.
- **Success criterion 053-003** A binary started before its migrations answers 503 with its gate closed, and serves
  within one heartbeat (12 s) after they are applied.
- **Success criterion 053-004** Phase 2 of a deactivation is refused while a live process has not applied phase 1's
  epoch, including a process whose beat overlapped phase 1.

### User stories

- **User story 053-01** An operator upgrades the platform during the day with runs in flight.
- **User story 053-02** An operator rolls back an upgrade that misbehaves, without losing a run.

## Goals

- An upgrade is a rolling deployment, as for any stateless service.
- A binary on the wrong schema cannot write state that another binary cannot read.

## Non-Goals

- **The 0.2.x to 0.3.0 upgrade.** It stays a stopped upgrade: 0.2.x binaries have no schema check.
- **Contract migrations.** No release before the first contract needs them; the design that ships the first one adds
  the rules (a contract ships a release after its expand, names the capability that must be `active` before it runs,
  is recorded with that capability in an owner table under the requirement row's share lock before it runs, and phase
  2 of that capability's deactivation reads that table under the epoch lock, so that a contract and a deactivation
  cannot race) and lifts the lint's refusal of `-- phase: contract`.
- **A version-based gate.** Binaries are told apart by capabilities and by the migrations they were built with, not by
  comparing release versions (Alternatives Considered); the registry keeps recording `binary_version` for operators.
- **Downgrading the schema.** A rollback replaces binaries; it never runs N-1's migrations, and the runbook forbids
  running N-1's migration job during one.
- **A rolling upgrade drill on Kubernetes.** It needs a paid runner (design 048, "Owner decisions needed"); the
  mixed-writer test on Compose is the gate.
- **Blue and green databases** (Alternatives Considered).

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6, with its reasoning.

1. **Two releases for every removal?** Yes, once contracts exist. A removal in the same release as its replacement
   breaks N-1 while both run; the cost is a second, small migration in the next release. Delegate.
2. **How is an older binary kept away from newer state?** By capabilities alone: every write N-1 cannot process is
   behind one, so a binary lacking it is refused by the registry. A second gate on release versions would need a
   version order shared by Rust, SQL, the SDKs and the Studio, and a version on `develop` distinct from the release,
   for no case the capabilities miss. Delegate.
3. **The decision record.** [Architecture decision 0131](../adr/ADR-0131-migrations-expand-before-and-contract-after-a-release-and-a-binary-refuses-a-schema-it-cannot-use.md),
   which refines Architecture decision 0082 (a gate reason for the schema, a deactivation in two phases with applied
   epochs).

## Critical User Journeys (CUJs)

### CUJ 1: A rolling upgrade

*   **Given**: release N-1 running.
*   **When**: the operator runs N's migrations and rolls the processes to N.
*   **Then**: N-1 and N processes run together; N writes no new state until the activation, which waits for the census.

### CUJ 2: A binary on the wrong schema

*   **Given**: release N deployed before its migrations were applied.
*   **When**: it starts.
*   **Then**: it registers with the capabilities the schema already knows, closes its serving gate with
    `SCHEMA_TOO_OLD`, answers 503, writes nothing but its registry rows, and opens the gate within one heartbeat after
    the migrations are applied.

### CUJ 3: A rollback

*   **Given**: release N running with its capabilities active, misbehaving.
*   **When**: the operator drains N's capabilities, waits for phase 2, and rolls the processes back.
*   **Then**: phase 1 stops new uses; phase 2 waits until every live process has applied the drain and the drain check
    passes; the binaries roll back to N-1 on N's schema, which N-1 can use. A capability whose drain check cannot pass
    (design 049's `journal-buckets-v1` once a dated run exists) is fixed by rolling forward, never back below it.

## Technical Plan (Architecture)

**The order of an upgrade**, written in `docs/runbooks/scaling.md` section 3 in place of the stopped upgrade, and in
`docs/runbooks/control-plane.md` and `docs/runbooks/execution-engine.md` so that they agree:

1. Apply N's migrations (`control-plane migrate`) while N-1 runs.
2. Roll the control planes and the engines to N, one at a time (`maxUnavailable: 0`, `maxSurge: 1`).
3. Activate N's capabilities.

Rollback before step 3 rolls the binaries back onto N's schema; after it, the two-phase deactivation comes first.

When no process runs (an upgrade the operator chose to stop, or a start after a restore), the order is: start N's
processes, which register with the capabilities the schema knows and confirm with their gate closed by
`SCHEMA_TOO_OLD`; apply N's migrations; the gates open within one heartbeat and the processes register again with
their full lists. `migrate` on a database that has been confirmed and has no live confirmed process exits with code 1
("retry later") and says so. The 0.2.x to 0.3.0 upgrade is the exception: the migration that creates design 048's
wait runs in it, before any process can confirm, and the wait admits a database never confirmed (Requirement 048-013).

| Change | Files | Requirement |
|---|---|---|
| The lint, its fixtures and the merge-base check | `scripts/check-migrations.py`, `scripts/check-migrations.toml`, `scripts/requirements-lint.txt`, `scripts/tests/`, CI | 053-001, 053-002, 053-003 |
| `migrate`'s loop | `apps/control-plane/src/lib.rs`, `crates/infra-postgres/src/lib.rs` | 053-004 |
| The schema check and the gate reason | `crates/infra-postgres/src/instances.rs`, `crates/application/src/instances.rs`, every entry point of `apps/` | 053-005 |
| The capability state, the two phases and the applied epoch | a migration, `crates/infra-postgres/src/instances.rs`, `crates/application/src/instances.rs`, the operator commands of `control-plane` | 053-006, 053-007 |
| The mixed-writer test | `scripts/test-mixed-writers.py`, the release workflow | 053-008 |
| The forward-merge check of hotfixes | `.github/workflows/release.yml` | 053-009 |

**The new objects**, each created by a migration of 0.3.0 (the stopped upgrade) with its grants and storage
classification:

| Object | Columns or signature | Runtime role |
|---|---|---|
| `platform_requirements` (new columns) | `state text NOT NULL DEFAULT 'active'` (`active`, `draining`), `draining_since timestamptz` | as the table today (read) |
| `serving_instances.applied_epoch` | `bigint` | written only through `instance_heartbeat_applied` |
| `sagat_schema_missing(bigint[])` | returns `bigint[]` | execute |
| `instance_heartbeat_applied(uuid, bytea, bigint)` | as `instance_heartbeat_reporting_change` | execute |
| `platform_requirement_drain(text, text)`, `platform_requirement_undrain(text, text)` | as `platform_requirement_remove` | none |

**Order of the build.** Design 048's Tasks 048-9 and 048-10 (the gate's reasons and the confirmation) before Tasks
053-3 and 053-4; Task 053-3 before design 049's Tasks 049-5 and 049-14 (the requirement set it keeps, and the schema
check granted to the retirement role); Task 053-5 before design 049's Task 049-13 (its drain check); design 051's driver
(its Task 051-2) before Task 053-6. The lint (Task 053-2) precedes the first migration of designs 048 and 049, for its
security rules and design 048's protected-names rule; its online rules start after 0.3.0, so no 0.3.0 migration needs a
waiver for them.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Old and new writers disagree on new state | Requirement 053-006; each capability's fence; the mixed-writer test and each capability's scenario on every release |
| A process that was silent at an activation or a drain writes the capability's state | The capability's database-side fence, conditional on its row, or its stated harmlessness |
| Phase 2 passes before a process applied the drain | The applied epoch, read under the epoch lock |
| The migration lint blocks a needed change | A waiver header naming the rule and the reason |
| A migration waits on a long transaction or locks many partitions | The lock and statement timeouts, the bounded retry, and the exit naming the table |
| A failed concurrent index build loops | At most 3 retries, then an exit naming the index |
| A hotfix's migration is missing from the next minor | The release workflow's forward-merge check |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No transition changes; new state waits for the fleet; one transaction per transition unchanged |
| II. Modular domain, ports and adapters | Pass | The schema check and the gate reasons are in the application's instance service; statements stay in `infra-postgres` |
| III. External effects are at-least-once; sagas compensate | Pass | Unchanged |
| IV. Compatibility is versioned and pinned | Pass | Expand-only migrations, the schema check, capabilities for new state, the two-phase deactivation, the mixed-writer test |
| V. Code, tenant boundary and secrets are isolated | Pass | The schema check is a definer function executable by the runtime role and the retirement role only; a 503 shows no version |
| VI. Observability and audit are part of the feature | Pass | `serving_gate_closed{reason}` with `SCHEMA_TOO_OLD`; the drain, the undrain and the removal recorded in the registry's history |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Unchanged |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Negative fixtures per rule, the injected failed index build, the barrier test of phase 2, the mixed-writer test; what it cannot prove before 0.3.0 is said |
| Delivery Constraints | Pass | Constitution 5.0.0 as merged (`docs/constitution.md`); 5.1.0, approved by the owner (Owner decision 2026-11-25-3) and being drafted, changes nothing this design relies on: its throughput regression gate (point 3) is design 052's and runs beside the mixed-writer test, and nothing here deletes journal rows (point 1) |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| The schema check and the migration runner | A binary on the wrong schema writes state nobody can read; one migration can block the hot path | Documentation only: the runbooks contradict each other today |
| The migration lint over a real parser | The rules are easy to forget and costly when broken; SQL with dollar quotes cannot be checked by patterns | Review only, or regular expressions |
| A two-phase deactivation with applied epochs | Processes learn of a change within one heartbeat, not at once, and a beat can overlap the change | One phase: a process may still write the capability's state after it is gone |

## Alternatives Considered

- **A maintenance window per upgrade (today).** Safe and simple, and it gives up the main operational property a
  durable-execution platform is chosen for.
- **Blue and green upgrades with two databases.** Logical replication of the whole schema, with its triggers and definer
  functions firing again on apply.
- **A release-version gate** (this design's first draft: a compatibility table, an oldest compatible release computed
  from the capabilities in use, `develop` carrying the next pre-release, a `BINARY_TOO_OLD` reason). Every case it
  refused is already refused by a capability the older binary lacks, or by sqlx for an older `migrate`; it needed a
  version order shared by SQL, Cargo, Python packaging and the Studio, and a release script that writes to `develop`.
- **A permanent flag on capabilities** (the first draft). The drain check that cannot pass once used says the same,
  where the removal already runs it.
- **The handshake returning every applied migration on every heartbeat, and registration returning the requirement
  set** (the first draft). The gate already reads the set again when the epoch moves; the schema cannot go back once it
  is right, so the check runs on beats only while it is not.
- **Squawk** for the lint. Its generic rules overlap ours only in part and need their own configuration; one parser
  with our rules keeps one owner per rule.

## Non-Functional Requirements

- **Non-functional requirement 053-001 Hot path.** No statement is added to a lease or a completion; the schema check
  runs at start, at each confirmation and on heartbeats only while the schema is too old; the heartbeat writes one more
  column in the row it already updates.
- **Non-functional requirement 053-002 Migration impact.** A migration blocks inserts on a hot table for at most 2 s plus
  one commit (Requirement 053-002), measured by the mixed-writer test on a database holding design 049's steady-state
  number of partitions.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Each task names its files and tests.

- [ ] T053-1 Revise Architecture decision 0131 to this text; add its refinement note to Architecture decision 0082; write the order of an upgrade, the order when no process runs and the hotfix rule in the three runbooks and the 0.3.0 release notes.
  Refs: Requirement 053-001, Requirement 053-006, Requirement 053-007, Requirement 053-009
- [ ] T053-2 Write `scripts/check-migrations.py` over `pglast` (pinned with its hash in `scripts/requirements-lint.txt`), `scripts/check-migrations.toml` (the cut-off, the online rules' start at 0.3.0's last migration, and the hot tables), a negative fixture per rule under `scripts/tests/` (a new `CHECK` on an existing table, a non-concurrent index on a hot table, two blocking locks, a missing statement timeout, `-- phase: contract`, an index on a partitioned table, a definer without a pinned `search_path` before and after the cut-off among them), the merge-base check of edited migrations, and their CI steps.
  Refs: Requirement 053-001, Requirement 053-002, Requirement 053-003, Success criterion 053-002
- [ ] T053-3 Add `sagat_schema_missing()`, the check at start, at confirmation and on beats while not empty in every entry point but `migrate` with its architecture check, the registration with the capabilities the schema knows and the second registration, the one-run commands' `missing_for` check, the gate reason `SCHEMA_TOO_OLD` in the registry contract of feature 018, with the tests `a_binary_before_its_migrations_serves_within_one_heartbeat_after_them` and `a_binary_whose_capability_the_schema_lacks_registers_without_it_then_with_it`, on a fixed pair of migration sets, as the runtime role.
  Refs: Requirement 053-005, Success criterion 053-003
- [ ] T053-4 Write `control-plane migrate`'s own loop (the confirmation, the lock and statement timeouts, the bounded retry on `55P03` and `57014`, the bounded invalid-index check) in `apps/control-plane/src/lib.rs` and `crates/infra-postgres/src/lib.rs`, with the tests `a_failed_concurrent_index_build_is_retried_three_times_then_named`, `a_lock_held_longer_than_the_retries_ends_migrate_naming_the_table` and `migrate_with_no_process_running_proceeds_once_a_new_process_confirms_with_its_gate_closed` (whose new process carries a capability its schema does not yet list).
  Refs: Requirement 053-004
- [ ] T053-5 Add `state` and `draining_since`, `serving_instances.applied_epoch`, `instance_heartbeat_applied`, `platform_requirement_drain` and `platform_requirement_undrain`, the guard's refusal of a delete that is not `draining` and its epoch check on updates, the handover and replay-floor checks on the move to `draining`, phase 2's applied-epoch check under the epoch lock and the capability's drain check, with the tests `phase_two_waits_for_a_beat_that_overlapped_phase_one` (the beat held inside phase 1 by a barrier), `a_draining_capability_stops_new_uses_at_the_next_heartbeat`, `an_active_row_cannot_be_deleted`, `draining_schedules_after_the_handover_is_refused` and `an_undrained_capability_is_active_again`, as the runtime role where the runtime role calls them.
  Refs: Requirement 053-006, Requirement 053-007, Success criterion 053-004
- [ ] T053-6 Write `scripts/test-mixed-writers.py` on the released-tree build of `scripts/test-release-upgrade-profile.py` and add it to the release workflow on a free standard runner, with the terminal-state check and the database at design 049's steady-state partitions; it uses design 051's driver (Task 051-2) and publishes its run with the runner named. Add the release workflow's forward-merge check of hotfix migrations.
  Refs: Requirement 053-008, Requirement 053-009, Success criterion 053-001

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs the suites of what it touches, `integration_claim_transition_efficiency` when it touches
`crates/infra-postgres/`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and the documentation and
architecture checkers of `scripts/`, the new lint among them. Each named test of Tasks 053-3 to 053-5 is seen failing
first, before the change; those of Tasks 053-3 and 053-5 also run with `SAGAT_TEST_AS_RUNTIME_ROLE=1`. Task 053-6 is the
release gate for upgrades.
