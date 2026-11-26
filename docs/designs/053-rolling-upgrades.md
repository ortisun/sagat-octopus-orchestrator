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

The in-region topology and the platform's side of a failover are [design 048](048-high-availability.md); this design was
split from it on 2026-11-25 by the coordinating session (the session that runs this
release's design work for the owner), under Owner decision 2026-11-24-4 (`docs/owner-decisions.md`).

**Terms used below.**

- The *instance registry* ([Architecture decision 0082](../adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md))
  records the live processes (`serving_instances`, one row each) and, for each *kind* of process (`CONTROL_PLANE`,
  `EXECUTION_ENGINE`), the *capabilities* it lists: tokens such as `schedules-v1`, each owned by the feature that
  introduces it. The schema holds the known tokens as *closed lists*, `CHECK` constraints on three registry tables.
- A *requirement* is a row of `platform_requirements`: a capability required of some kinds; *the capability's row* is
  that row. A process whose kind lacks a required capability keeps running with its *serving gate* closed (design 048
  makes the gate a set of reasons) and answers as design 048's Requirement 048-014 says: before authentication one
  constant `UNAVAILABLE` (HTTP 503 with `Retry-After`, gRPC `UNAVAILABLE`), the reason given only after it.
- The registry's *census* lists the processes seen within its liveness window of 30 s; its *activation*
  (`platform_requirement_activate`) records a requirement only when no live process of its kinds lacks it; the
  *requirement epoch* is a counter that every change of the requirements moves, under the *epoch lock* (its row locked
  `FOR UPDATE`), and a process reads the requirements again whenever its heartbeat shows the epoch moved. A process
  keeps a capability decision only inside its *capability window*: 25 s after its last successful heartbeat was sent
  (ADR-0082, amendment of 2026-11-24), so that it stops before the database stops counting it as live. A process the
  registry's full table (1,000 rows) did not take serves no capability call (decision D006-100 of feature 006).
- A removal's *drain check* is the feature's own `verify` step, which `PgInstanceStore::remove` already runs before it
  removes a requirement (for example, design 049's "no run of a dated bucket exists").
- A *release* is a version cut by the release workflow; *N-1* is the release before *N* in semantic-version order: for
  `X.0.0`, the newest release of `X-1`; for `X.Y.0`, the newest patch release of `X.(Y-1)`; for `X.Y.Z` with `Z` above
  0, `X.Y.(Z-1)`.
- The *hot tables* are the tables the hot path writes, listed by name in `scripts/check-migrations.toml` (a test
  requires every relation that design 049's budget probe records as locked by the hot path, Requirement 049-012, to be
  in the list): `run_events`,
  `runs`, `steps`, `worker_sessions`, `engine_leases`, `connector_leases`, `step_lease_blobs`, `step_log_quota_leases`,
  `scheduler_shards`, `event_outbox`, `audit_records`, `serving_instances`, `platform_requirements` and
  `platform_requirement_epoch` at first; the file also lists the partitioned parents.
- The *integrity oracle* is the check of `tests/performance/scalability-ha/` that no acknowledged run was lost, no lease
  was granted twice, and replaying each run's journal gives its projection; the *chain shape* is
  [design 051](051-competitive-benchmark.md)'s chain of three activities, driven by its driver; the *transition
  latency* is that driver's time from a step's completion to the next step's lease.

## The Problem

From the code and documents on `develop` at `546d5aa5`:

- `docs/runbooks/scaling.md` (section 3) and the 0.2.0 release notes say to stop every writer, migrate, then deploy;
  `docs/runbooks/control-plane.md` says the opposite.
- No binary checks the schema it runs on, and the runtime role may not read `_sqlx_migrations`.
- sqlx runs each migration file as one simple query, one implicit transaction, with no lock or statement timeout and no
  index built concurrently, so one migration can queue behind a long transaction and block the hot path behind it.
- The released-upgrade tests (`scripts/test-release-upgrade-profile.py`, the `released-upgrade` CI job) prove an old
  reader and an old worker against the new schema, not old and new writers together.
- The registry learns a requirement within one heartbeat (10 s, jittered to 12 s; ADR-0082, decision 4), and a process
  that was silent at an activation can serve until its next heartbeat after it reconnects. A removal of a requirement is
  one statement, and its drain check runs before the delete: a write that began before it can land after it.

## Requirements

### Migrations

- **Requirement 053-001 Only expand while two releases run.** Every migration after the online rules' start
  (Requirement 053-003) MUST be an *expand* migration: one that leaves the schema usable by every older binary the
  registry still admits, adding and never removing or narrowing. A migration on an existing table MUST NOT add a
  refusal an older binary can hit: no new `CHECK`, foreign key, `NOT NULL`, unique index or constraint, exclusion
  constraint or trigger that raises, unless (a) the refusal is conditional on a capability's row being `active` (a fence
  of Requirement 053-006, a trigger that reads `platform_requirements`), (b) a unique one covers all the columns of a
  unique constraint the table already has that is neither partial nor deferrable (it then refuses nothing new), or (c)
  it widens the closed capability lists (the rule `capability-list-widening` of Requirement 053-003). Removing what an
  older release used (a *contract* migration) is out of scope until a design needs the first one; that design adds the
  contract rules (Non-Goals) and, until it does, the lint refuses the header `-- phase: contract`. A migration on
  `develop` is never edited: CI compares each migration file a pull request touches with the same file at its merge
  base and refuses a change; sqlx's own checksums refuse an applied migration whose file changed.
- **Requirement 053-002 Migrations do not block the hot path.** A transactional migration MUST begin with
  `SET LOCAL lock_timeout = '2s'`, and MUST take at most one lock stronger than `SHARE UPDATE EXCLUSIVE` on a hot table
  (the locks that block inserts or wait behind them), in its last statement, preceded by
  `SET LOCAL statement_timeout = '2s'`, which counts every lock wait and the statement's own run, partitions included.
  The lock is held through the migration's own commit, which waits for a standby (design 048), so one attempt blocks
  inserts on a hot table for at most 2 s plus one commit. The one exception is `capability-list-widening`: one migration
  replaces the three closed lists (on `serving_instances`, `platform_requirements` and `platform_requirement_history`)
  each under its same name by a superset added `NOT VALID`, which changes only the catalog, after one statement
  `LOCK TABLE platform_requirements, serving_instances, platform_requirement_history IN ACCESS EXCLUSIVE MODE` under the
  2 s statement timeout (the order the registry's functions lock them), so that all its waits together are bounded by
  2 s and its hold by the catalog changes and one commit, and validates them in the next migration; widening them in separate migrations would let a binary register a token its activation then refuses.
  On a hot table a migration MUST NOT build an index, a unique constraint or a primary key except concurrently. A
  migration without a transaction (sqlx's first-line header `-- no-transaction`) MUST hold one concurrent statement only,
  `CREATE INDEX CONCURRENTLY IF NOT EXISTS`, and name its index in a header `-- index: <name>`; it runs without the 2 s
  timeouts, since it never blocks inserts, its statement timeout being the time left before `migrate`'s deadline
  (Requirement 053-004).
  Constraints are added `NOT VALID` and validated in a later migration (the existing static test
  `a_constraint_added_not_valid_is_validated_by_a_later_migration` stays the one owner of that rule). A migration MUST
  NOT rewrite a table, change a column's type in place, rename, add a column with a volatile default or a `NOT NULL`
  column without a default, or run `CREATE OR REPLACE` or `DROP` on a function, trigger, policy or view an earlier
  migration created (a changed function gets a new name; a security fix replaces in place under a waiver). A definer
  function pins `search_path` to `pg_catalog, pg_temp` and is revoked from `PUBLIC`.
- **Requirement 053-003 The lint.** `scripts/check-migrations.py` MUST check migrations by rules over the parse tree of
  each statement given by `pglast` (the Python binding of PostgreSQL's own parser, `libpg_query`, at the release that
  parses PostgreSQL 16, installed with `pip install --require-hashes --no-deps --only-binary=:all:` from
  `scripts/requirements-lint.txt` and audited by `pip-audit` in CI), which reads dollar-quoted bodies as PostgreSQL does;
  a statement it cannot parse fails the lint. The rules, each with an identifier, a positive and a negative fixture under
  `scripts/tests/check-migrations/`:

  | Identifier | Refuses | Applies after |
  |---|---|---|
  | `definer-search-path` | a `SECURITY DEFINER` function (created, or altered to it) without `SET search_path = pg_catalog, pg_temp`; `RESET search_path` or `SET search_path FROM CURRENT` on one | the cut-off |
  | `definer-public` | a definer function not revoked from `PUBLIC` in its migration; any `GRANT ... TO PUBLIC` on a function; `ALTER DEFAULT PRIVILEGES` | the cut-off |
  | `dynamic-ddl` | `EXECUTE` in a `DO` block or a function body of anything but `format()` with a literal template from the closed list of statement kinds in the toml file (design 049's partition statements at first) whose placeholders are `%I` or `%L`, never `%s` | the cut-off |
  | `expand-only` | a refusal of Requirement 053-001 outside its three exceptions; `-- phase: contract` | the online rules' start |
  | `hot-lock` | a missing first `lock_timeout`; a second blocking lock on a hot table, or one not last, or without its `statement_timeout`, or without the `-- blocks: <table>` header naming its table, except the locks a `capability-list-widening` migration takes (its header names `platform_requirements`); a non-concurrent index, unique constraint or primary key on a hot table; each statement's lock level taken from PostgreSQL 16's "Table-Level Locks" and its `ALTER TABLE` subcommand list, kept as a table in the lint | the online rules' start |
  | `no-transaction` | a migration without a transaction holding more or other than one `CREATE INDEX CONCURRENTLY IF NOT EXISTS`, or without its `-- index:` header, except a `-- partitioned-index` migration (one `CREATE INDEX IF NOT EXISTS` on a listed parent, run by the procedure; also exempt from `hot-lock`) | the online rules' start |
  | `partitioned-index` | an index on a partitioned parent listed in the toml file, outside the procedure of Requirement 053-004; until the loop builds that procedure, every `-- partitioned-index` migration (the design that builds it lifts the refusal) | the online rules' start |
  | `no-rewrite` | a rewrite, a type change in place, a rename, a volatile default, an undefaulted `NOT NULL` column, `CREATE OR REPLACE` or `DROP` of an object an earlier migration created | the online rules' start |
  | `capability-list-widening` | a change of a closed capability list that is not a same-name superset `NOT VALID` of all three in one migration, after one `LOCK TABLE` of the three under the statement timeout | the online rules' start |
  | `order` | a migration file absent at the merge base whose version is not above the newest version at the merge base, unless it is byte-identical to a file of a release tag's tree (a hotfix merged forward, Requirement 053-009) | the cut-off |

  `scripts/check-migrations.toml` records two boundaries as migration versions, which the `order` rule makes safe to
  compare (no new file can carry an older version). The *cut-off* is the newest migration on `develop` when the lint
  merges: the security rules (`definer-search-path`, `definer-public`, `dynamic-ddl`) and `order` apply to every
  migration after it, `.up.sql` and `.down.sql` alike, and design 048's protected-names rule uses it. The *online
  rules' start* is the last migration of release 0.3.0: while the key is absent, the online rules apply to no
  migration; the 0.3.0 release pull request sets it, and the release workflow refuses to tag `v0.3.0` unless it equals
  the newest migration of the tagged tree, and any later tag unless it equals its value in `v0.3.0`'s tree. Every migration of 0.3.0 runs in the stopped upgrade
  from 0.2.x, where the rules that assume running writers protect nothing. A waiver is a line `-- lint-allow: <rule>
  <reason>` in the file's leading comment block, naming a rule of the table and a reason of at least one word; the
  security rules cannot be waived; before the online rules' start any waiver naming another rule, or a rule unknown to
  the table (design 049's `stopped-upgrade`), is accepted and printed. The merge-base check also refuses a pull request
  that moves either boundary, removes a table from the hot list, a rule from the table or a kind from the closed list
  of dynamic statements, except one change: the online rules' start going from absent to the newest migration of the
  tree, which the 0.3.0 release pull request makes; it and the
  boundary checks run the base branch's copy of the lint, so a pull request is never judged by rules it edits itself.
  `CODEOWNERS` (a new file) names the owner for itself, `crates/infra-postgres/migrations/`,
  `scripts/check-migrations.py`, `scripts/check-migrations.toml`, `scripts/requirements-lint.txt`,
  `scripts/tests/check-migrations/`, `scripts/tests/mixed-writers/` (whose migrations the lint's security rules check
  too), `tests/integration/platform-foundation/migrations.rs` (the catalog test and its allowlist) and the workflow that runs them, a job with `permissions: contents: read`; it binds only with "Require
  review from Code Owners" on `develop` and `main`, a repository setting the owner turns on (Task 053-2 asks for it). What a parse cannot see is checked on the catalog:
  one test after every migration lists every `prosecdef` function of every schema but `pg_catalog` and
  `information_schema`, and requires `proconfig` to pin `search_path=pg_catalog, pg_temp`, `proowner` to be the
  migrations' owner role, the effective grants (`aclexplode(coalesce(proacl, acldefault('f', proowner)))`) to hold no
  `PUBLIC` and no grant option, and the definer functions and their grantees to equal a hand-written allowlist in both
  directions (a function that is listed but missing or no longer a definer fails it too); the allowlist holds design
  049's `sagat_retirer` set, this design's functions and each capability's fence functions.
- **Requirement 053-004 The runner.** `sagat_infra_postgres::migrate()` MUST become the loop below, and
  `scripts/check-architecture.py` refuses a direct `MIGRATOR.run` outside it, so that the integration suites exercise
  it; `control-plane migrate` calls `migrate_online()`, which is the same loop with design 048's confirmation before each
  migration (Requirement 048-013: on a database that has ever been confirmed, a wait of up to 60 s for a live registry
  row carrying the server's confirmed identity, so some process, N-1's or N's, must be running; Requirement 053-005 lets
  an N process confirm before N's migrations; the wait is outside the bounds below). The loop:
  - takes sqlx's migration lock (`Migrate::lock`) for the whole run under a 60 s lock timeout, and exits with code 1
    ("another `migrate` runs") when it expires, then sets the session's `lock_timeout` back to 0;
  - refuses a version left dirty (`Migrate::dirty_version`, recorded with `success = false`), lists the applied
    migrations through `Migrate::list_applied_migrations`, refuses one missing from the binary and one whose checksum
    differs, naming it (sqlx's own validation is private), and
    checks that the index named by each applied migration without a transaction is valid (for a `-- partitioned-index`
    migration, that every present partition has a valid attached index, never the parent's flag), exiting naming it
    otherwise;
  - applies each pending transactional migration through `Migrate::apply`; on `55P03` (lock not available), on
    `40P01` (a deadlock, whose victim it may be, since it takes a weaker lock before its blocking one), or on `57014`
    raised after at least the 2 s of its own statement timeout had elapsed (the message's text is not read, since it
    follows the server's language; an operator stops `migrate` by stopping its process, not by cancelling its
    statement), it retries after 1 s, doubling to at most 30 s, at most 10 attempts, then exits non-zero naming the hot table
    of the migration's last statement, which the lint requires in a header `-- blocks: <table>`. The nine waits sum to
    1 + 2 + 4 + 8 + 16 + 30 × 4 = 151 s and the ten attempts to at most 10 × 2 = 20 s, so it gives up within about 171 s;
    each attempt blocks inserts on its table for at most 2 s plus one commit. The backoff is a parameter, so that the
    tests run it in milliseconds;
  - for a migration without a transaction, drops its index concurrently if it exists invalid, runs the statement itself
    with `statement_timeout` set to the time left before a deadline (`SAGAT_MIGRATE_DEADLINE_SECS`, 1,800 s by
    default), so that the server cancels a build the deadline ends, checks `pg_index.indisvalid` together with the
    index's table (`indrelid`) and definition (`pg_get_indexdef`), so that an index of the same name elsewhere is never
    taken for it, and only then applies the file through `Migrate::apply`, whose `IF NOT EXISTS` statement is then a
    no-op that records the version; a failure or an invalid result is retried, at most 3 times in all, then exits naming
    the index; on the deadline's expiry it exits naming the oldest transaction of `pg_stat_activity`;
  - sets `transaction_timeout` to 0 for its session where the server has it.

  `SAGAT_MIGRATE_ON_START`, for development only, runs `migrate()` without the confirmation, and is refused when
  `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY` is true. `migrate` does not call the schema check of Requirement 053-005: its
  own release is always the newest, and the refusal of an applied migration missing from the binary covers an older
  one. **The partitioned-index procedure**, built by the first design that adds an index to a partitioned hot table
  after 0.3.0 (design 049 defers its later indexes to it), is defined here so that no design invents its own: a
  migration without a transaction whose header is `-- partitioned-index` and whose one statement is
  `CREATE INDEX IF NOT EXISTS <name> ON <parent> (...)`, with `<parent>` confirmed by `pg_partition_tree` and the toml
  list, is run by the loop as `CREATE INDEX IF NOT EXISTS <name> ON ONLY <parent>` under the lock and statement
  timeouts, then, for each partition `pg_inherits` lists, with every identifier quoted by `format('%I')`, an index of a
  deterministic name (the parent index's name and the partition's, shortened with a hash to 63 bytes, so that a rerun
  after a crash reuses a valid unattached one) built concurrently (dropped first if it exists invalid, at most 3 builds
  per partition), checked valid, and attached (`ALTER INDEX ... ATTACH PARTITION`, which locks the partition's index
  `ACCESS EXCLUSIVE`, under the 2 s lock timeout with the bounded retry of transactional migrations); a partition that
  vanished meanwhile (detached by design 049's retirement) is done; the partitions are listed again, at most 5 rounds,
  until every present partition has a valid attached index (a partition created meanwhile by `ensure_journal_buckets()`
  receives the index at its creation), judged per partition, since the parent's flag is not revalidated when the last
  missing partition is detached rather than attached; only then is the file applied through `Migrate::apply`; otherwise
  the loop exits naming the partition. Each attach blocks inserts into its partition for at most 2 s plus one commit,
  and the whole procedure is bounded by the deadline.

### The schema check

- **Requirement 053-005 A binary keeps its gate closed on a schema it cannot use.** A process calls
  `sagat_schema_missing(versions bigint[])` (owner-defined, `SECURITY DEFINER`, `search_path` pinned, executable by
  `sagat_app` and by design 049's `sagat_retirer` only), which refuses more than 10,000 versions or a null one and
  returns the versions among those it is given that are not applied with `success = true` in `_sqlx_migrations`; the
  process gives the versions of the migrations it was built with. It calls it at start, at each confirmation of design
  048 (Requirement 048-013), and with each heartbeat only while the answer was not empty, so that a gate it closed
  reopens within one heartbeat once the schema is right. The check fails closed: an error of any kind (`42883` on a
  schema older than 0.3.0, `42501` on a grant that is missing, a timeout) reads as a schema too old, logged with its
  SQLSTATE. While the schema is too old the process sets the gate's reason `SCHEMA_TOO_OLD`, writes nothing but its
  registry rows (registration, design 048's confirmation and the heartbeat) and the release of its shards, and answers
  as Requirement 048-014 says; an alert fires on `serving_gate_closed{reason="SCHEMA_TOO_OLD"}` held for 5 minutes,
  with a rule test. Such a process registers only the capabilities the schema's closed lists already hold, so that the
  registration is not refused (`23514`); it registers again with its full list once the answer is empty.
  `crates/infra-postgres` keeps, beside `MIGRATOR`, the map from each capability to the migration that first lists it,
  with a test that the mapped migration is the first whose closed list holds the token; the port returns a typed verdict
  (`SchemaVerdict { too_old: bool, known: Vec<Capability> }`, with `known` empty when the check fails closed), so that
  migration versions do not enter the application. Every function a binary calls before its migrations are applied (the registration, the heartbeat,
  design 048's confirmation and this check) MUST exist with the same name and signature in the schema of the release
  before it: a release that needs a new one adds it under a new name and the binary calls the old one until the
  check's answer is empty. The serving processes reach the database only through the one production constructor of
  the serving handle (design 048's `DbHandle::connect_lazy`, wrapped as `connect_serving()` in `apps/`), which wires
  this check into the registry member, and each one-run command of `control-plane` except `migrate` only through
  `connect_command(capabilities)`, which, on the connection design 048 admits for the wait, calls this check with the
  binary's migration versions and then design 048's `command_await_durability(capabilities)`, with its kind taken as
  `CONTROL_PLANE` and `capabilities` the full capability list a `control-plane` process registers, and returns the pool only when
  both pass; `scripts/check-architecture.py` refuses `sagat_infra_postgres::connect` and `open_pool`
  in `apps/` outside these two constructors and `migrate` (the commands' `open_pool` calls of `cleanup.rs`,
  `capability_activation.rs`, `handover.rs`, `inventory.rs` and `lib.rs` move to `connect_command`), with a negative
  fixture of a command calling `open_pool`; a command whose schema is too old exits with code
  1 ("retry later"), except `input-schema-inventory`, which exits with code 2 (its code 1 means non-conforming versions
  found, decision D043-072), tested in `apps/control-plane/tests/cli.rs`. Design 049's retirement commands connect as
  `sagat_retirer` through `connect_command`, so the role executes exactly design 049's retirement functions, this
  check, `command_await_durability` and design 048's `sagat_server_identity()` (every connection's `after_connect`
  calls it), and selects `registry_first_confirmation` (read by each new connection's fresh-database admission),
  nothing else; it is never granted membership of another role. The grants are made by design 049's Task 049-14, whose
  test runs the real `connect_command` path as `sagat_retirer`, and checked by the catalog test of Requirement
  053-003. The check's tests run as
  the runtime role.

### New behaviour and the fleet

- **Requirement 053-006 New behaviour waits for the fleet, and is fenced.** A change that writes state release N-1
  cannot process MUST be behind a registry capability, added to the closed lists by `capability-list-widening`, and
  required of every kind of process that could meet the state. Two late cases remain, and each capability's design MUST
  answer both:
  - **Activation.** A process that was not live at the activation (silent for 30 s, so an older binary not counted) can
    reconnect and serve until its next heartbeat, about 12 s, before the registry refuses it. The design states either
    why such a process can neither write state the others cannot process nor meet state it cannot process, or the
    fence that refuses it.
  - **Deactivation.** A write of state that some release a rollback can reach cannot process (the oldest such release
    is 0.3.0, since a rollback never crosses the stopped upgrade) MUST be fenced in the database, conditional on the
    capability's row being `active`, so that no write lands after phase 2 of Requirement 053-007, whatever process or command makes
    it (one-run commands never register, and a process outside the census is not counted by phase 2). `crates/infra-postgres` keeps, beside the capability map of Requirement 053-005, one descriptor per capability (its
    fence form, the table it writes, its drain check over a `PgConnection`), which `PgInstanceStore::remove` reads to
    take phase 2's locks and run the check, and to which `capability_activation.rs`'s `ACTIVATABLE` refers; a test over
    every `Capability` requires a descriptor whose fence is form (a), form (b), or the explicit variant "no fence"
    carrying its reason (for `journal-buckets-v1`, "every release a rollback can reach processes it"), or the explicit
    mark "writes nothing N-1 cannot process", and lists the capabilities of the last two kinds by name, so that adding
    one is a visible change. The fence takes
    one of two forms, named by the capability's design: (a) the writing transaction calls an owner-owned definer function of
    the capability that takes the capability's row `FOR KEY SHARE` and answers whether it is `active` (decision
    D006-032's pattern; `lease_withdrawal_required()`'s `FOR SHARE` becomes `FOR KEY SHARE` in 0.3.0, so that phase 1's
    update never waits for a withdrawal while it holds the epoch lock; the runtime role cannot lock the row itself,
    having only `SELECT`), and refuses the write when it is not, for a write of moderate rate; the function is in the
    catalog test's allowlist; or (b), defined here and built by the first capability that needs it (0.3.0 builds
    none), for a write on the hot path's rate, a trigger on the written table refuses the
    write when the row is absent or not `active`: under READ COMMITTED by a plain read in the volatile trigger function,
    which takes a new snapshot, and under REPEATABLE READ or SERIALIZABLE, whose snapshot may predate phase 2, by form
    (a)'s function, whose lock on a row deleted since the snapshot fails with a serialization error; the drain check
    runs after phase 2 has taken that table `IN SHARE MODE`, which waits for every write in flight and holds new ones
    for the check's length, under the 2 s statement timeout, so it MUST be backed by an index that answers it without a
    scan. Either way phase 2 sees every write that began before it, and refuses every one that begins after. A refused
    write must not fail its request while the capability drains: the fence raises with its own constraint name
    (`<capability>_fence`), or, under REPEATABLE READ or SERIALIZABLE, a serialization failure from the fence function,
    which the writer also reads as a fence refusal, and the writer retries the transaction once with the capability
    treated as inactive.

  Inside a process every use asks one gate method, `ServingGate::uses(capability, kinds, now)`, which replaces
  `ServingGate::requires` (its callers move to it) and answers true only
  when the requirements kept require it as `active` and were read at the epoch the last heartbeat saw (a failed read
  leaves it false), the process is counted by the registry (not a full-table refusal), and `now` is inside the
  capability window; so a process whose heartbeats fail stops using the capability before the
  database stops counting it. In release 0.3.0 the capabilities that write new state are design 049's
  `journal-buckets-v1` (activation: harmless, since a legacy root created by a process that has not learned of the
  activation is valid and processed by every release; deactivation: no fence, since its dated runs are processed by every
  release a rollback can reach, 0.3.0 and later; a dated root that lands after its removal harms no rollback, and its
  drain check still refuses the removal once a dated run exists, design 049's Task 049-20) and feature 006's
  lease-withdrawal capabilities, whose database gate `lease_withdrawal_required()` is form (a).
  [Design 052](052-hot-path-dispatch.md)'s ready queue needs no capability: it is an index of candidates filled by a
  database trigger whatever the binary, N-1 claims by today's scan and ignores it, and the sweeper repairs what either
  leaves (Requirements 052-001, 052-004 and 052-005); its switch is a setting.
- **Requirement 053-007 A deactivation drains first.** Deactivating a capability MUST take two phases, run by owner
  commands of `control-plane` that call owner-only functions; the functions are the interface the runbooks give, and
  `docs/runbooks/instance-registry.md` and `docs/runbooks/lease-withdrawal.md` change with it.
  - **Phase 1** (`control-plane capability-drain <capability> --operator <name>`, calling
    `platform_requirement_drain(capability, actor)`) moves the row to `draining`, records `draining_since` and the
    epoch it moves to in `draining_epoch`, and appends a history row `DRAINED`, under the epoch lock. Its outcomes are
    `DRAINED`, `ALREADY_DRAINING`, `NOT_RECORDED` and `REFUSED` (a guard, naming it). It is refused for a capability
    that an `active` requirement follows (feature 006's `census_step` order, decision D006-096) and, by
    `platform_requirements_handover_guard` on the move to `draining`, for `schedules-v1` once its cron triggers were
    handed over (the schedule pass would stop with nothing in its place). `platform_requirement_undrain(capability,
    actor)` (`capability-undrain`) moves a `draining` row back to `active` after the same census as an activation, with
    the epoch moved, `draining_epoch` cleared and a history row `UNDRAINED`; its outcomes are `UNDRAINED`,
    `NOT_DRAINING`, `NOT_RECORDED` and `CAPABILITY_MISSING` (with the count and up to 16 instance identifiers).
  - **What `draining` means to each reader of `platform_requirements`:**

    | Reader | A `draining` row |
    |---|---|
    | `instance_register` and `missing_for` (the application's pure copy of registration's rule) | still required: no process lacking it registers, so a rollback waits for phase 2 |
    | `requirements()`, `Requirement` | carried with its `state` |
    | `ServingGate::uses` | inactive: no new use |
    | `platform_requirement_activate` | refused, outcome `DRAINING` (use `capability-undrain`) |
    | `requirement_recorded`, `census_step`'s order (feature 006's check that a capability another one follows is already recorded) | not recorded: a capability that follows it cannot be activated |
    | `schedule_trigger_handover_complete`, `lease_withdrawal_required()` | inactive (`state = 'active'` added in 0.3.0) |
    | history | `DRAINED`, `UNDRAINED` join `ACTIVATED` and `REMOVED` |

    The registry's guard (`platform_requirements_guard`) admits an update only of `state`, `draining_since` and
    `draining_epoch`, only between `active` and `draining`, refuses the delete of a row that is not `draining`, and its
    deferred check that the epoch moved covers updates as well as inserts and deletes. The handover guard and the
    replay-floor guard return `NEW` on an update; the handover guard runs on the move to `draining` and on the delete,
    the replay floor on the delete only (phase 1 of lease withdrawal is harmless: it stops new withdrawals).
  - **Phase 2** (`control-plane capability-remove <capability> --operator <name>`, calling `platform_requirement_remove`
    and `PgInstanceStore::remove`) runs under READ COMMITTED and takes the epoch lock; checks that the row is `draining`
    (else `NOT_DRAINING`) and that every live registry row of the requirement's kinds has *applied* `draining_epoch`
    (else `NOT_APPLIED`, with the count and up to 16 instance identifiers), before any lock that holds writers; then,
    for form (b), takes the written table `IN SHARE MODE` (first, in the order its writers take their locks: the table,
    then the row through the fence function), then the capability's row `FOR UPDATE` (which waits for every transaction
    holding it `FOR KEY SHARE`, form (a)); then runs the capability's drain check (else
    `DRAIN_CHECK_REFUSED`, naming the check); then deletes the row with a history row `REMOVED`. Its own lock waits
    run under a 1 s lock timeout and its statements under a 2 s statement timeout, as the existing function does; a
    `55P03`, `57014` or `40P01` is retried like `NOT_APPLIED`. A process reports what it applied through the heartbeat
    `instance_heartbeat_applied(instance, token, applied_epoch)`, which replaces `instance_heartbeat_reporting_change`
    (the beat that never waits for the epoch lock, decision D006-098 of feature 006) for the binaries of 0.3.0 on and does what it does, and records in `serving_instances.applied_epoch`
    the lesser of the epoch sent and the current epoch, refusing a negative or null one; a row has applied it when
    `applied_epoch >= draining_epoch`; the process sends the lesser of its gate's epoch and the epoch of the requirements
    it keeps, so a failed read after a re-registration never reports
    an epoch it has not applied; a row registered since, or written by an older beat, holds null, which is not applied.
    Learning of phase 1 and reporting it takes two heartbeats, about 24 s, and a crashed process's row stays live up to
    30 s more, so the command retries `NOT_APPLIED` for up to 60 s and then exits with code 1 ("retry later"); it exits
    with 0 on `REMOVED` and with 2 on a refusal that waiting cannot clear. `capability-drain`, `capability-undrain` and
    `instance-evict` exit with 0 on their success or already-done outcome (`DRAINED`, `ALREADY_DRAINING`, `UNDRAINED`, an
    `EVICTED`, `ALREADY_EVICTED`), with 2 on `REFUSED`, `NOT_RECORDED`, `NOT_DRAINING` or `CAPABILITY_MISSING`, and with 1 on a
    timeout. A row that never advances (a process whose
    requirements read keeps failing, or a compromised one) is named by the refusal, and the owner-only
    `instance_evict(instance, actor)` (`control-plane instance-evict`; outcomes `EVICTED`, `ALREADY_EVICTED`) deletes it and records the instance in
    `serving_instance_evictions (instance_id uuid primary key, actor text, evicted_at timestamptz)`, the audit of the
    act, which `instance_register` reads to refuse that instance for good (outcome `EVICTED`, a closed gate);
    `instance_evict` takes the epoch row `FOR UPDATE`, which registration takes `FOR SHARE`, so an eviction and a
    registration of the same instance never interleave. The
    runbook's step stops the process first: a benign one is restarted and registers under a new instance; for a
    compromised one the `sagat_app` credential is rotated, since a holder of it could register under another
    instance. Only after phase 2 may the binaries roll back to N-1.
  - **A capability that cannot be removed** is one whose drain check cannot pass once used: design 049's
    `journal-buckets-v1` once a dated run exists (its phase 1 allowed: new roots are then legacy, which every release
    processes), and feature 006's `lease-withdrawal-v1` and `lease-withdrawal-v2` once a withdrawal is journaled (phase
    1 allowed, phase 2 refused by the replay floor); `schedules-v1` after its handover refuses phase 1 too.
  - `state` is `text NOT NULL DEFAULT 'active'`, so the existing rows and the older binaries' activation need no change.
- **Requirement 053-008 Old and new writers run together.** `scripts/test-mixed-writers.py` MUST run two control
  planes and two engines of N-1 and of N on one database of the development reference (`infra/docker/compose.yaml`),
  holding a legacy partition and 13 monthly ones of design 049 (a year of history and the current month), under the
  chain shape at a fixed open-loop rate, half of the capacity design 051's driver measures first in the same job on the
  same runner and used again for the baseline: start one N process on N-1's schema before `migrate` and require it to
  register, confirm, beat, answer `UNAVAILABLE` and open within 20 s after the migrations; apply N's migrations during
  the load, together with a fixture expand migration on `runs` and `steps` from `scripts/tests/mixed-writers/`, built into N's
  migrator by the test build feature `mixed-writers-fixture` (so that the loop runs it and later runs know it; the
  feature is refused in normal dependencies and checked absent from the release artifacts by `cargo tree`, as design
  048's `test-support` is), so
  that the latency window always holds a migration; roll the processes one at a time; activate N's capabilities; drain
  and remove those that can be removed, then, when every capability N added was removed, roll one process back to N-1
  and require it to open its gate and lease work, then roll it forward and activate them again; otherwise (an N-1
  process could not register), the run drains and undrains every capability that cannot be removed, activates again
  those it removed, and records "rollback not applicable". It passes when the integrity oracle passes, no start of a run is refused, the completion rate never falls to
  zero for more than 10 s, every run started during the test reaches a terminal state within 120 s after the load ends
  (a run N-1 ignores stays stuck, which the oracle alone does not see), and the 99th percentile transition latency
  during the migrations is no more than 2 s above a baseline run (the same run without migrations). Each capability's
  design adds a scenario: state written while it is active, then processed by N-1 after its removal (for one that
  cannot be removed, processed after its phase 1), and state written by N-1 processed by N. The test builds both releases' `control-plane` and `execution-engine` from their verified
  trees by the released-tree build of `scripts/test-release-upgrade-profile.py`, factored into a module that takes the
  tag or commit (the script is fixed to v0.2.0 today). Release N MUST NOT be published unless it passes against N-1
  (job `mixed-writers` of `.github/workflows/release.yml`). It first gates a release when 0.3.0 is N-1, since 0.2.x
  binaries have no schema check; until then the job runs on the release branch of 0.3.0 with N-1 built at the first commit
  of that branch on which Tasks 053-3 and 053-5 are both merged and N at its head, N built with the
  `mixed-writers-fixture` feature, whose test-only capability the run activates, drains, removes and activates again;
  this proves the harness, the oracle and the capability flow but not the migration rules (the online rules start with
  0.3.0) nor a real N-1, and the schema check is proved by tests on a fixed pair of migration sets (Task
  053-3). The run on the 0.3.0 release branch is informational and does not block the `v0.3.0` tag. The run names its
  runner, a free standard runner (Owner decision 2026-11-25-1).
- **Requirement 053-009 A hotfix upgrades forward.** A hotfix release adds expand migrations only, and each is merged
  forward to `develop` before the next minor release is cut: the release workflow refuses to cut a release whose tree
  lacks a migration file present in the tree of any earlier release tag, taking only tags that match
  `^v[0-9]+\.[0-9]+\.[0-9]+$` (protected tags, ordered as semantic versions, each quoted in the shell), since the loop
  refuses to upgrade a database holding a migration the binary lacks. The runbook states that a hotfix `X.Y.Z` upgrades
  only to a release cut after it.

### Success criteria

- **Success criterion 053-001** In the mixed-writer test, the completion rate never falls to zero for more than 10 s,
  the integrity oracle passes, every run reaches a terminal state within 120 s after the load, and the 99th percentile
  transition latency during the migrations is no more than 2 s above the baseline's.
- **Success criterion 053-002** The migration lint refuses each of its negative fixtures, passes each positive one, and
  passes every migration merged after its cut-off.
- **Success criterion 053-003** A binary started before its migrations answers `UNAVAILABLE` with its gate closed, and
  serves within 20 s after they are applied (one heartbeat of 12 s, the beat's own call of up to 5 s and the second
  registration).
- **Success criterion 053-004** Phase 2 of a deactivation is refused while a live process has not applied phase 1's
  epoch, and while a write of the capability's state that began before phase 2 is in flight, phase 2 waits for it and
  then sees it; it succeeds once both are clear and the drain check passes.

### User stories

- **User story 053-01** An operator upgrades the platform during the day with runs in flight.
- **User story 053-02** An operator rolls back an upgrade that misbehaves, without losing a run.

## Goals

- An upgrade is a rolling deployment, as for any stateless service.
- A binary on the wrong schema cannot write state that another binary cannot read.

## Non-Goals

- **The 0.2.x to 0.3.0 upgrade.** It stays a stopped upgrade: 0.2.x binaries have no schema check (Open Question 4).
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

Each was decided under the delegation of Owner decisions 2026-11-20-6 and 2026-11-24-4, with its reasoning.

1. **Two releases for every removal?** Yes, once contracts exist. A removal in the same release as its replacement
   breaks N-1 while both run; the cost is a second, small migration in the next release. Delegate.
2. **How is an older binary kept away from newer state?** By capabilities alone: every write N-1 cannot process is
   behind one, so a binary lacking it is refused by the registry. A second gate on release versions would need a
   version order shared by Rust, SQL, the SDKs and the Studio, and a version on `develop` distinct from the release,
   for no case the capabilities miss. Delegate.
3. **The decision record.** [Architecture decision 0131](../adr/ADR-0131-migrations-expand-before-and-contract-after-a-release-and-a-binary-refuses-a-schema-it-cannot-use.md),
   which refines ADR-0082 (a gate reason for the schema, a deactivation in two phases with applied epochs and fences)
   and amends the "Enforced by" line of [ADR-0132](../adr/ADR-0132-the-journal-is-partitioned-by-the-month-a-run-was-created-and-a-run-owns-its-payloads.md).
4. **What does "rolling N and N-1 upgrades" in 0.3.0 (Owner decision 2026-11-24-1) deliver?** The mechanism, its tests,
   its gate and the runbooks ship in 0.3.0; the first rolling upgrade is from 0.3.x to the release after it, since a
   0.2.x binary has no schema check and 0.3.0's conversions (design 049) need the writers stopped. Recorded as Owner
   decision 2026-11-24-521 (delegate) and in the 0.3.0 release notes.

## Critical User Journeys (CUJs)

### CUJ 1: A rolling upgrade

*   **Given**: release N-1 running.
*   **When**: the operator runs N's migrations and rolls the processes to N.
*   **Then**: N-1 and N processes run together; N writes no new state until the activation, which waits for the census.

### CUJ 2: A binary on the wrong schema

*   **Given**: release N deployed before its migrations were applied.
*   **When**: it starts.
*   **Then**: it registers with the capabilities the schema already knows, closes its serving gate with
    `SCHEMA_TOO_OLD`, answers `UNAVAILABLE`, writes nothing but its registry rows, and opens the gate within 20 s after
    the migrations are applied.

### CUJ 3: A rollback

*   **Given**: release N running with its capabilities active, misbehaving.
*   **When**: the operator drains N's capabilities, runs phase 2 (which retries for up to 60 s), and rolls the processes
    back.
*   **Then**: phase 1 stops new uses; phase 2 succeeds once every live process has applied the drain, every write in
    flight has landed and the drain check passes; the binaries roll back to N-1 on N's schema, which N-1 can use. A
    capability that cannot be removed (design 049's `journal-buckets-v1` once a dated run exists) is fixed by rolling
    forward, never back below it.

## Technical Plan (Architecture)

**The order of an upgrade**, written in `docs/runbooks/scaling.md` section 3 in place of the stopped upgrade, and in
`docs/runbooks/control-plane.md` and `docs/runbooks/execution-engine.md` so that they agree:

1. Apply N's migrations (`control-plane migrate`) while N-1 runs.
2. Roll the control planes and the engines to N, one at a time (`maxUnavailable: 0`, `maxSurge: 1`).
3. Activate N's capabilities.

Rollback before step 3 rolls the binaries back onto N's schema; after it, the two-phase deactivation comes first.

When no process runs (an upgrade the operator chose to stop, or a start after a restore), the order is: start N's
processes, which register with the capabilities the schema knows and confirm with their gate closed by
`SCHEMA_TOO_OLD`; apply N's migrations; the gates open within 20 s and the processes register again with their full
lists. `migrate` on a database that has been confirmed and has no live confirmed process exits with code 1 ("retry
later") and says so. The 0.2.x to 0.3.0 upgrade is the exception: the migration that creates design 048's wait runs in
it, before any process can confirm, and the wait admits a database never confirmed (Requirement 048-013).

| Change | Files | Requirement |
|---|---|---|
| The lint, its fixtures, the merge-base check and the catalog test of definer functions | `scripts/check-migrations.py`, `scripts/check-migrations.toml`, `scripts/requirements-lint.txt`, `scripts/tests/check-migrations/`, `CODEOWNERS`, `tests/integration/platform-foundation/migrations.rs`, CI | 053-001, 053-002, 053-003 |
| The loop | `crates/infra-postgres/src/lib.rs` (`migrate`, `migrate_online`), `apps/control-plane/src/lib.rs`, `scripts/check-architecture.py` | 053-004 |
| The schema check, the gate reason and the capability map | `crates/infra-postgres/src/instances.rs`, `crates/infra-postgres/src/lib.rs`, `crates/application/src/instances.rs`, `apps/control-plane/src/`, `apps/execution-engine/src/` | 053-005 |
| The capability state, the fences and their descriptors, the two phases, the applied epoch and eviction | a migration, `crates/infra-postgres/src/instances.rs` (the descriptors and drain checks), `crates/application/src/instances.rs`, `apps/control-plane/src/cli.rs`, `apps/control-plane/src/capability_activation.rs` (`ACTIVATABLE` referring to the descriptors) | 053-006, 053-007 |
| The mixed-writer test | `scripts/test-mixed-writers.py`, `scripts/tests/mixed-writers/`, `scripts/test-release-upgrade-profile.py`, `.github/workflows/release.yml` | 053-008 |
| The forward-merge check of hotfixes | `.github/workflows/release.yml` | 053-009 |

**The new objects**, each created by a migration of 0.3.0 (the stopped upgrade) with its grants and storage
classification:

| Object | Columns or signature | Runtime role |
|---|---|---|
| `platform_requirements` (new columns) | `state text NOT NULL DEFAULT 'active'` (`active`, `draining`), `draining_since timestamptz`, `draining_epoch bigint` | as the table today (read) |
| `platform_requirement_history` | the action check widened by `DRAINED` and `UNDRAINED` | as today |
| `serving_instances.applied_epoch` | `bigint` | written only through `instance_heartbeat_applied` |
| `sagat_schema_missing(bigint[])` | returns `bigint[]` | execute |
| `instance_heartbeat_applied(uuid, bytea, bigint)` | as `instance_heartbeat_reporting_change` | execute |
| `platform_requirement_drain(text, text)`, `platform_requirement_undrain(text, text)`, `instance_evict(uuid, text)` | as `platform_requirement_remove` | none |
| `serving_instance_evictions` | `instance_id uuid primary key`, `actor text`, `evicted_at timestamptz` | none; read by `instance_register` |
| `platform_requirements_guard`, the handover and replay-floor guards, `platform_requirement_remove`, `schedule_trigger_handover_complete`, `lease_withdrawal_required()`, `instance_register` (the eviction check, and `applied_epoch` set to null on every upsert) | replaced in the stopped upgrade as Requirements 053-006 and 053-007 say | as today |

**Order of the build.** Design 048's Tasks 048-9 and 048-10 (the gate's reasons and the confirmation) before Tasks
053-3 and 053-4; Task 053-3 before Task 053-4 (its test needs the reduced registration); Task 053-3 before design 049's
Task 049-14 (the schema check granted to the retirement role); Task 053-5 before design 049's Task 049-13 (its drain
check) and Task 049-20 (its fence); design 051's driver (its Task 051-2) before Task 053-6. The lint (Task 053-2)
precedes the first migration of designs 048 and 049, for its security rules and design 048's protected-names rule; its
online rules start with 0.3.0, so no 0.3.0 migration needs a waiver for them.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Old and new writers disagree on new state | Requirement 053-006; each capability's fence; the mixed-writer test and each capability's scenario on every release |
| A write that began before phase 2, or a process or command outside the census, writes after it | The database fence, conditional on the row, which phase 2 waits for |
| Phase 2 passes before a process applied the drain | The applied epoch, bounded by the current epoch, read under the epoch lock |
| A row never applies and holds phase 2 | The refusal names it; `instance-evict` |
| The migration lint blocks a needed change | A waiver header naming the rule and the reason (never a security rule) |
| A migration waits on a long transaction or locks many partitions | The lock and statement timeouts, the bounded retry, and the exit naming the table |
| A concurrent index build fails or waits for ever | At most 3 attempts with the validity checked before the version is recorded; the deadline naming the oldest transaction |
| A hotfix's migration is missing from the next minor | The release workflow's forward-merge check |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No transition changes; new state waits for the fleet; one transaction per transition unchanged |
| II. Modular domain, ports and adapters | Pass | The gate decides from a typed verdict; migration versions, the loop and every statement stay in `infra-postgres` |
| III. External effects are at-least-once; sagas compensate | Pass | Unchanged |
| IV. Compatibility is versioned and pinned | Pass | Expand-only migrations, the schema check, capabilities with fences for new state, the two-phase deactivation, the mixed-writer test |
| V. Code, tenant boundary and secrets are isolated | Pass | The schema check is a definer function executable by the runtime role and the retirement role only; the drain, undrain, removal and eviction are owner-only; the catalog test of every definer function; no tenant data |
| VI. Observability and audit are part of the feature | Pass | `serving_gate_closed{reason}` with `SCHEMA_TOO_OLD` and its alert; the drain, the undrain, the removal and the eviction recorded in the registry's history |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Unchanged |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Fixtures per rule, the injected failed index build, the barrier tests of phase 2, the mixed-writer test that must remove a capability and serve on N-1; what it cannot prove before 0.3.0 is said |
| Delivery Constraints | Pass | Constitution 5.0.0 as merged (`docs/constitution.md`); 5.1.0, approved by the owner (Owner decision 2026-11-25-3) and being drafted, changes nothing this design relies on: its throughput regression gate (point 3) is design 052's and runs beside the mixed-writer test, and nothing here deletes journal rows (point 1) |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| The schema check and the migration loop | A binary on the wrong schema writes state nobody can read; one migration can block the hot path | Documentation only: the runbooks contradict each other today |
| The migration lint over a real parser, and the catalog test | The rules are easy to forget and costly when broken; SQL with dollar quotes cannot be checked by patterns, and a parse cannot see what lands in the catalog | Review only, or regular expressions |
| A two-phase deactivation with applied epochs and database fences | Processes learn of a change within one heartbeat, a beat can overlap the change, and a write can be in flight across it | One phase: a process may still write the capability's state after it is gone |

## Alternatives Considered

- **A maintenance window per upgrade (today).** Safe and simple, and it gives up the main operational property a
  durable-execution platform is chosen for.
- **Blue and green upgrades with two databases.** Logical replication of the whole schema, with its triggers and definer
  functions firing again on apply.
- **A release-version gate** (this design's first draft: a compatibility table, an oldest compatible release computed
  from the capabilities in use, `develop` carrying the next pre-release, a `BINARY_TOO_OLD` reason). Every case it
  refused is already refused by a capability the older binary lacks, or by the loop for an older `migrate`; it needed a
  version order shared by SQL, Cargo, Python packaging and the Studio, and a release script that writes to `develop`.
- **A permanent flag on capabilities** (the first draft). The drain check that cannot pass once used says the same,
  where the removal already runs it.
- **The handshake returning every applied migration on every heartbeat, and registration returning the requirement
  set** (the first draft). The gate already reads the set again when the epoch moves; the schema cannot go back once it
  is right, so the check runs on beats only while it is not.
- **Phase 2 trusting the applied epochs alone.** A use begun before the drain can commit after it; only a fence in the
  database orders the two.
- **A table of capabilities with foreign keys in place of the closed lists.** `serving_instances.capabilities` is an
  array, which a foreign key cannot cover; the same-name superset in one migration changes only the catalog.
- **Squawk** for the lint. Its generic rules overlap ours only in part and need their own configuration; one parser
  with our rules keeps one owner per rule.

## Non-Functional Requirements

- **Non-functional requirement 053-001 Hot path.** No statement is added to a lease or a completion; the schema check
  runs at start, at each confirmation and on heartbeats only while the schema is too old; the heartbeat writes one more
  column in the row it already updates; a fence adds one read (form (b)) or one row lock (form (a)) only to the write it
  guards, and each capability's design states which. Phase 2 of a form (b) capability holds new writes of its table for
  at most its 1 s lock wait, the 2 s drain check and one commit; the runbook says so.
- **Non-functional requirement 053-002 Migration impact.** One attempt of a migration blocks inserts on a hot table for
  at most 2 s plus one commit, and a transactional migration for at most ten such attempts within about 171 s
  (Requirement 053-004); a concurrent build blocks no insert, and the partitioned-index procedure blocks each partition
  for at most 2 s plus one commit per attach, within the deadline;
  measured by the mixed-writer test on a database holding the partitions of Requirement 053-008.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Each task names its files and tests.

- [ ] T053-1 Keep Architecture decision 0131 in step with this text; add its refinement note to Architecture decision 0082; write the order of an upgrade, the order when no process runs, the two phases with `instance-evict`, and the hotfix rule in `docs/runbooks/scaling.md`, `docs/runbooks/control-plane.md`, `docs/runbooks/execution-engine.md`, `docs/runbooks/instance-registry.md` and `docs/runbooks/lease-withdrawal.md`; create `docs/releases/0.3.0.md` with Open Question 4's statement; add rows for the lint, the merge-base check, the catalog test, the entry-point check and the forward-merge check to `docs/engineering/architecture-principles.md`; amend `specs/018-schedules-bulk-operations/contracts/errors.md` and `data-model.md`, `openapi.yaml` and `sdks/SPEC.md` with `SCHEMA_TOO_OLD` as design 048's Requirement 048-014 lists reasons.
  Refs: Requirement 053-001, Requirement 053-006, Requirement 053-007, Requirement 053-009
- [ ] T053-2 Write `scripts/check-migrations.py` over `pglast` (`scripts/requirements-lint.txt`, installed with hashes, `pip-audit` in CI), `scripts/check-migrations.toml` (the cut-off, the absent online rules' start, the hot tables, the partitioned parents), a positive and a negative fixture per rule of Requirement 053-003 under `scripts/tests/check-migrations/` (a definer created inside a `DO` block, a waiver of a security rule, a moved boundary, an old timestamp and a narrowed capability list among them), the merge-base checks, `CODEOWNERS`, the release workflow's refusal to tag without the online rules' start, the request to the owner for "Require review from Code Owners" on `develop` and `main`, the catalog test of every definer function in `tests/integration/platform-foundation/migrations.rs`, and their CI steps.
  Refs: Requirement 053-001, Requirement 053-002, Requirement 053-003, Success criterion 053-002
- [ ] T053-3 Add `sagat_schema_missing()`, the capability map beside `MIGRATOR` with its test, `SchemaVerdict`, the check at start, at confirmation and on beats while too old, failing closed, `connect_serving()` with its architecture check and fixture, the registration with the capabilities the schema knows and the second registration, the one-run commands' check through `command_await_durability` and their exit codes, the gate reason `SCHEMA_TOO_OLD` with its alert and rule test, with the tests `a_binary_before_its_migrations_serves_within_one_heartbeat_after_them`, `a_binary_whose_capability_the_schema_lacks_registers_without_it_then_with_it` (on a test-only migration directory adding a test-only capability, with the expected versions injected), `a_missing_schema_function_closes_the_gate`, `the_schema_check_of_the_migrators_own_versions_is_empty_after_migrate_and_the_pending_set_before`, `every_one_run_command_reaches_the_database_through_connect_command` and `an_unavailable_answer_before_authentication_shows_no_version`, as the runtime role.
  Refs: Requirement 053-005, Success criterion 053-003
- [ ] T053-4 Make `sagat_infra_postgres::migrate()` the loop and add `migrate_online()` (the migration lock with its bound, the validation of applied migrations and of their indexes, the lock and statement timeouts, the bounded retry with an injectable backoff, the concurrent build checked before its version is recorded, the deadline, the architecture check against a direct `MIGRATOR.run`), with the tests `a_failed_concurrent_index_build_is_retried_three_times_then_named`, `an_invalid_index_is_never_recorded_as_applied`, `a_lock_held_longer_than_the_retries_ends_migrate_naming_the_table`, `a_lock_released_within_the_retries_lets_migrate_proceed`, `a_deadlock_victim_migration_is_retried`, `a_concurrent_build_waits_for_a_transaction_held_ninety_seconds`, `the_deadline_cancels_a_concurrent_build_on_the_server`, `a_same_named_index_on_another_table_is_not_taken_for_the_migrations`, `concurrent_runners_do_not_corrupt_or_fail` (kept) and `migrate_with_no_process_running_proceeds_once_a_new_process_confirms_with_its_gate_closed` (whose new process carries a capability its schema does not yet list).
  Refs: Requirement 053-004
- [ ] T053-5 Add `state`, `draining_since` and `draining_epoch`, the history actions, `serving_instances.applied_epoch`, `instance_heartbeat_applied`, `platform_requirement_drain`, `platform_requirement_undrain` and `instance_evict` with their commands and outcomes, the guards' rules of Requirement 053-007, `state = 'active'` in `schedule_trigger_handover_complete` and `lease_withdrawal_required()`, `ServingGate::uses` and the state in `requirements()`, the capability descriptors with their test over every `Capability`, phase 2's locks, checks and outcomes with `ChangeRefused::NotApplied` and `ChangeRefused::DrainCheckRefused`, the drain checks beside `ACTIVATABLE`, the call sites that remove an active row moved to a `drain_then_remove` fixture, with the tests `phase_two_refuses_a_process_paused_between_its_beat_and_its_refresh_then_removes_after_its_next_beat`, `phase_two_waits_for_a_use_that_began_before_phase_one_and_then_sees_it` (a barrier inside the use of a form (a) fence; the design that builds form (b) adds its own, with a REPEATABLE READ writer that neither deadlocks nor fails its request), `phase_two_removes_once_every_live_process_applied_and_the_drain_check_passes`, `phase_two_runs_the_drain_check`, `a_write_refused_by_the_fence_is_retried_with_the_capability_inactive`, `a_failed_requirements_read_makes_uses_false`, `an_applied_epoch_above_the_current_one_is_stored_as_the_current_one`, `a_failed_requirements_read_after_a_re_registration_reports_the_older_epoch`, `a_process_outside_its_capability_window_uses_no_capability`, `a_draining_capability_stops_new_uses_at_the_next_heartbeat`, `an_active_row_cannot_be_deleted`, `draining_schedules_after_the_handover_is_refused`, `a_capability_an_active_one_follows_cannot_be_drained`, `activating_a_draining_capability_is_refused`, `an_undrained_capability_is_active_again_after_its_census`, `an_evicted_row_no_longer_holds_phase_two` (its process kept running) and `an_evicted_running_process_cannot_re_register_under_its_id` (with a registration held by a barrier against the eviction), and one test per reader of the table of Requirement 053-007, as the runtime role where the runtime role calls them.
  Refs: Requirement 053-006, Requirement 053-007, Success criterion 053-004
- [ ] T053-6 Write `scripts/test-mixed-writers.py`, its fixture migrations under `scripts/tests/mixed-writers/` and the module factored out of `scripts/test-release-upgrade-profile.py`, and add the job `mixed-writers` to `.github/workflows/release.yml` on a free standard runner, with every pass condition of Requirement 053-008, the assertions that a capability was removed (or "rollback not applicable" recorded) and that the rolled-back process served, and the database at the partitions of Requirement 053-008; it uses design 051's driver (Task 051-2) and publishes its run with the runner named. Add the release workflow's forward-merge check of hotfix migrations.
  Refs: Requirement 053-008, Requirement 053-009, Success criterion 053-001

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs the suites of what it touches, `integration_claim_transition_efficiency` when it touches
`crates/infra-postgres/`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and the documentation and
architecture checkers of `scripts/`, the new lint among them. Each named test of Tasks 053-3 to 053-5 is seen failing
first, before the change; those of Tasks 053-3 and 053-5 also run with `SAGAT_TEST_AS_RUNTIME_ROLE=1`. Task 053-6 is the
release gate for upgrades.
