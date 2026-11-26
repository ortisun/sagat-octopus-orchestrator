# A reference high-availability topology for PostgreSQL, and the platform's side of a failover

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

PostgreSQL is the platform's only authority. Today its high availability is left to the operator, and a promotion and
the switch of the address the platform connects to are manual. This design makes both part of the product, inside one
region:

1. **A reference topology** that the repository runs and tests: CloudNativePG on Kubernetes (`infra/kubernetes/ha/`,
   with the platform's chart in `infra/helm`) and Patroni on Docker Compose (`infra/docker/compose.ha.yaml`). Both keep a
   primary and two standbys with synchronous replication (a commit returns when either standby has it), promote a
   standby without a person, and move the one address the platform uses to the new primary.
2. **The platform's side of a failover**: dead connections are found within 10 s by socket keepalive; a connection to
   a read-only or superseded server is not reused; the platform never cancels a commit that waits for a standby and
   reports an unknown outcome when its own deadline passes; after a database restart, a crash restart in place included,
   a process serves nothing until one write of its own has reached a standby; worker reports keep retrying through a
   failover, and the lease time to live is sized to the expected failover.
3. **What a standby may serve**: nothing on the path that decides or writes; one read that tolerates staleness, through
   an optional second address.
4. **Evidence**: a disruption and failover drill on Compose that gates every release, and a three-zone drill on
   Kubernetes, unscheduled while the owner pays for no runner (Owner decision 2026-11-25-1), both measured against the
   owner's objectives (RPO 0 for acknowledged work, failover within 5 minutes, Owner decision 2026-10-16, row 8).

Rolling upgrades with old and new writers together, which an earlier draft of this design also held, are design 053
(Owner decision 2026-11-24-4, the coordinating session's split of 2026-11-25). Cross-region failover is feature 032
(Architecture decision 0067); this design is the in-region base it builds on.

**Terms used below.** A *transition* is one step's result committed by the platform. A *lease* is a worker's right to
execute one attempt of a step until an expiry on the database clock, fenced by a token and a generation, so that a stale
attempt is refused; a *review item* is work that waits for a person because its outcome is unknown (feature 037). The
scheduler's work is divided into 64 *shards* held by engines under *shard leases*; the *keeper* renews them every
*keeper round* (a fifth of the shard lease, 3 s by default; `crates/scheduler/src/shards.rs`), and the *lost-lease pass*
reclaims expired worker leases. The *instance registry* (Architecture decision 0082) records the live processes; a
process whose requirement is unmet keeps running with its *serving gate* closed. The *integrity oracle* is the check of
`tests/performance/scalability-ha/` that no acknowledged run was lost, no lease was granted twice, and replaying each
run's journal gives its projection. The *chain shape* is design 051's chain of three activities, driven by design 051's
driver. A server's *identity* is the pair of its postmaster's start time and the token of its incarnation marker
(Requirement 048-013).

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
    server's warning (sqlx 0.9 logs notices); and nothing in the platform bounds a commit on the client. A crash restart
    in place (a backend killed, the postmaster reinitializing shared memory) keeps `pg_postmaster_start_time()`, so the
    start time alone cannot detect it.
  - Nothing pins `synchronous_commit`: a role's or a database's setting of `local` or `off` would void RPO 0 without the
    platform noticing.
  - The SDKs make 4 attempts of a failed call, 3 retries from 200 ms to at most 1.4 s with jitter, `CompleteStep` up to 5
    (`sdks/SPEC.md`); worker leases last 30 s (`SAGAT_LEASE_TTL_SECS`); an outage longer than a lease's remainder makes it
    expire, and since feature 037 such an activity waits for a person unless it has a `reconcileWith` lookup.
  - Readiness (`/v1/ready`, `/readyz`; `crates/api-http/src/routes/health.rs`) fails when the storage ping fails or the
    serving gate is closed, and when the registry's last answer is older than its liveness window (Architecture decision
    0082, decision 5); the chart probes it every 5 s with a failure threshold of 2.
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
  to the chart's connection budget (Task 048-3); every version (the first operator release that has the synchronous
  failover quorum, checked by Task 048-3) and image pinned by digest.
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
  time of 4 s, an interval of 2 s and 3 probes, and, on Linux, `TCP_USER_TIMEOUT` of 10 s. A peer that stopped answering
  is then found within 10 s of the last packet it sent: with nothing unacknowledged, probes leave at 4, 6 and 8 s and the
  third goes unanswered at 4 + 2 × 3 = 10 s (on Linux the user timeout ends the connection at the first probe tick at or
  after 10 s, which is the same 10 s, since 4 + 2 × 3 = 10); with bytes unacknowledged, Linux ends it when they have waited
  10 s (it clamps the retransmission timer to the user timeout since Linux 4.19). sqlx 0.9 has no option for either, so
  the repository vendors `sqlx-core` with this one change (as it vendors `rskafka`), verified by `scripts/check-vendor.py`
  and offered upstream. A live server that is only slow, a commit waiting for a standby among them, answers the probes and
  is not cut. Where a proxy ends the platform's TCP connection, the bound applies to the proxy: on Compose, HAProxy checks
  each node every second (`inter 1s fall 3`), so a dead primary is marked down within 3 s and its sessions are ended
  (`on-marked-down shutdown-sessions`); HAProxy sets no server or client timeout of its own. On Kubernetes the address is
  a Service, translated by the node's kernel, so the platform's socket reaches the database's own kernel.

### The platform's side of a failover

- **Requirement 048-010 Dead connections are found.** A statement or a commit to a server that stopped answering MUST
  fail within 10 s (Requirement 048-005); a statement to a live server is bounded by its `statement_timeout` (5 s). Idle
  connections are tested before reuse (sqlx's `test_before_acquire`, its default).
- **Requirement 048-011 Read-only and superseded servers.** SQLSTATE `25006` MUST be classified as internal when it comes
  from a transaction the platform opened read-only (`begin_safely` records the mode) or from the stale-read pool of
  Requirement 048-021, and as unavailable and retryable otherwise: a writer that meets it is on a server that is in
  recovery or was set read-only (`default_transaction_read_only`), and no question to the database is needed to tell.
  This holds in `identity.rs` and in `instances.rs`'s `registry_error`. `57P04` is not retryable. The connection that met
  an unavailable `25006` is closed, not returned to the pool. A writer connection's `after_connect` reads, in one
  statement, `pg_is_in_recovery()`, `current_setting('default_transaction_read_only')`,
  `current_setting('synchronous_commit')` and the server's identity (Requirement 048-013), and refuses the connection
  when the server is in recovery or read-only, or when `synchronous_commit` is not `on` or `remote_apply` (the optional
  read pool and a passive process of feature 032 are exempt from the first two). The writer pools send
  `synchronous_commit=on` as a startup option, which outranks a role's or a database's setting, and
  `scripts/check-architecture.py` refuses `synchronous_commit` in any statement of the platform. When
  `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY` is true (the default of both references' examples; false for a single
  development database), `after_connect` also refuses a server whose `synchronous_standby_names` is empty.
- **Requirement 048-012 Commits and writes.** Every write MUST be inside a transaction or a call of one writing function,
  and every commit MUST go through `commit_safely` in `pool.rs` and every writing function call outside a transaction
  through `execute_safely` beside it. Both move the connection, owned as a `PoolConnection`, into a task they spawn (as
  `begin_safely` does for `BEGIN`) and wait for it at most the commit deadline (`SAGAT_DB_COMMIT_DEADLINE_MS`, default
  10,000, at most 60,000, at least twice the statement timeout, as feature 032's decision D1585). The outcome is applied
  only when the server answers `COMMIT` (the command tag is asserted), or the function's result for `execute_safely`; it is
  not applied only when the server answers `ROLLBACK` or an error of class 23 (a deferred constraint) or 40 (transaction
  rollback); every other result, the deadline passing, an I/O error, any other SQLSTATE, a protocol error, a closed pool
  or a task that panicked, is `StoreError::CommitOutcomeUnknown`, the generalization of Architecture decision 0078's
  `CommitOutcomeUnknown`. The task keeps the connection until the server answers or the socket fails. After a hard limit
  of 60 s it detaches the connection from the pool (`PoolConnection::detach`), which frees the pool's slot, and keeps it
  open until the backend answers, so that no backend is orphaned; a process holds at most as many detached connections as
  its pool's maximum, after which a waiting commit keeps its slot. A process therefore opens at most twice its pools'
  maximum, which the chart's connection budget counts (Task 048-3). The API reports code `UNAVAILABLE` with reason
  `COMMIT_OUTCOME_UNKNOWN`, retryable only for an operation that carries an idempotency key or a command identifier; it is
  an API reason, not a step outcome code, so it is not in the list of unknown-outcome step codes. The platform never
  cancels or terminates a backend that waits for a standby: the writer pools set `client_connection_check_interval` to 0
  and, where the server has it, `transaction_timeout` to 0, and `scripts/check-architecture.py` refuses
  `pg_cancel_backend`, `pg_terminate_backend`, a commit outside `commit_safely`, and a string that calls one of the
  platform's writing functions (those its migrations create without `STABLE` or `IMMUTABLE`) in a function that neither
  opens a transaction nor calls `execute_safely`. Whether the server's own timeouts can cancel the wait is established by
  Task 048-6 before the other tasks of this section; if they can, `commit_safely` sends `SET LOCAL statement_timeout = 0`
  and `COMMIT` in one message.
- **Requirement 048-013 After a restart, nothing is served until a write reaches a standby.** A server's identity is the
  pair (`pg_postmaster_start_time()`, the token of `server_incarnation`). `server_incarnation` is an unlogged table of one
  row (`id smallint primary key check (id = 1)`, `token uuid`): PostgreSQL empties an unlogged table after a crash
  restart and at a promotion, and keeps it across a clean restart, whose start time changes instead; so the pair changes
  on every restart and promotion. Each writer connection's `after_connect` reads the identity (Requirement 048-011); when it
  differs from the one the process last confirmed, or the token is absent, or the process has confirmed none,
  `after_connect` refuses the connection, the process records the time (`identity_changed_at`), and sets the serving
  gate's reason `DATABASE_UNCONFIRMED`. One confirmation task per process then opens a connection of its own (not from a
  pool, with the keepalive of Requirement 048-005) and runs one transaction: `sagat_server_identity()` (owner-defined,
  `SECURITY DEFINER`, `search_path` pinned, executable by `sagat_app` only) inserts a fresh token if the row is absent and
  returns the identity, and an upsert of the process's row of `durability_beacons` (keyed by its instance identifier); its
  commit, through the rules of Requirement 048-012, waits for a standby. When it returns applied, the process clears the
  reason only if the identity read in that transaction is still the newest it has observed (a compare and clear under the
  gate's lock); otherwise it starts again. Since a standby receives WAL in order, everything committed before that
  write, including what a primary restarted after a crash marked committed, is then on a standby, and no connection of
  the process read anything before. The task keeps one connection and one waiting commit at a time; an unknown outcome
  keeps the reason and retries after 1 s, doubling to at most 5 s, on a new connection once the old one's backend has
  answered or its socket failed. While `DATABASE_UNCONFIRMED` is set, request paths answer `503 UNAVAILABLE` with
  `Retry-After: 2` without acquiring a connection, so that only the confirmation task connects. The pools'
  `before_acquire` hook discards, without a statement, every connection created before `identity_changed_at` (sqlx pings
  an idle connection before the hook, so a discarded one has cost one ping). A command that runs once (`migrate`,
  `secret-cleanup`, `provision-tenant` and the reseal command) confirms in the same way before its first statement, keyed
  by an identifier of its run, and exits non-zero if the identity changes during its run, to be run again; each is
  idempotent today. `migrate` on a database that does not yet have the two tables runs as today and confirms from the
  migration that creates them onwards. Rows of `durability_beacons` unseen for a day are deleted by housekeeping through a definer function.
- **Requirement 048-014 The serving gate has reasons.** The registry's serving gate (`crates/application/src/instances.rs`)
  MUST become a set of closing reasons (the registry's own and `DATABASE_UNCONFIRMED`; design 053 adds two), open only
  when the set is empty; each owner clears only its own reason, and a re-registration does not reopen a gate another
  reason holds. Readiness keeps today's rule (it fails while the gate is closed), so a process that waits for its
  confirmation receives no traffic from a load balancer.
- **Requirement 048-015 Reports survive a failover.** The six SDKs MUST keep retrying a `CompleteStep` or `FailStep` that
  failed as unavailable or as a transport error (a refused connection, a reset, a timeout, an ingress `502` or `503`)
  for a report retry window (`reportRetryWindow` in each SDK's idiom), 310 s by default, in place of the attempt count:
  the release gate (300 s) plus the backoff cap (5 s), the platform's `Retry-After` (2 s) and the acquire timeout (3 s),
  300 + 5 + 2 + 3 = 310 s. Each wait is clamped to the time left in the window, and one last attempt is made when the
  window ends, so the outcome does not depend on jitter. A heartbeat keeps today's retries and is never retried past the
  time of the next heartbeat. The conformance scenario `report-through-failover` answers `UNAVAILABLE` without
  `Retry-After` for 8 s with a window of 10 s, so the last attempt, at 10 s, lands after the outage (today's five
  attempts last at most 3 s, so today's SDKs fail it); a negative case answers for 12 s, where the last attempt, at
  10 s, fails and the SDK gives up.
- **Requirement 048-016 Leases and work in flight.** A failover MUST lose no acknowledged work. A worker lease is renewed
  by heartbeats at most 10 s apart, so it always has at least its time to live minus 10 s left; a failover whose outage, as
  the platform sees it, is shorter than that (30 − 10 = 20 s with the default time to live of 30 s) expires no delivered
  lease, and for workers on SDKs with the report window it repeats no completed activity and creates no review item for
  one. A claim whose commit outcome was unknown may have leased an attempt that no worker received; that lease expires
  and, without a `reconcileWith` lookup, becomes a review item (feature 037). The drills count these apart; the delivery
  acknowledgement (protocol 1.8, reserved by Owner decision 2026-11-21-6, pull request 264) is what lets such a
  lease be retried without a person.
  For longer failovers, the runbook's rule is a time to live of at least the expected outage plus two heartbeat intervals
  (`SAGAT_LEASE_TTL_SECS`); the drills report the leases expired, the activities repeated and the review items created.
  Carrying leases across a longer failover is a follow-up design (Non-Goals).

### What a standby may serve

- **Requirement 048-020 Standbys decide nothing.** No claim, lease, transition, replay, receipt, quota read,
  authorization read or secret read may be served by a standby.
- **Requirement 048-021 One stale-tolerant read.** A process MAY have a second, optional address (`DATABASE_READ_URL`,
  the HA manager's replica service, which routes only to standbys of the current primary; in-region in a feature 032
  cell) for one read: `GET /runs/{runId}/events` of a terminal run. It is authorized on the primary first, in one
  tenant-scoped statement that also reads the run's terminal state and its highest event sequence `S`. On the standby,
  one statement reads the events with a sequence of at most `S`, and only if the standby's own highest sequence for the
  run is at least `S`; so the response is exactly the primary's journal at the moment of authorization, whatever is
  appended later. If the check fails, and on any standby error, the primary answers; a miss, 404 or 403 is always the
  primary's. A response served by a standby carries `Sagat-Read-Source: standby` with no value. The read pool's sessions
  are read-only and its statement timeout 5 s; the reference bounds `max_standby_streaming_delay` at 30 s. The pool is a
  type, `StaleReadPool`, that only `crates/infra-postgres/src/stale_reads.rs` can build, which
  `scripts/check-architecture.py` checks. The read relieves the primary of journal pages, which are the largest reads; it
  gains no availability in a failover.

### Evidence

- **Requirement 048-040 The failover budget and the release gate.** For the loss of one database instance, the time from
  the fault to the first transition committed after it is reported at the median and the maximum, and so is the time until
  every shard is held again. The release gate is the owner's 300 s at the maximum. The median's target, a benchmark
  criterion, is 60 s. After the HA manager's promotion P and the surviving standby's attachment to the new primary R (both
  measured), a worker's report reaches a committed transition within 10 + 3 + 5 + E + 5 = 23 s + E: the platform finds a
  dead connection within 10 s (Requirement 048-005), opens and confirms one within the 3 s connect timeout (its beacon
  waits for R, already counted), turns ready at the next probe within 5 s, the load balancer's endpoints follow in a
  measured time E, and the worker's next retry comes within its 5 s backoff cap. So the target leaves P + R + E at most
  60 − 23 = 37 s. An engine holds a live shard within 10 + 3 + 3 = 16 s of P + R; a dead engine's shards are free once
  their 15 s lease, renewed last before the fault, has expired on the database's clock, and are held within one keeper
  round after both that and the 13 s reconnection, at most max(15 s, P + R + 13 s) + 3 s after the fault. On Compose the
  10 s detection is HAProxy's 3 s (Requirement 048-005).
- **Requirement 048-041 The drills.** The Compose drill (`infra/dr/ha-drill.sh`) MUST inject, five times each: the
  primary's container stopped; a node stopped; both standbys stopped (commits wait, the deadline reports an unknown
  outcome, nothing becomes visible); a standby throttled and then the primary stopped; a backend of the primary killed
  (`kill -9`, a crash restart in place); and the primary's network disconnected (`docker network disconnect`, a dead peer
  with acknowledged data). The Kubernetes drill (`tests/e2e/high-availability/`) adds, on six kind nodes in three labelled
  zones: deletion of the primary's pod; a drain of its node; the loss of a zone (its nodes' containers stopped); and the
  isolation of the primary's node (`docker network disconnect`). Both run the chain shape at a fixed open-loop rate
  through design 051's driver.
- **Requirement 048-042 Measured, not estimated.** For each repetition a drill MUST record: P, R and E; the times of
  Requirement 048-040; the time until the completion rate is back to 90% of its rate before the fault, held for three 10 s
  windows; acknowledged work lost; leases expired, delivered or not; activities run more than once; review items created,
  those of claims with an unknown outcome apart; and whether a former primary acknowledged a commit after the fork. For
  the last, before the former primary rejoins, the drill copies its data directory, starts the copy alone on a private
  port with no replication, and looks up in it and in the new primary every acknowledgment of the driver's ledger (one
  line per acknowledged start and completion, with the run identifier, the step's sequence and the time): an
  acknowledgment found on the copy but not on the new primary is a commit acknowledged by a primary that lost it. A
  negative control injects, with synchronous replication switched off on the isolated primary, a commit the driver sees
  acknowledged, and the comparison must find it. Reports go to `tests/performance/reports/high-availability.md` with the
  runner (Owner decision 2026-11-25-1), the environment and every failure.
- **Requirement 048-043 The drills gate the release.** A drill script MUST exit non-zero when any repetition loses
  acknowledged work, finds an acknowledged commit lost by a former primary, or exceeds 300 s to the first transition.
  The Compose drill runs on a free standard runner and gates every release; the Kubernetes drill needs a paid runner,
  stays unscheduled while the owner pays for none (Owner decision 2026-11-25-1), and until it runs the report says so.
  `infra/dr/drill.sh` keeps its single-host checks; its 15 s constant is replaced by the measured takeover of
  `ha-drill.sh` in the report.

### Success criteria

- **Success criterion 048-001** In every repetition of every fault, acknowledged work lost is 0 and no commit
  acknowledged by a former primary is missing from the new one; the negative control is found.
- **Success criterion 048-002** The median time to the first committed transition is reported for each fault; for the
  primary's container stopped, a node stopped and the pod deleted, it is within 60 s.
- **Success criterion 048-003** In the Compose drill, a failover whose outage is under 20 s expires no delivered lease,
  repeats no activity that had completed, and creates no review item except for a claim whose commit outcome was
  unknown, for workers on SDKs with the report window; for longer ones the counts are reported.

### User stories

- **User story 048-01** An operator installs the reference topology and loses a node or a zone without paging anyone for
  the database.
- **User story 048-02** A developer reproduces a failover on a laptop with one command.

## Goals

- A database failure inside a region is an event the platform rides through, not an incident.
- The recovery times are numbers anyone can rerun, not adjectives, each with the runner that measured it.

## Non-Goals

- **Rolling upgrades** (expand and contract, the schema handshake, N and N-1 writers): design 053.
- **Cross-region failover**, cells and the region epoch: feature 032.
- **Building an HA manager.** The platform elects nothing (Architecture decision 0067, decision 3).
- **A connection pooler in the reference.** The platform sets session options at connection and uses prepared
  statements; a pooler in transaction mode would need both changed.
- **High availability of the object store, the identity provider, OpenBao and the notification broker.** The object
  store is the provider's (gap 251 of feature 010); OpenBao's Raft is feature 017's; the broker carries hints only and its
  loss costs nothing (constitution, principle VII).
- **Workers on older SDKs** keep today's retries; the guarantees of Requirement 048-016 name the SDK versions they need.
- **Carrying leases across a failover longer than their remainder.** An earlier draft extended leases after a database
  restart or promotion; its review showed that detecting the event, fencing every reader of an expiry and doing it
  exactly once is a design of its own. It is a follow-up design; until then the time to live is sized to the expected
  failover (Requirement 048-016).
- **A claim on the performance of competitors.** No figure of these drills is compared with Temporal or Windmill
  (Owner decision 2026-11-25-1).

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6, with its reasoning.

1. **CloudNativePG or Patroni on Kubernetes?** CloudNativePG: it moves the read-write service itself, needs no external
   consensus store, and has the synchronous quorum, the failover quorum and backups to an object store. Patroni stays the
   reference outside Kubernetes. Delegate.
2. **Refuse to commit without a standby, or fall back to asynchronous?** Refuse. With two standbys, the loss of one still
   leaves a synchronous standby; writes stop only when both are gone. An operator may choose the preferred mode with its
   stated RPO, by setting `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY` to false. Delegate.
3. **Should standbys serve reads in 0.3.0?** Only the one read of Requirement 048-021, off by default. Delegate.
4. **How is a commit kept from being visible before a standby holds it?** The platform never cancels a waiting commit,
   bounds it on its own side and reports an unknown outcome; after a restart or promotion, detected by the identity, a
   process serves nothing until a write of its own reached a standby; whether the server's own timeouts can cancel the
   wait is established by a test first. Reading the server's warning after a cancelled wait, and a waited write on every
   replay path, were rejected: the driver cannot surface the warning, and the confirmation after a restart covers what a
   replay could read. Delegate.
5. **Extend leases after a database restart or promotion?** Not in this design. Two review rounds found that the
   detection, the fencing of every reader of an expiry and the exactly-once extension need a design of their own; the time
   to live sized to the expected failover covers the common case, and the drills count what a longer failover costs.
   Delegate.
6. **The decision record.** The topology, the commit rule, the confirmation and the read are
   [Architecture decision 0130](../adr/ADR-0130-postgresql-high-availability-is-a-reference-topology-with-synchronous-quorum-and-one-moving-address.md),
   which refines Architecture decision 0082 (a gate of several reasons). The upgrade rule is design 053's Architecture
   decision 0131.

## Owner decisions needed

- **The Kubernetes drill's runner.** Six kind nodes with the platform need a 16-vCPU runner, US$ 0.042 per minute on
  2026-11-24 (`docs.github.com/en/billing/reference/actions-runner-pricing`); 4 faults × 5 repetitions × 6 minutes = 120
  minutes plus 30 of preparation is 150 × 0.042 = US$ 6.30 per run. The owner pays for no runner in 0.3.0 (Owner decision
  2026-11-25-1), so Task 048-12 stays unscheduled until the owner decides otherwise.

## Critical User Journeys (CUJs)

### CUJ 1: The primary's node is lost

*   **Given**: the reference topology, runs in flight.
*   **When**: the primary's node stops.
*   **Then**: the HA manager promotes the standby that holds every acknowledged commit and moves the address; the
    platform's dead connections fail within 10 s; each process's first new connection finds the server's identity
    changed and the process waits, serving nothing and unready, until its beacon reaches a standby; workers' reports are
    retried until they land; no acknowledged run is lost.

### CUJ 2: Both standbys are gone

*   **Given**: a primary whose two standbys are unreachable.
*   **When**: a worker completes a step.
*   **Then**: the commit waits for a standby and stays invisible; at the commit deadline the worker gets
    `COMMIT_OUTCOME_UNKNOWN` and retries with its command identifier; after 60 s the commit's connection is detached from
    the pool and kept until its backend answers, so the pool is not exhausted and no backend is orphaned; when a standby
    returns, the waiting commit finishes and the retry finds it.

### CUJ 3: A backend crashes on the primary

*   **Given**: the reference topology, runs in flight.
*   **When**: a backend of the primary is killed and the postmaster restarts in place, marking committed what was waiting
    for a standby.
*   **Then**: every connection of every process breaks; the next one finds the incarnation token gone; each process
    serves nothing until its beacon's commit has reached a standby, which by then holds everything marked committed.

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
| `commit_safely`, `execute_safely`, the deadline, the detach and its cap, `CommitOutcomeUnknown`, the reason on `AppError::Unavailable` and its HTTP and gRPC mapping, writes inside transactions | `crates/infra-postgres/src/pool.rs`; about 233 commit sites and about 55 pool-level writes in `crates/infra-postgres`, `crates/application` and `crates/scheduler`; `crates/application/src/error.rs` (`StoreError`, `kind()`, `AppError`); `crates/infra-postgres/src/identity.rs` (`record()`); `crates/api-http` and `crates/api-grpc` error mapping | 048-012 |
| `25006` by the transaction's mode, `57P04`; the `after_connect` checks; `synchronous_commit` pinned | `identity.rs`, `instances.rs` (`registry_error`), `pool.rs` | 048-011 |
| The identity, the confirmation, the discard of older connections, the gate's reasons | `pool.rs` (`after_connect`, `before_acquire`), `crates/application/src/instances.rs` (the gate as a set; the confirmation task), `crates/infra-postgres/src/instances.rs` (the beacon transaction) | 048-013, 048-014 |
| The report window | `sdks/SPEC.md` and the six SDKs; `tests/contract/sdk-conformance/scenarios/report-through-failover.json` | 048-015 |
| The read pool and the one read | `apps/control-plane/src/config.rs`, `apps/execution-engine/src/settings.rs`, `crates/infra-postgres/src/stale_reads.rs`, the events route of `crates/api-http` | 048-021 |

**The new tables**, each created by an expand migration (design 053) with its grants and storage classification:

| Table | Columns | Runtime role |
|---|---|---|
| `durability_beacons` | `instance_id uuid primary key`, `written_at timestamptz` | `SELECT (instance_id)`, `INSERT`, `UPDATE` (an upsert needs the select on its conflict column) |
| `server_incarnation` (unlogged) | `id smallint primary key check (id = 1)`, `token uuid not null` | `SELECT`; written only through `sagat_server_identity()` |

**Why one waited commit protects what came before it.** A standby receives WAL in order; a commit returns only after a
standby has flushed WAL up to that commit's position, which lies after every earlier commit's record. So once a
process's beacon commit returns, every transaction committed before it, including one a crashed primary marked committed
on restart, is on a standby. A transaction must write a row for its commit to wait: one that only took a transaction
identifier writes no WAL and does not wait; the beacon's upsert is that row. The residual: a commit exposed by an
operator's cancel or termination of a waiting backend, or by the HA manager's fast shutdown of an isolated primary (whose
sessions it terminates too), is visible for a moment to sessions of that server not yet ended; the platform's code never
cancels (the architecture check), the runbook forbids it, and `database_backends_waiting_for_standby` counts the waiting
backends. A connection kept to a demoted or isolated former primary may answer reads until the first new connection of
its process observes the new identity or a write meets `25006`; those reads are of committed state and decide nothing,
since every decision is a write that such a server refuses or holds waiting.

### The drills

The Compose drill is `infra/dr/ha-drill.sh`; the Kubernetes drill is `tests/e2e/high-availability/run.sh`. Each fault is a
command in the script, the driver's per-second counter and ledger are the measurement, and a repetition is 6 minutes (2
before the fault, 4 after). The fault tests of the platform's side run on a new test fixture,
`tests/support/replicated_postgres.rs`: a primary and one synchronous standby started by Testcontainers on a network of
their own, each test first asserting `pg_stat_replication.sync_state = 'sync'`. A dead peer is made by `docker network
disconnect` of the primary; the test connects to the container's address on the bridge network, not to a published port,
because Docker's port proxy would answer the probes itself, so the dead-peer test runs on Linux only and prints that it
was skipped elsewhere. Races use barriers.

**Metrics and alerts.** `database_connections_refused_total{reason}`, `database_commit_outcome_unknown_total`,
`database_commits_detached` (a gauge), `database_backends_waiting_for_standby` (a gauge exported by the engine's monitor
under its lease, from the runtime role's own sessions in `pg_stat_activity`, which need no extra grant),
`serving_gate_closed{reason}` (a gauge); from the HA manager's exporter, the synchronous standbys, with the alerts
`SagatDatabaseOneSyncStandby` (warning) and `SagatDatabaseNoSyncStandby` (critical, writes stopped), whose rule tests join
CI's `promtool` list. Labels are closed sets.

**Order of the build.** Task 048-2 (the Compose topology) first; design 053's lint before the first migration of Task
048-7; design 051's driver (its Task 051-2) before Task 048-13.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A commit waits for ever when both standbys are gone | By design (RPO 0); the commit deadline reports an unknown outcome; the detach with its cap; one confirmation at a time; `reserved_connections`; the alerts |
| An operator exposes an unreplicated commit by cancelling a waiting backend | Forbidden in the runbook and in code; the metric; the residual bounded by the connection lifetime |
| A role or database setting weakens `synchronous_commit` | The startup option and the check in `after_connect` |
| A failover longer than a lease's remainder | The time to live sized by the runbook's rule; the drills count expired leases and review items; lease continuity is a follow-up design |
| The server's timeouts do cancel a waiting commit | Task 048-6 establishes it first; the alternative of Requirement 048-012 is then built |
| An unlogged table is not emptied as expected | Task 048-7 asserts it first, for a backend killed and for a promotion |
| The vendored `sqlx-core` drifts from upstream | One change, verified by `scripts/check-vendor.py`, offered upstream; removed when upstream has the options |
| CloudNativePG or Patroni has a bug | Versions pinned; the drills on every release; the platform's oracle is the judge |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No commit is read before a standby holds it except after an operator's forbidden act (048-012, 048-013); leases, tokens and generations are unchanged; one transaction per transition is unchanged |
| II. Modular domain, ports and adapters | Pass | The gate's reasons and the confirmation are in the application's instance service; statements stay in `infra-postgres`; the domain is untouched |
| III. External effects are at-least-once; sagas compensate | Pass | The failover rules reduce repeats and review items but promise none; idempotency keys still absorb repeats |
| IV. Compatibility is versioned and pinned | Pass | The reason, the gate reason, the header and the SDK window added to their contracts; images pinned by digest; migrations follow design 053 |
| V. Code, tenant boundary and secrets are isolated | Pass | Authorization, misses and secrets stay on the primary; the read header carries no value; the database's control network is closed; the platform runs as `sagat_app`, never as the owner; the operator namespace-scoped |
| VI. Observability and audit are part of the feature | Pass | The metrics and alerts above |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | A duplicate answered after a failover reads only what a standby holds (048-013) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Ten drill faults, a negative control, two proving tests first, fault tests on a replicated fixture; every report names its runner |
| Delivery Constraints | Pass | "Single region with high availability" is what this design builds (constitution 5.0.0 keeps it per cell, Owner decision 2026-11-24-2) |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Two reference topologies | CloudNativePG runs only on Kubernetes; developers and design 051 need one on Compose | One topology: no failover on a laptop, or a Kubernetes-only drill |
| `commit_safely`, `execute_safely` and the commit deadline | A commit to a dead server would wait for the kernel; an unknown outcome must not be reported as "not applied" | Dropping the commit future: leaves the connection in an unknown state |
| A vendored `sqlx-core` with socket keepalive | A dead peer with acknowledged data would hold a connection for ever | A wrapper around every statement: about 233 sites and borrowed executors |
| The identity with an unlogged marker, and the confirmation | A commit a restarted primary marked committed must not be read before a standby holds it, and the start time misses a crash restart in place | Trusting `synchronous_commit = on`; the start time alone |

## Alternatives Considered

- **Leave HA to the operator** (today). Every operator rebuilds the same topology and the platform's side stays untested.
- **Several hosts in the URL with `target_session_attrs=read-write`.** The driver does not support it.
- **Asynchronous standbys only.** Faster commits, RPO above zero.
- **A client bound on every statement by a wrapper.** About 233 sites and borrowed executors; socket keepalive bounds
  every statement at once, and only the commit needs its own deadline.
- **The kernel's retransmission limit (`tcp_retries2`).** Bounds unacknowledged data only, and on Kubernetes needs an
  unsafe-sysctl allowance on the nodes.
- **A beacon on every new connection.** It runs inside the pool's acquire timeout, and every attempt that times out
  leaves a waiting backend behind; one confirmation per process after a change of identity does not.
- **The WAL of the former primary read with `pg_waldump`.** It does not decode rows, so it cannot be matched with the
  driver's acknowledgments; a copy of the data directory started alone can be queried.
- **Extending leases after a restart or promotion.** A design of its own (Non-Goals).

## Non-Functional Requirements

- **Non-functional requirement 048-001 Hot path.** A lease and a completion gain no statement: `commit_safely` changes how
  the commit is awaited, not what is sent (unless Task 048-6 proves the server cancels waits, in which case the one-message
  commit adds a statement, recorded in design 038's budget). The identity read runs once per new connection; the
  beacon once per confirmation.
- **Non-functional requirement 048-002 Recovery.** As Requirement 048-040.
- **Non-functional requirement 048-003 Security.** The read address uses the runtime role with read-only sessions; the HA
  manager's roles and credentials are never the platform's; the Compose reference's credentials come from `.env`;
  `sagat_server_identity()` writes only its one row.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Each task names its files and tests; the metrics of a behaviour come with it.

- [ ] T048-1 Revise Architecture decision 0130 to this text; add its refinement note to Architecture decision 0082; write the topology and the one stale-tolerant read in `docs/architecture/storage-and-events.md` and `docs/runbooks/disaster-recovery.md`, and the lease time-to-live rule in `docs/runbooks/scaling.md`; keep feature 032's plan and tasks consistent (Task T032-058 builds on Task 048-7's `durability_beacons`).
  Refs: Requirement 048-001, Requirement 048-013, Requirement 048-016, Requirement 048-021
- [ ] T048-2 Add `infra/docker/compose.ha.yaml` and `infra/docker/patroni/Dockerfile` (PostgreSQL 16 by digest, Patroni 4, etcd and HAProxy pinned), the internal control network, the authentications from `.env`, HAProxy's checks and session shutdown, healthchecks.
  Refs: Requirement 048-002, Requirement 048-003, Requirement 048-005
- [ ] T048-3 Add `infra/kubernetes/ha/` (the operator's values for one namespace, the `Cluster` with managed roles, reserved connections, the connection limit, backups, the network policy), the chart's `topologySpread` values, a chart check that the role's connection limit covers (the autoscaler's ceiling + the surge) × (twice the request pool + the registry pool + the confirmation connection) + the jobs + a margin, `infra/helm/examples/ha-cloudnativepg.yaml`, and a check that every image of both references is pinned by digest and the example's user is `sagat_app`.
  Refs: Requirement 048-001, Requirement 048-004, Requirement 048-012
- [ ] T048-4 Vendor `sqlx-core` with the keepalive and `TCP_USER_TIMEOUT` change, extend `scripts/check-vendor.py`, and add the fault test `a_dead_peer_with_acknowledged_data_fails_within_ten_seconds` on the replicated fixture (`docker network disconnect` after the commit was acknowledged; the failure asserted within 12 s, the 10 s bound plus one probe interval for the kernel's timer).
  Refs: Requirement 048-005, Requirement 048-010
- [ ] T048-5 Classify `25006` by the transaction's mode and `57P04` in `identity.rs` and `instances.rs`, add the `after_connect` checks (recovery, read-only, `synchronous_commit`, the optional synchronous-standby check), the startup option and its architecture rule, and the fault tests `a_promoted_standby_with_the_old_primary_read_only_is_never_written_to` and `a_role_setting_of_synchronous_commit_local_is_refused`, with `database_connections_refused_total`.
  Refs: Requirement 048-011
- [ ] T048-6 First, the proving test `does_a_server_timeout_cancel_a_commit_waiting_for_a_standby` on the replicated fixture with the standby stopped, on PostgreSQL 16 and 17, asserting that `COMMIT` has not returned at the timeout plus 2 s; record its result in this design. Then `commit_safely` and `execute_safely` with the deadline, the outcome rule, the detach and its cap, the migration of every commit and every pool-level write, `CommitOutcomeUnknown`, the reason in the API and gRPC mapping and the error contracts, the architecture checks, and the tests `a_commit_past_its_deadline_is_unknown_and_invisible`, `waiting_commits_do_not_exhaust_the_pool` and `an_unlisted_failure_after_commit_was_sent_is_unknown`, with `database_commit_outcome_unknown_total` and `database_commits_detached`.
  Refs: Requirement 048-012, Non-functional requirement 048-001
- [ ] T048-7 First, the proving test `a_backend_killed_empties_the_unlogged_marker_and_keeps_the_start_time` (and the same after a promotion) on the replicated fixture. Then `server_incarnation`, `sagat_server_identity()`, `durability_beacons`, the identity read in `after_connect`, the confirmation task with its compare and clear, the discard in `before_acquire`, the fast refusal while unconfirmed, the confirmation of the one-run commands, and the gate reason `DATABASE_UNCONFIRMED`, with the tests `a_primary_restarted_in_place_is_not_read_before_its_standby_catches_up`, `a_confirmation_overtaken_by_a_second_restart_does_not_open_the_gate` and `one_beacon_waits_per_process`, run as the runtime role, with `database_backends_waiting_for_standby`.
  Refs: Requirement 048-013
- [ ] T048-8 Make the serving gate a set of reasons in `crates/application/src/instances.rs`, with the test `a_re_registration_does_not_reopen_a_gate_another_reason_holds`.
  Refs: Requirement 048-014
- [ ] T048-9 Add the report retry window with its clamped last attempt and the heartbeat rule to `sdks/SPEC.md` and the six SDKs, and the conformance scenario `report-through-failover` with its negative case.
  Refs: Requirement 048-015, Requirement 048-016
- [ ] T048-10 Add `DATABASE_READ_URL`, `StaleReadPool` in `crates/infra-postgres/src/stale_reads.rs`, the events read bounded by the sequence read on the primary and `Sagat-Read-Source`, the architecture rule, the header in `specs/001-platform-foundation/contracts/openapi.yaml`, and the tests `another_tenants_run_gets_the_same_404_without_the_header` and `a_standby_behind_the_primarys_sequence_does_not_serve`.
  Refs: Requirement 048-020, Requirement 048-021
- [ ] T048-11 Add the two database alerts with their rule tests in CI's `promtool` list.
  Refs: Requirement 048-012, Requirement 048-013
- [ ] T048-12 Write the Kubernetes drill in `tests/e2e/high-availability/run.sh` with the comparison on a copy of the former primary and its negative control, run it, and publish its section of `tests/performance/reports/high-availability.md`.
  Refs: Requirement 048-041, Requirement 048-042, Requirement 048-043, Success criterion 048-001, Success criterion 048-002
  Owner-gate: pending, the owner's row paying for the drill's runner of "Owner decisions needed" (Owner decision 2026-11-25-1: nothing paid in 0.3.0)
- [ ] T048-13 Write `infra/dr/ha-drill.sh` on the Compose reference with the comparison on a copy of the former primary and its negative control, run it on a free standard runner in the `scale` job and the release workflow, and publish its section with the runner named; it uses design 051's driver (Task 051-2).
  Refs: Requirement 048-040, Requirement 048-041, Requirement 048-042, Requirement 048-043, Success criterion 048-003

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs the suites of what it touches, `integration_claim_transition_efficiency` when it touches
`crates/infra-postgres/`, the fault suite of feature 010 (`tests/fault/failover`), the new fault tests on the replicated
fixture, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and the documentation and architecture
checkers of `scripts/`. Each named test of Tasks 048-4 to 048-10 is seen failing first, by a hand mutation where the
behaviour exists or before the change where it does not; those of Tasks 048-7 and 048-8 also run with
`SAGAT_TEST_AS_RUNTIME_ROLE=1`. Task 048-9 runs the SDK conformance scenario in all six languages. Tasks 048-12 and
048-13 are the gate for recovery.
