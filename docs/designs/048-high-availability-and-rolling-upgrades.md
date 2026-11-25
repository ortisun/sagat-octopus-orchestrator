# A reference high-availability topology for PostgreSQL, and rolling upgrades with N and N-1 writers

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

PostgreSQL is the platform's only authority. Today its high availability is left to the operator, a promotion and the
switch of the address the platform connects to are manual, and an upgrade stops every writer. This design makes all
three part of the product, inside one region:

1. **A reference topology** that the repository runs and tests: CloudNativePG on Kubernetes (`infra/kubernetes/ha/`,
   with the platform's chart in `infra/helm`) and Patroni on Docker Compose (`infra/docker/compose.ha.yaml`). Both keep a
   primary and two standbys with synchronous replication (a commit returns when either standby has it), promote a
   standby without a person, and move the one address the platform uses to the new primary.
2. **The platform's side of a failover**: dead connections are found within 12 s by socket keepalive; a connection to
   a read-only database is not reused; the platform never cancels a commit that waits for a standby and reports an
   unknown outcome when its own deadline passes; after a database restart a process serves nothing until one write of its
   own has reached a standby; worker reports keep retrying through a failover, and the lease time to live is sized to the
   expected failover.
3. **What a standby may serve**: nothing on the path that decides or writes; two reads that tolerate staleness, through
   an optional second address.
4. **Rolling upgrades with old and new writers together**: migrations that expand before a release and contract after
   it, a lint of new migrations, a handshake by which a binary refuses a schema it cannot use, and a test that runs N-1
   and N writers on one database under load.
5. **Evidence**: a disruption and failover drill on Compose that gates every release, and a three-zone drill on
   Kubernetes once its runner is approved, both measured against the owner's objectives (RPO 0 for acknowledged work,
   failover within 5 minutes, Owner decision 2026-10-16, row 8).

Cross-region failover is feature 032 (Architecture decision 0067); this design is the in-region base it builds on.

**Terms used below.** A *transition* is one step's result committed by the platform. A *lease* is a worker's right to
execute one attempt of a step until an expiry on the database clock, fenced by a token and a generation, so that a stale
attempt is refused; a *review item* is work that waits for a person because its outcome is unknown (feature 037). The
scheduler's work is divided into 64 *shards* held by engines under *shard leases*; the *keeper* renews them every
*keeper round* (a fifth of the shard lease, 3 s by default; `crates/scheduler/src/shards.rs`), and the *lost-lease pass*
reclaims expired worker leases. The *instance registry* (Architecture decision 0082) records the live processes and their
*capabilities*; a process whose requirement is unmet keeps running with its *serving gate* closed; the registry's
*census* says whether an older release is still live, and its *activation* switches a capability on only when none is.
The *integrity oracle* is the check of `tests/performance/scalability-ha/` that no acknowledged run was lost, no lease was
granted twice, and replaying each run's journal gives its projection. The *chain shape* is design 051's chain of three
activities, driven by design 051's driver.

## The Problem

From the code and documents on `develop` at `546d5aa5`:

- **No topology.** Every environment the repository defines runs one PostgreSQL container (`infra/docker/compose.yaml`,
  `postgres:16-alpine`, not pinned by digest). The chart does not run PostgreSQL (`docs/runbooks/scaling.md`, section 4).
  The Kubernetes validation runs a single PostgreSQL Deployment (`tests/e2e/scalability-ha/kubernetes/dependencies.yaml`).
- **Failover is a manual procedure.** `docs/runbooks/disaster-recovery.md` says to fence the old primary, run
  `pg_ctl promote`, then point the platform at the new one. The drill (`infra/dr/drill.sh`) promotes a standby on
  loopback in 0.2 s but never points the platform at it, and adds a 15 s shard takeover as a constant
  (`infra/dr/drill.sh:45`). Gaps 250 and 251 of feature 010 record what was not covered; gap 247 is resolved for its
  task, with node drain and multi-node disruption left as its residual.
- **The platform is not ready for a failover.**
  - One `DATABASE_URL` per process; nothing routes a query to a standby (gap 242 of feature 010).
  - SQLSTATE `25006` (`read_only_sql_transaction`) is classified as internal in `crates/infra-postgres/src/identity.rs`
    and as "other" in the registry's `registry_error` (`crates/infra-postgres/src/instances.rs`), so a read-only primary
    would close the registry's gate after 30 s; class 57 is all retryable, `57P04` (database dropped) included.
  - The pool sets `acquire_timeout` 3 s and `statement_timeout` 5 s and no hook (`crates/infra-postgres/src/pool.rs`;
    sqlx's own maximum lifetime of 30 minutes applies). sqlx 0.9 opens plain TCP sockets with no keepalive
    (`sqlx-core/src/net/socket/mod.rs`), so a statement whose bytes a server acknowledged before it vanished waits for
    ever, and unacknowledged bytes wait for the kernel's retransmission timeout (about 15 minutes by default).
  - What happens to a commit that waits for a standby is not established. As reviewers read PostgreSQL's source, the
    server turns the statement timeout off before it commits, so its timeout never cancels the wait; feature 032 lists the
    point as unproven (its gap 1578). What is known: an explicit cancel or termination of a waiting backend, and a
    primary that restarts after a crash, make a commit visible that no standby holds; the driver does not surface the
    server's warning (sqlx 0.9 logs notices); and nothing in the platform bounds a commit on the client.
  - The SDKs make 4 attempts of a failed call, 3 retries from 200 ms to at most 1.4 s with jitter, `CompleteStep` up to 5
    (`sdks/SPEC.md`); worker leases last 30 s (`SAGAT_LEASE_TTL_SECS`); an outage longer than a lease's remainder makes it
    expire, and since feature 037 such an activity waits for a person unless it has a `reconcileWith` lookup.
- **Upgrades stop the world.** `docs/runbooks/scaling.md` (section 3) and the 0.2.0 release notes say to stop every
  writer, migrate, then deploy; `docs/runbooks/control-plane.md` says the opposite. No binary checks the schema it runs on
  (and the runtime role may not read `_sqlx_migrations`). sqlx runs each migration file as one simple query, one implicit
  transaction, with no lock timeout and no index built concurrently. The released-upgrade tests prove an old reader and an
  old worker against the new schema, not old and new writers together.
- **The chart spreads but does not insist.** Topology spread is hard-coded `whenUnsatisfiable: ScheduleAnyway`
  (`infra/helm/templates/_helpers.tpl`), with no value to change it.

## Requirements

### The reference topology

- **Requirement 048-001 Kubernetes reference.** `infra/kubernetes/ha/` MUST hold a CloudNativePG `Cluster` of three
  instances, one per zone (required pod anti-affinity on `topology.kubernetes.io/zone`), on PostgreSQL 16 (the major CI
  uses), with synchronous replication of quorum one, the data durability that refuses to commit without a synchronous
  standby, the failover quorum that promotes only a standby known to hold every acknowledged commit, and the primary
  isolation check; WAL archiving and a daily base backup to an object store through the Barman Cloud plugin; the operator
  installed for one namespace only (not cluster-wide), the database in a namespace apart from the platform's Secrets; the
  runtime role `sagat_app` created through the cluster's managed roles with a password Secret, the owner's Secret mounted
  only by the migrate job, superuser access off; an ingress network policy that admits only the platform's pods and the
  operator; `reserved_connections` for the operator's and monitoring's roles and a `CONNECTION LIMIT` on `sagat_app` equal
  to the chart's connection budget; every version (the first operator release that has the synchronous failover quorum,
  checked by Task 048-3) and image pinned by digest.
- **Requirement 048-002 Development reference.** `infra/docker/compose.ha.yaml` MUST start three PostgreSQL 16 nodes
  under Patroni 4 in quorum synchronous mode with the strict setting, an etcd of three members, and an HAProxy that sends
  the platform's one address to the current primary (`GET /primary`) and ends a demoted node's sessions; etcd and
  Patroni's REST interface on a network of their own (`internal: true`) that only the database nodes and HAProxy join,
  with Patroni's REST authentication and etcd's authentication from `.env` (`${VAR:?}`), no host port for etcd, HAProxy's
  statistics off; the platform's services reach only HAProxy's port; images pinned by digest, the platform's ports on the
  loopback, a healthcheck per service (`AGENTS.md`, rule 16).
- **Requirement 048-003 One address.** The platform MUST connect through one address that the HA manager moves to the
  primary. No platform process may need a person, a restart or new configuration after a promotion.
- **Requirement 048-004 Spread that insists.** The chart MUST expose `topologySpread.whenUnsatisfiable` (default
  `ScheduleAnyway`) and `topologySpread.nodeTaintsPolicy` (default `Ignore`) in `infra/helm/values.yaml`, used by
  `_helpers.tpl`; the example `infra/helm/examples/ha-cloudnativepg.yaml`, which starts from `ha.yaml`, sets
  `DoNotSchedule` and `Honor`. The existing disruption budgets stay.
- **Requirement 048-005 Socket keepalive.** Every database socket of the platform MUST set `SO_KEEPALIVE` with an idle
  time of 5 s, an interval of 2 s and 3 probes, and, on Linux, `TCP_USER_TIMEOUT` of 12 s, so that a dead peer is found
  within 5 + 2 × 3 = 11 s when nothing is unacknowledged and within 12 s when something is. sqlx 0.9 has no option for
  either, so the repository vendors `sqlx-core` with this one change (as it vendors `rskafka`), verified by
  `scripts/check-vendor.py` and offered upstream. A live server that is only slow, a commit waiting for a standby among
  them, answers the probes and is not cut. On Compose, HAProxy checks each node every second (`inter 1s fall 3`), ends a
  demoted node's sessions (`on-marked-down shutdown-sessions`), and sets no server or client timeout of its own.

### The platform's side of a failover

- **Requirement 048-010 Dead connections are found.** A statement or a commit to a server that stopped answering MUST
  fail within 12 s (Requirement 048-005); a statement to a live server is bounded by its `statement_timeout` (5 s). Idle
  connections are tested before reuse (sqlx's `test_before_acquire`, its default).
- **Requirement 048-011 Read-only and unreachable databases.** SQLSTATE `25006` MUST be classified as unavailable and
  retryable when the database is in recovery (`pg_is_in_recovery()` on a fresh connection), and as internal otherwise
  (the platform's own read-only transactions); in `identity.rs` and in `instances.rs`'s `registry_error`. `57P04` is not
  retryable. A process that sees an unavailable `25006` records the time; the pool's `before_acquire` hook discards,
  without a statement, every connection created before it. A new connection of a writer pool is refused when the
  database is in recovery (the optional read pool and a passive process of feature 032 are exempt). Readiness follows
  Architecture decision 0082 unchanged: a process stays ready while the registry's last answer is within its liveness
  window (30 s), and answers requests that need the database with `503 UNAVAILABLE` and `Retry-After: 2`.
- **Requirement 048-012 Commits and writes.** Every write MUST be inside a transaction, and every commit MUST go through
  one function, `commit_safely` in `pool.rs`, which moves the transaction into a task it spawns (as `begin_safely` does
  for `BEGIN`) and waits for it at most the commit deadline (`SAGAT_DB_COMMIT_DEADLINE_MS`, default 10,000, at most
  60,000, at least twice the statement timeout, as feature 032's decision D1585). Past the deadline, and on any failure
  after `COMMIT` was sent (an I/O error, class 08, `57P01` to `57P03`), the caller gets
  `StoreError::CommitOutcomeUnknown`, the generalization of Architecture decision 0078's `CommitOutcomeUnknown`. The task
  keeps the connection until the server answers or the socket fails; after a hard limit of 60 s it drops the connection
  (the backend keeps waiting, counted by a metric), so that waiting commits cannot exhaust the pool. The API reports code
  `UNAVAILABLE` with reason `COMMIT_OUTCOME_UNKNOWN`, retryable only for an operation that carries an idempotency key or
  a command identifier; it is an API reason, not a step outcome code, so it is not in the list of unknown-outcome step
  codes. The platform never cancels or terminates a backend that waits for a standby: the writer pools set
  `client_connection_check_interval` to 0 and, where the server has it, `transaction_timeout` to 0, and
  `scripts/check-architecture.py` refuses `pg_cancel_backend`, `pg_terminate_backend`, a write statement executed on the
  pool outside a transaction, and a commit outside `commit_safely`. Whether the server's own timeouts can cancel the wait
  is established by Task 048-6 before the other tasks of this section; if they can, `commit_safely` sends `SET LOCAL
  statement_timeout = 0` and `COMMIT` in one message.
- **Requirement 048-013 After a restart, nothing is served until a write reaches a standby.** Each writer connection's
  `after_connect` MUST read `pg_postmaster_start_time()` and `pg_is_in_recovery()` in one statement. When the start time
  differs from the one the process last confirmed (or the process has confirmed none), `after_connect` refuses the
  connection, the process sets the serving gate's reason `DATABASE_UNCONFIRMED`, and one background task commits, through
  the registry's pool, an upsert of the process's row of `durability_beacons` (keyed by its instance identifier), which
  waits for a standby; when it returns, the process records the start time and clears the reason. Since a standby
  receives WAL in order, everything committed before that write, including what a primary restarted after a crash marked
  committed, is then on a standby, and no connection of the process read anything before. At most one beacon per process
  waits at a time.
- **Requirement 048-014 The serving gate has reasons.** The registry's serving gate (`crates/application/src/instances.rs`)
  MUST become a set of closing reasons (the registry's own, `DATABASE_UNCONFIRMED`, `SCHEMA_TOO_OLD`, `BINARY_TOO_OLD`),
  open only when the set is empty; each owner clears only its own reason, and a re-registration does not reopen a gate
  another reason holds.
- **Requirement 048-015 Reports survive a failover.** The six SDKs MUST keep retrying a `CompleteStep` or `FailStep` that
  failed as unavailable or as a transport error (a refused connection, a reset, a timeout, an ingress `502` or `503`)
  for a report retry window (`reportRetryWindow` in each SDK's idiom), 310 s by default, in place of the attempt count:
  the release gate (300 s) plus the backoff cap (5 s), the platform's `Retry-After` (2 s) and the acquire timeout (3 s),
  300 + 5 + 2 + 3 = 310 s. A heartbeat keeps today's retries and is never retried past the time of the next heartbeat. The
  conformance scenario `report-through-failover` answers `UNAVAILABLE` without `Retry-After` for 8 s with a window of 10 s
  (today's five attempts last at most 3 s, so today's SDKs fail it), and a negative case answers for 12 s, where the SDK
  must give up.
- **Requirement 048-016 Leases and work in flight.** A failover MUST lose no acknowledged work. A worker lease is renewed
  by heartbeats at most 10 s apart, so it always has at least its time to live minus 10 s left; a failover whose outage, as
  the platform sees it, is shorter than that (20 s with the default time to live of 30 s) expires no lease, and for workers
  on SDKs with the report window it repeats no completed activity and creates no review item. For longer failovers, the
  runbook's rule is a time to live of at least the expected outage plus two heartbeat intervals
  (`SAGAT_LEASE_TTL_SECS`); the drills report the leases expired, the activities repeated and the review items created.
  Carrying leases across a longer failover is a follow-up design (Non-Goals).

### What a standby may serve

- **Requirement 048-020 Standbys decide nothing.** No claim, lease, transition, replay, receipt, quota read,
  authorization read or secret read may be served by a standby.
- **Requirement 048-021 One stale-tolerant read.** A process MAY have a second, optional address (`DATABASE_READ_URL`,
  the HA manager's replica service, which routes only to standbys of the current primary; in-region in a feature 032
  cell) for one read: `GET /runs/{runId}/events` of a terminal run. It is authorized on the primary first (the tenant
  scope and the run's terminal state, in the tenant-scoped query); on the standby, one statement reads the events only if
  the run is terminal in the standby's own snapshot, which proves the standby has every event of the run (they all precede
  the terminal transition); if it is not, and on any standby error, the primary answers; a miss, 404 or 403 is always the
  primary's. A response served by a standby carries `Sagat-Read-Source: standby` with no value. The read pool's sessions
  are read-only and its statement timeout 5 s; the reference bounds `max_standby_streaming_delay` at 30 s. The pool is a
  type, `StaleReadPool`, that only `crates/infra-postgres/src/stale_reads.rs` can build, which
  `scripts/check-architecture.py` checks. The read relieves the primary of journal pages, which are the largest reads; it
  gains no availability in a failover.

### Rolling upgrades

- **Requirement 048-030 Expand and contract.** Every migration after the lint's cut-off MUST declare its phase on the
  line after an optional `-- no-transaction`: `-- phase: expand` or `-- phase: contract`, and a contract names its expand
  and the capability that must be active before it runs (`-- contract-of: <version> <capability>`). An expand migration
  leaves the schema usable by release N-1; a contract migration removes only what N-1 no longer uses and ships in a
  release after its expand. Applied migrations are never edited: a test compares their checksums with
  `crates/infra-postgres/migration-checksums.txt`, written when a release is cut.
- **Requirement 048-031 Migrations do not block the hot path.** A transactional migration MUST take at most one lock that
  blocks inserts, in its last statement, under `SET LOCAL lock_timeout = '2s'`; a migration without a transaction MUST
  hold one concurrent statement only (an index built concurrently, preceded in its own earlier migration by `DROP INDEX
  CONCURRENTLY IF EXISTS` of the same name) and be idempotent; constraints are added `NOT VALID` and validated in a later
  migration; a migration MUST NOT rewrite a table, change a column's type in place, rename, add a column with a volatile
  default or a `NOT NULL` column without a default, run `SET NOT NULL` except after a validated check in a contract
  migration, or replace a function, trigger, policy or view in an expand migration (a changed function gets a new name; a
  security fix replaces in place under a waiver). A definer function pins `search_path` and is revoked from `PUBLIC`. A
  waiver is a header line `-- lint-allow: <rule> <reason>`, printed by the lint.
- **Requirement 048-032 The lint and the runner.** `scripts/check-migrations.py` MUST refuse a migration after the
  cut-off (recorded in `scripts/check-migrations.toml`, the newest migration on `develop` when the lint merges) that breaks
  Requirement 048-030 or 048-031, by rules on its statements, with a negative fixture per rule under `scripts/tests/`;
  `.down.sql` files are checked only for the security rules. `control-plane migrate` MUST apply migrations one by one
  through its own loop over sqlx's migrator, retry a migration that failed with `55P03` (lock not available) with backoff,
  after each migration without a transaction check `pg_index.indisvalid` of every index it names and, on an invalid one,
  drop it concurrently and retry, set `transaction_timeout` to 0 for its session where the server has it, refuse a
  contract migration unless its named capability is active and the census shows no process older than the release before
  it, and refuse to run from an older release than the database's newest row of `schema_compatibility` (a database
  without the table, fresh or 0.2.x, is migrated from the start).
- **Requirement 048-033 A binary refuses a schema it cannot use.** When a release is cut, `scripts/release-version.sh`
  writes `crates/infra-postgres/releases.toml` (`version`, `last_migration`, `oldest_compatible`) and a migration that
  inserts that release's row into `schema_compatibility (release text primary key, last_migration bigint,
  oldest_compatible text, recorded_at timestamptz)`; a hotfix release adds expand migrations only, ordered by semantic
  version. A binary knows its release from `CARGO_PKG_VERSION` and its migrations from sqlx's embedded list. At start
  and on each confirmation of Requirement 048-013 a process calls `sagat_schema_handshake()` (owner-defined, `SECURITY
  DEFINER`, `search_path` pinned, executable by `sagat_app` only), which returns the applied versions and the newest row;
  and the registry's functions in their new versions, `instance_register_v2` and `instance_heartbeat_v2`, refuse a process
  whose release is older than `oldest_compatible` in the transaction that already reads the requirement epoch, so that an
  activation and a registration cannot race. The process sets the gate's reason `SCHEMA_TOO_OLD` (a migration it was built
  with is missing) or `BINARY_TOO_OLD` (its release is older than `oldest_compatible`), writes nothing but its registry
  heartbeat and the release of its shards, and answers 503. Every entry point that opens the database calls it: the control
  plane, the engine, each command of `control-plane` (the `secret-cleanup` job included) and `migrate`, checked by
  `scripts/check-architecture.py`. The handshake tests run as the runtime role. `BINARY_TOO_OLD` wins over
  `BINARY_CAPABILITY_MISSING` when both apply. A 503 shows no version to the caller; the versions go to the log.
- **Requirement 048-034 New behaviour waits for the fleet.** A change that writes state release N-1 cannot process MUST
  be behind a registry capability, added to the capability lists by an expand migration (the list is a closed check,
  rewritten `NOT VALID` and validated in the next migration), and switched on by the registry's activation through
  `platform_requirement_activate_v2`, which raises `oldest_compatible` to N in the same transaction; its deactivation
  lowers it again and is refused once a contract migration naming the capability is applied. Each capability states, in
  its design, the check that the runs using it are drained before a deactivation (for design 049, no row of a dated
  bucket). In release 0.3.0 the behaviours behind capabilities are design 049's dated journal buckets and design 052's
  queue claim.
- **Requirement 048-035 Old and new writers run together.** `scripts/test-mixed-writers.py` MUST run engines and control
  planes of N-1 (built from its release's commit, pinned by SHA) and N on one database under the chain shape: apply N's
  migrations during the load, roll the processes one at a time, activate N's capabilities, deactivate and roll one back to
  N-1, and pass the integrity oracle, with the 99th percentile transition latency during the migrations no more than 2 s
  (the lock timeout) above a baseline run. Release N MUST NOT be published unless it passes against N-1. For release
  0.3.0, N-1 is the commit at which Task 048-13's handshake merged; the test first gates a release when 0.3.0 is N-1.

### Evidence

- **Requirement 048-040 The failover budget and the release gate.** For the loss of one database instance, the time from
  the fault to the first transition committed after it is reported at the median and the maximum, and so is the time until
  every shard is held again. The release gate is the owner's 300 s at the maximum. The median's target, a benchmark
  criterion, is 60 s: after the HA manager's promotion P and the surviving standby's attachment to the new primary R
  (both measured), the platform finds a dead connection within 12 s, opens one within 3 s (its beacon waits for R,
  already counted), and holds a shard within one keeper round of 3 s, so the first transition follows P + R by about
  12 + 3 + 3 = 18 s, leaving P + R about 42 s; a dead engine's shards wait for the rest of their 15 s lease plus a
  round, at most P + R + 12 + 15 + 3 = P + R + 30 s.
- **Requirement 048-041 The drills.** The Compose drill (`infra/dr/ha-drill.sh`) MUST inject, five times each: the
  primary's container stopped; a node stopped; both standbys stopped (commits wait, the deadline reports an unknown
  outcome, nothing becomes visible); a standby throttled and then the primary stopped; a backend of the primary killed
  (`kill -9`, a crash restart in place); and the primary's network disconnected (`docker network disconnect`, a dead peer
  with acknowledged data). The Kubernetes drill (`tests/e2e/high-availability/`) adds, on six kind nodes in
  three labelled zones: deletion of the primary's pod; a drain of its node; the loss of a zone (its nodes' containers
  stopped); the isolation of the primary's node (`docker network disconnect`); and a rolling upgrade from N-1 to N
  (`helm upgrade` during the load). Both run the chain shape at a fixed open-loop rate through design 051's driver.
- **Requirement 048-042 Measured, not estimated.** For each repetition a drill MUST record: P and R; the times of
  Requirement 048-040; the time until the completion rate is back to 90% of its rate before the fault, held for three
  10 s windows; acknowledged work lost; leases expired; activities run more than once; review items created; and, before the former
  primary rejoins, its WAL after the fork position read with `pg_waldump` from its data directory, compared with the
  driver's ledger of acknowledgments (one line per acknowledged start and completion, with its time), so that a commit
  acknowledged by two primaries is found. A negative control injects, with synchronous replication switched off on the
  isolated primary, a commit the driver sees acknowledged, and the comparison must fail. Reports go to
  `tests/performance/reports/high-availability.md` with the environment and every failure.
- **Requirement 048-043 The drills gate the release.** A drill script MUST exit non-zero when any repetition loses
  acknowledged work, finds an acknowledged commit after a fork, or exceeds 300 s to the first transition. The Compose
  drill gates every release; the Kubernetes drill gates it once its runner is approved (Owner decision 2026-11-24-515), and
  until then the report says it has not run. `infra/dr/drill.sh` keeps its single-host checks; its 15 s constant is
  replaced by the measured takeover of `ha-drill.sh` in the report.

### Success criteria

- **Success criterion 048-001** In every repetition of every fault, acknowledged work lost is 0 and no acknowledged commit
  is found after a fork on a former primary; the negative control fails as it must.
- **Success criterion 048-002** The median time to the first committed transition is reported for each fault; for the
  primary's container stopped, a node stopped and the pod deleted, it is within 60 s.
- **Success criterion 048-003** In the mixed-writer test, the completion rate never falls to zero for more than 10 s and
  the integrity oracle passes.
- **Success criterion 048-004** In the Compose drill, a failover whose outage is under 20 s expires no lease, creates no
  review item and repeats no activity that had completed, for workers on SDKs with the report window; for longer ones the
  counts are reported.
- **Success criterion 048-005** The migration lint refuses each of its negative fixtures and passes every migration
  merged after its cut-off.

### User stories

- **User story 048-01** An operator installs the reference topology and loses a node or a zone without paging anyone for
  the database.
- **User story 048-02** An operator upgrades the platform during the day with runs in flight.
- **User story 048-03** A developer reproduces a failover on a laptop with one command.

## Goals

- A database failure inside a region is an event the platform rides through, not an incident.
- An upgrade is a rolling deployment, as for any stateless service.
- The recovery times are numbers anyone can rerun, not adjectives.

## Non-Goals

- **Cross-region failover**, cells and the region epoch: feature 032.
- **Building an HA manager.** The platform elects nothing (Architecture decision 0067, decision 3).
- **A connection pooler in the reference.** The platform sets session options at connection and uses prepared
  statements; a pooler in transaction mode would need both changed.
- **High availability of the object store, the identity provider, OpenBao and the notification broker.** The object
  store is the provider's (gap 251 of feature 010); OpenBao's Raft is feature 017's; the broker carries hints only and its
  loss costs nothing (constitution, principle VII).
- **The 0.2.0 to 0.3.0 upgrade.** It stays a stopped upgrade: 0.2.0 binaries have no handshake.
- **Workers on older SDKs** keep today's retries; the guarantees of Requirement 048-016 name the SDK versions they need.
- **Carrying leases across a failover longer than their remainder.** An earlier draft extended leases after a database
  restart or promotion; its review showed that detecting the event, fencing every reader of an expiry and doing it
  exactly once is a design of its own. It is a follow-up design; until then the time to live is sized to the expected
  failover (Requirement 048-016).

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6, with its reasoning.

1. **CloudNativePG or Patroni on Kubernetes?** CloudNativePG: it moves the read-write service itself, needs no external
   consensus store, and has the synchronous quorum, the failover quorum and backups to an object store. Patroni stays the
   reference outside Kubernetes. Delegate.
2. **Refuse to commit without a standby, or fall back to asynchronous?** Refuse. With two standbys, the loss of one still
   leaves a synchronous standby; writes stop only when both are gone. An operator may choose the preferred mode with its
   stated RPO. Delegate.
3. **Should standbys serve reads in 0.3.0?** Only the two reads of Requirement 048-021, off by default. Delegate.
4. **How is a commit kept from being visible before a standby holds it?** The platform never cancels a waiting commit,
   bounds it on its own side and reports an unknown outcome; after a restart, a process serves nothing until a write of
   its own reached a standby; whether the server's own timeouts can cancel the wait is established by a test first.
   Reading the server's warning after a cancelled wait, and a waited write on every replay path, were rejected: the
   driver cannot surface the warning, and the confirmation after a restart covers what a replay could read. Delegate.
5. **Extend leases after a database restart or promotion?** Not in this design. Two review rounds found that the
   detection, the fencing of every reader of an expiry and the exactly-once extension need a design of their own; the time
   to live sized to the expected failover covers the common case, and the drills count what a longer failover costs.
   Delegate.
6. **The decision records.** The topology, the commit rule, the leases and the reads are
   [Architecture decision 0130](../adr/ADR-0130-postgresql-high-availability-is-a-reference-topology-with-synchronous-quorum-and-one-moving-address.md);
   the upgrade rule is
   [Architecture decision 0131](../adr/ADR-0131-migrations-expand-before-and-contract-after-a-release-and-a-binary-refuses-a-schema-it-cannot-use.md).
   Architecture decision 0130 refines the lease rule of `docs/architecture/execution-semantics.md` ("an expired lease's
   token is stale") for this case only, and Architecture decision 0131 refines Architecture decision 0082 (two more gate
   reasons, an activation that raises the oldest compatible release).

## Owner decisions needed

- **The Kubernetes drill's runner.** Six kind nodes with the platform need a 16-vCPU runner, US$ 0.042 per minute on
  2026-11-24 (`docs.github.com/en/billing/reference/actions-runner-pricing`); 5 faults × 5 repetitions × 6 minutes = 150
  minutes plus 30 of preparation is 180 × 0.042 = US$ 7.56 per run. No paid run is planned until the owner's budget is
  confirmed (Owner decision 2026-11-24-515). Gates Task 048-16.

## Critical User Journeys (CUJs)

### CUJ 1: The primary's node is lost

*   **Given**: the reference topology, runs in flight.
*   **When**: the primary's node stops.
*   **Then**: the HA manager promotes the standby that holds every acknowledged commit and moves the address; the
    platform's dead connections fail within 12 s; each process's first new connection finds the server's start time
    changed and the process waits, serving nothing, until its beacon reaches a standby; workers' reports are retried until
    they land; no acknowledged run is lost.

### CUJ 2: Both standbys are gone

*   **Given**: a primary whose two standbys are unreachable.
*   **When**: a worker completes a step.
*   **Then**: the commit waits for a standby and stays invisible; at the commit deadline the worker gets
    `COMMIT_OUTCOME_UNKNOWN` and retries with its command identifier; the commit's task drops its connection after 60 s,
    so the pool is not exhausted; when a standby returns, the waiting commit finishes and the retry finds it.

### CUJ 3: A rolling upgrade

*   **Given**: release N-1 running.
*   **When**: the operator runs N's migrations and rolls the processes to N.
*   **Then**: N-1 and N processes run together; N writes no new state until the activation, which waits for the census;
    the contract migrations of N's expands ship with N+1 and wait for their capability.

### CUJ 4: A binary on the wrong schema

*   **Given**: release N deployed before its migrations were applied.
*   **When**: it starts.
*   **Then**: it closes its serving gate with `SCHEMA_TOO_OLD`, answers 503, writes nothing, and opens the gate once the
    migrations are applied.

## Technical Plan (Architecture)

### Topology

```
     node or zone a            node or zone b            node or zone c
   +----------------+        +----------------+        +----------------+
   | PostgreSQL     |  WAL   | PostgreSQL     |        | PostgreSQL     |
   | primary        |------->| standby        |        | standby        |
   |                |---------------------------------->|                |
   +----------------+        +----------------+        +----------------+
          ^  the one address (moved by the HA manager at promotion)
          |  a commit returns when either standby has flushed it
   engines and control planes, spread over the nodes or zones
```

On Compose the three nodes run on one machine; on Kubernetes, one per zone.

### The platform's side

| Change | Files | Requirement |
|---|---|---|
| Socket keepalive and `TCP_USER_TIMEOUT` | `vendor/sqlx-core` (one change in `src/net/socket/mod.rs`), the root `Cargo.toml` (`[patch.crates-io]`), `scripts/check-vendor.py` | 048-005 |
| `commit_safely`, the deadline and the hard limit, `CommitOutcomeUnknown`, the reason on `AppError::Unavailable` and its HTTP and gRPC mapping, writes inside transactions | `crates/infra-postgres/src/pool.rs`; about 233 commit sites and about 55 pool-level writes in `crates/infra-postgres`, `crates/application` and `crates/scheduler`; `crates/application/src/error.rs` (`StoreError`, `kind()`, `AppError`); `crates/infra-postgres/src/identity.rs` (`record()`); `crates/api-http` and `crates/api-grpc` error mapping | 048-012 |
| `25006` and `57P04`; the `before_acquire` discard; the recovery refusal | `identity.rs`, `instances.rs` (`registry_error`), `pool.rs` | 048-011 |
| The confirmation after a restart and the gate's reasons | `pool.rs` (`after_connect`), `crates/application/src/instances.rs` (the gate as a set; the beacon task), `crates/infra-postgres/src/instances.rs` (the beacon upsert) | 048-013, 048-014 |
| The report window | `sdks/SPEC.md` and the six SDKs; `tests/contract/sdk-conformance/scenarios/report-through-failover.json` | 048-015 |
| The read pool and the one read | `apps/control-plane/src/config.rs`, `apps/execution-engine/src/settings.rs`, `crates/infra-postgres/src/stale_reads.rs`, the events route of `crates/api-http` | 048-021 |

**The new tables**, each created by an expand migration with its grants and storage classification:

| Table | Columns | Runtime role |
|---|---|---|
| `durability_beacons` | `instance_id uuid primary key`, `written_at timestamptz` | `SELECT (instance_id)`, `INSERT`, `UPDATE` (an upsert needs the select on its conflict column); rows of instances gone for a day are deleted by housekeeping through a definer function |
| `schema_compatibility` | as Requirement 048-033 | none; read through `sagat_schema_handshake()` |

**Why one waited commit protects what came before it.** A standby receives WAL in order; a commit returns only after a
standby has flushed WAL up to that commit's position, which lies after every earlier commit's record. So once a
process's beacon commit returns, every transaction committed before it, including one a crashed primary marked committed
on restart, is on a standby. A transaction must write a row for its commit to wait: one that only took a transaction
identifier writes no WAL and does not wait. The residual: a commit exposed by an operator's cancel or termination of a
waiting backend, or by the HA manager's fast shutdown of an isolated primary (whose sessions it terminates too), is
visible for a moment to sessions of that server not yet ended; the platform's code never cancels (the architecture check),
the runbook forbids it, and `database_backends_waiting_for_standby` counts the waiting backends.

### Rolling upgrades

**The order of an upgrade**, written in `docs/runbooks/scaling.md` section 3 in place of the stopped upgrade, and in
`docs/runbooks/control-plane.md` and `docs/runbooks/execution-engine.md` so that they agree:

1. Apply N's migrations (`control-plane migrate`) while N-1 runs; contract migrations among them wait for their named
   capability and the census.
2. Roll the control planes and the engines to N, one at a time (`maxUnavailable: 0`, `maxSurge: 1`).
3. Activate N's capabilities.

Rollback before step 3 rolls the binaries back onto N's schema; after it, deactivating a capability lowers the oldest
compatible release, and the runs that used the capability are drained before the binaries roll back. The runbook forbids
running N-1's migration job during a rollback.

### The drills

The Compose drill is `infra/dr/ha-drill.sh`; the Kubernetes drill is `tests/e2e/high-availability/run.sh`. Each fault is a
command in the script, the driver's per-second counter and ledger are the measurement, and a repetition is 6 minutes (2
before the fault, 4 after). The fault tests of the platform's side run on a new test fixture,
`tests/support/replicated_postgres.rs`: a primary and one synchronous standby started by Testcontainers on a network of
their own, each test first asserting `pg_stat_replication.sync_state = 'sync'`; a dead peer is made by `docker network
disconnect` of the primary (a pause would still acknowledge); races use barriers.

**Metrics and alerts.** `database_connections_refused_total{reason}`, `database_commit_outcome_unknown_total`,
`database_backends_waiting_for_standby` (a gauge exported by the engine's monitor under its lease, from the runtime role's
own sessions in `pg_stat_activity`, which need no extra grant), `serving_gate_closed{reason}` (a
gauge); from the HA manager's exporter, the synchronous standbys, with the alerts `SagatDatabaseOneSyncStandby` (warning)
and `SagatDatabaseNoSyncStandby` (critical, writes stopped), whose rule tests join CI's `promtool` list. Labels are closed
sets.

**Order of the build.** Task 048-2 (the Compose topology) first; design 051's driver (its Task 051-2) before Tasks 048-15
and 048-17.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A commit waits for ever when both standbys are gone | By design (RPO 0); the commit deadline reports an unknown outcome; one connection attempt at a time; `reserved_connections`; the alerts |
| An operator exposes an unreplicated commit by cancelling a waiting backend | Forbidden in the runbook and in code; the metric; the residual bounded by the connection lifetime |
| A failover longer than a lease's remainder | The time to live sized by the runbook's rule; the drills count expired leases and review items; lease continuity is a follow-up design |
| The server's timeouts do cancel a waiting commit | Task 048-6 establishes it first; the alternative of Requirement 048-012 is then built |
| The vendored `sqlx-core` drifts from upstream | One change, verified by `scripts/check-vendor.py`, offered upstream; removed when upstream has the options |
| Old and new writers disagree on new state | Requirement 048-034; the mixed-writer test on every release |
| The migration lint blocks a needed change | A waiver header naming the rule and the reason |
| CloudNativePG or Patroni has a bug | Versions pinned; the drills on every release; the platform's oracle is the judge |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No commit is read before a standby holds it except after an operator's forbidden act (048-012, 048-013); leases, tokens and generations are unchanged; one transaction per transition is unchanged |
| II. Modular domain, ports and adapters | Pass | The gate's reasons and the confirmation are in the application's instance service; statements stay in `infra-postgres`; the domain is untouched |
| III. External effects are at-least-once; sagas compensate | Pass | The failover rules reduce repeats and review items but promise none; idempotency keys still absorb repeats |
| IV. Compatibility is versioned and pinned | Pass | Expand and contract, the compatibility table and the handshake; images pinned by digest; the reason, the gate reasons, the header and the SDK window added to their contracts |
| V. Code, tenant boundary and secrets are isolated | Pass | Authorization, misses and secrets stay on the primary; the read header carries no value; the database's control network is closed; the platform runs as `sagat_app`, never as the owner; the operator namespace-scoped |
| VI. Observability and audit are part of the feature | Pass | The metrics and alerts above |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | A duplicate answered after a failover reads only what a standby holds (048-013) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Ten drill faults, a negative control, a proving test first, fault tests on a replicated fixture |
| Delivery Constraints | Pass | "Single region with high availability" is what this design builds (constitution 4.7.0; 5.0.0, Owner decision 2026-11-24-2, keeps it per cell) |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Two reference topologies | CloudNativePG runs only on Kubernetes; developers and design 051 need one on Compose | One topology: no failover on a laptop, or a Kubernetes-only drill |
| `commit_safely` and the commit deadline | A commit to a dead server would wait for the kernel; an unknown outcome must not be reported as "not applied" | Dropping the commit future: leaves the connection in an unknown state |
| A vendored `sqlx-core` with socket keepalive | A dead peer with acknowledged data would hold a connection for ever | A wrapper around every statement: about 233 sites and borrowed executors |
| The confirmation after a restart | A commit a restarted primary marked committed must not be read before a standby holds it | Trusting `synchronous_commit = on` |
| The compatibility table, the handshake and the migration runner | A binary on the wrong schema writes state nobody can read; a contract must wait for its capability | Documentation only: the runbooks contradict each other today |
| The migration lint | The rules are easy to forget and costly when broken | Review only |

## Alternatives Considered

- **Leave HA to the operator** (today). Every operator rebuilds the same topology and the platform's side stays untested.
- **Several hosts in the URL with `target_session_attrs=read-write`.** The driver does not support it.
- **Asynchronous standbys only.** Faster commits, RPO above zero.
- **A client bound on every statement by a wrapper.** About 233 sites and borrowed executors; socket keepalive bounds
  every statement at once, and only the commit needs its own deadline.
- **The kernel's retransmission limit (`tcp_retries2`).** Bounds unacknowledged data only, and on Kubernetes needs an
  unsafe-sysctl allowance on the nodes.
- **A beacon on every new connection.** It runs inside the pool's acquire timeout, and every attempt that times out
  leaves a waiting backend behind; one confirmation per process after a restart does not.
- **Extending leases after a restart or promotion.** A design of its own (Non-Goals).
- **Blue and green upgrades with two databases.** Logical replication of the whole schema, with its triggers and definer
  functions firing again on apply.

## Non-Functional Requirements

- **Non-functional requirement 048-001 Hot path.** A lease and a completion gain no statement: `commit_safely` changes how
  the commit is awaited, not what is sent (unless Task 048-6 proves the server cancels waits, in which case the one-message
  commit adds a statement, recorded in design 038's budget). The start-time read runs once per new connection; the
  beacon and the handshake once per confirmation.
- **Non-functional requirement 048-002 Recovery.** As Requirement 048-040.
- **Non-functional requirement 048-003 Security.** The read address uses the runtime role with read-only sessions; the HA
  manager's roles and credentials are never the platform's; the Compose reference's credentials come from `.env`.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Each task names its files and tests; the metrics of a behaviour come with it.

- [ ] T048-1 Revise Architecture decisions 0130 and 0131 to this text; add their refinement note to Architecture decision 0082; write the topology and the one stale-tolerant read in `docs/architecture/storage-and-events.md` and `docs/runbooks/disaster-recovery.md`, and the lease time-to-live rule in `docs/runbooks/scaling.md`; keep feature 032's plan and tasks consistent (Task T032-058 builds on Task 048-7's `durability_beacons`).
  Refs: Requirement 048-001, Requirement 048-013, Requirement 048-016, Requirement 048-021, Requirement 048-030
- [ ] T048-2 Add `infra/docker/compose.ha.yaml` and `infra/docker/patroni/Dockerfile` (PostgreSQL 16 by digest, Patroni 4, etcd and HAProxy pinned), the internal control network, the authentications from `.env`, HAProxy's checks and session shutdown, healthchecks.
  Refs: Requirement 048-002, Requirement 048-003, Requirement 048-005
- [ ] T048-3 Add `infra/kubernetes/ha/` (the operator's values for one namespace, the `Cluster` with managed roles, reserved connections, the connection limit, backups, the network policy), the chart's `topologySpread` values, a chart check that the role's connection limit covers (the autoscaler's ceiling + the surge) × (the request pool + the registry pool) + the jobs + a margin, `infra/helm/examples/ha-cloudnativepg.yaml`, and a check that every image of both references is pinned by digest and the example's user is `sagat_app`.
  Refs: Requirement 048-001, Requirement 048-004
- [ ] T048-4 Vendor `sqlx-core` with the keepalive and `TCP_USER_TIMEOUT` change, extend `scripts/check-vendor.py`, and add the fault test `a_dead_peer_with_acknowledged_data_fails_within_twelve_seconds` on the replicated fixture (`docker network disconnect` after the commit was acknowledged).
  Refs: Requirement 048-005, Requirement 048-010
- [ ] T048-5 Classify `25006` (by `pg_is_in_recovery()`) and `57P04` in `identity.rs` and `instances.rs`, add the `before_acquire` discard and the recovery refusal, and the fault test `a_promoted_standby_with_the_old_primary_read_only_is_never_written_to`, with `database_connections_refused_total`.
  Refs: Requirement 048-011
- [ ] T048-6 First, the proving test `does_a_server_timeout_cancel_a_commit_waiting_for_a_standby` on the replicated fixture with the standby stopped, on PostgreSQL 16 and 17; record its result in this design. Then `commit_safely` with its deadline and hard limit, the migration of every commit and every pool-level write, `CommitOutcomeUnknown`, the reason in the API and gRPC mapping and the error contracts, the architecture checks, and the tests `a_commit_past_its_deadline_is_unknown_and_invisible` and `waiting_commits_do_not_exhaust_the_pool`, with `database_commit_outcome_unknown_total`.
  Refs: Requirement 048-012, Non-functional requirement 048-001
- [ ] T048-7 Add `durability_beacons`, the start-time read in `after_connect`, the confirmation task and the gate reason `DATABASE_UNCONFIRMED`, with the tests `a_primary_restarted_in_place_is_not_read_before_its_standby_catches_up` and `one_beacon_waits_per_process`, run as the runtime role, with `database_backends_waiting_for_standby`.
  Refs: Requirement 048-013
- [ ] T048-8 Make the serving gate a set of reasons in `crates/application/src/instances.rs`, with the test `a_re_registration_does_not_reopen_a_gate_another_reason_holds`.
  Refs: Requirement 048-014
- [ ] T048-9 Add the report retry window and the heartbeat rule to `sdks/SPEC.md` and the six SDKs, and the conformance scenario `report-through-failover` with its negative case.
  Refs: Requirement 048-015, Requirement 048-016
- [ ] T048-10 Add `DATABASE_READ_URL`, `StaleReadPool` in `crates/infra-postgres/src/stale_reads.rs`, the events read with the terminal check on the standby and `Sagat-Read-Source`, the architecture rule, the header in `specs/001-platform-foundation/contracts/openapi.yaml`, and the tests `another_tenants_run_gets_the_same_404_without_the_header` and `a_standby_that_lags_the_terminal_transition_does_not_serve`.
  Refs: Requirement 048-020, Requirement 048-021
- [ ] T048-11 Add the two database alerts with their rule tests in CI's `promtool` list.
  Refs: Requirement 048-012, Requirement 048-013
- [ ] T048-12 Write `scripts/check-migrations.py`, `scripts/check-migrations.toml` (the cut-off), its negative fixtures under `scripts/tests/`, the checksum test in `tests/integration/platform-foundation/migrations.rs` against `crates/infra-postgres/migration-checksums.txt`, and its CI step.
  Refs: Requirement 048-030, Requirement 048-031, Requirement 048-032, Success criterion 048-005
- [ ] T048-13 Add `schema_compatibility`, `releases.toml` written by `scripts/release-version.sh`, `sagat_schema_handshake()`, `instance_register_v2` and `instance_heartbeat_v2`, the handshake in every entry point with its architecture check, the gate reasons in the registry contract of feature 018, tested as the runtime role.
  Refs: Requirement 048-033
- [ ] T048-14 Write `control-plane migrate`'s own loop (retry on `55P03`, the invalid-index check, the contract refusals, the release check) in `apps/control-plane/src/lib.rs` and `crates/infra-postgres/src/lib.rs`, with a test that injects a failed concurrent index build.
  Refs: Requirement 048-031, Requirement 048-032
- [ ] T048-15 Add `platform_requirement_activate_v2` and its deactivation with the contract refusal, the upgrade order in the three runbooks, and `scripts/test-mixed-writers.py` in the release workflow; it uses design 051's driver (Task 051-2).
  Refs: Requirement 048-034, Requirement 048-035, Success criterion 048-003
- [ ] T048-16 Write the Kubernetes drill in `tests/e2e/high-availability/run.sh` with the WAL comparison and its negative control, run it, and publish its section of `tests/performance/reports/high-availability.md`.
  Refs: Requirement 048-041, Requirement 048-042, Requirement 048-043, Success criterion 048-001, Success criterion 048-002
  Owner-gate: pending, the owner's row approving the drill's runner of "Owner decisions needed" (Owner decision 2026-11-24-515: no paid run until it is confirmed)
- [ ] T048-17 Write `infra/dr/ha-drill.sh` on the Compose reference with the WAL comparison and its negative control, run it in the `scale` job and the release workflow, and publish its section; it uses design 051's driver (Task 051-2).
  Refs: Requirement 048-040, Requirement 048-041, Requirement 048-042, Requirement 048-043, Success criterion 048-004

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs the suites of what it touches, `integration_claim_transition_efficiency` when it touches
`crates/infra-postgres/`, the fault suite of feature 010 (`tests/fault/failover`), the new fault tests on the replicated
fixture, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and the documentation and architecture
checkers of `scripts/`. Each named test of Tasks 048-4 to 048-10 is seen failing first, by a hand mutation where the
behaviour exists or before the change where it does not; those of Tasks 048-7 and 048-8 also run with
`SAGAT_TEST_AS_RUNTIME_ROLE=1`. Task 048-9 runs the SDK conformance scenario in all six
languages. Task 048-15 is the release gate for upgrades; Tasks 048-16 and 048-17 are the gate for recovery.
