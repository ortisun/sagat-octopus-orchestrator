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
   used with the platform's chart in `infra/helm`) and Patroni on Docker Compose for development
   (`infra/docker/compose.ha.yaml`). Both keep a primary and two standbys in three zones with synchronous replication,
   promote a standby without a person, and move the one address the platform uses to the new primary.
2. **The platform's side of a failover**: it finds out quickly that its connections are dead, never keeps a connection
   to a database that is read-only, never acknowledges work whose commit the standbys did not receive, and does not let
   an outage count against the leases of workers.
3. **What a standby may serve**: nothing on the path that decides or writes; only a closed list of reads that tolerate
   staleness, each with a bound, through an optional second address.
4. **Rolling upgrades with old and new writers together**: migrations split into an expand phase that release N-1 still
   runs on and a contract phase that waits for release N everywhere; a handshake by which a binary refuses a schema it
   cannot use; a lint of every new migration; and a test that runs N-1 and N engines on the same database under load.
5. **Evidence**: a three-zone disruption and failover drill on Kubernetes and a drill on Compose, whose recovery times
   are measured, published, and checked against the owner's objectives (RPO 0 for acknowledged work, failover within
   5 minutes, Owner decision 2026-10-16, row 8).

Cross-region failover is feature 032; this design is the in-region base it builds on.

## The Problem

From the code and documents on `develop` at `546d5aa5`:

- **No topology.** Every environment the repository defines runs one PostgreSQL container
  (`infra/docker/compose.yaml`, `postgres:16-alpine`). The chart does not run PostgreSQL ("PostgreSQL, the selected
  NATS/Redis service and the identity provider are yours", `docs/runbooks/scaling.md`, section 4). The Kubernetes
  validation runs a single PostgreSQL Deployment (`tests/e2e/scalability-ha/kubernetes/dependencies.yaml`).
- **Failover is a manual procedure.** `docs/runbooks/disaster-recovery.md` says to fence the old primary, run
  `pg_ctl promote`, then point the platform at the new one through "a service name, a virtual IP, a pooler, a DNS record".
  The drill (`infra/dr/drill.sh`) promotes a standby on loopback in 0.2 s but never points the platform at it, and adds
  a 15 s shard takeover as a constant, not a measurement (`tests/performance/reports/disaster-recovery.md`). Gaps 250
  and 251 of feature 010 record that no standby in another zone, no reconnection through a virtual address, and no
  object store were covered.
- **The platform is not ready for a failover.**
  - One `DATABASE_URL` per process; nothing routes a query to a standby (gap 242 of feature 010).
  - SQLSTATE `25006` (`read_only_sql_transaction`) is classified as an internal error, not as unavailable
    (`crates/infra-postgres/src/identity.rs`), so a writer connected to a demoted primary logs errors and never sees
    itself as unavailable. Nothing calls `pg_is_in_recovery()`; readiness checks only that the database answers.
  - The pool sets no maximum lifetime and no client-side bound on a query whose server vanished without closing the
    connection (`crates/infra-postgres/src/pool.rs`): the server's `statement_timeout` of 5 s does not help when the
    server is gone.
  - With `synchronous_commit = on`, PostgreSQL returns success to a `COMMIT` whose wait for the standby was cancelled
    (by `statement_timeout`, a cancel or a terminated connection), with only a warning that the transaction "has already
    committed locally, but might not have been replicated". The platform reads no notice, so such a commit is
    acknowledged although a failover would lose it. A retry of the same command then finds the row and answers
    `already_applied`, which acknowledges it again through a read (Architecture decision 0067, force 3, states the same
    for regions).
  - The SDKs retry a failed report 4 times from 200 ms up to 5 s (`sdks/SPEC.md`, "retry"), so a report sent during a
    failover of more than a few seconds is given up. Worker leases last 30 s on the database clock
    (`SAGAT_LEASE_TTL_SECS`); an outage longer than what is left of a lease makes the lease expire, and since feature
    037 an activity cut off that way waits for a person unless it has a `reconcileWith` lookup
    (`docs/runbooks/scaling.md`, "Draining workers before a restart").
- **Upgrades stop the world.** `docs/runbooks/scaling.md` (section 3), `docs/runbooks/security-governance.md` and the
  0.2.0 release notes say to stop every writer, migrate, then deploy: "mixed old/new writers are unsupported". Older
  text says the opposite (`docs/runbooks/control-plane.md` and `docs/runbooks/execution-engine.md`: migrate, then roll,
  "the previous binary keeps working against the new schema"). No binary checks the schema version it runs on. The
  released-upgrade tests prove that a 0.2.0 binary reads the new schema and that an old worker reports to a new server,
  not that old and new writers run together or that a new binary refuses an old schema
  (`tests/integration/.../release_upgrade*.rs`, `scripts/test-release-upgrade-profile.py`). Every migration runs in one
  transaction, none builds an index `CONCURRENTLY`, and nothing limits how long a migration may wait for a lock, so a
  migration can queue behind a long transaction and block every transition behind it.
- **The chart spreads but does not insist.** Topology spread of the engine and the control plane is
  `whenUnsatisfiable: ScheduleAnyway` (`infra/helm/templates/_helpers.tpl`), so all replicas can land in one zone.
  Node drain and zone loss have never been tested (gap 247 of feature 010).

## Requirements

### The reference topology

- **Requirement 048-001 Kubernetes reference.** `infra/kubernetes/ha/` MUST hold a CloudNativePG `Cluster` of three
  instances, one per zone (required pod anti-affinity on `topology.kubernetes.io/zone`), with synchronous replication of
  quorum one (`method: any`, `number: 1`) and the data-durability mode that refuses to commit when no synchronous
  standby is reachable; continuous WAL archiving and a daily base backup to an object store; the operator's version and
  every image pinned by digest; and an example of the platform's chart values (`infra/helm/examples/ha-cloudnativepg.yaml`)
  that connects to the cluster's read-write service.
- **Requirement 048-002 Development reference.** `infra/docker/compose.ha.yaml` MUST start three PostgreSQL nodes under
  Patroni with an etcd of three members and an HAProxy that sends the platform's one address to the current primary
  (Patroni's REST health check), with the same synchronous rule (quorum one, refuse to commit without a standby), images
  pinned by digest, ports on the loopback only and a healthcheck per service (`AGENTS.md`, rule 16). It MUST be usable
  by `infra/dr/` and by design 051's crash fault.
- **Requirement 048-003 One address.** The platform MUST connect to the database through one address that the HA
  manager moves to the primary (the read-write service in Kubernetes, the HAProxy in Compose). No platform process may
  need a person, a restart or a new configuration after a promotion.
- **Requirement 048-004 Spread that insists.** The chart's example for the reference topology MUST spread the engine
  and the control plane over zones with `whenUnsatisfiable: DoNotSchedule` and keep at least one replica of each per
  surviving zone after the loss of one zone; the chart MUST keep `ScheduleAnyway` as the default for clusters with fewer
  than three zones.

### The platform's side of a failover

- **Requirement 048-010 Dead connections are found.** Every database connection MUST have a client-side bound on how
  long a statement may wait for an answer: the server's `statement_timeout` plus 2 s, after which the connection is
  closed and the operation fails as unavailable. New connections MUST time out after the pool's `acquire_timeout`
  (3 s). Connections MUST have a maximum lifetime (30 minutes by default), so that a pool does not keep connections to
  one node for ever.
- **Requirement 048-011 No connection to a read-only database is used.** A new connection MUST be refused when
  `pg_is_in_recovery()` is true. SQLSTATE `25006` and the class `57P0x` (administrator or crash shutdown) MUST be
  classified as unavailable and retryable, and the connection that returned `25006` MUST be closed, never put back in
  the pool. A process's readiness MUST be false while its last check of the database found it read-only or unreachable;
  liveness MUST NOT depend on the database.
- **Requirement 048-012 No acknowledgment without the standby.** When the database is configured with synchronous
  standbys, the platform MUST NOT acknowledge an operation on the strength of a `COMMIT` that returned after its wait
  for synchronous replication was cancelled or terminated. The adapter MUST recognise that warning and fail the
  operation as unknown and retryable. An answer that acknowledges earlier work without writing (an idempotent replay
  from a receipt, an inbox or connector duplicate, a worker's `already_applied`) MUST be given only after a commit of its
  own transaction that waited for the standby, so that what it acknowledges is known to be on a standby. This is the
  in-region form of Architecture decision 0067, decision 6; feature 032 extends it across regions.
- **Requirement 048-013 An outage does not count against leases.** The engine MUST record, every keeper round, the
  database time of its last successful round in a continuity row. After a failover, the first lost-lease pass MUST
  extend every worker lease and shard lease whose expiry fell inside the gap between the last recorded round and the
  first round after the failover, by the length of that gap, before it reclaims any lease. An extension never shortens
  a lease, never changes a token or a generation, and is audited once per failover with the gap's length.
- **Requirement 048-014 Reports survive a failover.** The six SDKs MUST keep retrying a `CompleteStep`, `FailStep` or
  `Heartbeat` that failed as unavailable for a report retry window of 120 s by default (configurable), with the existing
  backoff capped at 5 s, before giving the attempt up. The window must cover the failover budget of Requirement
  048-030 plus one backoff, 60 + 5 = 65 s; 120 s leaves a margin of 55 s for a slow promotion. The conformance suite
  MUST cover it.
- **Requirement 048-015 Work in flight is not lost or done twice by a failover.** Under the reference topology, a
  failover during load MUST lose no acknowledged work, MUST NOT make an activity that completed during the failover run
  again, and MUST NOT put an activity in review because of the failover alone, when the failover finishes within the
  failover budget of Requirement 048-030.

### What a standby may serve

- **Requirement 048-020 Standbys decide nothing.** No claim, lease, transition, idempotent replay, receipt, quota read,
  authorization read (membership, bindings, roles) or secret read may be served by a standby.
- **Requirement 048-021 A closed list of stale-tolerant reads.** A process MAY have a second, optional address
  (`SAGAT_DATABASE_READ_URL`, the cluster's read-only service) for a closed list of reads named in
  `docs/architecture/storage-and-events.md`: the journal pages of a terminal run, run listing and search, and the
  operational dashboards. Each such read MUST be authorized on the primary first, MUST refuse a standby whose replay is
  more than 5 s behind (`now() - pg_last_xact_replay_timestamp()`), and MUST fall back to the primary when it refuses.
  The response MUST say that it may be stale and by how much at most. No read outside the list may use the second
  address.

### Rolling upgrades

- **Requirement 048-030 The failover budget.** The reference topology's recovery-time target for the loss of one
  database instance or one zone, measured from the fault to the first transition committed after it, is 60 s at the
  median and the owner's 300 s at the worst of the drill's repetitions. The 300 s bound is the release gate; the 60 s
  median is a benchmark criterion that is reported, not a promise. What the platform adds to the HA manager's
  promotion is bounded by this design: a dead connection is found within 5 s + 2 s = 7 s (Requirement 048-010), a new
  connection within 3 s, and a shard is held again within one keeper round of 3 s, so 7 + 3 + 3 = 13 s; the target
  leaves 60 - 13 = 47 s for the HA manager to promote and move the address, which the drill measures separately.
- **Requirement 048-031 Expand and contract.** Every migration MUST be marked, in its first line, as `expand` or
  `contract`. An expand migration MUST leave the schema usable by the binaries of the release before it (N-1). A
  contract migration MUST remove only what no binary of release N-1 uses, and MUST ship in a release after the one whose
  expand migration made it possible.
- **Requirement 048-032 Migrations do not block the hot path.** A new migration MUST set a `lock_timeout` (2 s) and be
  safe to run again after a timeout; MUST build an index on an existing table `CONCURRENTLY` (in a migration of its own
  without a transaction); MUST add a constraint `NOT VALID` and validate it in a later migration; and MUST NOT rewrite a
  table, change a column type in place, rename, or add a column with a volatile default. A lint,
  `scripts/check-migrations.py`, MUST refuse a new migration that breaks one of these rules, with a negative fixture
  per rule.
- **Requirement 048-033 A binary refuses a schema it cannot use.** Each release's migrations MUST record in a table the
  oldest release whose binaries may run on the schema they leave. At start, and at every instance heartbeat, a process
  MUST compare: if its own release is older than that oldest release, or if a migration it was built with is not
  applied, it MUST close its serving gate (the instance registry of Architecture decision 0082) with a typed reason and
  write nothing.
- **Requirement 048-034 New behaviour waits for the fleet.** A change that writes state a binary of release N-1 cannot
  process MUST be behind a capability of the instance registry, off until the registry's census shows no live process
  of release N-1. Switching it on is the first step of the contract phase.
- **Requirement 048-035 Old and new writers run together.** A test MUST run engines and control planes of releases N-1
  and N on the same database (after N's expand migrations) under the chain shape of design 051, restart each in turn,
  and pass the integrity oracle of `tests/performance/scalability-ha/` (no lost acknowledged run, no duplicate lease,
  replay equals projection). Release N MUST NOT be published unless it passes against N-1.

### Evidence

- **Requirement 048-040 A three-zone drill on Kubernetes.** A drill MUST run the reference topology and the platform
  (three engines, three control planes, a worker pool) on a cluster of six nodes in three labelled zones, under the chain
  shape at a fixed open-loop rate, and inject, five times each: the `kill -9` of the primary's PostgreSQL process; the
  deletion of the primary's pod; a node drain of the primary's node; the loss of a whole zone (its nodes stopped); the
  isolation of the primary's node from the network; and a rolling upgrade of the platform from N-1 to N.
- **Requirement 048-041 Measured, not estimated.** For each repetition the drill MUST record: the time to a writable
  primary; the time to the first transition committed after the fault; the time until the completion rate is back to 95%
  of its rate before the fault; acknowledged work lost; activities run more than once; review items created; and
  whether two primaries accepted a commit at any moment. A dated report, `tests/performance/reports/high-availability.md`,
  MUST state the environment and every failure.
- **Requirement 048-042 The drill gates the release.** A release MUST fail when any repetition loses acknowledged work,
  shows two primaries accepting commits, or exceeds 300 s to the first transition, as the drill of feature 010 gates it
  today.

### Success criteria

- **Success criterion 048-001** In every repetition of every fault of Requirement 048-040, acknowledged work lost is 0
  and no moment has two primaries accepting commits.
- **Success criterion 048-002** The median time to the first committed transition is reported for each fault; for the
  `kill -9`, the pod deletion and the node drain it is within the 60 s of Requirement 048-030.
- **Success criterion 048-003** In the rolling upgrade from N-1 to N, the completion rate never falls to zero for more
  than 10 s and the integrity oracle passes.
- **Success criterion 048-004** A failover of the Compose reference during the chain shape creates no review item and
  repeats no activity that had completed (Requirement 048-015).
- **Success criterion 048-005** The migration lint refuses each of its negative fixtures and passes every migration
  merged after it.

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

- **Cross-region failover**, cells and the region epoch: feature 032 (Architecture decision 0067).
- **Building an HA manager.** The platform elects nothing (Architecture decision 0067, decision 3); CloudNativePG and
  Patroni decide promotion.
- **A connection pooler in the reference.** The platform sets session options at connection and uses prepared
  statements; a pooler in transaction mode would need both changed. The chart's connection budget stays the bound. A
  pooler is a later, measured option.
- **High availability of the object store, the identity provider and OpenBao.** The object store is the provider's
  (gap 251 of feature 010 stays); OpenBao's three-voter Raft is feature 017's.
- **The 0.2.0 to 0.3.0 upgrade.** It stays a stopped upgrade, as its release notes say: 0.2.0 binaries have no
  handshake. The N and N-1 promise starts with 0.3.0 as N-1.

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6 (and Owner decision 2026-11-24-4 where it
accepts or rejects), with its reasoning.

1. **CloudNativePG or Patroni on Kubernetes?** CloudNativePG: it is a Kubernetes operator that moves the read-write
   service itself at promotion, needs no external consensus store (it uses the Kubernetes API), and supports
   synchronous quorum and backups to an object store. Patroni on Kubernetes needs its own consensus store and an
   address switch of our own. Patroni stays the reference outside Kubernetes, where CloudNativePG does not run.
   Delegate (Owner decision 2026-11-20-6).
2. **Refuse to commit without a standby, or fall back to asynchronous?** Refuse. The owner's objective is RPO 0 for
   acknowledged work; with two standbys in two other zones, the loss of one zone still leaves a synchronous standby, so
   writes stop only when two of three zones are lost. An operator may choose the preferred mode and its stated RPO.
   Delegate (Owner decision 2026-11-20-6).
3. **Should standbys serve reads in 0.3.0?** Only the closed list of Requirement 048-021, behind an optional address,
   off by default. Reads that decide must see the primary; a revocation must not be missed because a standby is behind.
   Delegate (Owner decision 2026-11-20-6).
4. **How does the adapter recognise a cancelled synchronous wait?** By the warning PostgreSQL sends before the
   `COMMIT` returns. The adapter enables notices on its connections and the reference topology sets `lc_messages = 'C'`
   in the server configuration, so that the text is stable; a fault test stops both standbys and asserts that the
   operation fails as unknown. If the driver cannot expose notices, the alternative is to disable the timeout for the
   `COMMIT` itself and bound it on the client, which Task 048-11 measures against the statement budget. Delegate (Owner
   decision 2026-11-20-6).
5. **Extend leases after an outage?** Yes (Requirement 048-013). Without it, every activity in flight during a failover
   longer than its remaining lease becomes a retry or, since feature 037, a review item; the extension costs one
   statement per failover and never weakens a fence. Delegate (Owner decision 2026-11-20-6).
6. **The decision records.** The topology, the address and the read rule are
   [Architecture decision 0130](../adr/ADR-0130-postgresql-high-availability-is-a-reference-topology-with-synchronous-quorum-and-one-moving-address.md);
   the upgrade rule is
   [Architecture decision 0131](../adr/ADR-0131-migrations-expand-before-and-contract-after-a-release-and-a-binary-refuses-a-schema-it-cannot-use.md).

## Owner decisions needed

None that blocks the design. Running the Kubernetes drill on a larger runner costs money, which stays the owner's
(Owner decision 2026-11-24-4): six kind nodes with the platform need a 16-vCPU runner, at US$ 0.042 per minute on
2026-11-24; a drill of 6 faults × 5 repetitions × 6 minutes = 180 minutes plus 30 of preparation is 210 × 0.042 =
US$ 8.82 per run. Gates Task 048-16.

## Critical User Journeys (CUJs)

### CUJ 1: A zone is lost

*   **Given**: the reference topology on Kubernetes, the primary in zone a, runs in flight.
*   **When**: zone a disappears.
*   **Then**: CloudNativePG promotes the standby that has every acknowledged commit, the read-write service moves, the
    engines' dead connections fail within 7 s and new ones reach the new primary, shard and worker leases are extended
    by the gap, workers' reports are retried until they land, and no acknowledged run is lost or activity repeated.

### CUJ 2: Both standbys are gone

*   **Given**: a primary whose two standbys are unreachable.
*   **When**: a worker completes a step.
*   **Then**: the commit waits; when the client bound cancels the wait, PostgreSQL commits locally and warns; the adapter
    reads the warning, fails the report as unknown and retryable, and the worker retries; the retry's replay waits for
    a standby before it answers. Nothing is acknowledged until a standby has it.

### CUJ 3: A rolling upgrade

*   **Given**: release N-1 running, release N's expand migrations applied.
*   **When**: the operator rolls the engines and control planes to N.
*   **Then**: N-1 and N processes run together and both progress runs; a process of N does not start writing the new
    state until the registry shows no N-1 process; the contract migration ships with release N+1.

### CUJ 4: A binary on the wrong schema

*   **Given**: release N deployed before its migrations were applied.
*   **When**: it starts.
*   **Then**: it closes its serving gate with reason `SCHEMA_TOO_OLD`, reports not ready, writes nothing, and opens the
    gate once the migrations are applied.

## Technical Plan (Architecture)

### Topology

```
            zone a                    zone b                    zone c
   +------------------+      +------------------+      +------------------+
   | PostgreSQL       | sync | PostgreSQL       |      | PostgreSQL       |
   | primary          |----->| standby          |      | standby          |
   |                  |------------------------------->|                  |
   +------------------+      +------------------+      +------------------+
            ^  read-write service (moved by the HA manager at promotion)
            |
   engine x1, control plane x1   engine x1, control plane x1   engine x1, control plane x1
```

Quorum one of two standbys: a commit returns when either standby has flushed it. The HA manager promotes only a
standby that is in the synchronous set and has the highest flushed position, so an acknowledged commit survives the
loss of the primary.

**Kubernetes** (`infra/kubernetes/ha/`): the CloudNativePG operator from its published chart, pinned; a `Cluster`
manifest with three instances, `postgresql.synchronous` quorum one with the required durability, `lc_messages = 'C'`,
the platform's server settings (`max_connections` from the chart's budget, the WAL and checkpoint settings of the
performance report), pod anti-affinity across zones, the Barman Cloud plugin for WAL archiving and a
`ScheduledBackup`. The runtime role `sagat_app` and the owner role are created by the cluster's bootstrap from the
platform's `postgres-init` scripts. The chart example sets `database.existingSecret` to a Secret whose URL names
`<cluster>-rw`.

**Compose** (`infra/docker/compose.ha.yaml`): `etcd-1..3`, `postgres-1..3` from an image built in
`infra/docker/patroni/` (the PostgreSQL image of `compose.yaml` by digest plus a pinned Patroni), and `haproxy` checking
each node's `GET /primary`. The platform's services use `haproxy:5432` as their database address.

### The platform's side

| Change | Where | Requirement |
|---|---|---|
| Client bound per statement (`statement_timeout` + 2 s), maximum connection lifetime | `crates/infra-postgres/src/pool.rs` | 048-010 |
| `after_connect` refuses a connection in recovery; `25006` and `57P0x` are unavailable; a `25006` connection is closed | `pool.rs`, `identity.rs` (the classifier) | 048-011 |
| Readiness reads the pool's last writability check | `crates/api-http/src/routes/health.rs`, the engine's `/readyz` | 048-011 |
| The cancelled-wait warning fails the operation as unknown; replay answers commit a waited transaction first | the adapter's commit wrapper (`begin_safely`'s counterpart); the replay paths of every store module | 048-012 |
| Continuity row and the one-time lease extension | a migration; `crates/scheduler` (the keeper and the lost-lease pass) | 048-013 |
| Report retry window | `sdks/SPEC.md` and the six SDKs; the conformance scenario `report-through-failover` | 048-014 |
| Optional read address and the closed list | `apps/*/config.rs`, `crates/infra-postgres` (a second pool used only by the listed reads) | 048-021 |

**The replay rule in practice.** A replay transaction writes one row of a small table (`durability_beacons`, one row
per process, updated in place), which gives it a transaction id and makes its `COMMIT` wait for the standby; only then
does it return the stored answer. The write is on the replay path only, which is rare; the statement budget of design
038 records its cost for a duplicate report in a new line.

**The lease extension in practice.** The keeper's round, which already writes the shard lease rows, also updates
`database_continuity.last_round_at = clock_timestamp()`. The first lost-lease pass of a primary whose
`pg_control_checkpoint()` timeline differs from the one recorded in that row takes the gap
`clock_timestamp() - last_round_at`, adds it in one statement to every worker lease and shard lease whose `expires_at`
lies in `[last_round_at, clock_timestamp())`, records the new timeline, writes one audit record, and then proceeds as
today. The new primary's clock is used throughout; a clock offset between the old and the new primary shifts the gap by
that offset, which is why the gap, and not the old expiry, is what is added.

### Rolling upgrades

**Marking.** Each migration's first line is `-- phase: expand` or `-- phase: contract`. The migrations before the lint
are treated as of their release.

**The compatibility table.** `schema_compatibility(release text primary key, oldest_compatible_release text, applied_at)`
gains a row in the last migration of each release. The binary embeds its release and the list of its migrations (the
sqlx migrator already does). At start and at every instance heartbeat (ADR-0082's registry already runs one), the
process reads the newest row and `_sqlx_migrations`, and closes its gate with `SCHEMA_TOO_OLD` (a migration it needs is
missing) or `BINARY_TOO_OLD` (its release is older than `oldest_compatible_release`).

**The lint.** `scripts/check-migrations.py` reads the migrations a pull request adds and refuses: a missing phase line;
a missing `lock_timeout`; `CREATE INDEX` without `CONCURRENTLY` on a table that existed before the pull request;
`ADD CONSTRAINT` without `NOT VALID` on such a table; `ALTER COLUMN ... TYPE`; `RENAME`; `ADD COLUMN ... DEFAULT` with a
volatile expression; `DROP` in an expand migration; and a contract migration in the same release as its expand. Each
rule has a negative fixture under `scripts/tests/`.

**The mixed-writer test.** `scripts/test-mixed-writers.py` builds release N-1 from its tag (as
`scripts/test-release-upgrade-profile.py` does for 0.2.0), applies N's expand migrations, starts two N-1 engines, one N
engine, one control plane of each, and a worker pool, drives the chain shape for 10 minutes, restarts each engine in
turn, and runs the integrity oracle. The release workflow runs it against the previous release.

**The order of an upgrade**, written in `docs/runbooks/scaling.md` section 3 in place of the stopped upgrade:

1. Apply N's expand migrations (`control-plane migrate --phase expand`) while N-1 runs.
2. Roll the control planes and the engines to N, one at a time (`maxUnavailable: 0`, `maxSurge: 1`).
3. When the registry's census shows no N-1 process, switch on N's new capabilities.
4. N's contract migrations ship with N+1 and run at step 1 of the next upgrade.

Rollback: before step 3, roll the binaries back; the expand schema is what N-1 runs on. After step 3, rolling back
needs the new capabilities switched off first and the runs that used them drained, as today.

### The drills

**Kubernetes** (`tests/e2e/high-availability/`): a kind cluster of six workers labelled with three zones, the
CloudNativePG operator and cluster of `infra/kubernetes/ha/`, the platform's chart with the HA example, and design 051's
driver at a fixed rate. Faults by command: `kill -9` inside the primary's container; `kubectl delete pod`; `kubectl
drain`; `docker stop` of both nodes of a zone; a network isolation of the primary's node with `docker network
disconnect`. The driver's per-second counter gives the recovery times; a sampler polls `pg_is_in_recovery()` on every
instance every 100 ms to detect two primaries accepting commits (a commit observed on two timelines). A full drill is
6 faults × 5 repetitions × 6 minutes (2 before the fault, 4 after) = 180 minutes.

**Compose** (`infra/dr/ha-drill.sh`): the same faults that make sense on one machine (`kill -9` of the primary, stop
of a node, stop of both standbys) on `compose.ha.yaml`, run by the `scale` job of CI on the standard runner and by the
release workflow, replacing the constant 15 s of today's drill with a measurement.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The warning text of a cancelled wait changes between PostgreSQL versions | `lc_messages = 'C'` in the reference; the fault test runs on every PostgreSQL major the platform supports, so a change fails a test, not a customer |
| The lease extension keeps a dead worker's lease alive longer | By the gap only, once per failover; a dead worker's step is retried after the extended expiry, which is at most the failover budget later |
| An isolated old primary keeps accepting commits from clients that reach it | With the required durability it cannot commit without a standby; the adapter's warning check covers a cancelled wait; the drill's two-primary sampler fails the release if it ever happens |
| Old and new writers disagree on new state | Requirement 048-034: new state waits for the fleet census; the mixed-writer test runs on every release |
| The migration lint blocks a needed change | A rule is waived only by a footer that names the rule and the reason, printed by the lint for the reviewer, as `check-docs-first.py` does |
| CloudNativePG or Patroni has a bug | Versions pinned; the drill runs on every release; the platform's own integrity oracle is the judge, not the HA manager's status |
| The kind drill is not a real cloud | Stated in the report; the same drill runs on a cloud cluster when the owner provides one |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Nothing is acknowledged without the standby (048-012); the lease extension changes no token or generation and is a database write under the existing fence; one transaction per transition is unchanged |
| II. Modular domain, ports and adapters | Pass | All changes are in `infra-postgres`, the scheduler and configuration; the domain is untouched |
| III. External effects are at-least-once; sagas compensate | Pass | The failover rules reduce repeats but promise none; idempotency keys still absorb them |
| IV. Compatibility is versioned and pinned | Pass | Expand and contract with a compatibility table and a handshake (048-031 to 048-035); images and operators pinned by digest |
| V. Code, tenant boundary and secrets are isolated | Pass | Authorization and secret reads stay on the primary (048-020); the read address carries no new privilege |
| VI. Observability and audit are part of the feature | Pass | Metrics for refused read-only connections, cancelled waits, extensions and gate closures, with closed labels; an audit record per extension |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Inbox duplicates acknowledge only after a waited commit (048-012) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Six faults in three zones, five times each, measured; the both-standbys-gone fault; the mixed-writer test |
| Delivery Constraints | Pass | "Single region with high availability" is what this design builds; the HA manager is a declared dependency of the reference, not of the platform |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Two reference topologies (CloudNativePG, Patroni) | CloudNativePG runs only on Kubernetes; developers and design 051 need one on Compose | One topology: no failover on a laptop, or a Kubernetes-only drill |
| The cancelled-wait check and the waited replay | Without them RPO 0 is not true when both standbys stall | Trusting `synchronous_commit = on`: PostgreSQL itself returns success after a cancel |
| The lease extension | A failover longer than a lease's remainder creates retries and review items | Longer leases for everyone: slower recovery from a real worker crash |
| The compatibility table and the gate | A binary on the wrong schema writes state nobody can read | Documentation only: the 0.2.0 runbooks contradict each other today |
| The migration lint | The rules are easy to forget and costly when broken | Review only |

## Alternatives Considered

- **Leave HA to the operator** (today). Every operator rebuilds the same topology and the platform's side of a failover
  stays untested.
- **Several hosts in the URL with `target_session_attrs=read-write`.** The driver does not support it, and it moves
  the choice of primary into every process; one address moved by the HA manager is simpler and is what both managers
  provide.
- **Asynchronous standbys only.** Faster commits, RPO above zero; contradicts the owner's objective.
- **Blue and green upgrades with two databases.** Needs logical replication of the whole schema, with its triggers and
  definer functions firing again on apply (Architecture decision 0067, option 3, for the same reason).
- **A maintenance window for every upgrade** (today). Loses the main operational advantage this design is for.

## Non-Functional Requirements

- **Non-functional requirement 048-001 Hot path.** Requirements 048-010 to 048-013 add no statement to a lease or a
  completion that is not a replay; the budget of design 038 is unchanged for them, and the replay line is added.
- **Non-functional requirement 048-002 Recovery.** As Requirement 048-030.
- **Non-functional requirement 048-003 Security.** The read address uses the runtime role; the HA manager's own roles
  are separate and never given to the platform; the Compose reference is for development only.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T048-1 Write Architecture decisions 0130 and 0131 and update `docs/architecture/storage-and-events.md` and `docs/runbooks/disaster-recovery.md` with the topology and the closed list of stale-tolerant reads.
  Refs: Requirement 048-001, Requirement 048-021, Requirement 048-031
- [ ] T048-2 Add `infra/docker/compose.ha.yaml` and `infra/docker/patroni/` with pinned images, loopback ports and healthchecks.
  Refs: Requirement 048-002, Requirement 048-003
- [ ] T048-3 Add `infra/kubernetes/ha/` (operator values, `Cluster`, backups) and `infra/helm/examples/ha-cloudnativepg.yaml` with zone spread that insists.
  Refs: Requirement 048-001, Requirement 048-004
- [ ] T048-4 Bound every statement on the client, set a maximum connection lifetime, and test a server that vanishes mid-query.
  Refs: Requirement 048-010
- [ ] T048-5 Refuse connections in recovery, classify `25006` and `57P0x`, close a read-only connection, and make readiness follow writability.
  Refs: Requirement 048-011
- [ ] T048-6 Fail an operation whose commit's synchronous wait was cancelled, with the fault test that stops both standbys.
  Refs: Requirement 048-012
- [ ] T048-7 Make every replay answer commit a waited transaction first, with the budget line for a duplicate report.
  Refs: Requirement 048-012, Non-functional requirement 048-001
- [ ] T048-8 Add the continuity row and the one-time lease extension after a failover, with its audit record and fault test.
  Refs: Requirement 048-013, Requirement 048-015
- [ ] T048-9 Add the report retry window to `sdks/SPEC.md`, the six SDKs and the conformance scenario `report-through-failover`.
  Refs: Requirement 048-014
- [ ] T048-10 Add the optional read address and route only the closed list through it, with staleness refusal and fallback.
  Refs: Requirement 048-020, Requirement 048-021
- [ ] T048-11 Measure the cost of the cancelled-wait check and of the waited replay against the statement budget, and choose the mechanism of Open Question 4.
  Refs: Requirement 048-012
- [ ] T048-12 Write `scripts/check-migrations.py` with a negative fixture per rule, mark every migration's phase, and run it in CI.
  Refs: Requirement 048-031, Requirement 048-032, Success criterion 048-005
- [ ] T048-13 Add the compatibility table and the schema handshake on the instance registry's gate.
  Refs: Requirement 048-033
- [ ] T048-14 Put new write behaviour behind registry capabilities switched on after the census, and write the upgrade order in `docs/runbooks/scaling.md`.
  Refs: Requirement 048-034
- [ ] T048-15 Write `scripts/test-mixed-writers.py` and run it in the release workflow against the previous release.
  Refs: Requirement 048-035, Success criterion 048-003
- [ ] T048-16 Write the Kubernetes drill in `tests/e2e/high-availability/` with the two-primary sampler, run it, and publish `tests/performance/reports/high-availability.md`.
  Refs: Requirement 048-040, Requirement 048-041, Success criterion 048-001, Success criterion 048-002
  Owner-gate: OD-2026-11-24-1
- [ ] T048-17 Write `infra/dr/ha-drill.sh` on the Compose reference, measure the takeover instead of assuming it, and run it in the `scale` job and the release workflow.
  Refs: Requirement 048-041, Requirement 048-042, Success criterion 048-004
- [ ] T048-18 Correct `docs/runbooks/control-plane.md`, `execution-engine.md`, `scaling.md` and `disaster-recovery.md` so that they agree on failover and upgrades.
  Refs: Requirement 048-003, Requirement 048-031

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs the suites of what it touches, `integration_claim_transition_efficiency` when it touches
`crates/infra-postgres/`, the fault suite of feature 010 (`tests/fault/failover`), `cargo clippy --workspace
--all-targets --locked -- -D warnings` and the three checkers. Tasks 048-4 to 048-8 each add a fault test seen failing
first: a server that stops answering without closing the connection (048-4), a promoted standby with the old primary
still reachable read-only (048-5), both standbys stopped during a completion (048-6 and 048-7), and a failover longer
than a lease's remainder with a worker whose report arrives after it (048-8). Task 048-9 runs the SDK conformance
scenario in all six languages. Task 048-15 is the release gate for upgrades; Tasks 048-16 and 048-17 are the gate for
recovery, both stated with their environment.
