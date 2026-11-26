# A reference high-availability topology for PostgreSQL, and the platform's side of a failover

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

PostgreSQL is the platform's only authority. Today its high availability is left to the operator, and a promotion and
the switch of the address the platform connects to are manual. This design makes both part of the product, inside one
region:

1. **A reference topology** that the repository runs and tests: Patroni on Docker Compose (`infra/docker/compose.ha.yaml`),
   which gates every release, and CloudNativePG on Kubernetes (`infra/kubernetes/ha/`, with the platform's chart in
   `infra/helm`), built and smoke-tested but not offered for production until its full drill has run. Both keep a
   primary and two standbys with synchronous replication (a commit returns when either standby has it), promote a
   standby without a person, and move the one address the platform uses to the new primary.
2. **The platform's side of a failover**: dead connections are found within 10 s by socket keepalive; every write goes
   through one transaction type whose commit is bounded and whose unknown outcome is reported as such; the platform
   never cancels a commit that waits for a standby; after a database restart (a crash restart in place included), a
   promotion or an unknown outcome, a process serves nothing until one write of its own has reached a standby; worker
   reports and heartbeats keep retrying through a failover.
3. **Evidence**: a disruption and failover drill on Compose that gates every release on a free runner, a smoke failover
   on a free Kubernetes runner, and a three-zone Kubernetes drill that stays unscheduled while the owner pays for no
   runner (Owner decision 2026-11-25-1), all measured against the owner's objectives (RPO 0 for acknowledged work,
   failover within 5 minutes, Owner decision 2026-10-16, row 8).

Rolling upgrades are design 053 (split from this design, Owner decision 2026-11-24-4). Cross-region failover is feature
032 (Architecture decision 0067), which builds on this design. Reads from a standby, which an earlier draft offered for
one route, are left out: they gain no availability and feature 032 refuses stale reads (its decision D1580).

**Terms used below.** A *transition* is one step's result committed by the platform. A *lease* is a worker's right to
execute one attempt of a step until an expiry on the database clock, fenced by a token and a generation; a *review item*
is work that waits for a person because its outcome is unknown (feature 037). The scheduler's work is divided into 64
*shards* held by engines under *shard leases* of 15 s; the *keeper* renews them, waiting up to a fifth of the lease plus
20% (3.6 s) between rounds (`crates/scheduler/src/shards.rs`), and the *lost-lease pass* reclaims expired worker leases.
The *instance registry* (Architecture decision 0082) records the live processes; its *serving gate* closes a process
that must not serve; the registry *heartbeat* (every 10 s) is a write. A *command identifier* or *idempotency key* lets
a client repeat a request safely. `begin_safely` is today's way to open a transaction (Architecture decision 0014). The
*integrity oracle* is the check of `tests/performance/scalability-ha/` that no acknowledged run was lost, no lease was
granted twice, and replaying each run's journal gives its projection. A server's *identity* is the pair of its
postmaster's start time and the token of its incarnation marker (Requirement 048-013). The *fork* is the point where a
promoted standby's history leaves the former primary's.

## The Problem

From the code and documents on `develop` at `546d5aa5`:

- **No topology.** Every environment the repository defines runs one PostgreSQL container (`infra/docker/compose.yaml`,
  `postgres:16-alpine`, not pinned by digest, on the default network). The chart does not run PostgreSQL
  (`docs/runbooks/scaling.md`, section 4). The Kubernetes validation runs a single PostgreSQL Deployment
  (`tests/e2e/scalability-ha/kubernetes/dependencies.yaml`).
- **Failover is a manual procedure.** `docs/runbooks/disaster-recovery.md` says to fence the old primary, run
  `pg_ctl promote`, then point the platform at the new one. The drill (`infra/dr/drill.sh`) promotes a standby on
  loopback but never points the platform at it, and adds a 15 s shard takeover as a constant (`infra/dr/drill.sh:45`).
  Gaps 250 and 251 of feature 010 record what was not covered.
- **The platform is not ready for a failover.**
  - One `DATABASE_URL` per process. Pools connect eagerly at start (`crates/infra-postgres/src/pool.rs`), set
    `acquire_timeout` 3 s and `statement_timeout` 5 s, and sqlx 0.9 opens plain TCP sockets with no keepalive, so a
    statement whose bytes a vanished server acknowledged waits for ever and unacknowledged bytes wait for the kernel's
    retransmission timeout (about 15 minutes).
  - Every write runs in a `sqlx::Transaction` or as a statement on the pool. sqlx does not expose `COMMIT`'s answer
    beyond success or error, cannot hand back a transaction's `PoolConnection`, and stamps a connection's creation after
    its `after_connect` hook returns.
  - SQLSTATE `25006` (`read_only_sql_transaction`) is classified as internal (`crates/infra-postgres/src/identity.rs`)
    and as "other" by the registry (`crates/infra-postgres/src/instances.rs`); class 57 is all retryable, `57P04`
    included. Five modules have their own `CommitOutcomeUnknown` (Architecture decision 0078), and
    `confirmed_abort` (`capacity_finish.rs`) treats only class 23, `40001` and `40P01` as a definite rollback.
  - What happens to a commit that waits for a standby is not established; feature 032 lists it as unproven (its gap
    1578). What is known: an explicit cancel or termination of a waiting backend, and a primary that restarts after a
    crash, make a commit visible that no standby holds. A crash restart in place keeps `pg_postmaster_start_time()`.
  - Nothing pins `synchronous_commit`: a role's or a database's setting of `local` would void RPO 0 silently.
  - The SDKs make 4 attempts of a failed call within about 1.4 s, `CompleteStep` up to 5 within about 3 s, and a failed
    heartbeat waits for the next one 10 s later (`sdks/SPEC.md`); worker leases last 30 s.
  - Readiness (`crates/api-http/src/routes/health.rs`) pings storage, then checks the gate, and fails when the registry
    has been silent for 30 s (Architecture decision 0082, decision 5); the chart probes it every 5 s, failure threshold 2.
- **The chart spreads but does not insist.** Topology spread is hard-coded `whenUnsatisfiable: ScheduleAnyway`
  (`infra/helm/templates/_helpers.tpl`).

## Requirements

### The reference topology

- **Requirement 048-001 Kubernetes reference.** `infra/kubernetes/ha/` MUST hold a CloudNativePG `Cluster` of three
  instances (`spec.instances: 3`), one per zone (required pod anti-affinity on `topology.kubernetes.io/zone`), on
  PostgreSQL 16, with `spec.postgresql.synchronous` set to `method: any`, `number: 1` and `dataDurability: required`, the
  failover quorum (in the operator release Task 048-4 pins, by the field or annotation that release documents, with its
  maturity stated), the primary isolation check (`spec.probes.liveness.isolationCheck.enabled: true`) and
  `smartShutdownTimeout: 5`; the operator installed for one namespace only; the database in a namespace apart from the
  platform's; the runtime role `sagat_app` created through the cluster's managed roles, its password Secret copied into
  the platform's namespace by the runbook's procedure (and on each rotation), never the operator's generated `-app`
  Secret; the owner role `sagat_owner` created the same way, its Secret copied only for the operator's migrate run (the
  chart has no migrate job); superuser access off; a network policy whose every peer pairs, in one `from` entry, a
  `namespaceSelector` of the platform's namespace with a `podSelector` of the control planes, the engines or the migrate
  run (`app.kubernetes.io/component`), and admits besides only the instances to each other and the operator; and
  `database-runner`'s `denyCidrs` covering the cluster's pod range (or an address pool reserved for the database) and
  its Service range, checked by Task 048-4.
  `reserved_connections` serve the operator's and monitoring's roles, and `sagat_app` has a `CONNECTION LIMIT` equal to
  the chart's connection budget (Task 048-4). Every version and image is pinned by digest. The reference is built and
  smoke-tested, and is not offered for production until the Kubernetes drill of Requirement 048-041 has passed; backups
  are the subject of `docs/runbooks/disaster-recovery.md`, not of this reference.
- **Requirement 048-002 Development reference.** `infra/docker/compose.ha.yaml`, a file of its own (not an overlay of
  `compose.yaml`), MUST start three PostgreSQL 16 nodes under Patroni 4 with `synchronous_mode: quorum`,
  `synchronous_mode_strict: true`, `synchronous_node_count: 1`, `ttl: 30`, `loop_wait: 10` and `retry_timeout: 10`; an
  etcd of three members (the shape of the production consensus store, whose loss the drill does not inject) with a root
  user and a Patroni user limited to its key prefix; and an HAProxy that sends the platform's one address to the current
  primary (`GET /primary`) and ends a demoted node's sessions. Three networks: `db-control` (`internal: true`: etcd,
  Patroni's REST interface with its `allowlist`, the nodes' replication), carrying etcd's traffic in plaintext inside
  that network only; `db-access` (HAProxy and the nodes' client port); and the platform's, from which only HAProxy's
  port is reachable. `pg_hba` allows replication only from the node addresses, the superuser only on the local socket,
  and through HAProxy only `sagat_app` and the owner with `scram-sha-256`; it ends with `include_if_exists` of a node-local file that only the drill writes (Requirement 048-042). Every password (Patroni's roles, etcd's
  users, the REST authentication) comes from `.env` (`${VAR:?}`), listed in `.env.example`; no host port is published
  for the nodes, etcd or Patroni's REST interface; the platform's ports are on the loopback; images are pinned by digest;
  every service has a healthcheck (`AGENTS.md`, rule 16).
- **Requirement 048-003 One address.** The platform MUST connect through one address that the HA manager moves to the
  primary. No platform process may need a person, a restart or new configuration after a promotion.
- **Requirement 048-004 Spread that insists.** The chart MUST expose `topologySpread.whenUnsatisfiable` (default
  `ScheduleAnyway`) and `topologySpread.nodeTaintsPolicy` (default `Ignore`) in `infra/helm/values.yaml`; the example
  `infra/helm/examples/ha-cloudnativepg.yaml` sets `DoNotSchedule`, `Honor` and `SAGAT_LEASE_TTL_SECS: 90`
  (Requirement 048-016). The existing disruption budgets stay.
- **Requirement 048-005 Socket keepalive.** Every database socket of the platform MUST set `SO_KEEPALIVE` with an idle
  time of 4 s, an interval of 2 s and 3 probes, and, on Linux, `TCP_USER_TIMEOUT` of 10 s. A peer that stopped answering
  is then found within 10 s of the last packet it sent: with nothing unacknowledged, the third probe goes unanswered at
  4 + 2 × 3 = 10 s, and on Linux the user timeout ends the connection at the first probe tick at or after 10 s, the same
  10 s; with bytes unacknowledged, Linux ends it when they have waited 10 s (the retransmission timer is clamped to the
  user timeout since Linux 4.19). sqlx 0.9 has neither option, so the repository vendors `sqlx-core` with this change,
  which also adds the `socket2` dependency to its manifest (as it vendors `rskafka`), verified by
  `scripts/check-vendor.py` and offered upstream. A live server that is only slow, a commit waiting for a standby
  among them, answers the probes. On Compose the platform's connection ends at HAProxy, which checks each node every
  second (`inter 1s fall 3`), so a dead primary is marked down within 3 s and its sessions are ended
  (`on-marked-down shutdown-sessions`); HAProxy sets no timeout of its own on these connections. On Kubernetes the
  address is a Service translated by the node's kernel, so the socket reaches the database's own kernel.

### The platform's side of a failover

- **Requirement 048-010 Dead connections are found.** A statement or a commit to a server that stopped answering MUST
  fail within 10 s (Requirement 048-005); a statement to a live server is bounded by its `statement_timeout` (5 s). Idle
  connections are tested before reuse (sqlx's `test_before_acquire`, its default).
- **Requirement 048-011 Every connection is checked, and writes are explicit.** Every database connection of the
  platform (the pools of the control plane and the engine, the registry's, and every command of `control-plane`) MUST
  send, as startup options, `synchronous_commit=on` and `default_transaction_read_only=on`, which outrank a role's or a
  database's setting; `scripts/check-architecture.py` refuses `synchronous_commit`, `set_config` and
  `default_transaction_read_only` in any statement of `crates/infra-postgres`, `apps/` and the migrations. Its
  `after_connect` calls `sagat_server_identity()` (Requirement 048-013) and refuses the connection when the server is in
  recovery, when `synchronous_commit` is not `on`, or, when `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY` is true, when
  `synchronous_standby_names` is empty. That setting defaults to true in the binary; `infra/docker/compose.yaml`, which
  runs one database for development, sets it false, and a process that runs with it false logs at start, and exports
  `database_synchronous_standby_required 0`, that RPO 0 does not hold. Every write MUST go through `WriteTx`
  (Requirement 048-012), whose `BEGIN READ WRITE` is the only way to write on these sessions: any other write fails at
  once with `25006`, which is classified as internal, a defect. A `25006` inside a `WriteTx` (in recovery, `BEGIN READ
  WRITE` itself fails) is classified as unavailable and retryable; the `WriteTx` closes its connection
  (`PoolConnection::close_on_drop`) and the process treats it as a change of identity (Requirement 048-013). `57P04` is
  not retryable. The registry re-reads `synchronous_standby_names` with each heartbeat and closes the serving gate with
  the reason `DATABASE_NOT_SYNCHRONOUS` while it is empty and required; it learns of an emptied setting within one
  heartbeat (10 s, jittered to 12 s), and when the reason clears it sets `DATABASE_UNCONFIRMED`, so that what was
  committed locally in that window is served only after a confirmation.
- **Requirement 048-012 Commits.** `WriteTx`, in `crates/infra-postgres/src/pool.rs`, owns a `PoolConnection`, sends
  `BEGIN READ WRITE`, runs its statements on that connection, and has no way to commit but `commit_safely`. If any of its
  statements fails, the `WriteTx` rolls back and the caller gets that error, a definite failure. `commit_safely` sends
  `COMMIT` in a task it spawns (as `begin_safely` does for `BEGIN`) and waits for it at most the commit deadline
  (`SAGAT_DB_COMMIT_DEADLINE_MS`, default 10,000, at least twice the statement timeout, the name feature 032 also uses).
  The outcome is applied when `COMMIT` succeeds; not applied when the server answers it with an error of severity
  `ERROR` on a session that stays alive, since PostgreSQL raises every error of a commit (deferred constraints, a
  serialization failure, the deferred quota triggers' `54000`) before it writes the commit record and turns any failure
  after it into a `PANIC`. A cancel that reaches a commit while it waits for a standby is different: PostgreSQL answers
  success with a `WARNING`, the commit written locally only. The platform never sends one (the rules below), Task 048-6
  establishes whether its statement timeout can (and if it can, `commit_safely` turns it off for the commit), and an
  operator's cancel is the residual of the Technical Plan; Task 048-6 records that answer too (this generalizes `confirmed_abort` of `capacity_finish.rs` for the commit itself, and
  keeps today's mapping of `54000` to `QuotaExceeded`); and unknown otherwise: the deadline passing, an I/O error, a
  `FATAL` or `PANIC`, a closed pool or a task that panicked are `StoreError::CommitOutcomeUnknown`, which replaces the five module-level variants of the same
  name (Architecture decision 0078). After an unknown outcome the task keeps the connection, and its slot, until the
  server answers or the socket fails, and the process closes its serving gate (Requirement 048-013), so that no new
  write starts and the slots held are those of the commits in flight. The API answers `UNAVAILABLE` with the reason
  `COMMIT_OUTCOME_UNKNOWN`; `AppError::Unavailable` gains a reason field for it and for the reasons of Requirement
  048-014; whether to retry is the client's, which retries only an operation that carries an idempotency key or a
  command identifier (the SDKs always send one on reports). The platform never cancels or terminates a backend that waits
  for a standby: the sessions set `client_connection_check_interval` to 0 and, where the server has it,
  `transaction_timeout` to 0, and `scripts/check-architecture.py` refuses, in `crates/infra-postgres` and `apps/`,
  `pg_cancel_backend`, `pg_terminate_backend`, a `sqlx::Transaction` or `.begin()` outside `pool.rs`, a raw `BEGIN`,
  `START TRANSACTION`, `COMMIT`, `END`, `ABORT`, `ROLLBACK`, `READ WRITE`, `SESSION CHARACTERISTICS` or `transaction_read_only` outside `pool.rs`,
  and `fetch()` on a statement that writes. Whether the server's own timeouts can cancel the wait is established by Task 048-6 first; if
  they can, `commit_safely` sends `SET LOCAL statement_timeout = 0` and `COMMIT` in one message.
- **Requirement 048-013 After a change of server, nothing is served until a write reaches a standby.** A server's
  identity is the pair (`pg_postmaster_start_time()`, the token of `server_incarnation`). `server_incarnation` is an
  unlogged table of one row (`id smallint primary key check (id = 1)`, `token uuid not null`): PostgreSQL empties an
  unlogged table after a crash restart and at a promotion and keeps it across a clean restart, whose start time changes
  instead, so the pair changes on every restart and promotion. `sagat_server_identity()` (owner-defined, `SECURITY
  DEFINER`, `search_path` pinned to `pg_catalog, pg_temp` with every name qualified, `REVOKE ALL ... FROM PUBLIC`,
  executable by `sagat_app` and the owner) returns the start time, the token, `pg_is_in_recovery()`, `synchronous_commit`
  and whether `synchronous_standby_names` is empty. In recovery it returns a null token without reading the table (an
  unlogged table cannot be read during recovery); otherwise it reads the row, and a missing row is a null token. It
  only reads, so it runs on the read-only sessions of every connection. The token is written only inside the
  confirmation's `WriteTx`, by `instance_confirm_durability()` (below), a definer function that inserts it (`INSERT ...
  ON CONFLICT (id) DO NOTHING`, then a read); the runtime role has no privilege on the table. A null token means "unconfirmed", never an
  identity; the registry's connection, and the one connection a command of `control-plane` uses to wait for a
  confirmed identity (below), are admitted while the identity is unconfirmed, and every other connection is refused.
  The process keeps one `DatabaseIdentity` (application-owned, in `crates/application/src/instances.rs`, behind a port
  implemented in `crates/infra-postgres/src/instances.rs`): the confirmed identity, the set of identities seen
  superseded, and `identity_changed_at`. `after_connect` reports what it read; under the gate's lock, an identity equal
  to the confirmed one is accepted, one in the superseded set is refused, and any other (or a null token, or none
  confirmed) sets the reason `DATABASE_UNCONFIRMED` only if it differs from the confirmed one, records the time, adds the
  confirmed identity to the superseded set, refuses the connection and signals the confirmation task. The pools'
  `before_acquire` hook discards, without a statement, every connection created before `identity_changed_at` plus the
  connect timeout of 3 s, since a connection's creation is stamped after its `after_connect` returns. An unknown commit
  outcome (Requirement 048-012) and an unavailable `25006` (Requirement 048-011) do the same, without a new identity.
  The confirmation task lives as long as the process and waits for signals. A confirmation is one `WriteTx` on the
  registry's connection: `sagat_server_identity()`, then `instance_confirm_durability(instance, credential)`, a
  definer function that checks the process's registry credential against its `token_hash` as every registry function
  does, inserts the token when it is absent, reads the identity itself (never from its arguments) and writes it into the
  process's row of the registry (`serving_instances`) and, the first time any process confirms, into the one-row
  `registry_first_confirmation`, and returns the identity it wrote, which the process compares as the one its
  confirmation read; it is committed
  through `commit_safely` with no deadline (the keepalive bounds a dead peer, and an answer that comes late is still
  used). When it is applied, the process clears the reason, under the gate's lock, only if no identity different from
  the one it read has been observed since that read; the confirmation's own read counts as an observation. Since a
  standby receives WAL in order, everything committed before that write is then on a standby. Then the process sends a
  registry heartbeat at once, so that readiness returns. Pools are built lazily (`connect_lazy_with`); a process starts
  by registering, a write that waits for a standby, then confirms before it serves. While `DATABASE_UNCONFIRMED` is set,
  every path checks the gate before acquiring a connection (requests, gRPC worker calls, readiness, the passes and the
  keeper), and only the registry's connection connects. A command of `control-plane` that writes (`migrate`, through
  design 053's Task 053-4, `cleanup-secrets`, `provision-tenant`, `reseal-secrets`, `quota-ceiling`,
  `scheduling-weight`, `schedule-trigger-handover`, `capability-activate`) waits, up to 60 s, until a live registry row
  carries the server's current identity as confirmed, and otherwise exits with code 1 ("retry later"); it then adopts
  that identity as its own confirmed one, so that its `after_connect` refuses any connection whose identity differs. It
  proceeds without waiting only on a database where `registry_first_confirmation` is empty (a fresh one, which no
  process has ever confirmed), not on one whose processes are merely down or deregistered. `seed-dev`, `script-key`, `healthcheck`, `help` and `version` are exempt.
- **Requirement 048-014 The serving gate has reasons.** The registry's serving gate (`crates/application/src/instances.rs`)
  MUST keep its epoch and capability state and gain a set of closing reasons (`DATABASE_UNCONFIRMED`,
  `DATABASE_NOT_SYNCHRONOUS`; design 053 adds two); it is open only when the registry's state allows it and the set is
  empty. Each owner clears only its own reason; a re-registration does not reopen a gate another reason holds. Readiness
  checks the gate before it pings storage. The reasons are added to `specs/001-platform-foundation/contracts/openapi.yaml`,
  `sdks/SPEC.md` as reasons an operator reads. Every `UNAVAILABLE` that a client can reach before authentication is one
  constant answer, `Retry-After` of 2 s and no reason, whatever the cause (a closed gate, `BINARY_CAPABILITY_MISSING`
  included, a failed storage ping, a silent registry, the authenticator's failure to fetch the identity provider's keys or
  to look up a session), so that an anonymous caller learns nothing of the cause: the authenticator's refusals
  (`crates/api-http/src/error.rs`'s `AuthError` arm and `extract.rs`, shared by both transports), the webhook ingress
  (`crates/api-http/src/routes/events.rs` and `auth/webhooks.rs`, which take no caller), the
  HTTP router's `require_serving` and the public readiness `/v1/ready` (`crates/api-http/src/limits.rs`,
  `routes/health.rs`), and the gRPC admission and capability refusal, whose `REGISTRY_SILENT` moves after
  authentication (`crates/api-grpc/src/limits.rs`, `service.rs`). This changes Architecture decision 0082's decision 3,
  which shows `BINARY_CAPABILITY_MISSING` to every caller; Architecture decision 0130 records it. The reasons are given only to operators: the metric
  `serving_gate_closed{reason}`, the log, and the engine's readiness on its admin listener (`/readyz`). The registry's
  `registry_error` classifies `CommitOutcomeUnknown` and an `after_connect` refusal as a failed connection (Architecture
  decision 0082, decision 5).
- **Requirement 048-015 Reports survive a failover.** The six SDKs MUST keep retrying a `CompleteStep` or `FailStep` that
  failed as unavailable or as a transport error (a refused connection, a reset, a timeout, an ingress `502` or `503`)
  for a report retry window (`reportRetryWindow`), 310 s by default: the release gate (300 s, Requirement 048-043) plus
  the backoff cap (5 s), `Retry-After` (2 s) and the acquire timeout (3 s), 300 + 5 + 2 + 3 = 310 s. Each wait is clamped
  to the time left, and one last attempt is made when the window ends, so the outcome does not depend on jitter. A report
  that comes after its lease was reclaimed is refused with `STALE_ATTEMPT`, as today. The conformance scenario
  `report-through-failover` uses a new time-based fault action (`unavailableForMs`) and a scenario field
  (`reportRetryWindowMs`), implemented in each SDK's fault engine (`sdks/*/conformance/`), with a lease time to live of
  30 s: it answers `UNAVAILABLE` for 8 s with a window of 10 s, so the last attempt, at 10 s, lands after the outage;
  a negative case answers for 12 s, and the SDK gives up at 10 s.
- **Requirement 048-016 Leases and work in flight.** A failover MUST lose no acknowledged work. The SDKs retry a failed
  heartbeat until the lease's remaining time less 1 s, with the same clamped backoff, instead of waiting for the next
  heartbeat; a lease renewed at most 10 s apart then survives an outage, as the platform sees it, shorter than its time to
  live less one heartbeat interval: 30 − 10 = 20 s by default. The HA examples set the time to live to 90 s, sized from
  the 60 s target of Requirement 048-040 plus two heartbeat intervals and a margin of 10 s (60 + 2 × 10 + 10 = 90). After a
  confirmation the lost-lease pass waits one heartbeat interval plus the backoff cap (10 + 5 = 15 s), so that retried
  reports land before any lease is reclaimed. A claim whose commit outcome was unknown may have leased an attempt that no
  worker received; that lease expires and, without a `reconcileWith` lookup (feature 037), becomes a review item; the
  drills count these apart, and the delivery acknowledgement of protocol 1.8 (reserved by Owner decision 2026-11-21-6,
  pull request 264) is what lets such a lease be retried without a person. The SDK releases that carry Tasks 048-11 are
  recorded in `sdks/SPEC.md`'s register; older SDKs keep today's retries.
- **Requirement 048-020 Standbys serve nothing.** No statement of the platform runs on a standby.

### Evidence

- **Requirement 048-040 The failover budget and the release gate.** For the loss of one database instance the drills
  report, at the median and the maximum, the time from the fault to the first transition committed after it, the time
  until every shard is held again and the time until every process is ready. The release gate is the owner's 300 s. The
  median's target, a benchmark criterion, is 60 s. Its terms, each measured: P, from the fault until the new primary
  accepts writes (the HA manager's own detection is inside it); R, from the promotion until the surviving standby is
  synchronous to the new primary; E, from a pod's readiness until the load balancer routes to it. The platform's own
  detection (at most 10 s, 3 s through HAProxy) runs from the fault, inside P. After P + R, a process confirms within the
  3 s connect timeout (its write waits for R, already counted), sends its heartbeat at once and turns ready at the next
  probe, within 5 s; a worker's next retry comes within its 5 s backoff cap. So a worker's report reaches a committed
  transition within P + R + 3 + 5 + E + 5 = P + R + E + 13 s, and the target leaves P + R + E at most 60 − 13 = 47 s. An
  engine holds its shards again within P + R + 3 s plus one keeper wait of 3.6 s and its round; a dead engine's shards are
  free once their 15 s lease, renewed last before the fault, has expired on the database's clock.
- **Requirement 048-041 The drills.** The Compose drill (`infra/dr/ha-drill.sh`) MUST inject, three times each: (1) the
  primary's container stopped; (2) the primary's container and the platform's containers that share its host label
  stopped together (a node lost); (3) both standbys stopped, then started again after 60 s; (4) a standby's replication
  throttled (`tc` on its `db-control` interface) and then the primary stopped; (5) a backend of the primary killed
  (`kill -9`, a crash restart in place; the drill asserts that the start time stayed the same and the token changed);
  (6) the primary disconnected from `db-access` and `db-control` (`docker network disconnect`), a dead peer with
  acknowledged data. The free Kubernetes smoke (`tests/e2e/high-availability/smoke.sh`) runs on three kind nodes, one per
  zone label: one deletion of the primary's pod. The Kubernetes drill (`tests/e2e/high-availability/run.sh`) runs on six
  kind nodes in three zones: deletion of the primary's pod, a drain of its node, the loss of a zone (its nodes'
  containers stopped) and the isolation of the primary's node, three times each. All run a chain of three activities at
  a fixed open-loop rate of 20 starts per second through `dr_acked_workload` and check with `dr_verify`
  (`tests/performance/scalability-ha/dr.rs`), extended as Task 048-14 says.
- **Requirement 048-042 Measured, not estimated.** A repetition MUST run 2 minutes before the fault and 6 after it
  (360 s, above the 300 s gate plus the 30 s of recovery held and a margin of 30 s) and record: P, R and E; the times
  of Requirement 048-040; the outage, from the fault to the first transition committed after it; the time until the
  completion rate is back to 90% of its rate before the fault, held for three 10 s windows; the integrity oracle's
  results; acknowledged work lost; leases expired, delivered or not; activities run more than once; review items, those
  of claims with an unknown outcome apart. The workload's ledger records each acknowledgment with its run, step sequence,
  attempt, generation, lease token and a digest of its result, and `dr_verify` looks each up on the new primary, by key
  and content. A negative control proves the check: the drill blocks replication to both standbys, commits a ledger row
  through its own connection to the primary with `synchronous_commit=local`, stops the primary, and the check must find
  that acknowledgment missing. In the isolation fault, the drill revokes a test principal's grant on the new primary
  and checks that every request with that principal that starts after both 22 s from the isolation and the
  revocation's commit is refused or answered `UNAVAILABLE` (Technical Plan, "Why one waited write protects what came
  before it"). On Compose this fault runs one extra control plane, the probe, on a network `db-probe` (`internal: true`)
  shared only with the primary's node, through which it reaches that node's client port; the network and its one
  `pg_hba` line (`sagat_app`, `scram-sha-256`, that subnet only) live in a file of the drill
  (`infra/dr/compose.ha-probe.yaml`), never in the development reference, and `ha-drill.sh` attaches the current primary
  to it at the fault's start, so that the check exercises the bound when
  the fault disconnects the node from `db-access` and `db-control` but not from `db-probe`; the probe is started for
  this fault only, stopped after the check, and not counted among the processes that must be ready again. The
  Kubernetes drill asserts the bound too. Reports go to
  `tests/performance/reports/high-availability.md` with the runner (Owner decision 2026-11-25-1), the environment and
  every failure.
- **Requirement 048-043 The drills gate the release.** A drill MUST exit non-zero when any repetition breaks the
  integrity oracle, loses acknowledged work, leaves a process not ready, or exceeds 300 s to the first transition or to
  the 90% recovery; for fault (3), the window is counted from the standbys' return, and the drill also fails if a commit
  made while they were down becomes visible before they return. The Compose drill runs on a free standard runner in a job
  of its own (`ha-drill`, timeout 240 minutes: 6 faults × 3 repetitions × 8 minutes = 144 minutes plus the setup) and
  gates every release; the Kubernetes smoke runs on a free runner and is reported; the Kubernetes drill needs a paid
  runner and stays unscheduled while the owner pays for none (Owner decision 2026-11-25-1). `infra/dr/drill.sh` keeps its
  single-host checks, and its 15 s constant is replaced by the measured takeover.

### Success criteria

- **Success criterion 048-001** In every repetition of every Compose fault, the integrity oracle passes, acknowledged
  work lost is 0, every process is ready again within 300 s, and the negative control is found.
- **Success criterion 048-002** The median time to the first committed transition is reported for each fault; for the
  primary's container stopped and a node lost on Compose, it is within 60 s.
- **Success criterion 048-003** In the Compose drill, a failover whose outage is under 20 s expires no delivered lease,
  repeats no completed activity, and creates no review item except for a claim whose commit outcome was unknown, for
  workers on SDKs that carry Task 048-11; this is reported, and gates the release from the first release that ships
  those SDKs.

### User stories

- **User story 048-01** An operator running the Compose reference, or the Kubernetes reference once its drill has
  passed, loses a node without paging anyone for the database.
- **User story 048-02** A developer reproduces a failover on a laptop with one command.

## Goals

- A database failure inside a region is an event the platform rides through, not an incident.
- The recovery times are numbers anyone can rerun, each with the runner that measured it.

## Non-Goals

- **Rolling upgrades**: design 053.
- **Cross-region failover**, cells and the region epoch: feature 032.
- **Reads from a standby.** No route needs one, and feature 032 refuses stale reads.
- **Building an HA manager.** The platform elects nothing (Architecture decision 0067, decision 3).
- **A connection pooler in the reference.** The platform sets session options at connection and uses prepared
  statements.
- **Backups.** They are `docs/runbooks/disaster-recovery.md`'s, not this reference's.
- **High availability of the object store, the identity provider, OpenBao and the notification broker.**
- **Carrying leases across a failover longer than their remainder**: a follow-up design; until then the time to live is
  sized to the expected failover (Requirement 048-016).
- **A claim on the performance of competitors** (Owner decision 2026-11-25-1).

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6, with its reasoning.

1. **CloudNativePG or Patroni on Kubernetes?** CloudNativePG: it moves the read-write Service itself and needs no external
   consensus store. Patroni stays the reference outside Kubernetes. Delegate.
2. **Refuse to commit without a standby, or fall back?** Refuse. An operator may run without a synchronous standby by
   setting `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY` to false, and the platform then says, at start and in a metric, that
   RPO 0 does not hold (constitution, Delivery Constraints). Delegate.
3. **How is a commit kept from being read before a standby holds it?** The platform never cancels a waiting commit,
   bounds it on its own side and reports an unknown outcome; after a change of server, a promotion or an unknown outcome,
   a process serves nothing until a write of its own reached a standby. A waited write on every replay path, which
   Architecture decision 0067's decision 6(b) required, is replaced by this (Architecture decision 0130). Delegate.
4. **Extend leases after a restart or promotion?** Not in this design: their detection, the fencing of every reader of an
   expiry and an exactly-once extension need a design of their own. Delegate.
5. **The decision record.**
   [Architecture decision 0130](../adr/ADR-0130-postgresql-high-availability-is-a-reference-topology-with-synchronous-quorum-and-one-moving-address.md),
   which refines Architecture decisions 0082, 0078 and 0014 and supersedes in part Architecture decision 0067's
   decision 6(b) and (d) and its bullet on unlogged tables.

## Owner decisions needed

- **The Kubernetes drill's runner.** Six kind nodes with the platform need a 16-vCPU runner, US$ 0.042 per minute on
  2026-11-24 (`docs.github.com/en/billing/reference/actions-runner-pricing`); 4 faults × 3 repetitions × 8 minutes = 96
  minutes plus 30 of preparation is 126 × 0.042 = US$ 5.29 per run. The owner pays for no runner in 0.3.0 (Owner decision
  2026-11-25-1), so Task 048-17 stays unscheduled, and the Kubernetes reference is not offered for production until it
  runs.

## Critical User Journeys (CUJs)

### CUJ 1: The primary's node is lost

*   **Given**: the reference topology, runs in flight.
*   **When**: the primary's node stops.
*   **Then**: the HA manager promotes the standby that holds every acknowledged commit and moves the address; the
    platform's dead connections fail within 10 s; each process's first new connection finds the identity changed, and
    the process serves nothing, unready, until its confirmation reaches a standby; workers' reports and heartbeats retry
    until they land; no acknowledged run is lost.

### CUJ 2: Both standbys are gone

*   **Given**: a primary whose two standbys are unreachable.
*   **When**: a worker completes a step.
*   **Then**: the commit waits for a standby and stays invisible; at the commit deadline the worker gets
    `COMMIT_OUTCOME_UNKNOWN` and the process closes its gate; reads stop too, and every process turns unready within the
    registry's 30 s as its heartbeat waits as well: writes and reads stop by choice (RPO 0). When a standby returns, the
    waiting commits finish at once; each process confirms and serves again, and the worker's retry, within its report
    window, finds its completion.

### CUJ 3: A backend crashes on the primary

*   **Given**: the reference topology, runs in flight.
*   **When**: a backend is killed and the postmaster restarts in place, marking committed what was waiting for a standby.
*   **Then**: every connection breaks; the next one finds the token gone; each process serves nothing until its
    confirmation has reached a standby, which by then holds everything marked committed.

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

### The platform's side

| Change | Files | Requirement |
|---|---|---|
| Socket keepalive and `TCP_USER_TIMEOUT` | `vendor/sqlx-core` (`src/net/socket/mod.rs` and its manifest's `socket2`), the root `Cargo.toml` (`[patch.crates-io]`), `scripts/check-vendor.py` (a list of vendored crates, each with its archive hash and its allowed diff) | 048-005 |
| `WriteTx`, `commit_safely`, the outcome rule, `CommitOutcomeUnknown`, the reason on `AppError::Unavailable` and its HTTP and gRPC mapping | `crates/infra-postgres/src/pool.rs`, `capacity_finish.rs` (`confirmed_abort` moved to `pool.rs`); about 233 commit sites and about 55 pool-level writes in `crates/infra-postgres` and `crates/scheduler`; `crates/application/src/error.rs`; `crates/api-http/src/error.rs` and `crates/api-grpc` | 048-012 |
| The refusals before authentication without a reason | `crates/api-http/src/limits.rs`, `routes/health.rs`, `error.rs` (the `AuthError` arm), `extract.rs`, `routes/events.rs`, `auth/webhooks.rs`; `crates/api-grpc/src/limits.rs`, `service.rs` | 048-014 |
| Startup options, the `after_connect` checks, `25006` | `pool.rs`, `identity.rs`, `instances.rs` (`registry_error`) | 048-011 |
| The identity, the confirmation, the gate's reasons | `crates/application/src/instances.rs` (`DatabaseIdentity`, the gate, the confirmation task), `crates/infra-postgres/src/instances.rs` (the port), `pool.rs` (the hooks, lazy pools), `crates/api-http/src/routes/health.rs`, `crates/api-http/src/limits.rs` | 048-013, 048-014 |
| The report window, heartbeat retries | `sdks/SPEC.md`, the six SDKs, `tests/contract/sdk-conformance/scenarios/report-through-failover.json`, the fault engines | 048-015, 048-016 |

**New in the database**, each by a migration of design 053's rules with its grants and storage classification:
`server_incarnation` (unlogged; runtime role: none; read through `sagat_server_identity()`, written only through
`instance_confirm_durability()`); `sagat_server_identity()` and `instance_confirm_durability()` (both `SECURITY
DEFINER`, `search_path` pinned to `pg_catalog, pg_temp` with every name qualified, `REVOKE ALL ... FROM PUBLIC`,
`EXECUTE` to `sagat_app`, and the first also to the owner); `serving_instances.confirmed_server_start timestamptz` and
`confirmed_server_token uuid`, with `GRANT SELECT (confirmed_server_start, confirmed_server_token)` to `sagat_app` and no
table-level grant (the registry grants by column and leaves out `token_hash`); and the one-row
`registry_first_confirmation (only_row boolean primary key, at timestamptz)`, `GRANT SELECT` to `sagat_app`. The last
two are written only through `instance_confirm_durability()`.

**Why one waited write protects what came before it.** A standby receives WAL in order; a commit returns only after a
standby has flushed WAL up to its position, which lies after every earlier commit's record. So once a process's
confirmation returns, every transaction committed before it, including one a crashed primary marked committed on
restart, is on a standby. The confirmation writes a row of a logged table, so its commit waits. The residual: a commit
exposed by an operator's cancel or termination of a waiting backend (forbidden in code and in the runbook), or by the HA
manager's fast shutdown of an isolated primary, is visible for a moment to that server's sessions not yet ended; the
platform's processes hold such sessions only until their next commit's outcome is unknown or a new connection observes
the new identity, and close their gate then. A process that still reaches an isolated former primary refuses every
request that starts more than 22 s after the isolation with the default settings: its registry heartbeat, a write,
comes within 12 s (10 s jittered) and waits for a standby the isolated server cannot reach, so its commit deadline of
10 s makes the outcome unknown and closes the gate (12 + 10 = 22 s; raising the commit deadline lengthens it by as
much). A request already running keeps its connection until it ends, within the HTTP request timeout of 15 s
(`crates/api-http/src/lib.rs`), so no HTTP request reads from that server more than 22 + 15 = 37 s after the isolation;
a gRPC call is bounded by its own deadline; a blob transfer has none and runs until it ends, reading only the blob it
was authorized for at its start; a server-sent stream ends when the gate closes. `smartShutdownTimeout: 5` and HAProxy's session shutdown end those sessions sooner where they
apply, and the isolation drill asserts the bound.

### The drills

The Compose drill is `infra/dr/ha-drill.sh`; the smoke is `tests/e2e/high-availability/smoke.sh`; the Kubernetes drill
is `tests/e2e/high-availability/run.sh`. The fault tests of the platform's side run on a new fixture,
`tests/support/replicated_postgres.rs` (a primary and one synchronous standby by Testcontainers on a network of their
own, PostgreSQL 16 and 17 by a parameter, each test first asserting `pg_stat_replication.sync_state = 'sync'`), in a new
test target, `fault_high_availability` (`tests/Cargo.toml`). A dead peer is made by `docker network disconnect`; the
test connects to the container's bridge address, not a published port, because Docker's port proxy would answer the
probes itself, so the dead-peer test runs on Linux only and prints that it was skipped elsewhere. Races use barriers.

**Metrics and alerts.** `database_connections_refused_total{reason}`, `database_commit_outcome_unknown_total`,
`database_synchronous_standby_required`, `serving_gate_closed{reason}`. From the HA managers' exporters (Patroni's
`/metrics` on Compose, CloudNativePG's on Kubernetes), the synchronous standbys, with `SagatDatabaseOneSyncStandby`
(warning) and `SagatDatabaseNoSyncStandby` (critical: writes and reads stop while the standby is required), one rule file
per manager; the contract test of observability (`tests/contract/observability-operations/main.rs`) gains an allow-list
of the exporters' metrics.

**Order of the build.** Task 048-2 (the fixture) first; Task 048-9 (the gate's reasons) before Task 048-10 (the
identity); design 053's Task 053-2 (the lint) before the first migration of Task 048-10; Task 048-11 before Task 048-15
gates Success criterion 048-003.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A commit waits for ever when both standbys are gone | By design (RPO 0); the deadline reports an unknown outcome and closes the gate; the alerts |
| A role or database setting weakens `synchronous_commit`, or the standby names empty at run time | The startup options and the checks at connect; the registry's re-read with each heartbeat |
| A connection to a superseded server serves | The identity check at connect, the superseded set, the discard by time, the gate on an unknown outcome |
| An operator cancels a waiting backend | Forbidden in the runbook and in code |
| A failover longer than a lease's remainder | Heartbeats retried within the remainder; the time to live of the HA examples; the drills count what remains |
| The server's timeouts cancel a waiting commit | Task 048-6 establishes it first |
| An unlogged table is not emptied as expected | Task 048-10 asserts it first, for a killed backend and for a promotion |
| The vendored `sqlx-core` drifts | Verified by `scripts/check-vendor.py`; offered upstream |
| CloudNativePG's failover quorum is immature | Its release and maturity named; the reference not offered for production until its drill passes |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No commit is read before a standby holds it, except after an operator's forbidden act; one transaction per transition |
| II. Modular domain, ports and adapters | Pass | `DatabaseIdentity` and the gate are the application's, behind a port; statements stay in `infra-postgres` |
| III. External effects are at-least-once; sagas compensate | Pass | The failover rules reduce repeats and review items but promise none |
| IV. Compatibility is versioned and pinned | Pass | The reasons and the SDK window in their contracts; images pinned by digest; migrations follow design 053 |
| V. Code, tenant boundary and secrets are isolated | Pass | The database's networks, `pg_hba` and policy; secrets from `.env` and Secrets; the platform runs as `sagat_app`; a refusal before authentication shows no reason |
| VI. Observability and audit are part of the feature | Pass | The metrics and alerts above |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | A duplicate answered after a failover reads only what a standby holds |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Six faults, a negative control, two proving tests, fault tests on a replicated fixture; every report names its runner |
| Delivery Constraints | Pass | Single region with high availability; running without a synchronous standby says so |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Two reference topologies | CloudNativePG runs only on Kubernetes; developers and the release gate need one on Compose | One topology |
| `WriteTx` and `commit_safely` | A commit to a dead server would wait for the kernel, and sqlx's transaction cannot report or bound it | Dropping the commit future |
| Read-only sessions by default | A write outside `WriteTx` would bypass the commit rule; the server refuses it at once | A lint over strings |
| A vendored `sqlx-core` | A dead peer with acknowledged data would hold a connection for ever | A wrapper around every statement |
| The identity and the confirmation | The start time misses a crash restart in place; a restarted primary's commits must not be read before a standby holds them | Trusting `synchronous_commit` |

## Alternatives Considered

- **Leave HA to the operator** (today). Every operator rebuilds the same topology, untested.
- **Several hosts in the URL with `target_session_attrs=read-write`.** The driver does not support it.
- **Asynchronous standbys only.** RPO above zero.
- **A waited write on every replay path** (Architecture decision 0067's decision 6(b)). It adds a write to every
  duplicate answer; the confirmation after a change of server and the gate on an unknown outcome cover what a replay
  could read.
- **A waiting barrier on the standby's flush position, or an identity from the timeline.** The first needs a query of the
  standby on every confirmation; the timeline does not change at a crash restart in place.
- **A table of beacons** (this design's earlier draft). The registry's row already exists for every process and its
  heartbeat already writes it.
- **Detaching a waiting commit's connection** (this design's earlier draft). It doubled the connection budget and left
  backends waiting; closing the gate stops new writes instead.
- **Extending leases after a restart or promotion.** A design of its own.

## Non-Functional Requirements

- **Non-functional requirement 048-001 Hot path.** A lease and a completion gain no statement: `WriteTx` sends `BEGIN
  READ WRITE` where `begin_safely` sent `BEGIN`, and `commit_safely` changes how the commit is awaited (unless Task 048-6
  proves the server cancels waits, in which case the one-message commit is recorded in design 038's budget). The identity
  read runs once per new connection.
- **Non-functional requirement 048-002 Recovery.** As Requirement 048-040.
- **Non-functional requirement 048-003 Security.** As Requirements 048-001, 048-002 and 048-014.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Each task names its files and tests; the metrics of a behaviour come with it.

- [ ] T048-1 Revise Architecture decision 0130 to this text, with its supersession in part of Architecture decision 0067 (decision 6(b) and (d), the bullet on unlogged tables) and its refinement notes in Architecture decisions 0082, 0078 and 0014; write the topology in `docs/architecture/storage-and-events.md`, `docs/runbooks/disaster-recovery.md` (with the Secret's copy and rotation) and the lease rule in `docs/runbooks/scaling.md`; amend feature 032's plan, tasks and contracts so that they use this design's reason `DATABASE_UNCONFIRMED`, its `SAGAT_DB_COMMIT_DEADLINE_MS` and Task 048-7's commit (T032-024), allow `server_incarnation` by name in T032-009, and key T032-058's durability lease on the registry's confirmed identity.
  Refs: Requirement 048-001, Requirement 048-013, Requirement 048-016
- [ ] T048-2 Add `tests/support/replicated_postgres.rs` (a primary and a synchronous standby, PostgreSQL 16 and 17 by a parameter, the bridge address, a helper that blocks replication) and the target `fault_high_availability` in `tests/Cargo.toml`, with a test that the standby is synchronous.
  Refs: Requirement 048-010
- [ ] T048-3 Add `infra/docker/compose.ha.yaml`, `infra/docker/patroni/` (Dockerfile pinned by digest, `patroni.yml`, `pg_hba.conf`, the init of `sagat_app` and the owner), `infra/docker/haproxy/haproxy.cfg`, the etcd users, the networks and the `.env.example` keys, with a smoke test in the `scale` job that stops the primary once and checks with `dr_verify`.
  Refs: Requirement 048-002, Requirement 048-003, Requirement 048-005
- [ ] T048-4 Add `infra/kubernetes/ha/` (the operator's values for one namespace and its release pinned with the failover quorum's maturity, the `Cluster`, managed roles, reserved connections, the connection limit, the network policy) and `infra/helm/examples/ha-cloudnativepg.yaml`, the chart's `topologySpread` values, `kubeconform` and `helm template` checks in CI, a policy test of the admitted labels, a check that `database-runner`'s `denyCidrs` cover the cluster's pod range (or the database's reserved address pool) and its Service range, a check that the role's connection limit covers, summed per deployment (the control plane and the engine, each with its own autoscaler's ceiling and surge), (ceiling + surge) × 2 × (its request pool + its registry pool) + the jobs + a margin of 10 (the doubling covers the backends a pod replaced while the standbys are down leaves waiting, the registry's confirmation among them, which hold their slots until the standbys return, as the runbook says), and a check that every image is pinned by digest.
  Refs: Requirement 048-001, Requirement 048-004
- [ ] T048-5 Vendor `sqlx-core` with the keepalive and `TCP_USER_TIMEOUT` change and `socket2` in its manifest, generalize `scripts/check-vendor.py` to a list of vendored crates with their archive hashes and allowed diffs, and add `a_dead_peer_with_acknowledged_data_fails_within_ten_seconds` to `fault_high_availability` (asserted within 12 s: the 10 s bound plus one probe interval for the kernel's timer).
  Refs: Requirement 048-005, Requirement 048-010
- [ ] T048-6 The proving test `does_a_server_timeout_cancel_a_commit_waiting_for_a_standby` in `fault_high_availability`, with the standby blocked, on PostgreSQL 16 and 17, asserting that `COMMIT` has not returned at the timeout plus 2 s; record its result in this design.
  Refs: Requirement 048-012
- [ ] T048-7 Add `WriteTx` and `commit_safely` in `pool.rs` with the outcome rule (`confirmed_abort` moved there), `StoreError::CommitOutcomeUnknown` replacing the five module variants, the reason on `AppError::Unavailable` and its HTTP and gRPC mapping and contracts, with `a_commit_past_its_deadline_is_unknown_and_invisible`, `an_unlisted_failure_after_commit_was_sent_is_unknown` and `a_failed_statement_rolls_back_definitely`, and `database_commit_outcome_unknown_total`.
  Refs: Requirement 048-012
- [ ] T048-8 Move every write of `crates/infra-postgres` and `crates/scheduler` to `WriteTx`, one pull request per module group; then the startup options (`synchronous_commit=on`, `default_transaction_read_only=on`), the `25006` classification and the architecture rules of Requirements 048-011 and 048-012, with `a_write_outside_write_tx_fails_at_once` and `a_role_setting_of_synchronous_commit_local_is_refused`.
  Refs: Requirement 048-011, Requirement 048-012, Non-functional requirement 048-001
- [ ] T048-9 Make the serving gate keep a set of reasons beside its epoch and capability state, make every `UNAVAILABLE` before authentication (`crates/api-http/src/limits.rs`, the public readiness, the authenticator's mapping in `crates/api-http/src/error.rs` and `extract.rs` at those call sites only, since `unavailable()` also serves after authentication, the webhook ingress in `routes/events.rs` and `auth/webhooks.rs`, `crates/api-grpc/src/limits.rs` and `service.rs`, `REGISTRY_SILENT` moved after authentication) one constant answer, `Retry-After` of 2 s and no reason, with one test asserting identical status, headers and body for each cause (a closed gate, a failed storage ping, a silent registry, a failed fetch of the identity provider's keys, a failed session lookup, a webhook whose verifier cannot fetch keys), and amend what states the old public reason (`docs/runbooks/instance-registry.md`, `docs/runbooks/control-plane.md`, `specs/001-platform-foundation/contracts/openapi.yaml`, `specs/018-schedules-bulk-operations/contracts/errors.md`, `specs/022-flow-control-step-policies/contracts/errors.md`, `tests/contract/schedules-bulk-operations/errors.rs`, `tests/integration/schedules-bulk-operations/instances_rest.rs`, `tests/integration/schedules-bulk-operations/instances_process.rs`, `tests/support/api.rs`, the doc comments of `crates/api-http/src/lib.rs` and `limits.rs`, and the hint branch of `crates/api-http/src/error.rs` that becomes dead), with a back-link in Architecture decision 0082 once Architecture decision 0130 is accepted, readiness check the gate before storage, and every path check the gate before acquiring, with `a_re_registration_does_not_reopen_a_gate_another_reason_holds` and the reasons in the contracts.
  Refs: Requirement 048-014
- [ ] T048-10 First, the proving test `a_backend_killed_empties_the_unlogged_marker_and_keeps_the_start_time` (and after a promotion). Then `server_incarnation`, `sagat_server_identity()`, `instance_confirm_durability()`, the registry's columns, `DatabaseIdentity` and its port, the hooks, lazy pools, the confirmation task, the gate on an unknown outcome and on an unavailable `25006`, the registry's re-read of the standby names, and the wait of the commands of `control-plane`, with `a_primary_restarted_in_place_is_not_read_before_its_standby_catches_up`, `a_confirmation_overtaken_by_a_second_change_does_not_open_the_gate`, `a_connection_to_a_superseded_server_is_refused`, `a_process_starts_and_confirms_on_a_lazy_pool` and `the_standby_names_emptied_at_run_time_close_the_gate`, run as the runtime role.
  Refs: Requirement 048-011, Requirement 048-013, Requirement 048-014
- [ ] T048-11 Add the report retry window, the heartbeat retry within the lease's remainder and the register of SDK releases to `sdks/SPEC.md` and the six SDKs, the fault action `unavailableForMs` and the field `reportRetryWindowMs` in the six fault engines, and the scenario `report-through-failover` with its negative case.
  Refs: Requirement 048-015, Requirement 048-016
- [ ] T048-12 Hold the lost-lease pass for 15 s after a confirmation (`crates/scheduler`), with `the_lost_lease_pass_waits_after_a_confirmation`, and the HA examples' lease time to live.
  Refs: Requirement 048-016
- [ ] T048-13 Add the two alerts with one rule file per manager, their rule tests in CI's `promtool` step, and the exporters' allow-list in the observability contract test.
  Refs: Requirement 048-012, Requirement 048-013
- [ ] T048-14 Extend `dr_acked_workload` and `dr_verify` with the ledger's key and content and the open-loop rate, write `infra/dr/ha-drill.sh` with the six faults, the negative control and the revocation check (with the probe's `infra/dr/compose.ha-probe.yaml`, whose `pg_hba` line the drill writes to a node-local file that the reference's `pg_hba` includes with `include_if_exists`, placed before any line that rejects (a missing file changes nothing), and removes in a `trap` that then reloads PostgreSQL and asserts that a connection from the probe's subnet is refused, so that an aborted drill leaves nothing behind; the drill also asserts, as a positive control, that the probe answered at least one authorized request through `db-probe` after the isolation began), the `ha-drill` job on a free standard runner in the release workflow, replace `drill.sh`'s constant, and publish its section of the report with the runner named.
  Refs: Requirement 048-040, Requirement 048-041, Requirement 048-042, Requirement 048-043, Success criterion 048-001, Success criterion 048-002, Success criterion 048-003
- [ ] T048-15 Write `tests/e2e/high-availability/smoke.sh` on three kind nodes and run it on a free runner, publishing its result as a smoke.
  Refs: Requirement 048-041
- [ ] T048-16 Write the Kubernetes drill `tests/e2e/high-availability/run.sh` with the same checks.
  Refs: Requirement 048-041, Requirement 048-042
- [ ] T048-17 Run the Kubernetes drill and publish its section of the report.
  Refs: Requirement 048-043, Success criterion 048-002
  Owner-gate: pending, the owner's row paying for the drill's runner of "Owner decisions needed" (Owner decision 2026-11-25-1: nothing paid in 0.3.0)

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs the suites of what it touches as `scripts/select-test-targets.py` selects them (the new
`fault_high_availability` when it touches `crates/infra-postgres/src/pool.rs`, `instances.rs` or `identity.rs`),
`integration_claim_transition_efficiency` when it touches `crates/infra-postgres/`, `cargo clippy --workspace
--all-targets --locked -- -D warnings`, and the documentation and architecture checkers of `scripts/`. Each named test
of Tasks 048-5 to 048-12 is seen failing first, by a hand mutation where the behaviour exists or before the change where
it does not; those of Tasks 048-8 to 048-10 also run with `SAGAT_TEST_AS_RUNTIME_ROLE=1`. Task 048-11 runs the SDK
conformance scenario in all six languages. Task 048-14 is the gate for recovery.
