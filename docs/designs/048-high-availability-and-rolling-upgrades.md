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
   with the platform's chart in `infra/helm`) and Patroni on Docker Compose for development
   (`infra/docker/compose.ha.yaml`). Both keep a primary and two standbys in three zones with synchronous replication
   (a commit returns when either standby has it), promote a standby without a person, and move the one address the
   platform uses to the new primary.
2. **The platform's side of a failover**: it finds dead connections quickly, never uses a connection to a read-only
   database, never lets the database cancel a commit's wait for a standby (so nothing unreplicated becomes visible), makes
   every new connection wait for the standby before it is used, does not let a database outage count against workers'
   leases, and keeps worker reports retrying through a failover.
3. **What a standby may serve**: nothing on the path that decides or writes; a closed list of reads that tolerate
   staleness, through an optional second address.
4. **Rolling upgrades with old and new writers together**: migrations that expand before a release and contract after
   it, a lint of new migrations, a handshake by which a binary refuses a schema it cannot use, and a test that runs
   N-1 and N engines on one database under load.
5. **Evidence**: a three-zone disruption and failover drill on Kubernetes and a drill on Compose, measured, published and
   checked against the owner's objectives (RPO 0 for acknowledged work, failover within 5 minutes, Owner decision
   2026-10-16, row 8).

Cross-region failover is feature 032 (Architecture decision 0067); this design is the in-region base it builds on.

**Terms used below.** A *transition* is one step's result committed by the platform. A *lease* is a worker's right to
execute one attempt of a step until an expiry on the database clock, fenced by a token and a generation, so a stale
attempt is refused; a *review item* is work that waits for a person because its outcome is unknown (feature 037). The
scheduler's work is divided into 64 *shards* held by engines under *shard leases*; the *keeper* renews them every *keeper
round* (3 s by default; `docs/runbooks/scaling.md`), and the *lost-lease pass* reclaims expired worker leases. The
*instance registry* (Architecture decision 0082) records the live processes and their *capabilities*; a process whose
requirement is unmet keeps running with its *serving gate* closed; the registry's *census* says whether an older release
is still live, and its *activation* switches a capability on only when none is. The *integrity oracle* is the check of
`tests/performance/scalability-ha/` that no acknowledged run was lost, no lease granted twice, and that replaying each
run's journal gives its projection. The *chain shape* is design 051's chain of three activities.

## The Problem

From the code and documents on `develop` at `546d5aa5`:

- **No topology.** Every environment the repository defines runs one PostgreSQL container (`infra/docker/compose.yaml`,
  `postgres:16-alpine`). The chart does not run PostgreSQL (`docs/runbooks/scaling.md`, section 4). The Kubernetes
  validation runs a single PostgreSQL Deployment (`tests/e2e/scalability-ha/kubernetes/dependencies.yaml`).
- **Failover is a manual procedure.** `docs/runbooks/disaster-recovery.md` says to fence the old primary, run
  `pg_ctl promote`, then point the platform at the new one. The drill (`infra/dr/drill.sh`) promotes a standby on
  loopback in 0.2 s but never points the platform at it, and adds a 15 s shard takeover as a constant. Gaps 250 and 251
  of feature 010 record that no standby in another zone, no reconnection through a moving address, and no object store
  were covered.
- **The platform is not ready for a failover.**
  - One `DATABASE_URL` per process; nothing routes a query to a standby (gap 242 of feature 010).
  - SQLSTATE `25006` (`read_only_sql_transaction`) is classified as internal, not as unavailable
    (`crates/infra-postgres/src/identity.rs`); nothing calls `pg_is_in_recovery()`.
  - The pool sets no maximum lifetime and no client-side bound on a statement whose server vanished without closing the
    connection (`crates/infra-postgres/src/pool.rs`).
  - The pool's `statement_timeout` (5 s) also applies to `COMMIT`. With synchronous replication, when it cancels a commit
    that is waiting for a standby, PostgreSQL ends the transaction locally and returns success with only a warning, which
    the driver does not surface (sqlx 0.9 turns notices into log events). The transaction is then visible to every
    reader, status reads and dispatchers included, although no standby has it; a failover loses it after the platform
    acknowledged it. Feature 032 decided the remedy for regions (its decision D1566); in one region the problem is the
    same.
  - The SDKs make 4 attempts in all of a failed report, from 200 ms to at most 1.4 s (`sdks/SPEC.md`); worker leases last
    30 s (`SAGAT_LEASE_TTL_SECS`); an outage longer than a lease's remainder makes the lease expire, and since feature 037
    such an activity waits for a person unless it has a `reconcileWith` lookup.
- **Upgrades stop the world.** `docs/runbooks/scaling.md` (section 3) and the 0.2.0 release notes say to stop every
  writer, migrate, then deploy; older text in `docs/runbooks/control-plane.md` says the opposite. No binary checks the
  schema it runs on (and the runtime role may not read `_sqlx_migrations`). Every migration runs in one transaction,
  none builds an index concurrently, and nothing limits how long a migration waits for a lock. The released-upgrade tests
  prove an old reader and an old worker against the new schema, not old and new writers together.
- **The chart spreads but does not insist.** Topology spread is `whenUnsatisfiable: ScheduleAnyway`
  (`infra/helm/templates/_helpers.tpl`). Node drain and zone loss have never been tested (gap 247 of feature 010).

## Requirements

### The reference topology

- **Requirement 048-001 Kubernetes reference.** `infra/kubernetes/ha/` MUST hold a CloudNativePG `Cluster` of three
  instances, one per zone (required pod anti-affinity on `topology.kubernetes.io/zone`), with synchronous replication of
  quorum one (`method: any`, `number: 1`), the data durability that refuses to commit without a synchronous standby, the
  failover quorum that promotes only a standby known to hold every acknowledged commit, and the primary isolation check;
  continuous WAL archiving and a daily base backup to an object store; the operator's version (one that has these
  settings) and every image pinned by digest; and a chart example (`infra/helm/examples/ha-cloudnativepg.yaml`) that
  connects to the cluster's read-write service.
- **Requirement 048-002 Development reference.** `infra/docker/compose.ha.yaml` MUST start three PostgreSQL nodes under
  Patroni 4 in quorum synchronous mode with the strict setting (refuse to commit without a standby), an etcd of three
  members, and an HAProxy that sends the platform's one address to the current primary (Patroni's `GET /primary`) and
  ends a demoted node's sessions (`on-marked-down shutdown-sessions`); images pinned by digest, ports on the loopback,
  a healthcheck per service (`AGENTS.md`, rule 16).
- **Requirement 048-003 One address.** The platform MUST connect through one address that the HA manager moves to the
  primary. No platform process may need a person, a restart or new configuration after a promotion.
- **Requirement 048-004 Spread that insists.** The chart's example MUST spread the engine and the control plane over
  zones with `whenUnsatisfiable: DoNotSchedule` and `nodeTaintsPolicy: Honor` (so a lost zone's tainted nodes do not
  block placement), keep a disruption budget per component, and keep `ScheduleAnyway` as the chart's default for
  clusters with fewer than three zones.

### The platform's side of a failover

- **Requirement 048-010 Dead connections are found.** Every statement MUST have a client-side bound of its
  transaction's effective `statement_timeout` plus 2 s (none when the timeout is 0, as for migrations), enforced by a task
  that owns the connection and closes it hard on expiry (the pattern of `begin_safely` in
  `crates/infra-postgres/src/pool.rs`, so that dropping a future never leaves a connection mid-statement); TCP keepalive
  on every connection; a new connection times out after the pool's `acquire_timeout` (3 s); connections have a maximum
  lifetime (30 minutes by default).
- **Requirement 048-011 No connection to a read-only database is used for writing.** A new connection of a writer pool
  MUST be refused when `pg_is_in_recovery()` is true (the optional read pool of Requirement 048-021 and a passive
  process of feature 032 are exempt). SQLSTATE `25006` MUST be classified as unavailable and retryable and its connection
  closed, never returned to the pool; `57P04` (database dropped) is not retryable. While the database is unreachable or
  read-only, a process MUST stay ready and answer typed `503 UNAVAILABLE` with `Retry-After`, as Architecture decision
  0082 decision 5 keeps processes serving through a connection failure; liveness never depends on the database.
- **Requirement 048-012 No commit is visible before a standby has it.** With synchronous standbys configured, the
  database MUST NOT cancel a commit's wait for a standby: the last statement of every transaction disables the timeout
  for the commit (`set_config('statement_timeout', '0', true)` folded into it; a path that cannot fold it adds one
  statement, recorded in design 038's budget), the platform's roles set `client_connection_check_interval` to 0, and the
  commit is bounded on the client only. A commit that passes its client bound MUST be reported as code `UNAVAILABLE`
  with reason `COMMIT_OUTCOME_UNKNOWN` (retryable by the caller's idempotency key), from a new `OutcomeUnknown` error of
  the store, distinct from "nothing was applied". The runbook MUST forbid cancelling or terminating a backend that waits
  for a standby, and a metric MUST count such backends.
- **Requirement 048-013 A new connection waits for the standby before it is used.** A connection MUST, before its first
  use, commit a write that waits for the standby (one row per backend of `durability_beacons`, updated in place), so that
  everything committed before it, including what a primary restarted in place marked committed after a crash, is on a
  standby before this connection reads it. This is the in-region form of Architecture decision 0067 decision 6; feature
  032 adds the cross-region quorum and its durability lease on this mechanism.
- **Requirement 048-014 A database outage does not count against worker leases.** The continuity row
  (`database_continuity`) MUST record, at every keeper round, the database time, the server's start time
  (`pg_postmaster_start_time()`) and its timeline (from the current WAL file name). In the transaction that a new
  connection commits under Requirement 048-013, the platform MUST compare them; if the start time or the timeline changed
  and the gap since the last recorded round is between 6 s (two keeper rounds) and 300 s, it MUST, under `FOR UPDATE` on
  the continuity row and by compare-and-set on the recorded values (so that exactly one connection does it), extend every
  worker lease and every attempt deadline whose expiry is at or after the last recorded round by the gap, with a floor of
  10 s from now, then record the new values and one audit record. Shard leases are not extended (their takeover is fenced
  by generation). A gap above 300 s, or a change of the region epoch (feature 032), extends nothing. Because every
  connection passes this transaction before its first use, no report, claim or pass runs on the new primary before the
  extension.
- **Requirement 048-015 Reports survive a failover.** The six SDKs MUST keep retrying a `CompleteStep` or `FailStep` that
  failed as unavailable (a refused connection and an ingress `502` or `503` included) for a report retry window of 310 s
  by default (configurable), with the existing backoff capped at 5 s; a heartbeat is not retried, the next one replaces
  it. The window covers the release gate of Requirement 048-040 plus one backoff, 300 + 5 = 305 s, rounded up. The
  conformance scenario `report-through-failover` covers it.
- **Requirement 048-016 Work in flight is not lost, repeated or sent to review by a failover.** Under the reference
  topology, a failover within the release gate MUST lose no acknowledged work, MUST NOT run again an activity that
  completed during it, and MUST NOT create a review item by itself. A claim whose commit outcome is unknown leaves at
  most an undelivered lease, which the existing withdrawal of undelivered leases handles without a review item.

### What a standby may serve

- **Requirement 048-020 Standbys decide nothing.** No claim, lease, transition, replay, receipt, quota read,
  authorization read (membership, bindings, roles) or secret read may be served by a standby.
- **Requirement 048-021 A closed list of stale-tolerant reads.** A process MAY have a second, optional address
  (`SAGAT_DATABASE_READ_URL`, the cluster's read-only service, in-region in a feature 032 cell) for exactly two reads:
  the journal pages of a terminal run, and run listing and search. Each is authorized on the primary first; there it
  also reads `pg_current_wal_lsn()`, and the standby serves only if its `pg_last_wal_replay_lsn()` is at or past it,
  else the primary serves. A response served by a standby carries `Sagat-Read-Staleness-Lsn`. The pool for that address
  is a type that only the module of these two reads can build, which `scripts/check-architecture.py` checks. These reads
  gain no availability during a failover (their authorization needs the primary), only load relief.

### Rolling upgrades

- **Requirement 048-030 Expand and contract.** Every new migration MUST declare its phase on the line after an optional
  `-- no-transaction`: `-- phase: expand` or `-- phase: contract`, and a contract names its expand (`-- contract-of:
  <version>`). An expand migration MUST leave the schema usable by release N-1; a contract migration removes only what
  N-1 no longer uses and ships in a release after its expand. Applied migrations are never edited (sqlx checks their
  checksums); the lint treats every migration up to a recorded cut-off as of its release.
- **Requirement 048-031 Migrations do not block the hot path.** A new migration MUST set a `lock_timeout` of 2 s on every
  statement that takes a lock that blocks inserts, and `migrate` MUST retry such a statement with backoff; it MUST build
  an index on an existing table concurrently, preceded by `DROP INDEX CONCURRENTLY IF EXISTS` (a build stopped by a
  timeout leaves an invalid index), with no `lock_timeout` on that statement; it MUST add a constraint `NOT VALID` and
  validate it in a later migration; it MUST NOT rewrite a table, change a column's type in place, rename, add a column
  with a volatile default or a `NOT NULL` column without a default, run `SET NOT NULL` on an existing table, or replace
  a function, trigger, policy or view in an expand migration (a changed function gets a new name). A rule is waived only
  by a header line `-- lint-allow: <rule> <reason>`, which the lint prints for the reviewer.
- **Requirement 048-032 The lint.** `scripts/check-migrations.py` MUST refuse a new migration that breaks Requirement
  048-030 or 048-031, with a negative fixture per rule under `scripts/tests/`, and a test that the checksums of applied
  migrations are unchanged.
- **Requirement 048-033 A binary refuses a schema it cannot use.** The last migration of each release MUST record in
  `schema_compatibility` the oldest release whose binaries may run on the schema; release numbers and the version of
  each release's last migration come from a manifest made when the release is cut (`crates/infra-postgres/releases.toml`).
  At start and at every instance heartbeat, a process MUST read, through an owner-defined function the runtime role may
  call, the applied versions and that row, and close its serving gate with `SCHEMA_TOO_OLD` (a migration it was built
  with is missing) or `BINARY_TOO_OLD` (its release is older than the oldest compatible one), writing nothing. The
  handshake tests run as the runtime role.
- **Requirement 048-034 New behaviour waits for the fleet.** A change that writes state that release N-1 cannot process
  MUST be behind a registry capability, switched on by the registry's activation (which refuses while a live process
  lacks it, and refuses such a process afterwards), and the activation raises the oldest compatible release in the same
  transaction. `migrate` MUST refuse to apply a contract migration while the census shows a process older than the
  release before it.
- **Requirement 048-035 Old and new writers run together.** A test MUST run engines and control planes of releases N-1
  and N on one database under the chain shape: apply N's migrations during the load, roll the processes one at a time,
  activate N's capabilities, roll back to N-1 before an activation, and pass the integrity oracle with the transition
  latency never above the lock timeout. Release N MUST NOT be published unless it passes against N-1. For release 0.3.0
  the test is built and runs against an earlier 0.3.0 commit with the handshake; it first gates a release when 0.3.0 is
  N-1.

### Evidence

- **Requirement 048-040 The failover budget and the release gate.** For the loss of one database instance or one zone,
  the time from the fault to the first transition committed after it is reported at the median and the maximum of the
  drill's repetitions, and so is the time until every shard is held again. The release gate is the owner's 300 s at the
  maximum. The median's target, a benchmark criterion, is 60 s: what the platform adds to the HA manager's promotion P is
  a dead connection found within 5 + 2 = 7 s, a new connection within 3 s and its beacon commit, and a shard held within
  one keeper round, 3 s, so the first transition follows P by about 7 + 3 + 3 = 13 s; a dead engine's shards wait for the
  rest of their 15 s lease plus a round, at most P + 7 + 15 + 3 = P + 25 s; the target leaves P about 35 s.
- **Requirement 048-041 A three-zone drill on Kubernetes.** A drill MUST run the reference topology and the platform
  (three engines, three control planes, a worker pool) on six nodes in three labelled zones, under the chain shape at a
  fixed open-loop rate, and inject, five times each: `kill -9` of the primary's PostgreSQL; deletion of the primary's
  pod; a drain of the primary's node; the loss of a zone (its nodes stopped); the isolation of the primary's node from
  the network; a standby made to lag and then the primary killed; and a rolling upgrade of the platform from N-1 to N.
- **Requirement 048-042 Measured, not estimated.** For each repetition the drill MUST record: the time to a writable
  primary; the times of Requirement 048-040; the time until the completion rate is back to 90% of its rate before the
  fault, held for three 10 s windows; acknowledged work lost; activities run more than once; review items created; and,
  after the old primary rejoins, the commits on its timeline after the fork position (read from its WAL), compared with
  the driver's ledger of acknowledgments, so that a commit accepted by two primaries is found. A dated report,
  `tests/performance/reports/high-availability.md`, states the environment and every failure.
- **Requirement 048-043 The drill gates the release.** A release MUST fail when any repetition loses acknowledged work,
  finds a commit after the fork that the driver saw acknowledged, or exceeds 300 s to the first transition.

### Success criteria

- **Success criterion 048-001** In every repetition of every fault, acknowledged work lost is 0 and no acknowledged commit
  is found after a fork on a former primary.
- **Success criterion 048-002** The median time to the first committed transition is reported for each fault; for the
  `kill -9`, the pod deletion and the node drain it is within 60 s.
- **Success criterion 048-003** In the rolling upgrade from N-1 to N, the completion rate never falls to zero for more
  than 10 s and the integrity oracle passes.
- **Success criterion 048-004** A failover of the Compose reference during the chain shape creates no review item and
  repeats no activity that had completed.
- **Success criterion 048-005** The migration lint refuses each of its negative fixtures and passes every migration
  merged after its cut-off.

### User stories

- **User story 048-01** An operator installs the reference topology and loses a zone without paging anyone for the
  database.
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
- **High availability of the object store, the identity provider and OpenBao.** The object store is the provider's (gap
  251 of feature 010 stays); OpenBao's three-voter Raft is feature 017's.
- **The 0.2.0 to 0.3.0 upgrade.** It stays a stopped upgrade: 0.2.0 binaries have no handshake.
- **An outage of the address or the network alone** (the database never restarted or promoted) does not extend leases;
  activities cut off longer than their lease's remainder follow feature 037's rules. The report states it.

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6, with its reasoning.

1. **CloudNativePG or Patroni on Kubernetes?** CloudNativePG: it moves the read-write service itself at promotion, needs
   no external consensus store, and has the synchronous quorum, the failover quorum and backups to an object store.
   Patroni stays the reference outside Kubernetes. Delegate.
2. **Refuse to commit without a standby, or fall back to asynchronous?** Refuse. With two standbys in two other zones,
   the loss of one zone still leaves a synchronous standby; writes stop only when two of three zones are lost. An operator
   may choose the preferred mode and its stated RPO. Delegate.
3. **Should standbys serve reads in 0.3.0?** Only the two reads of Requirement 048-021, behind an optional address, off
   by default. Delegate.
4. **How is a commit kept from becoming visible before a standby has it?** By never letting the database cancel its wait
   (feature 032's decision D1566 applied in one region), and by a waited beacon commit on every new connection. Reading
   the server's warning after a cancelled wait was the first draft's choice; the driver cannot surface it. Delegate.
5. **Extend leases after an outage?** Yes, only after a database restart or promotion, only by the gap, at most 300 s,
   with a floor. Without it, every activity in flight during a failover longer than its lease's remainder becomes a retry
   or a review item. Delegate.
6. **The decision records.** The topology, the commit rule, the leases and the reads are
   [Architecture decision 0130](../adr/ADR-0130-postgresql-high-availability-is-a-reference-topology-with-synchronous-quorum-and-one-moving-address.md);
   the upgrade rule is
   [Architecture decision 0131](../adr/ADR-0131-migrations-expand-before-and-contract-after-a-release-and-a-binary-refuses-a-schema-it-cannot-use.md).
   Architecture decision 0130 refines 0020 (decision 2: a lease that ran out is extended only by Requirement 048-014),
   and 0131 refines 0082 (the gate closes on a schema mismatch too).

## Owner decisions needed

- **The Kubernetes drill's runner.** Six kind nodes with the platform need a 16-vCPU runner, US$ 0.042 per minute on
  2026-11-24 (`docs.github.com/en/billing/reference/actions-runner-pricing`); a drill of 7 faults × 5 repetitions × 6
  minutes = 210 minutes plus 30 of preparation is 240 × 0.042 = US$ 10.08 per run. Money stays the owner's (Owner
  decision 2026-11-24-4, pull request 274). Gates Task 048-16.

## Critical User Journeys (CUJs)

### CUJ 1: A zone is lost

*   **Given**: the reference topology on Kubernetes, the primary in zone a, runs in flight.
*   **When**: zone a disappears.
*   **Then**: CloudNativePG promotes the standby that holds every acknowledged commit and moves the read-write service;
    the engines' dead connections fail within 7 s; each new connection commits its beacon and, the first of them, the
    lease extension; workers' reports are retried until they land; no acknowledged run is lost or activity repeated.

### CUJ 2: Both standbys are gone

*   **Given**: a primary whose two standbys are unreachable.
*   **When**: a worker completes a step.
*   **Then**: the commit waits for a standby and is never cancelled by the database, so nothing becomes visible; the
    client bound reports `COMMIT_OUTCOME_UNKNOWN`; the worker retries with its idempotency key; when a standby returns,
    the waiting commit finishes and the retry finds it.

### CUJ 3: A rolling upgrade

*   **Given**: release N-1 running.
*   **When**: the operator runs N's migrations and rolls the engines and control planes to N.
*   **Then**: N-1 and N processes run together; N writes no new state until the activation, which waits for the census;
    the contract migrations of N's expands ship with N+1, and `migrate` refuses them while an N-1 process lives.

### CUJ 4: A binary on the wrong schema

*   **Given**: release N deployed before its migrations were applied.
*   **When**: it starts.
*   **Then**: it closes its serving gate with `SCHEMA_TOO_OLD`, answers 503, writes nothing, and opens the gate once the
    migrations are applied.

## Technical Plan (Architecture)

### Topology

```
            zone a                    zone b                    zone c
   +------------------+      +------------------+      +------------------+
   | PostgreSQL       |      | PostgreSQL       |      | PostgreSQL       |
   | primary          |----->| standby          |      | standby          |
   |                  |------------------------------->|                  |
   +------------------+      +------------------+      +------------------+
            ^  read-write service (moved by the HA manager at promotion)
            |  a commit returns when either standby has flushed it
   engine x1, control plane x1   engine x1, control plane x1   engine x1, control plane x1
```

**Kubernetes** (`infra/kubernetes/ha/`): the CloudNativePG operator from its published chart, pinned; a `Cluster` with
three instances, synchronous quorum one with required durability, failover quorum, the isolation check, the platform's
server settings, `transaction_timeout` 60 s for the platform's roles (PostgreSQL 17 and later), anti-affinity across
zones, the Barman Cloud plugin for WAL archiving and a `ScheduledBackup`. The runtime role `sagat_app` and the owner role
come from the cluster's bootstrap with the platform's `postgres-init` scripts; the HA manager's own roles are separate and
never given to the platform; credentials are Kubernetes Secrets the cluster creates, never in the repository.

**Compose** (`infra/docker/compose.ha.yaml`): `etcd-1..3`, `postgres-1..3` from an image built in `infra/docker/patroni/`
(the PostgreSQL image of `compose.yaml` by digest plus a pinned Patroni 4), and `haproxy`. Development credentials only,
as in `compose.yaml`.

### The platform's side

| Change | Where | Requirement |
|---|---|---|
| Client bound per statement from the transaction's timeout, closing hard; keepalive; maximum lifetime | `crates/infra-postgres/src/pool.rs` | 048-010 |
| Refuse a writer connection in recovery; `25006` unavailable and its connection closed | `pool.rs` (`after_connect`), `identity.rs` (the classifier) | 048-011 |
| Ready with a typed 503 while the database is unreachable | `crates/api-http/src/routes/health.rs`, the engine's `/readyz`, the request middleware | 048-011 |
| The commit's timeout disabled in the last statement; `OutcomeUnknown`; `client_connection_check_interval` | the store's commit path; `crates/application/src/error.rs`; the topology's role settings | 048-012 |
| The beacon commit and the continuity check in `after_connect` | `pool.rs`, a migration (`durability_beacons`, `database_continuity`), `crates/scheduler` (the keeper's round writes the row) | 048-013, 048-014 |
| The report retry window | `sdks/SPEC.md` and the six SDKs; the conformance scenario | 048-015 |
| The read pool type and the two reads | `apps/*/config.rs`, `crates/infra-postgres` | 048-021 |

**Why one waited commit protects what came before it.** A standby receives WAL in order; a commit returns only after a
standby has flushed WAL up to that commit's position, which lies after every earlier commit's record. So once a
connection's beacon commit returns, every transaction committed before it, including one a crashed primary marked
committed on restart, is on a standby. A transaction must write a row for its commit to wait: one that only took a
transaction identifier writes no WAL and does not wait, which is why the beacon updates a row.

**The continuity row and the extension.** The keeper's round updates `database_continuity` (database time, server start
time, timeline). The `after_connect` transaction reads it `FOR UPDATE` and compares; the first connection after a
restart or a promotion finds a difference, extends in batches of 10,000 rows, records the new values and the audit
record, and commits (its commit is the beacon's wait, so it is replicated); the others wait on the row lock, then find
nothing to do. Every value is the new primary's clock; the gap is measured from the last recorded round on that clock,
because the row was replicated with the old primary's value.

### Rolling upgrades

**The order of an upgrade**, written in `docs/runbooks/scaling.md` section 3 in place of the stopped upgrade, and in
`docs/runbooks/control-plane.md` and `execution-engine.md` so that they agree:

1. Apply N's migrations (`control-plane migrate`, all of them) while N-1 runs; N's contract migrations are those of
   N-1's expands, safe because no N-2 process runs, which `migrate` checks.
2. Roll the control planes and the engines to N, one at a time (`maxUnavailable: 0`, `maxSurge: 1`).
3. Activate N's capabilities (the registry refuses while an N-1 process lives).

Rollback before step 3 rolls the binaries back onto N's schema; after it, the capabilities are switched off and the runs
that used them drained first. The runbook forbids running N-1's migration job during a rollback.

**The mixed-writer test.** `scripts/test-mixed-writers.py` builds N-1 from its tag (as
`scripts/test-release-upgrade-profile.py` builds 0.2.0), starts two N-1 engines, one control plane and a worker pool
under the chain shape with design 051's driver, applies N's migrations during the load, rolls each process to N, rolls
one back, activates, and runs the integrity oracle, asserting that the transition latency never exceeded the lock
timeout. The release workflow runs it against the previous release.

### The drills

**Kubernetes** (`tests/e2e/high-availability/`): a kind cluster of six workers labelled with three zones, the
CloudNativePG cluster of `infra/kubernetes/ha/`, the platform's chart with the HA example, and design 051's driver at a
fixed rate. Faults by command: `kill -9` inside the primary's container; `kubectl delete pod`; `kubectl drain`; `docker
stop` of both nodes of a zone; `docker network disconnect` of the primary's node; a standby throttled then the primary
killed. A full drill is 7 faults × 5 repetitions × 6 minutes (2 before, 4 after) = 210 minutes.

**Compose** (`infra/dr/ha-drill.sh`): the faults that make sense on one machine (the primary's container stopped, a
node stopped, both standbys stopped) on `compose.ha.yaml`, in the `scale` job of CI and in the release workflow,
replacing the constant 15 s of today's drill with a measurement.

**Metrics and alerts.** `database_connections_refused_total{reason}`, `database_commit_outcome_unknown_total`,
`database_backends_waiting_for_standby`, `lease_extensions_total`, `schema_gate_closed{reason}`; from the HA manager's
exporter, the number of synchronous standbys, with the alerts `SagatDatabaseOneSyncStandby` (warning, one left) and
`SagatDatabaseNoSyncStandby` (critical, writes stopped), with their rule tests in CI's `promtool` list. Labels are closed
sets only.

**Order of the build.** This design's Task 048-2 (the Compose topology) comes first; design 051's driver (its Task 051-2)
before Tasks 048-15 to 048-17, which reuse it.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A commit waits for ever when both standbys are gone | By design (RPO 0); the client bound reports an unknown outcome; the alert fires when one standby is left; an operator may choose the preferred mode with its stated RPO |
| An operator cancels a waiting backend and exposes an unreplicated commit | The runbook forbids it; the metric of waiting backends; the residual is stated |
| The lease extension keeps a dead worker's lease alive longer | By the gap only, at most 300 s, once per restart or promotion, with a floor of 10 s |
| Old and new writers disagree on new state | Requirement 048-034; the mixed-writer test on every release |
| The migration lint blocks a needed change | A waiver header that names the rule and the reason, printed for the reviewer |
| CloudNativePG or Patroni has a bug | Versions pinned; the drill on every release; the platform's own oracle is the judge |
| The kind drill is not a real cloud | Stated in the report; the same drill runs on a cloud cluster when the owner provides one |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No commit is visible before a standby has it (048-012, 048-013); the lease extension changes no token or generation, is a database write under a compare-and-set, and is audited; one transaction per transition is unchanged |
| II. Modular domain, ports and adapters | Pass | Changes are in `infra-postgres`, the scheduler, the application's error type and configuration; the domain is untouched |
| III. External effects are at-least-once; sagas compensate | Pass | The failover rules reduce repeats and review items but promise none; idempotency keys still absorb repeats |
| IV. Compatibility is versioned and pinned | Pass | Expand and contract, the compatibility table and the handshake; images and operators pinned by digest; the new error reason and the response header added to the contracts |
| V. Code, tenant boundary and secrets are isolated | Pass | Authorization and secret reads stay on the primary; the HA manager's roles are never the platform's; no credential in the repository |
| VI. Observability and audit are part of the feature | Pass | The metrics and alerts above with closed labels; an audit record per extension |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | A duplicate answered after a failover reads only what a standby holds (048-013) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Seven faults in three zones, five times each; both standbys stopped; a server that stops answering; the mixed-writer test |
| Delivery Constraints | Pass | "Single region with high availability" is what this design builds |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Two reference topologies | CloudNativePG runs only on Kubernetes; developers and design 051 need one on Compose | One topology: no failover on a laptop, or a Kubernetes-only drill |
| The commit's timeout disabled, and the beacon on every connection | Without them a commit becomes visible before a standby has it | Trusting `synchronous_commit = on`: PostgreSQL returns success after a cancelled wait |
| The lease extension | A failover longer than a lease's remainder creates retries and review items | Longer leases for everyone: slower recovery from a real worker crash |
| The compatibility table and the gate | A binary on the wrong schema writes state nobody can read | Documentation only: the runbooks contradict each other today |
| The migration lint | The rules are easy to forget and costly when broken | Review only |

## Alternatives Considered

- **Leave HA to the operator** (today). Every operator rebuilds the same topology and the platform's side stays untested.
- **Several hosts in the URL with `target_session_attrs=read-write`.** The driver does not support it, and each process
  would choose a primary.
- **Asynchronous standbys only.** Faster commits, RPO above zero, against the owner's objective.
- **Read the warning of a cancelled wait.** The driver does not surface notices per connection; and the transaction is
  already visible by the time the warning is read.
- **A waited write on every replay path** (the first draft). The beacon on each new connection covers what a replay
  could read, at one write per connection instead of per replay.
- **Blue and green upgrades with two databases.** Logical replication of the whole schema, with its triggers and definer
  functions firing again on apply.

## Non-Functional Requirements

- **Non-functional requirement 048-001 Hot path.** A lease and a completion gain no statement where the commit's timeout
  is folded into the last statement; a path that cannot fold it gains one, recorded in design 038's budget. The beacon
  and the continuity check run once per new connection, not per transaction.
- **Non-functional requirement 048-002 Recovery.** As Requirement 048-040.
- **Non-functional requirement 048-003 Security.** The read address uses the runtime role; the HA manager's roles are
  separate; the Compose reference is for development only.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T048-1 Write Architecture decisions 0130 and 0131, their refinement notes in Architecture decisions 0020 and 0082, and the topology and the two stale-tolerant reads in `docs/architecture/storage-and-events.md` and `docs/runbooks/disaster-recovery.md`.
  Refs: Requirement 048-001, Requirement 048-021, Requirement 048-030
- [ ] T048-2 Add `infra/docker/compose.ha.yaml` and `infra/docker/patroni/` with pinned images, loopback ports and healthchecks.
  Refs: Requirement 048-002, Requirement 048-003
- [ ] T048-3 Add `infra/kubernetes/ha/` (operator values, `Cluster`, backups) and `infra/helm/examples/ha-cloudnativepg.yaml` with spread that insists and disruption budgets, and a check that every image of both is pinned by digest.
  Refs: Requirement 048-001, Requirement 048-004
- [ ] T048-4 Bound every statement on the client from its transaction's timeout with a hard close, add keepalive and a maximum lifetime, and test a server that stops answering mid-statement.
  Refs: Requirement 048-010
- [ ] T048-5 Refuse writer connections in recovery, classify `25006` and `57P04`, close a read-only connection, keep readiness with a typed 503, and test a promoted standby with the old primary still reachable read-only.
  Refs: Requirement 048-011
- [ ] T048-6 Disable the commit's timeout in the last statement of every transaction, add `OutcomeUnknown` and the `COMMIT_OUTCOME_UNKNOWN` reason to the error contracts, and test, with both standbys stopped, that no reader sees the commit and the caller gets the unknown outcome.
  Refs: Requirement 048-012, Non-functional requirement 048-001
- [ ] T048-7 Add `durability_beacons` and the waited beacon commit before a connection's first use, with a test of a primary restarted in place.
  Refs: Requirement 048-013
- [ ] T048-8 Add `database_continuity`, the keeper's write, and the extension in the `after_connect` transaction, with fault tests of a promotion and of a restart in place longer than a lease's remainder, an attempt deadline inside the gap, and two connections racing to extend.
  Refs: Requirement 048-014, Requirement 048-016
- [ ] T048-9 Add the report retry window to `sdks/SPEC.md`, the six SDKs and the conformance scenario `report-through-failover`.
  Refs: Requirement 048-015
- [ ] T048-10 Add the optional read address, its pool type, the two reads with the position check and the header, and the architecture rule.
  Refs: Requirement 048-020, Requirement 048-021
- [ ] T048-11 Add the metrics and the two alerts with their rule tests.
  Refs: Requirement 048-012, Requirement 048-014
- [ ] T048-12 Write `scripts/check-migrations.py` with its cut-off, its waiver header, a negative fixture per rule and the checksum test, and run it in CI.
  Refs: Requirement 048-030, Requirement 048-031, Requirement 048-032, Success criterion 048-005
- [ ] T048-13 Add the release manifest, `schema_compatibility`, the owner-defined function and the handshake on the registry's gate, tested as the runtime role.
  Refs: Requirement 048-033
- [ ] T048-14 Put new write behaviour behind registry capabilities with the oldest compatible release raised at activation, make `migrate` retry and refuse a contract early, and write the upgrade order in the three runbooks.
  Refs: Requirement 048-031, Requirement 048-034
- [ ] T048-15 Write `scripts/test-mixed-writers.py` and run it in the release workflow against the previous release.
  Refs: Requirement 048-035, Success criterion 048-003
- [ ] T048-16 Write the Kubernetes drill in `tests/e2e/high-availability/` with the fork comparison, run it, and publish `tests/performance/reports/high-availability.md`.
  Refs: Requirement 048-041, Requirement 048-042, Requirement 048-043, Success criterion 048-001, Success criterion 048-002
  Owner-gate: pending, the owner's row approving the drill's runner of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T048-17 Write `infra/dr/ha-drill.sh` on the Compose reference, measure the takeover, and run it in the `scale` job and the release workflow.
  Refs: Requirement 048-040, Requirement 048-042, Requirement 048-043, Success criterion 048-004

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs the suites of what it touches, `integration_claim_transition_efficiency` when it touches
`crates/infra-postgres/`, the fault suite of feature 010 (`tests/fault/failover`), `cargo clippy --workspace
--all-targets --locked -- -D warnings`, and the documentation and architecture checkers of `scripts/`. Tasks 048-4 to
048-8 each add a fault test seen failing first (named in each task). Task 048-9 runs the SDK conformance scenario in all
six languages. Task 048-15 is the release gate for upgrades; Tasks 048-16 and 048-17 are the gate for recovery, both
stated with their environment.
