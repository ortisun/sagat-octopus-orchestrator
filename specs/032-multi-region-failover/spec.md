# Feature Specification: Multi-region replication and active-passive failover

**Feature Branch**: `feature/032-multi-region-failover`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: Active-passive cross-region replication and failover: PostgreSQL physical replication,
object-storage replication, NATS as a regional hint only, namespace region placement, failover with region
epochs and fencing (extending ADR-0020) so that two regions never both acknowledge work for the same
data, RPO/RTO as benchmark criteria, routing of clients, workers and webhook senders, inbox/outbox
behaviour after failover (duplicates expected) and DR drills. Active-active stays out of scope. The
single-region Delivery Constraint and ADR-0009 are addressed by the proposed
[ADR-0067](../../docs/adr/ADR-0067-a-cell-has-one-writable-region-and-fails-over-by-a-fenced-epoch.md)
and a stated, not applied, constitution amendment (owner decision O1560).

## Context and delivery boundary

Today a deployment is one PostgreSQL authority in one region, with in-region high availability: a
synchronous standby, point-in-time recovery and a drill that gates releases against the owner's
reference objectives (RPO 0 for acknowledged work, failover RTO at most 5 minutes, PITR RTO at most
1 hour; feature 010, D241, `docs/runbooks/disaster-recovery.md`). The runbook states that the
multi-region case is out of scope, and feature 010 records the gaps this feature closes or narrows:
no standby in another zone or region (G250), no object-store replication (G251), no read replicas
(G242).

This feature lets an operator run the same installation in a second (and optionally a third) region
as a **standby** that can take over. The words used below:

- **Region**: a failure domain named by the operator (for example `eu-west`, `eu-central`).
- **Cell**: one PostgreSQL authority with its object store and its platform processes, deployed in
  a fixed set of two or three regions. Exactly one region of a cell is **active** at a time; the
  others are **standby** regions that hold a physical replica of the database and of the object
  store. A deployment has one or more cells; cells share nothing.
- **Region epoch**: a number, recorded in the cell's database, that goes up by one every time a
  region is activated. It names which region may act and fences what was granted before it.
- **Replication mode**: `sync_quorum` (every acknowledged commit is durable in another region
  before the platform acknowledges it) or `async` (the standby may lag; the loss after a failover
  is the lag at that moment).

What the platform does **not** do, by design: it does not elect the active region, it does not run
consensus, it does not forward or redirect requests between regions, and it never lets two regions
write the same cell's data. The decision to promote is the operator's (or the operator's PostgreSQL
HA manager's); the platform verifies the preconditions, records the epoch, fences what came before
and reports what may have been lost. Temporal Cloud and Temporal's multi-cluster replication accept
a split brain and reconcile divergent histories afterwards (research §External semantics); this
platform's state is not an event-sourced history that can be merged, so it fences instead and treats
the former region's divergent writes as lost work to be reported and reconciled.

## Dependencies and ownership

Reuses feature 001 (identity, OpenAPI, error envelope), 002 (journal, leases, outbox), 003
(reconciliation, compensation), 004 (inbox, connectors, outbound deliveries; ADR-0012, ADR-0025),
006 (five SDKs, conformance), 007 (Studio follows runs by journal sequence; ADR-0016), 008
(telemetry, alerts; ADR-0017), 009 (tenant boundary, roles, quotas; ADR-0027), 010 (shard leases and
fence, DR drill; ADR-0020), 016/ADR-0033 (classification of anything new that is stored) and 017
(OpenBao authority; ADR-0035). It changes ADR-0009 in part and refines ADR-0020 through ADR-0067.

New features of the 018–032 set that interact, referenced and not specified here (coordinator
decisions C2, C11, C12, C13 of 2026-10-27):

- **020** keys analytic records deterministically by `(tenant, run, seq)`; 020 adds the epoch component
  of this feature to its record keys, dedupe tokens and object names (value 0 when 032 is absent), and
  its sinks rewind with `run_tombstone` records on activation (FR-032-034).
- **021** reports a CDC flush LSN to the customer database only after the inbox commit returned from the
  quorum-waited commit in a `sync_quorum` cell; under `async`, CDC changes inside the RPO are lost and
  need slot re-creation or a backfill (FR-032-015).
- **026** carries the epoch in its Workflow Streams cursor (`Last-Event-ID`) and resets readers on a
  mismatch, by the rule of FR-032-018.
- **028** owns the time-class catalog; every instant of this feature is class `infrastructure`
  (FR-032-017).
- **019, 022, 025, 026, 028, 031** hold keys listed in this feature's key inventory (FR-032-020).
- **019** completion tokens and push dispatch are bound to attempts and fenced by the existing lease
  checks; **030** stays inside one tenant, hence one cell (FR-032-024); **029** treats
  `placement:manage` as one more catalog permission.

Where any of these is absent, this feature works with today's behaviour.

## User Scenarios & Testing *(mandatory)*

### US01 — Run a cell with a standby region and see its state (Priority: P1)

A platform operator deploys a cell in `eu-west` (active) and `eu-central` (standby), declares the
regions and the replication mode once, and from then on can see, without guessing, which region is
active, at which epoch, how far behind the standby is and whether acknowledged work is currently
protected in another region.

**Why this priority**: every other story depends on a declared topology, an epoch and processes that
know their role; and an operator who cannot see replication lag cannot decide a failover.

**Independent Test**: start the region-pair harness (two PostgreSQL clusters with streaming
replication, two object stores, two sets of platform processes), run `region-init`, and check the
role, epoch, lag and protection reported by both regions; check that every tenant call to the standby
region is refused with a typed reason and that no write is attempted there.

**Acceptance Scenarios**:

1. **Given** a cell with no region configuration, **When** the platform is upgraded to this feature,
   **Then** it behaves exactly as before (role `UNCONFIGURED`, no new refusals, no new loops), and the
   existing suites pass unchanged.
2. **Given** the operator runs `region-init` with regions `eu-west,eu-central`, active `eu-west` and
   mode `sync_quorum`, **When** the processes refresh their role, **Then** `eu-west` reports `ACTIVE`
   at epoch 1, `eu-central` reports `PASSIVE` at epoch 1, `GET /v1/ready` answers 503 in
   `eu-central`, and `GET /v1/region` in both regions names the same cell, epoch and active region.
3. **Given** the standby region, **When** a client calls any tenant route, posts a webhook or a
   worker registers or polls, **Then** the call is refused with `UNAVAILABLE`, reason
   `REGION_PASSIVE` and a `Retry-After`, before authentication of a write or any database write, and
   the refusal is counted with fixed labels only.
4. **Given** `sync_quorum` is declared but no standby in another region is currently acknowledging,
   **When** the replication monitor observes it, **Then** the cell reports protection `UNPROTECTED`,
   the alert `SagatRegionUnprotected` fires, and nothing claims RPO 0 for that period.

---

### US02 — Switch the active region over without losing acknowledged work (Priority: P1)

An operator moves the cell from `eu-west` to `eu-central` for maintenance or a drill. In-flight work
finishes or is handed over; nothing the platform acknowledged is lost; clients, workers and webhook
senders end up in `eu-central` without configuration changes.

**Why this priority**: a planned switchover is the procedure that proves the topology works and is
the only way to rehearse the unplanned one safely.

**Independent Test**: with a workload that records every acknowledged start, signal, webhook 202 and
completion in a ledger, run drain → demote → verify catch-up → promote → activate → route; compare the
ledger with the new authority and replay every run.

**Acceptance Scenarios**:

1. **Given** an active `eu-west` at epoch 1, **When** the operator runs `region-drain`, **Then** new
   admissions (start, signal, update, approval decision, webhook, event publish) and worker claims in
   `eu-west` are refused with reason `REGION_DRAINING`, while reports of leases already granted are
   still accepted, the engines give back their shards and singleton leases, and reads keep working.
2. **Given** the drained primary has been shut down cleanly and `eu-central` promoted, **When** the
   operator runs `region-activate` with kind `planned` and the final LSN of the former primary,
   **Then** activation succeeds only if the promoted database replayed at least that LSN, the epoch
   becomes 2 with active `eu-central`, every shard lease, singleton lease and replica membership of
   epoch 1 is invalidated in the same transaction, and the history records the cut point.
3. **Given** activation at epoch 2, **When** the operator's DNS/GSLB points the cell endpoints at
   `eu-central`, **Then** SDK clients and workers reconnect without configuration changes, webhook
   senders keep posting to the same URL, and the ledger shows zero acknowledged operations missing.
4. **Given** the operator changes their mind during the drain, **When** they run `region-undrain`
   before demoting the primary, **Then** `eu-west` returns to `ACTIVE` at epoch 1 and nothing was
   lost or duplicated.

---

### US03 — Fail over after losing a region, without a split brain (Priority: P1)

`eu-west` becomes unreachable. The operator promotes `eu-central` and activates it. If `eu-west` is
in fact alive and partitioned, it must not keep acknowledging work that the new authority will never
see (in `sync_quorum` mode), and as soon as it can see the new epoch it must stop by itself.

**Why this priority**: this is the disaster the feature exists for; it must be correct before it is
fast.

**Independent Test**: partition the regions with the harness's proxy while a workload runs, keep the
former primary alive, promote and activate the standby, then heal the partition; count acknowledgments
from each region against the ledger, with and without the epoch fence (negative control).

**Acceptance Scenarios**:

1. **Given** `sync_quorum` and a partition between the regions, **When** clients still reach
   `eu-west`, **Then** nothing is acknowledged by `eu-west` after it lost its cross-region quorum:
   commits wait until the commit deadline (`COMMIT_OUTCOME_UNKNOWN`), a cancelled replication wait is
   never reported as success, keyed replays, inbox duplicates and worker `already_applied` answers wait
   for a quorum-waited commit of their own, and outcome reads stop with `DURABILITY_UNCONFIRMED` once the
   durability lease lapses; after a restart of the former primary it serves nothing (FR-032-009).
2. **Given** an unplanned failover, **When** the operator runs `region-activate` with kind
   `unplanned`, **Then** the command refuses without a bounded written attestation that the former
   primary is fenced and that every other standby of it was detached, records the attestation, the
   reason and the replication cut point, and the epoch becomes 2.
3. **Given** `async` mode, **When** the failover completes, **Then** the report states how many
   acknowledged operations and which time window may be missing, and the divergence report (US07)
   lists them by identifier when the former primary is reachable.
4. **Given** the partition heals and `eu-west` processes can reach `eu-central`, **When** their peer
   probe sees epoch 2 for their cell, **Then** within one probe interval they stop all work, report
   `SUPERSEDED`, refuse every call with reason `REGION_SUPERSEDED`, raise `SagatRegionPeerEpochAhead`,
   and never promote themselves back.
5. **Given** the former primary, **When** it is to rejoin, **Then** it rejoins only as a standby of
   the new primary after `pg_rewind` or a new base backup, and processes connected to it come up
   `PASSIVE`.

---

### US04 — Work continues correctly after a failover (Priority: P1)

After activation, every run continues from what the new authority holds: timers fire once, expired
leases are retried, the outbox is republished, deliveries and inbound events are deduplicated, unknown
external outcomes are looked up, and every follower of a run (Studio, SDK) converges on the
authority's history even where the former region had shown transitions that were lost.

**Why this priority**: a failover that leaves runs stuck, doubled or silently diverged is worse than
an outage.

**Independent Test**: fail over at injected points (between a worker's external effect and its
report, between an inbox commit and the provider's 202, during outbox publish, during a compensation)
and assert the authority's state with `verify_run` and the ledgers of the fake provider and receiver.

**Acceptance Scenarios**:

1. **Given** a webhook the former region answered with 202 and that replicated, **When** the provider
   resends it to the new region, **Then** the inbox answers `DUPLICATE` with 202 and no second run or
   wake-up exists; one that did not replicate is admitted as new.
2. **Given** deliveries pending at the cut point, **When** the new region's delivery worker runs,
   **Then** they are sent again with the same delivery id and event id, and subscribers that opted in
   receive one `ops.region.activated` notice per namespace and epoch.
3. **Given** a worker that completed an external effect under a lease the new authority does not
   know, **When** it reports to the new region, **Then** the report is refused `STALE_ATTEMPT`, the
   step runs again with the same idempotency key, and a step whose outcome is unknown goes to
   reconciliation or manual review as today.
4. **Given** a Studio page following a run whose last transitions were lost, **When** it reconnects to
   the new region, **Then** it receives a reset and redraws the authority's graph; it never shows a
   mixture of both histories.
5. **Given** a payload blob whose object had not yet replicated, **When** a step needs it within the
   replica grace after activation, **Then** the read waits as retryable `BLOB_NOT_REPLICATED`; after
   the grace it fails as `BLOB_LOST`; bytes that do not match the recorded digest are never served.

---

### US05 — Place a namespace and declare where its data may live (Priority: P2)

A tenant administrator sees, for each namespace, the cell, the regions its data is replicated to, the
active region and the epoch, and can declare the regions the namespace's data may reside in. The
operator cannot add a region to a cell that a namespace forbids.

**Why this priority**: residency is a compliance requirement for some tenants, but a cell works
without it.

**Independent Test**: create namespaces with and without `allowedRegions`; read placements; attempt to
narrow residency below the cell's regions and to add a forbidden region to the cell; run the
cross-tenant battery over the placement routes.

**Acceptance Scenarios**:

1. **Given** a namespace, **When** an authorized member reads `GET /v1/namespaces/{id}/placement`,
   **Then** it returns the cell, its regions, the active region, the epoch, the replication mode, the
   declared `allowedRegions` and whether residency is satisfied; another tenant's namespace is an
   opaque `NOT_FOUND`.
2. **Given** a cell in `eu-west,eu-central`, **When** an administrator declares `allowedRegions:
   [eu-west]`, **Then** the change is refused `VALIDATION_FAILED` with reason
   `PLACEMENT_RESIDENCY_CONFLICT`, because the namespace's data is already replicated to
   `eu-central`.
3. **Given** a namespace that allows only `eu-west,eu-central`, **When** the operator runs
   `region-topology` to add `us-east`, **Then** the change is refused and reports how many namespaces
   forbid it.
4. **Given** a multi-cell deployment, **When** the operator runs `region-routing-manifest`, **Then** it
   prints the cell endpoints and the tenant slugs placed in the cell, for the operator's GSLB, and
   nothing else.

---

### US06 — Drill a region failover and measure RPO and RTO (Priority: P2)

An operator rehearses switchover, unplanned failover, partition and failback on a reproducible
environment and gets a report that states, for each scenario, what was lost, how long each phase
took and on what environment, without the numbers becoming promises.

**Why this priority**: the constitution makes targets benchmark criteria; untested DR is a hope (the
runbook's words).

**Independent Test**: run `infra/dr/region-drill.sh`; it writes
`tests/performance/reports/multi-region-failover.md` and fails when a criterion of SC-032-011 is
violated (acknowledged loss in a mode that forbids it), not when a duration is merely slow.

**Acceptance Scenarios**:

1. **Given** the drill, **When** it runs the planned switchover, unplanned `sync_quorum`, unplanned
   `async`, partition-with-live-former-primary, region loss mid-commit, restart of the former primary,
   in-region failover inside the cell and failback scenarios,
   **Then** the report gives per scenario the acknowledged operations missing, the time window of
   possible loss, and the RTO broken into detection, promotion, activation, routing propagation,
   shard takeover and first successful start through the cell endpoint.
2. **Given** the report, **When** it is read, **Then** it names the hardware, PostgreSQL version and
   settings, injected round-trip time and loss, object-store replication method, DNS TTL used and
   every failure, and says that a simulated WAN on one host is not a real region pair.
3. **Given** commit latency is the price of `sync_quorum`, **When** the benchmark runs at the
   injected round-trip times, **Then** it reports p50/p95/p99 commit and start latency and
   transitions per second for both modes on the same environment.

---

### US07 — Reconcile what the former region accepted (Priority: P3)

After an unplanned failover in `async` mode (or a forced one), an operator compares the former primary
with the new authority before rebuilding it, gets a bounded, payload-free list of what only the former
held, and can re-admit the inbound events it had accepted into the new authority's inbox.

**Why this priority**: valuable for reducing data loss, but only after the core failover is correct.

**Independent Test**: produce divergence on purpose (async, partition, writes on both sides), run
`region-divergence-report` and `region-reingest-inbox`, and compare with the harness ledger.

**Acceptance Scenarios**:

1. **Given** a reachable former primary, **When** the operator runs `region-divergence-report`,
   **Then** it lists, per tenant and namespace, the identifiers of runs whose journal head differs,
   inbox events, deliveries, request receipts and audit records present only in the former, bounded
   per table, with no payload, secret or token.
2. **Given** inbox events present only in the former, **When** the operator runs
   `region-reingest-inbox`, **Then** each is admitted through the normal inbox admission of the new
   authority with its original source and event id, duplicates are absorbed, quotas apply, and each
   admission is audited.

---

### Edge Cases

- **Partition with a live former primary in `async` mode**: the former region keeps acknowledging while
  clients reach it and it cannot see the new epoch; only the operator's fencing and, once connectivity
  returns, the peer probe end it (G1561). Stated, not hidden.
- **`ANY 1 (b, c)` quorum**: after promoting `b`, the former primary could still be acknowledged by `c`;
  activation requires the attestation that `c` was detached, and the preflight checks what it can see.
- **Cancelled synchronous wait / former primary restarted while commits wait**: PostgreSQL keeps those
  commits locally and makes them visible (research §2); replays, duplicates and reads could then
  acknowledge them, which FR-032-009 (b)–(d) prevents; they are never merged. An external cancel of a
  waiting backend by a superuser leaves a window of at most one durability lease for outcome reads
  (G1579).
- **Epoch-unaware binary in the standby region after promotion**: it cannot write (role default
  read-only) nor take a lease (triggers) in an initialized cell (FR-032-006).
- **In-region failover inside a multi-region cell**: keeps feature 010's RPO 0 only under the quorum or
  verification rule of FR-032-010.
- **Promotion without activation**: the promoted database still names the former region; standby
  processes stay `PASSIVE` until activation. **Activation twice**: compare-and-set, `EPOCH_CONFLICT`.
- **Wrong cell or unknown region in a process's configuration**: fail closed, not ready, alert.
- **Clocks**: timers fire by the new database's clock; skew beyond lease margins is G1570; UUIDv7 order
  across a failover is approximate. **Schema**: no sequence or unlogged table exists (verified at
  `70f09435`) and a contract test keeps it so.
- **Journal sequence reuse**: transitions shown by the former region may be lost and their sequence
  numbers reused (FR-032-018).
- **Object store behind the database**: bounded grace, then `BLOB_LOST` (FR-032-019).
- **Webhook senders that do not retry on 503** lose events posted to a passive or draining region
  (G1574). **Regional brokers with mirrored offsets** are not supported (G1567).
- **OpenBao older than the database**: fail closed, no fallback (FR-032-020).
- **Fan-out interrupted**: resumable and idempotent per tenant and epoch. **Mixed versions**: enabling
  regions while an epoch-unaware engine runs is forbidden by procedure and only partly detectable (G1572).

## Requirements *(mandatory)*

### Functional Requirements

**Topology and authority**

- **FR-032-001 — Cell, regions and process identity**: An operator MUST be able to declare a cell
  identity and a set of two or three region names, exactly one of them active. Every control-plane,
  engine, sandbox-runner and database-runner process MUST be configured with its cell and region
  when the cell has more than one region, and MUST connect only to the database and object-store
  endpoints of its own region. A process with no region configuration runs in single-region mode
  with today's behaviour.
- **FR-032-002 — Region authority and epoch**: The cell's database MUST hold one authority record
  (cell, regions, active region, epoch, state `ACTIVE` or `DRAINING`, declared replication mode) and
  an append-only history of every change. The epoch MUST only increase, by one, by compare-and-set
  from the epoch the operator expects. Only operator commands with the owner role write them; the
  runtime role can only read them.
- **FR-032-003 — Role of a process**: Each process MUST derive its role (`ACTIVE`, `PASSIVE`,
  `DRAINING`, `SUPERSEDED`, `UNCONFIGURED`) from its configured region, the authority record of the
  database it is connected to, whether that database is in recovery and the peer probe, re-evaluated
  at a bounded interval. Any inconsistency (unknown region, wrong cell, an `ACTIVE` claim on a
  database in recovery) MUST fail closed: not ready, no work, alert.
- **FR-032-004 — Passive and draining control plane**: In `PASSIVE` and `SUPERSEDED` roles the
  control plane MUST refuse every tenant route and the webhook ingress with `503 UNAVAILABLE`, reason
  `REGION_PASSIVE` or `REGION_SUPERSEDED`, `Retry-After`, before any database write; in `DRAINING` it
  MUST refuse new admissions with `REGION_DRAINING` and keep serving reads. `GET /v1/ready` MUST
  answer 503 in every role but `ACTIVE` and `UNCONFIGURED`. The platform MUST NOT forward, proxy or
  redirect a request to another region.
- **FR-032-005 — Passive and draining engine**: In every role but `ACTIVE` and `UNCONFIGURED` an engine
  MUST hold no shard lease and no singleton lease and run no scheduler pass, outbox dispatcher,
  delivery worker, connector, telemetry projector or monitor that writes; the worker API MUST refuse
  registration and claims with `UNAVAILABLE` and `x-sagat-error` `REGION_PASSIVE`,
  `REGION_DRAINING` or `REGION_SUPERSEDED`. In `DRAINING` it MUST still accept heartbeats and reports
  of leases already granted, give back its shards and stop within the drain bound.

**Fencing**

- **FR-032-006 — Epoch in the scheduler fence, enforced by the database**: Shard leases and singleton
  leases MUST record the epoch they were taken in; the fence of ADR-0020 MUST additionally require the
  current epoch, in the same row read it already performs. Once a cell is initialized, triggers on
  `scheduler_shards`, `engine_leases`, `scheduler_members` and `connector_leases` MUST refuse any write
  that grants or renews (an expiry in the future) unless the session's `sagat.region` setting names the
  active region, the authority is `ACTIVE` and, where the row has one, its epoch is the current epoch;
  and the runtime role's default MUST become `default_transaction_read_only = on`, overridden per
  connection only by region-aware processes of the active region (roles `ACTIVE` and `DRAINING`). A binary unaware of epochs therefore
  cannot take work or write at all in an initialized cell; no operator flag is involved.
- **FR-032-007 — Activation is one transaction**: `region-activate` MUST, in one transaction on the
  promoted database: verify it is not in recovery, compare-and-set the epoch, verify the kind's
  preconditions (FR-032-012, FR-032-013), set the new active region, record the history row with the
  cut point (`pg_last_wal_replay_lsn()`, `pg_last_xact_replay_timestamp()`, timeline), raise the
  generation and expire every shard lease, remove every singleton lease, replica membership and
  connector lease of the previous epoch, and record the pending fan-out. Any failure MUST leave nothing
  changed. Other leases (step leases, script build/test job leases, secret-cleanup claims) are fenced
  by their tokens and only delay work until their TTL expires; activation does not touch them.
- **FR-032-008 — Worker sessions and leases across an epoch**: A session or lease that the new
  authority does not hold MUST be refused by the existing checks (`UNAUTHENTICATED`, register again;
  `STALE_ATTEMPT`); one it holds MUST remain valid under the existing lease-token, generation and
  expiry rules. No new worker fence is added; tests MUST prove both cases, including a report whose
  task id the new authority reissued with another lease token.
- **FR-032-009 — No acknowledgment without quorum durability, on any path**: In a `sync_quorum` cell:
  (a) no operation MUST be acknowledged on the strength of a COMMIT that returned after a cancelled or
  terminated replication wait; a commit that misses the commit deadline is `UNAVAILABLE` with reason
  `COMMIT_OUTCOME_UNKNOWN` and the client repeats it with its idempotency key; (b) every answer that
  acknowledges earlier work without a new write — an idempotent replay from a stored receipt, an inbox
  `DUPLICATE`, a connector acknowledgment of a duplicate, a worker `already_applied` — MUST be sent only
  after a commit of its own transaction that assigned a transaction id and waited for the quorum (its
  commit record follows the replayed work in the WAL, so its durability implies the replayed work's);
  (c) answers that report outcomes (run, step, graph, stream, query, inspection and result reads) MUST
  be served only while the process holds a durability lease, renewed by a quorum-waited beacon commit
  at each role refresh; when it lapses the process refuses with `UNAVAILABLE`, reason
  `DURABILITY_UNCONFIRMED`; (d) after a process starts, reconnects, sees a changed
  `pg_postmaster_start_time()` or observes `COMMIT_OUTCOME_UNKNOWN`, it MUST serve nothing until a beacon
  commit completes — a former primary restarted with locally committed waits therefore never serves
  again once its standby was promoted; (e) these transactions MUST run with `synchronous_commit` at
  least `on`, platform roles MUST have `client_connection_check_interval = 0`, and the runbook MUST
  forbid cancelling or terminating platform backends waiting for replication.
- **FR-032-010 — Replication mode, protection and degradation**: The declared mode MUST be compared
  with what the database reports (synchronous standbys and their regions, lag in bytes and seconds)
  by a monitor that has read-only monitoring access; the cell MUST report protection `PROTECTED`,
  `UNPROTECTED` or `UNKNOWN`. A `sync_quorum` configuration that in-region standbys alone could
  satisfy (`ANY k`/`FIRST k` with k not greater than the number of in-region members) MUST be reported
  as `UNPROTECTED`. Changing the declared mode MUST be an explicit, audited operator command with a
  reason. The in-region objectives of feature 010 (RPO 0 for an in-region failover) MUST be kept in a
  multi-region cell either by a quorum that always includes the in-region synchronous standby (for
  example `ANY 2 (eu-west-2, eu-central-1)`) or by an in-region promotion that first verifies the
  standby received the highest flush LSN reported by any synchronous standby (catching up by cascading
  replication or the WAL archive); the monitor reports which of the two holds, and without either the
  in-region RPO is reported as not guaranteed.
- **FR-032-011 — Peer epoch probe**: Each process MUST probe `GET /region` on the admin listener of the
  other regions' processes (operator-configured URLs on the internal network, verified TLS, no token;
  the public `GET /v1/region` keeps requiring a token) at a bounded interval; on seeing a higher epoch for its
  own cell it MUST move to `SUPERSEDED` at once and never back without a restart that finds a
  consistent authority. The probe MUST only ever stop a process, never activate one.

**Procedures**

- **FR-032-012 — Planned switchover**: The platform MUST provide drain, undrain and planned activation.
  Planned activation MUST require that the expected epoch is in state `DRAINING` and that the promoted
  database replayed at least the final LSN the operator gives from the cleanly stopped former primary;
  otherwise it MUST refuse and point to the unplanned procedure.
- **FR-032-013 — Unplanned failover**: Unplanned activation MUST require a bounded attestation (8 to
  2,000 characters) that the former primary is fenced and that every other standby of it was detached
  or repointed, and a reason; it MUST run a preflight whose blocking checks (database writable, epoch
  compare-and-set, key-ring coverage and the key fingerprints classed blocking in FR-032-020, no other
  standby of the cell visible as streaming from the former primary) refuse activation and whose
  warning checks (other key fingerprints, OpenBao reachable, blob replica lag, broker reachable) are
  reported and recorded.
  The platform MUST NOT activate a region by itself.
- **FR-032-014 — Former primary and failback**: A former primary MUST NOT serve as primary again; it
  rejoins only as a standby of the new primary after `pg_rewind` or a new base backup. Failback is a
  planned switchover in the other direction.

**Data paths after a failover**

- **FR-032-015 — Inbox and connectors**: Inbox deduplication by `(tenant, namespace, source, event
  id)` MUST hold across epochs; connectors MUST resume from the replicated positions; a provider MUST
  be acknowledged only after the inbox commit is acknowledged under the cell's mode (FR-032-009). A
  customer-database CDC position (feature 021) is a provider acknowledgment: in `sync_quorum` it is
  reported only after the quorum-waited commit; under `async`, changes whose inbox rows fell inside the
  RPO are lost because the slot moved past them, and recovery needs slot re-creation from a known LSN
  or a backfill (021's procedure); the divergence report lists them when the former primary is
  reachable and `region-reingest-inbox` can re-admit them.
- **FR-032-016 — Outbox, deliveries and the failover notice**: Pending outbox rows and deliveries
  MUST be republished by the new region with their existing ids. Outbound subscriptions MAY opt in to
  a new event kind `ops.region.activated`, delivered once per namespace and epoch through the existing
  pipeline (ADR-0025) with a deterministic event id, carrying the cell, epoch, regions, kind and cut
  point and no run data.
- **FR-032-017 — Timers, retries, leases, schedules and loops**: After activation, timers and
  deadlines MUST be decided by the new database's clock, expired leases reissued with a new
  generation, cron ticks keyed as today so that a tick never starts twice in the authority, and the
  telemetry projector MUST resume from its replicated cursor. Computed span ids (ADR-0017) MUST include
  the epoch of the journal event they derive from (the formula is unchanged for epoch 0): re-exports of
  replicated events are identical, re-executed attempts get new ids, and spans the former region
  exported for lost work remain in the trace backend as a record of what ran there. Every instant this
  feature adds is class `infrastructure` in 028's time-class catalog
  (`crates/infra-postgres/time-classification.json`); a test clock never advances it.
- **FR-032-018 — Journal followers resynchronize**: Every journal event MUST record the epoch in which
  it was committed (`run_events.region_epoch`). Run streams and graph-change reads MUST carry the epoch
  of the event at the returned sequence (`X-Sagat-Region-Epoch` header, additive `epoch` field on
  `GraphChanges`) and accept it back (additive optional `epoch` query parameter); when the authority's
  event at the client's sequence has another epoch, the answer MUST be the existing `GraphChanges` with
  `reset: true` and the whole graph, as it already is when the cursor is ahead of the run
  (`getRuntimeGraphChanges`). Studio and the five SDKs MUST send the pair they last saw; 026 applies the
  same `(epoch, seq)` cursor to Workflow Streams.
- **FR-032-019 — Object storage**: Each region MUST use its own bucket, replicated by the provider;
  the platform does not copy objects. Reads MUST verify the recorded digest (existing). Within the
  replica grace after an activation a missing object MUST be retryable `BLOB_NOT_REPLICATED`; after
  it, `BLOB_LOST` (non-retryable), and the run follows its failure policy. A bounded verifier MUST
  sample committed blobs and script artifacts in the other region's bucket and report what is
  missing.
- **FR-032-020 — Key inventory, secrets and OpenBao**: The cell MUST keep a closed inventory of every
  key it holds, with how it reaches each region, how it rotates and what divergence between regions
  does (research §Key inventory: secrets key ring, script signing key, 017 intent-binding key, 019 push
  signing key and fairness digest key, 022 flow-control digest key, 026 decision-link keys, 031
  codec-assertion keys, 028 template-index pins; 025 MCP tokens and 028 Git credentials are stored
  hashed or as namespace secrets and need no key of their own). Processes of the active region MUST
  record a fingerprint of each configured key (an HMAC of a fixed label under a symmetric key, the
  SHA-256 of a public key) in `cell_key_fingerprints`; passive processes MUST compare their own keys
  with it and report `region_key_mismatch{key}`; activation preflight MUST refuse on a mismatch of a key
  whose divergence is unsafe (key ring, script signing, intent binding, flow-control digest) and warn on
  the others. A credential missing in the region's OpenBao MUST fail closed as today.
- **FR-032-021 — Broker hints are regional**: Each region MUST use its own NATS; after activation the
  reconciler MUST re-emit hints for ready work; no correctness MUST depend on a message published in
  the former region.

**Placement and routing**

- **FR-032-022 — Namespace placement view**: `GET /v1/namespaces/{namespaceId}/placement` MUST return
  the cell, regions, active region, epoch, declared mode, protection, `allowedRegions` and whether
  residency is satisfied, under `namespace:read`, tenant-scoped.
- **FR-032-023 — Residency policy**: A namespace MAY declare `allowedRegions` at creation or later
  (permission `placement:manage`); the platform MUST refuse a declaration that excludes a region of
  the cell and MUST refuse a cell topology change that adds a region any namespace forbids. Placement
  changes are audited in the same transaction.
- **FR-032-024 — Tenant home cell and routing manifest**: All namespaces of a tenant MUST live in one
  cell in this feature; cross-cell placement of one tenant is out of scope (O1562). The operator MUST
  be able to export the cell endpoints and the tenant slugs of a cell for a GSLB.
- **FR-032-025 — Cell endpoints and client routing**: Every URL the platform renders for others
  (webhook endpoint URLs, Studio links, SDK configuration examples) MUST use the cell endpoint, never a
  region endpoint. On `REGION_PASSIVE`, `REGION_DRAINING` or `REGION_SUPERSEDED` the five SDKs MUST
  close pooled connections to that endpoint and resolve it again before the next attempt, within the
  existing retry rules (`sdks/SPEC.md` §5, §6.2), which do not change.

**Operations and evidence**

- **FR-032-026 — Operator commands**: `control-plane` MUST provide `region-init`, `region-status`,
  `region-drain`, `region-undrain`, `region-activate`, `region-mode`, `region-topology`,
  `region-divergence-report`, `region-reingest-inbox`, `region-emit-tombstones` and
  `region-routing-manifest`, with explicit
  environment inputs, one-line JSON output with a closed reason code, and exit codes 0 / 1 (database or
  unknown outcome) / 2 (configuration or refused precondition).
- **FR-032-027 — Observability**: Spans for activation, role refresh, peer probe, fan-out and blob
  verification; metrics with fixed labels only (role, kind, reason, surface, operator-named standby
  region, closed key name); alerts for replication lag, lost protection, peer epoch ahead, role
  mismatch, long drain, missing replicated objects, lapsed durability lease, key mismatch and stuck
  replication waiters; redacted logs with no tenant data at INFO or above in the commands.
- **FR-032-028 — Audit**: Every authority change MUST write its history row in the same transaction;
  each tenant of the cell MUST receive a `region.activated` audit record per epoch through a bounded,
  resumable fan-out; placement changes and re-ingested events are audited per tenant in their
  transaction.
- **FR-032-029 — Divergence report and inbox re-ingest**: Read-only on the former primary, bounded per
  table, identifiers and counts only; re-ingest uses the normal inbox admission and its quotas and is
  bounded per command.
- **FR-032-030 — Region drill and benchmark report**: A reproducible drill with the scenarios of US06
  MUST produce a report with the environment and every failure; RPO and RTO are measured, not
  published as objectives, unless the owner decides otherwise (O1561).
- **FR-032-031 — Compatibility, migration and enablement**: Migrations are additive with a down
  migration that refuses to drop history; contracts grow only; a single-region deployment is
  unchanged. Enabling regions is enforced by the database (FR-032-006): after `region-init`, a process
  that does not identify its region and epoch cannot write; the release notes tell operators to replace
  every process before `region-init`.
- **FR-032-032 — Finite ceilings and typed errors**: Every new resource has a default and a hard
  ceiling (plan, Finite ceilings); zero is not unlimited; errors use the existing codes refined by
  closed `details.reason` values and `x-sagat-error` trailers.
- **FR-032-033 — Studio region view**: Studio MUST show, read-only under `ops:read`, the cell, active
  region, epoch, role of the region it is served from, protection, lag and last activation, with a
  table alternative, design tokens only and no drawn sample values when there is no data.
- **FR-032-034 — Downstream projections after an activation**: Every record derived from the journal
  and keyed by a journal sequence MUST be able to carry the event's epoch; this feature provides
  `run_events.region_epoch` and the history of cuts, and 020 adds the epoch component to its
  `(tenant, run, seq)` keys, dedupe tokens and object names. After an unplanned activation, projectors
  (telemetry and 020 sinks) MUST continue from their replicated cursors and, whenever a run's next event
  has a later epoch than the event before it, emit a `run_tombstone` (020's record kind) for that run,
  epochs earlier than the new one, from that sequence on, before the new event. Runs held only by the
  former primary MUST get a whole-run tombstone through `region-emit-tombstones` from the divergence
  report.

### Non-functional requirements

- **NFR-032-001 (durability)**: In `sync_quorum` with protection `PROTECTED`, zero acknowledged
  operations are missing after a planned or unplanned failover in every tested scenario; in `async`
  the loss is measured and reported, never claimed zero. No path (commit, replay, duplicate, outcome
  read) acknowledges work that is not durable on the quorum. After activation no process holding a
  lease of an earlier epoch commits a scheduler action.
- **NFR-032-002 (security and tenant boundary)**: Region and placement data carry no tenant data where
  unauthenticated or deployment-level; placement routes pass the cross-tenant battery; the commands
  print no payload, secret or token (canaries); the peer probe accepts only operator-configured URLs
  over verified TLS; authorization fails closed in every role.
- **NFR-032-003 (observability)**: Every activation, drain, mode or topology change is explainable
  afterwards from the history row and the tenant audit; metrics carry no tenant, namespace, run or
  activity.
- **NFR-032-004 (compatibility)**: In-flight runs survive a failover with their pinned versions;
  OpenAPI, AsyncAPI, worker protocol and SDKs change additively; old SDKs keep working (they retry
  `UNAVAILABLE` and re-register on `UNAUTHENTICATED` today) without the re-resolution improvement.
- **NFR-032-005 (performance)**: The cost of `sync_quorum` (commit and start latency, transitions per
  second) and the RTO components are benchmark criteria on a stated environment; no figure is a
  promise.
- **NFR-032-006 (operability)**: The runbook describes switchover, failover, partition, failback,
  degradation and reconciliation as procedures with the command, its preconditions and its checks.

### Key Entities

- **Cell**: one PostgreSQL authority, its object store and its processes in a fixed region set.
- **Region authority**: the single record of the active region, epoch, state and declared mode.
- **Region history entry**: append-only record of every change, with actor, reason, attestation and
  cut point.
- **Namespace placement**: the declared residency of a namespace and its derived view.
- **Activation fan-out item**: per tenant and epoch, whether the audit record and notices were written.
- **Divergence report**: an operator artifact listing identifiers only in the former primary.
- **Key fingerprint**: a non-reversible fingerprint of a configured key, per key name and key id,
  recorded by the active region and compared by the others.
- **Durability beacon**: a per-process row whose quorum-waited update renews the process's durability
  lease in a `sync_quorum` cell.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-032-001**: In the planned-switchover drill, zero acknowledged operations of the ledger (starts,
  signals, updates, approval decisions, webhook 202s, completions) are missing from the new
  authority, the journal digest of the cut is identical in both regions and every run replays to its
  projections; the failback in the opposite direction, after the former primary rejoined by rewind,
  meets the same criterion; and an in-region failover inside the two-region cell (primary lost, the
  in-region standby promoted under FR-032-010's rule) loses zero acknowledged operations.
  (FR-032-002, 007, 010, 012, 014)
- **SC-032-002**: In the `sync_quorum` partition scenario with the former primary alive, zero
  operations are acknowledged by the former region after it lost its cross-region quorum — counting
  commits, keyed replays, inbox `DUPLICATE` answers, worker `already_applied` answers and outcome reads
  — including with a statement timeout, a cancel, a backend termination and a restart of the former
  primary injected during the replication wait; with the durability lease or the replay commit
  disabled, the same test fails (negative control). (FR-032-009, 010, 013)
- **SC-032-003**: In the `async` scenario, the report states the count and window of missing
  acknowledged operations, and the divergence report lists 100% of them by identifier when the former
  primary is reachable. (FR-032-013, 029)
- **SC-032-004**: After activation, zero scheduler actions, singleton-loop writes or claims are
  committed under a lease of an earlier epoch; the same tests fail with the epoch check removed
  (negative control); an engine binary that does not set its region and epoch takes no lease and
  commits no write in an initialized cell. (FR-032-006, 007, 008, 011, 031)
- **SC-032-005**: In a passive region, 100% of tenant routes, webhooks and worker calls in the
  contract list are refused with the documented reason, zero database writes are attempted, and
  readiness is 503; a process configured for an unknown region or another cell is not ready and does no
  work. (FR-032-001, 003, 004, 005)
- **SC-032-006**: Re-delivering the cut-point window (provider resends, connector re-reads, outbox
  republishes, workers repeat reports), with the former region's broker gone, creates zero second runs
  per idempotency key or event key and zero second transitions, leaves no ready step without a hint
  after one reconciler cycle, and receivers observe only stable delivery and event ids.
  (FR-032-015, 016, 017, 021)
- **SC-032-007**: Every follower connected across a failover that lost transitions ends with the
  authority's graph in all tested cases, and a follower whose sequence was replicated is not reset;
  after an unplanned activation, a test sink receives one `run_tombstone` per run whose events changed
  epoch, before that run's first new-epoch record, and for every run listed only in the former primary.
  (FR-032-017, 018, 034)
- **SC-032-008**: A step needing a not-yet-replicated blob resumes when the object arrives within the
  grace or fails `BLOB_LOST` after it; zero wrong bytes are served. (FR-032-019)
- **SC-032-009**: All five SDKs pass the region conformance scenarios (re-resolution, re-registration,
  stream reset). (FR-032-008, 018, 025)
- **SC-032-010**: The cross-tenant battery passes over the placement routes; secret and payload
  canaries are absent from the output of every command and from `/v1/region`; activation is refused when
  the key ring does not cover the stored secrets or a blocking key fingerprint differs, a warning key
  fingerprint difference is recorded, and a credential missing in the region's OpenBao fails closed with
  no fallback. (FR-032-020, 022..024, 029)
- **SC-032-011**: The drill report gives RPO and the six RTO components for every scenario of US06,
  and commit/start latency p50/p95/p99 for both modes at each injected round-trip time, with the
  environment and every failure; a scenario that loses acknowledged work in a mode that forbids it
  fails the drill. (FR-032-030, NFR-032-005)
- **SC-032-012**: Every authority change in the tests has its history row in the same transaction
  (crash-injection proves no change without it); every tenant gets exactly one `region.activated`
  audit record per epoch after an interrupted and resumed fan-out; alerts fire in the drill for lag,
  lost protection and peer epoch ahead. (FR-032-026..028, 033)
- **SC-032-013**: A single-region deployment upgraded without region configuration passes the existing
  suites; migrations go up and down; the OpenAPI, AsyncAPI and worker-protocol compatibility checks
  pass. (FR-032-001, 031, 032)

## Assumptions

- The operator provides the infrastructure: PostgreSQL streaming replication between regions (managed
  service, CloudNativePG replica clusters, Patroni standby clusters or equivalent), bucket replication
  (S3 cross-region replication, MinIO site replication or equivalent), one NATS per region, a DNS or
  GSLB for the cell endpoints, OpenBao and the operator secrets in every region.
- Hosts are NTP-synchronized; skew of seconds is absorbed by lease TTLs; larger skew is a gap (G1570).
- Two regions are the default; a third region is for a quorum standby, never a second active.
- Product, scope and the constitution amendment are the owner's decisions (O1560–O1562).

## Out of Scope

- Active-active: two regions writing the same cell's data, conflict resolution, global ordering.
- Automatic activation by the platform, leader election, a witness service or consensus.
- Reads served from a standby region (stale reads), request forwarding or redirects between regions.
- Logical replication of a subset of tenants; per-namespace failover inside a cell.
- Relocating a tenant or namespace between cells; one tenant across several cells.
- Replicating NATS streams; mirrored Kafka with offset translation; OpenBao cross-region replication.
- Rewriting or merging the former primary's divergent writes into the authority (only inbound events
  may be re-admitted, US07).

## Owner questions

- **O1560 — Accept the direction of ADR-0067; the acceptance itself waits for evidence.** Recommended
  default: the owner may accept the direction now so that implementation can start; ADR-0067's status
  `Accepted`, ADR-0009's "Superseded in part by ADR-0067" line and the constitution amendment (Delivery
  Constraints: "one active region per cell with high availability; an optional cross-region standby
  with active-passive failover under ADR-0067"; multi-region active-active still out of scope; 1.0.0 →
  1.1.0, MINOR) wait for the region drill report and the commit-latency benchmark, as ADR-0009 and the
  constitution's Governance require (T032-063). The amendment text stays in ADR-0067.
- **O1561 — Cross-region objectives and default mode.** Recommended default: `sync_quorum` is the
  documented reference mode; RPO and RTO across regions stay benchmark criteria (not published
  objectives and not a release gate) until a drill has run on a real two-region installation; the
  single-region objectives of D241 are unchanged.
- **O1562 — Placement granularity.** Recommended default: a tenant's namespaces live in one cell in
  this feature; namespace-level placement across cells needs a cross-cell tenant directory and is a
  later feature with its own ADR.
