# Implementation Plan: Multi-region replication and active-passive failover

**Branch**: `feature/032-multi-region-failover` | **Date**: 2026-10-26 | **Spec**: [spec.md](./spec.md)

Status: approved for implementation (Owner decision 2026-11-11-1), in release 0.3.0 (Owner decision 2026-11-24-1), not
started. Until 2026-11-24 this line read "Proposed — not started".

**Input**: [spec.md](./spec.md), [research.md](./research.md),
[ADR-0067](../../docs/adr/ADR-0067-a-cell-has-one-writable-region-and-fails-over-by-a-fenced-epoch.md) (accepted
2026-11-24). The gate "Implementation MUST NOT start before the owner answers O1560" is lifted: the owner answered it
(Owner decision 2026-11-24-2, constitution 5.0.0). Implementation starts after the constitution 5.0.0 amendment is
merged, because the amendment is what admits it (Task T032-001).

## Readiness against `develop` (2026-11-24)

Read again against `develop` at `546d5aa5`. None of 032 is built: there is no region epoch, cell or region code (the
`epoch` columns that exist belong to the instance registry and to capacity groups). Where this section and an older
line below disagree, this section is right.

**Corrections.**

1. **Governance.** The amendment is MAJOR (5.0.0), decided now, with the wording "one writable region per cell with
   fenced failover (032)"; D1592's sequencing (the amendment after the evidence) and the "1.0.0 to 1.1.0" text are
   overtaken. The drill report and the commit-latency benchmark (Task T032-063) are the release gate of this feature.
   The constitution is 4.7.0 until the amendment merges; the transitional clause lets this spec, plan and tasks stand in
   for a design doc; tasks that depend on an owner decision now carry `Owner-gate:` lines.
2. **Sequences.** "No sequence or identity column exists" is no longer true: `platform_requirement_history.id` is an
   identity column (migration `20270113000000_instance_registry`). Decision D032-001: an identity column is allowed when its
   values are never exported, never used as a cursor and never compared across an activation (an operator's history
   ordered by time); the contract test of Task T032-009 lists such columns by name with that reason and fails on any
   other.
3. **Six SDKs.** The Rust SDK exists (feature 034): every "five SDKs" and every list of five languages means six,
   Rust included.
4. **Protocol minor.** 1.7 is taken by step logs (design 041) and 1.8 by design 051 (the next step in a completion);
   this feature takes the next free minor at build time.
5. **The instance registry (Architecture decision 0082) must be reconciled.** A passive process today fails to
   register on a read-only standby and answers the registry's `BINARY_CAPABILITY_MISSING` instead of `REGION_PASSIVE`;
   registry rows of the former region's processes replicate and look live for 30 s; the registry's census is the way to
   know that every process is region-aware. Decision D032-002: the region check runs before the registry's serving gate
   and `/v1/region` is exempt from both; a passive process skips the registry heartbeat; activation removes the serving
   rows of earlier epochs in its transaction; region-awareness is a registry capability `region-v1`, and `region-init`
   refuses while the census shows a live process without it (this replaces "replace every process before region-init"
   by a check). New Tasks T032-065 to T032-067.
6. **More loops to gate by role.** The engine also runs the instance heartbeat, housekeeping, trigger schedules, the
   schedule pass, the batch pass, the reset pass and the blob cleanup (`apps/execution-engine/src/lib.rs`). FR-032-005's
   rule covers every loop that writes; schedules and batches are shard-scoped and so fenced by the epoch with the shards.
   The `secret-cleanup` CronJob opens the database as the runtime role and must carry `sagat.region`; the sandbox and
   database runners do not open the platform's database and need no region identity. New Task T032-068.
7. **More replay paths.** About 25 store modules answer from a receipt or a duplicate (among them `business_ids.rs`,
   `batch.rs`, `execution/resets.rs`, the `capacity_*` modules, `scoped_leaf_finish.rs`, `approval_progress.rs`,
   `trigger_schedules.rs`, `declared_operations.rs`, `database_results.rs`, `managed_secrets.rs`,
   `execution/scoped_events.rs`). Design 048 does not add a write per replay (Architecture decision 0130 supersedes
   ADR-0067's decision 6(b) and (d) in part): it never lets a commit's wait be cancelled (Requirement 048-012), and after
   a change of server identity, an unknown commit outcome or a read-only refusal a process serves nothing until its
   registry confirmation, a waited write, has reached a standby (Requirement 048-013, Task T048-10). Task T032-058 builds
   the cross-region quorum and ADR-0067's durability lease on that confirmation (the registry row's confirmed identity,
   renewed on a schedule), which replaces the `region_beacons` of `data-model.md`; a transaction must write a row for its
   commit to wait, so D1590's `pg_current_xact_id()` alone would not wait. This feature uses design 048's reason
   `DATABASE_UNCONFIRMED` (for its `DURABILITY_UNCONFIRMED`), its `SAGAT_DB_COMMIT_DEADLINE_MS` (for
   `SAGAT_COMMIT_DEADLINE_MS`) and its commit (Task T048-7, for T032-024's guard); T032-009's contract test allows the
   unlogged `server_incarnation` by name, which holds no state of record.
8. **In-region high availability is now design 048's reference topology.** The HA manager (CloudNativePG or Patroni)
   owns `synchronous_standby_names`; this feature MUST NOT set it by hand (D1591's manual setting is replaced). Gap
   G1585: CloudNativePG replica clusters and Patroni standby clusters stream asynchronously, so `sync_quorum` across
   regions needs one HA cluster stretched across the regions (Patroni with an etcd of three regions, or CloudNativePG on
   a Kubernetes cluster that spans them); Task T032-003's harness decides which reference it uses and records the gap's
   answer. Design 048 relies on the HA manager's failover quorum for in-region promotion (its Requirement 048-001); FR-032-010's verification of the highest synchronous flush position stays this feature's.
9. **Rolling upgrades follow design 053** (split from design 048 on 2026-11-25). This feature's migration is expand-only (inert lease triggers, a down that
   refuses once a cell is initialized); the epoch-aware behaviour waits for the `region-v1` capability (item 5).
10. **Journal partitioning (design 049) and the epoch column.** `run_events.region_epoch` is added on the partitioned
    parent if 049 lands first, or carried by 049's partitioning if this lands first; the divergence report's head scan
    and `infra/dr/journal_digest.sql` read across partitions; the bucket-maintenance function of 049 runs only in the
    active region. Recorded in both task lists.
11. **Other code facts.** Span ids are computed in `crates/telemetry/src/span.rs` (`event_span_id`), not in the
    application; the admin listener is plain HTTP, so the peer probe over verified TLS (D1598) is new work; the
    notification transport may be NATS or Redis Streams (Architecture decision 0069), so "each region uses its own NATS"
    means its own selected transport; the operator CLI may use flags (`schedule-trigger-handover` does); the time
    classification file of feature 028 does not exist yet, so Task T032-006 records its entries in this feature's
    research until it does.
12. **Cursors that carry an epoch.** Step logs (design 041, approved) page by `(stored_xid, sequence)`; after a lossy
    activation transaction ids can be reused, so the step-log cursor carries the epoch too (Architecture decision 0067,
    decision 9). New Task T032-069. Feature 026's cursor `<epoch>.<offset>` uses "epoch" for its own stream generation;
    Decision D032-003: 032's region epoch is a separate component, and feature 026's build adds it (gap G1586 until then).
13. **Hot-path budget.** Changes under `crates/infra-postgres/` select `integration_claim_transition_efficiency`; the
    epoch default on every journal row and the epoch predicate in the fence are recorded in that budget in the change
    that adds them (Task T032-054).

**Decisions taken on 2026-11-24.** O1560 answered by the owner (Owner decision 2026-11-24-2); ADR-0067 accepted
(delegate, Owner decision 2026-11-24-4, Owner decision 2026-11-24-507); O1561 and O1562 as recommended (delegate, Owner
decision 2026-11-20-6, Owner decisions 2026-11-24-508 and 2026-11-24-509); the five unticked items of
`checklists/requirements.md` accepted as they stand (delegate, Owner decision 2026-11-24-4, Owner decision
2026-11-24-510); D032-001 to D032-003 and gaps G1585 and G1586 in `research.md`.

**Dependencies, built or not.** Built: 001 to 012 (010 included), 016, 017 (OpenBao), 034 (Rust SDK). Mostly built: 014,
018 (with the registry), 033, 037. Not built: 019, 020, 021, 022, 025, 026, 028, 029, 030, 031, 036; the keys and cursors
those would add are absent and this feature works with today's behaviour where they are absent (spec, "Dependencies").
New and needed first: design 048 (in-region topology and the waited replay).

**Build slices**, in order (each mergeable alone, single-region behaviour unchanged until a cell is initialized):

1. Foundation and observe-only: T032-002 to T032-017, with T032-065 to T032-068. Needs design 048's topology and the
   constitution 5.0.0 merge.
2. Planned switchover and the fence: T032-018 to T032-022 and T032-059.
3. Unplanned failover and durability: T032-023 to T032-029, T032-058 (on design 048's Task T048-7) and T032-062. The
   riskiest slice; it proves G1578.
4. Work after a failover, server side: T032-030 to T032-033, T032-035 to T032-037, T032-061 and T032-069; then the
   clients: T032-034 (Studio), T032-038 and T032-039 (six SDKs).
5. Placement: T032-040 to T032-044, in parallel after slice 1.
6. The drill and the benchmark: T032-045 to T032-047 and T032-063 (the release gate).
7. Reconciliation: T032-048 and T032-049.
8. Polish: T032-050 to T032-057.

T032-060 (tombstones downstream) and T032-064 (change data capture) stay parked behind features 020 and 021.

*Note (2026-11-26):* constitution 5.0.0 is merged (`81e87775`); the Delivery Constraints row of the Constitution
Check below reads against its "Regions" bullet, whose evidence gate is Task T032-063.


## Summary

A cell keeps one writable PostgreSQL primary in its active region and physical replicas in one or two
standby regions. A region epoch, recorded in the cell's database and advanced only by an operator's
activation, says which region may act; the ADR-0020 fence and the singleton leases carry it, and
activation invalidates everything granted under the previous epoch in one transaction. Processes in a
standby region run in a passive role that refuses work with a typed reason. The platform never elects,
forwards or merges: the operator (or the HA manager) promotes, the platform verifies, records, fences,
re-routes through cell endpoints and reports what may have been lost. Duplicates after a failover are
absorbed by the existing inbox, delivery ids, lease tokens and idempotency keys; journal followers reset
on an epoch change; blobs missing in the new region wait a bounded grace. `sync_quorum` mode removes
acknowledged loss at the price of cross-region commit latency, on every path that acknowledges: commits,
replays from receipts, inbox duplicates and outcome reads (a durability lease renewed by a quorum-waited
beacon); `async` reports its loss. The database itself enforces the epoch on lease tables and makes the
runtime role read-only by default once a cell is initialized, so binaries unaware of regions cannot act.
Journal events carry their epoch; projectors and 020 sinks rewind with `run_tombstone` after an unplanned
activation. RPO and RTO are measured by a region drill and stay benchmark criteria.

## Technical Context

**Language/Version**: Rust stable pinned by `rust-toolchain.toml`; TypeScript (Studio); the five SDKs.

**Primary Dependencies**: existing only — Tokio, Axum, tonic, SQLx, reqwest (in `infra-egress`),
`object_store` (in `infra-blobstore`). No new crate, no consensus library (D1562, D1584).

**Storage**: PostgreSQL with physical streaming replication across regions (operator-provided);
S3-compatible buckets per region with provider replication; NATS JetStream per region.

**Testing**: `cargo test -p sagat-tests` targets `integration_multi_region_failover`,
`fault_multi_region_failover`, `contract_multi_region_failover`, `e2e_multi_region_failover`,
`perf_multi_region_failover`; `sdk_conformance` scenarios; Studio Vitest and Playwright; the region drill
`infra/dr/region-drill.sh`.

**Target Platform**: Linux containers/Kubernetes per region; Docker Compose region-pair profile for
development and the drill.

**Project Type**: Rust modular monolith + React Studio + five SDKs.

**Performance Goals**: none promised. Benchmark criteria: commit/start latency p50/p95/p99 and
transitions per second for `sync_quorum` and `async` at injected round-trip times of 2, 20 and 80 ms;
RTO components; RPO per scenario (SC-032-011).

**Constraints**: one writable authority per cell; no forwarding; activation is an operator action;
additive contracts; single-region deployments unchanged.

**Scale/Scope**: 2–3 regions per cell; one or more independent cells; up to 64 engine replicas per region
as feature 010.

## Constitution Check

These are design checks, not implementation approvals or test results.

| Principle | Before research | After design | Evidence / future gate |
|---|---|---|---|
| I — durable atomic transitions, fencing | Pass | Pass | Activation is one transaction (FR-032-007); epoch in the existing fence (FR-032-006); no acknowledgment after a cancelled replication wait or through a replay or read path (FR-032-009); epoch enforced by the database (FR-032-006); T032-020, T032-021, T032-023, T032-024, T032-058, T032-059 |
| II — pure domain, inward dependencies | Pass | Pass | Pure role and transition rules in `crates/domain/src/region.rs`; clients only in `infra-*`; no new crate; T032-005, T032-054 |
| III — at-least-once effects, honest compensation | Pass | Pass | No exactly-once claim; lost work and phantom effects stated (G1562); unknown outcomes reconciled; T032-030, T032-032 |
| IV — pinned versions, five SDKs, additive contracts | Pass | Pass | Runs keep their versions; OpenAPI/AsyncAPI/proto additive; five-SDK scenarios; T032-038, T032-039, T032-050 |
| V — isolation, tenant boundary, secrets | Pass | Pass | Region data has no tenant data; placement tenant-scoped; commands print no payload/secret; peer probe on verified TLS only; T032-040, T032-048, T032-053 |
| VI — OTel, audit, typed errors | Pass | Pass | History row in the same transaction; tenant fan-out; fixed-label metrics; reasons over existing codes; T032-015, T032-016, T032-035 |
| VII — inbox/outbox, redeliverable events | Pass | Pass | Dedupe across epochs; republish with stable ids; reconciler re-hints in the new region; T032-030, T032-031 |
| VIII — failure tests, honest benchmarks | Pass | Pass | Region loss mid-commit, partition, duplicates, timeout/signal race, lost broker, failed compensation, restart; drill report with environment; T032-023..032, T032-045..047 |
| Delivery Constraints — single region with HA (5.0.0: Regions) | **Justified (amendment required)**; Pass since 5.0.0 | **Justified (amendment required, after evidence)**; Pass since 5.0.0, production gated on the same evidence | the owner may accept the direction (O1560); ADR-0067 `Accepted`, ADR-0009's "Superseded in part" line and the constitution amendment of D1583 wait for the drill report and the commit-latency benchmark (D1592, T032-063); active-active stays out of scope. 2026-11-24: amended in 5.0.0 (Owner decision 2026-11-24-2); the row now reads **Pass** against the "Regions" bullet, whose production gate is the same evidence |

## Project Structure

### Documentation (this feature)

```text
specs/032-multi-region-failover/
├── spec.md  research.md  plan.md  data-model.md  quickstart.md  tasks.md
├── checklists/requirements.md
└── contracts/
    ├── rest.openapi.yaml         # /v1/region, placement, namespace create field, stream epoch, reasons
    ├── events.asyncapi.yaml      # RegionActivatedWebhook (ops.region.activated)
    ├── worker-protocol.md        # trailers, RegisterWorkerResponse fields, minor version
    ├── operator-cli.md           # region-* commands, inputs, outputs, exit codes, preconditions
    ├── errors.md                 # reasons and their mapping
    └── sdk-conformance.md        # five-SDK scenarios
docs/adr/ADR-0067-a-cell-has-one-writable-region-and-fails-over-by-a-fenced-epoch.md
```

### Source placement (existing crates; planned additions)

```text
crates/domain/src/region.rs                 RegionName, CellId, Epoch, Role, Mode, Protection, transitions (pure)
crates/application/src/region.rs            ports RegionAuthorityStore, ReplicationObserver, PeerProbe; services
                                            RoleWatcher, Activation, Placement, Fanout, BlobReplicaPolicy
crates/infra-postgres/src/region.rs         authority, history, activation transaction, placement, fan-out SQL
crates/infra-postgres/src/partitioning.rs   epoch in acquire/renew/fence (ADR-0020 extension)
crates/infra-postgres/src/pool.rs           commit guard, connection settings (sagat.region, read-only override), barrier (D1566, D1590, D1593)
crates/infra-postgres/src/{definitions,execution,events,connectors}.rs  quorum-waited replay transactions (command_dedupe and other receipts, already_applied, inbox and connector duplicates)
crates/application/src/telemetry.rs         span ids include the event's epoch; tombstone rule shared with 020 (D1594)
crates/infra-postgres/migrations/<ts>_region_authority.{up,down}.sql
crates/infra-egress/src/peer.rs             peer epoch probe client (operator URLs, verified TLS)
crates/infra-blobstore/src/backend.rs       missing-object classification for the replica grace
crates/scheduler/src/region.rs              role refresh, replication monitor, fan-out, blob verifier loops
crates/scheduler/src/shards.rs              keeper takes shards only when ACTIVE
crates/api-http/src/region.rs               role middleware, /v1/region, placement routes
crates/api-http/src/routes/execution.rs     epoch header/parameter and reset on run streams
crates/api-grpc/src/service.rs              REGION_* trailers on RegisterWorker/Poll/Stream
apps/control-plane/src/{cli.rs,region.rs}   region-* commands
apps/execution-engine/src/{loops.rs,settings.rs}  role-gated loops, region settings
frontend/apps/studio/src/api/follow.ts      epoch and reset
frontend/apps/studio/src/features/operations/RegionPanel.tsx
frontend/apps/studio/src/app/NamespacesPage.tsx, CreateNamespaceDialog.tsx  placement and residency
sdks/{typescript,go,python,dotnet,java}     re-resolution on REGION_*; epoch on streams
infra/helm/ (values, templates, examples/multi-region.yaml); infra/docker/compose.region.yaml
infra/dr/region-drill.sh; infra/observability/alerts.yaml
tests/{integration,fault,contract,e2e,performance}/multi-region-failover/; tests/support/{region,wan}.rs
docs/runbooks/multi-region-failover.md
```

## Architecture

### Roles and their derivation

`role = f(configured cell and region, authority row of the connected database, pg_is_in_recovery(), peer
probe)`, a pure function in the domain, evaluated by the `RoleWatcher` every `SAGAT_REGION_REFRESH_MS`
(default 1,000 ms) and cached for the request path. Truth table:

| Authority row | In recovery | Active region = mine | State | Peer ahead | Role |
|---|---|---|---|---|---|
| absent | any | — | — | — | `UNCONFIGURED` (single region; today's behaviour) |
| present, my cell | no | yes | `ACTIVE` | no | `ACTIVE` |
| present, my cell | no | yes | `DRAINING` | no | `DRAINING` |
| present, my cell | yes | no | any | no | `PASSIVE` |
| present, my cell | no | no | any | no | `PASSIVE` (promoted, not yet activated) |
| present, my cell | any | any | any | yes | `SUPERSEDED` (sticky until restart) |
| present, my cell | yes | yes | any | — | fail closed: `PASSIVE`, alert `SagatRegionRoleMismatch` |
| other cell, or my region not listed | any | any | any | any | fail closed, not ready, alert |

### What each role does

| Surface | ACTIVE / UNCONFIGURED | DRAINING | PASSIVE / SUPERSEDED |
|---|---|---|---|
| REST reads | served | served | 503 `REGION_PASSIVE` / `REGION_SUPERSEDED` |
| REST admissions, webhook ingress | served | 503 `REGION_DRAINING` | 503 `REGION_PASSIVE` / `REGION_SUPERSEDED` |
| `/v1/health`, `/v1/region` | served | served | served |
| `/v1/ready` | 200 | 503 | 503 |
| Worker register/poll/stream | served | `UNAVAILABLE` `REGION_DRAINING` | `UNAVAILABLE` `REGION_PASSIVE` / `REGION_SUPERSEDED` |
| Worker heartbeat/complete/fail, managed reports | served | served (granted leases only) | `UNAVAILABLE` (the database is read-only or superseded) |
| Shard keeper, passes, outbox, deliveries, connectors, projector, monitor writes | run | stop, release shards and singleton leases | not started |
| Replication monitor, peer probe, role refresh | run | run | run (read-only) |
| Durability beacon (`sync_quorum` only) | quorum-waited commit each refresh; lapse → outcome reads and replays 503 `DURABILITY_UNCONFIRMED` | same | none (read-only database) |
| Database connection settings | `sagat.region=<r>`, `default_transaction_read_only=off` | same (reports of granted leases and releases still write; the lease triggers refuse new takes because the authority is `DRAINING`) | role default (read-only); no override |

### Activation sequence (planned)

`region-drain` (epoch N, `DRAINING`) → processes stop admissions, give back shards (≤ drain bound) →
operator stops the former primary cleanly and reads its final LSN → standby promoted (operator/HA
manager) → `region-activate` kind `planned` with that LSN → epoch N+1 → operator flips the cell endpoints
→ new region's role refresh sees `ACTIVE`, keepers take 64 shards, reconciler re-hints, fan-out runs →
former primary rebuilt as a standby (`pg_rewind`).

### Activation sequence (unplanned)

Operator decides → promotes the best standby → detaches or repoints the other standbys → fences the former
primary (power, network, routing) → `region-activate` kind `unplanned` with attestation and reason →
preflight → epoch N+1 → endpoints flipped → `region-divergence-report` if the former primary is reachable
→ former primary rewound or recloned → `region-mode` to record the protection the cell now has →
`region-emit-tombstones` from the divergence report when 020 sinks exist.

### In-region failover inside a multi-region cell

Feature 010's procedure stays (promote the in-region synchronous standby; epoch unchanged, the active
region does not change). It keeps RPO 0 only if the quorum always includes the in-region standby (D1591)
or if the promoted standby first reaches the highest flush LSN reported by any synchronous standby; the
replication monitor reports which holds (`region_inregion_rpo_guaranteed`).

## Transaction boundaries

| Operation | Commits together | Never in the same transaction |
|---|---|---|
| `region-init` | authority row (epoch 1, `ACTIVE`), history row `INIT` | anything of tenants |
| `region-drain` / `region-undrain` | CAS epoch + state change, history row | process behaviour (follows by refresh) |
| `region-activate` | not-in-recovery check, CAS epoch N→N+1, precondition evidence, authority update, history row with cut point, warnings and attestation, `scheduler_shards` generation+1 / expiry / epoch for all 64 rows, `engine_leases`, `scheduler_members` and `connector_leases` rows deleted, `region_fanout` row for the epoch (the owner role sets `sagat.region` to the target region so the lease triggers accept the reset) | tenant audit records, notices (fan-out), DNS changes; step, script-job and secret-cleanup leases (token-fenced, left to expire) |
| Acknowledging replay (`sync_quorum`) | the existing receipt lookup + `pg_current_xact_id()`; COMMIT with `synchronous_commit` ≥ `on` before the answer is sent | — |
| Durability beacon (`sync_quorum`) | `UPDATE region_beacons SET at = now()` for this process (insert on first use), `synchronous_commit` ≥ `on` | any business write |
| Journal append (existing) | unchanged statement; `run_events.region_epoch` filled by its column default `sagat_region_epoch()` | — |
| Key fingerprints (active region, at start) | upsert of `cell_key_fingerprints` rows for configured keys | — |
| `region-mode` / `region-topology` | CAS epoch, authority update, history row; topology: residency check of every placement under the authority row lock | — |
| Scheduler action (existing) | unchanged; the fence's `FOR SHARE` read of the shard row also compares `region_epoch` | — |
| Singleton lease take/renew (existing) | unchanged statement with `region_epoch = current` predicate | — |
| Fan-out batch | for ≤ batch tenants: `region_epoch_notices` rows, tenant `audit_records`, outbound deliveries of `ops.region.activated` for opted-in subscriptions, cursor advance | the activation itself |
| Placement update | authority row `FOR SHARE`, placement row update with revision CAS, tenant audit record | — |
| Re-ingest of one event | normal inbox admission of feature 004 (row, quota, dedupe) + tenant audit `event.reingested` | reads of the former primary (separate read-only connection) |
| Divergence report | read-only on both databases; then one history row `DIVERGENCE_REPORT` (counts only) | any write to the former primary |

**Lock order**: `region_authority` row first, then `scheduler_shards` rows in shard order, then
`engine_leases`, then `scheduler_members`, then namespace placement rows. Placement updates take the
authority row `FOR SHARE` before their own row, the same order as topology changes. Activation runs
when no engine of the new region holds anything (they are passive), so it meets no competing fence.

## Authorization matrix

| Action | Who | Mechanism |
|---|---|---|
| `region-*` commands | platform operator | owner database role from an operator machine (like `quota-ceiling`); not exposed over the API |
| `GET /v1/region` | any authenticated caller | token, no permission (like `/v1/version`); no tenant data |
| `GET /v1/namespaces/{id}/placement` | `namespace:read` | tenant-scoped; foreign = opaque `NOT_FOUND` |
| `PUT /v1/namespaces/{id}/placement` | `placement:manage` (admin) | tenant-scoped, revision CAS, audited |
| `allowedRegions` at namespace creation | `namespace:create` | validated against the cell |
| Studio region panel | `ops:read` | read-only |
| Divergence report, re-ingest | platform operator | owner role on the current authority, a read-only role on the former primary |
| Peer probe answer | none: it reads `GET /region` on the peer's admin listener (internal network, like `/metrics`) | verified TLS to operator-configured URLs; answers cell, region, role, epoch only (D1598) |

## Finite ceilings

| Resource | Default | Hard ceiling |
|---|---:|---:|
| Regions per cell | 2 | 3 |
| Peer endpoints per process | 1 | 2 |
| Role refresh interval | 1,000 ms | 250–10,000 ms |
| Peer probe interval / timeout | 5 s / 2 s | 1–60 s / 10 s |
| Peer probe response size | 4 KiB | 4 KiB |
| Drain bound (engines give back shards; command waits) | 60 s | 600 s |
| Commit deadline in `sync_quorum` cells | 10 s | 60 s (≥ 2 × statement timeout) |
| `Retry-After` on `REGION_*` refusals | 5 s | 1–60 s |
| Durability lease (`sync_quorum`) | 3 s | 1–30 s, ≥ 2 × role refresh |
| Beacon rows | one per live process | 1,024 per cell (refuse more) |
| Key fingerprints | one per configured key id | 13 key names × 4 ids |
| Blob replica grace after activation | 900 s | 3,600 s |
| Blob verifier sample per round / interval | 100 / 60 s | 1,000 / ≥ 10 s |
| Fan-out tenants per transaction | 100 | 1,000 |
| Attestation and reason text | 8–2,000 chars | 2,000 chars |
| `allowedRegions` entries | ≤ regions of the cell | 3 |
| Namespaces checked by one `region-topology` | all of the cell | 100,000 (refuse above; operator splits the cell) |
| Divergence report rows per table | 10,000 | 100,000 |
| Re-ingested events per command | 1,000 | 10,000 |
| Minimum interval between two activations | 60 s | not lowerable below 10 s |

Effective value = min(request, deployment setting, hard ceiling). Zero is not unlimited: a zero grace
means "missing is lost at once"; a zero fan-out batch is refused at start. Existing quotas (content
bytes, `events_per_second`) apply to re-ingest.

## Failure and recovery matrix

| Failure | Expected outcome | Proven by |
|---|---|---|
| Active region lost after COMMIT sent, before acknowledgment (`sync_quorum`) | commit durable in the standby or absent; the client's keyed retry gets the original result or creates it once | T032-023 |
| Partition during synchronous wait, statement timeout or cancel fires | no acknowledgment; `UNAVAILABLE` unknown outcome; local-only commit discarded by rewind | T032-023, T032-024 |
| Former primary restarts while commits wait | locally committed, absent in the new authority; the barrier never completes there, so it serves nothing; reported by divergence report | T032-023, T032-058, T032-048 |
| Keyed replay, inbox duplicate, worker `already_applied`, status read in the former region after a cancelled wait | replay/duplicate answers wait for their own quorum-waited commit (never sent during the partition); reads stop with `DURABILITY_UNCONFIRMED` when the lease lapses; negative controls acknowledge and fail | T032-023, T032-058 |
| Epoch-unaware binary in the standby region after promotion | no lease (trigger), no write (role default read-only) | T032-059 |
| In-region failover inside a two-region cell | zero acknowledged loss under D1591's rule; reported not guaranteed otherwise | T032-062 |
| Key fingerprint differs in the target region | blocking keys refuse activation; others warn and are recorded | T032-061 |
| Lost journal tail re-used by new events | sink receives `run_tombstone` before the run's first new-epoch record; follower whose sequence replicated is not reset | T032-033, T032-060 |
| Async loss window | counted against the ledger; listed by divergence report | T032-028, T032-048 |
| Crash in the middle of `region-activate` | nothing changed (one transaction); retry with the same expected epoch | T032-020 |
| Activation run twice | second refused `EPOCH_CONFLICT` | T032-020 |
| Replica with a pre-activation lease snapshot | fenced by epoch and generation; negative control fails without the check | T032-021 |
| Worker reports a lease the authority lacks | `STALE_ATTEMPT`; step re-run with the same idempotency key | T032-030 |
| Duplicate webhook / connector re-read / outbox republish | inbox `DUPLICATE`; same delivery ids; no second run | T032-030 |
| Timeout and signal racing across a failover | the authority's first commit wins; the other is an audited no-op | T032-030 |
| Lost broker delivery (former region's NATS gone) | reconciler re-hints ready work in the new region | T032-031 |
| Compensation in flight at the loss; compensation that keeps failing | retried in the new region with the same key; manual review persists | T032-032 |
| Engine restart during fan-out; control plane restart during drain | fan-out resumes idempotently; drain state is durable | T032-056 |
| Missing replicated blob | retryable within grace, `BLOB_LOST` after; digest never bypassed | T032-036 |
| OpenBao snapshot older than the database | credential resolution fails closed, typed | T032-037 |
| Peer reports a higher epoch | `SUPERSEDED` within one interval; never re-activates | T032-027 |
| Quota/ceiling exhaustion (fan-out batch, report rows, re-ingest, residency entries) | refused before work, typed | T032-055 |

## Observability

- **Spans** (`sagat.region`, `sagat.cell`, `sagat.region.epoch`, `sagat.outcome`; no tenant on platform
  spans): `region.activate`, `region.drain`, `region.role.refresh` (sampled), `region.peer.probe`,
  `region.fanout.batch`, `region.blob.verify`.
- **Metrics** (fixed labels only): `region_role{role}`, `region_epoch`,
  `region_last_activation_timestamp_seconds{kind}`, `region_replication_lag_seconds{standby_region}`,
  `region_replication_lag_bytes{standby_region}`, `region_protection{state}`,
  `region_refusals_total{reason,surface}`, `region_fenced_total{kind}`, `region_peer_epoch_ahead`,
  `region_fanout_pending`, `db_commit_outcome_unknown_total`, `blob_replication_missing{kind}`,
  `blob_replica_reads_total{outcome}`, `region_durability_lease_valid`, `region_durability_refusals_total`,
  `region_key_mismatch{key}`, `region_inregion_rpo_guaranteed`, `region_tombstones_total{kind}` (kind ∈
  {run, former_only}). `key` ∈ the closed key names of research §3b. Label sets: role ∈ {active, draining, passive, superseded,
  unconfigured}; kind ∈ {planned, unplanned} or {shard, singleton}; reason ∈ {passive, draining,
  superseded}; surface ∈ {rest, webhook, grpc}; state ∈ {protected, unprotected, unknown}; standby_region ∈
  the cell's configured regions (≤ 3); outcome ∈ {found, not_replicated, lost}.
- **Alerts** (`infra/observability/alerts.yaml`): `SagatRegionReplicationLag`, `SagatRegionUnprotected`,
  `SagatRegionPeerEpochAhead` (critical), `SagatRegionRoleMismatch` (critical), `SagatRegionDrainingTooLong`,
  `SagatRegionFanoutStuck`, `SagatBlobReplicationMissing`, `SagatRegionDurabilityUnconfirmed` (critical),
  `SagatRegionKeyMismatch`, `SagatRegionSyncWaitersStuck` (the monitor sees platform backends in
  wait event `SyncRep` longer than the commit deadline).
- **Audit**: history kinds `INIT`, `DRAIN`, `UNDRAIN`, `ACTIVATE_PLANNED`, `ACTIVATE_UNPLANNED`,
  `MODE_CHANGE`, `TOPOLOGY_CHANGE`, `DIVERGENCE_REPORT`, `REINGEST`, `TOMBSTONES`; tenant actions `region.activated`,
  `namespace.placement.changed`, `event.reingested`.
- **Logs**: commands print one JSON line with closed reason codes; no SQL text, payload, attestation text
  or tenant names at INFO or above; the attestation lives in the history row only.

## Migration and compatibility

- Migration `<ts>_region_authority` (timestamp assigned at implementation, after every migration present
  then) is additive: authority, history, placements, fan-out pair, beacons and key fingerprints;
  `region_epoch` on `scheduler_shards` and `engine_leases` (default 0) and on `run_events` (default 0, then
  default `sagat_region_epoch()`); the lease triggers of D1593 (inert while no authority row exists);
  grants; append-only triggers; storage-classification and 028 time-classification entries. The down
  migration refuses while `region_history` has rows (its audit must not be dropped by a rollback); a
  rollback of the code is a rollback of the binaries.
- With no authority row every new path is inert (role `UNCONFIGURED`), so engines of the previous version
  and this one run together on a single-region cell exactly as ADR-0020 D243 allows.
- `region-init` sets `ALTER ROLE sagat_app SET default_transaction_read_only = on` in its transaction;
  from then on only processes that identify their region and epoch can write or take leases (D1593). No
  operator flag. The chart also refuses to render region values with an image older than this feature.
- OpenAPI grows (new paths, an optional field, a header, a parameter, reasons); AsyncAPI grows (one
  message, one enum value); `worker.proto` grows (trailer values, optional response fields) at the next
  free minor; SDKs add behaviour without changing retry rules.

## Complexity Tracking

| Decision | Cost | Simpler alternative rejected because |
|---|---|---|
| Amend the single-region Delivery Constraint | governance change | keeping one region leaves G250/G251 open and makes regional loss a restore-from-backup event |
| Epoch in the fence plus activation invalidation | one column read in an existing row read | generation bump alone works but hides provenance and leaves replicated memberships to expire slowly |
| Commit guard for `sync_quorum` | a client-side deadline and a statement-timeout exemption on COMMIT | relying on the server statement timeout acknowledges local-only commits (D1566) |
| Peer probe | an internal HTTP client and a loop | operator fencing alone leaves a healed former region acting until someone notices |
| Tenant home cell | no cross-cell tenant | namespace-level cells need a cross-cell directory, i.e. a second authority |
| Durability lease and quorum-waited replays | one small commit per process per refresh; a commit per replay | guarding only COMMIT lets replays, duplicates and reads acknowledge local-only work (review Blocking 1) |
| `run_events.region_epoch` | 8 bytes and a function call per journal row | keys without an epoch make sinks keep the lost timeline (coordinator C2) |
| Lease triggers and read-only role default | a trigger on four low-volume tables; a connection setting | an operator flag cannot stop an old binary in the standby region (review Should-fix 4) |

## Delivery sequence

1. Foundation: domain rules, migration, ports/adapters, role watcher, configuration, schema contract test.
2. US01: passive/draining behaviour, `/v1/region`, `region-init|status`, monitor, alerts, chart.
3. US02: drain/undrain, planned activation transaction, epoch fence, switchover e2e, runbook.
4. US03: commit guard, unplanned activation with preflight and attestation, peer probe, degradation,
   partition and region-loss-mid-commit faults, rejoin.
5. US04: duplicates, lost broker, compensation, follower reset (server, Studio, SDKs), fan-out and notice,
   blob grace and verifier, five-SDK conformance.
6. US05: placement view, residency, topology validation, routing manifest, Studio.
7. US06: region drill, benchmark, report.
8. US07: divergence report, re-ingest.
9. Polish: canonical contract merge, docs, reviews, runtime-role sweep, gates.

## Verification plan

Every task names its test; a task is ticked only with the command and its output. Real PostgreSQL
clusters with streaming replication (two per test, started by `tests/support/region.rs`), real NATS per
region, filesystem blob stores with a controllable replicator, a TCP proxy for RTT/loss/partition
(`tests/support/wan.rs`). Docker may be unavailable on a developer machine; the harness uses the
PostgreSQL server binaries named by `PGBIN`, as `infra/dr/drill.sh` does. The drill and benchmark run on a
stated environment and report every failure; a criterion that fails stays failed in the report.
Independent read-only security and correctness reviews before declaring the feature done (T032-052).
