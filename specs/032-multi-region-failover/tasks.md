# Tasks: Multi-region replication and active-passive failover

**Input**: [spec.md](./spec.md), [plan.md](./plan.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/](./contracts/), [quickstart.md](./quickstart.md).

Status: Proposed — not started. No task is done. A task is ticked only with the named test passing and the
command and its output recorded (AGENTS.md "Start here"). Tests are written first and seen failing for
the right reason (AGENTS.md rule 18); races use barriers or held locks, never sleeps.

**Format**: `- [ ] T032-NNN [P?] [USn?] Description — files — requirements — evidence`. `[P]` = different
files and no dependency on an unfinished task.

## Phase 1: Setup

- [ ] T032-001 Owner gate: record the answers to O1560 (acceptance of ADR-0067's **direction**), O1561 and
  O1562 in `docs/owner-decisions.md`. ADR-0067 stays `Proposed (direction accepted)`; its `Accepted`
  status, ADR-0009's line and the constitution amendment are T032-063, after the evidence exists. No
  implementation task starts before this. — FR-032-031 — evidence: the owner's recorded decision.

IDs T032-058 to T032-064 were added in the review round of 2026-10-27 and keep their numbers (IDs are never
renumbered); they sit in the phase where they run.
- [ ] T032-002 [P] Add test targets `integration_multi_region_failover`, `fault_multi_region_failover`,
  `contract_multi_region_failover`, `e2e_multi_region_failover`, `perf_multi_region_failover` to
  `tests/Cargo.toml` with `main.rs` in `tests/{integration,fault,contract,e2e,performance}/multi-region-failover/`.
  — evidence: `cargo test -p sagat-tests --test contract_multi_region_failover` compiles and runs zero tests.
- [ ] T032-003 [P] Region-pair harness: `tests/support/region.rs` (three PostgreSQL clusters from `PGBIN`
  or Testcontainers — primary, in-region standby, cross-region standby — streaming replication
  `sync_quorum` (`ANY 2 (eu-west-2, eu-central-1)` by default)/`async`, promote, clean stop with final LSN,
  restart, rewind) and `tests/support/wan.rs` (TCP proxy with injected RTT, loss, one-way and full partition,
  held by a barrier). — FR-032-030 — evidence: harness self-tests in `tests/integration/multi-region-failover/harness.rs`
  (a write on the primary is replayed; a partition stops replay; promotion makes the standby writable).
- [ ] T032-004 [P] Compose region profile `infra/docker/compose.region.yaml`: pinned images, loopback-only
  ports, health checks, two of each component (AGENTS.md rule 16). — FR-032-030 — evidence:
  `docker compose -f infra/docker/compose.yaml -f infra/docker/compose.region.yaml config` and the smoke
  test of the compose job.

## Phase 2: Foundational (blocks every story)

- [ ] T032-005 Domain `crates/domain/src/region.rs`: `RegionName`, `CellId`, `Epoch`, `Role`,
  `AuthorityState`, `ReplicationMode`, `Protection`, `ActivationKind`, `Residency`; the pure role function
  (plan truth table) and the transitions with their closed refusals (data-model). No serde_json, SQLx or
  HTTP. — FR-032-001..003, 010, 023 — evidence: `crates/domain/tests/region.rs` including a property test
  that the epoch never decreases or skips through any sequence of transitions.
- [ ] T032-006 Migration `crates/infra-postgres/migrations/<ts>_region_authority.{up,down}.sql`:
  `region_authority` (singleton, `cardinality(regions) BETWEEN 2 AND 3`, `active_region = ANY (regions)`,
  epoch trigger), `region_history` (append-only triggers, `reason` 8–2,000, attestation required for
  `ACTIVATE_UNPLANNED`), `namespace_placements`, `region_fanout`, `region_epoch_notices`,
  `region_beacons`, `cell_key_fingerprints`, `region_epoch bigint NOT NULL DEFAULT 0` on
  `scheduler_shards` and `engine_leases`, `run_events.region_epoch` (`DEFAULT 0`, then
  `sagat_region_epoch()`), the function `sagat_region_epoch()`, the trigger `region_lease_guard()` on
  `scheduler_shards`, `engine_leases`, `scheduler_members` and `connector_leases` (data-model "Functions
  and triggers"), the `ops.region.activated` kind on `webhook_subscriptions`, grants, entries in
  `crates/infra-postgres/storage-classification.json` and in 028's
  `crates/infra-postgres/time-classification.json` (class `infrastructure`); the down migration refuses
  when history exists. — FR-032-002, 006, 016, 017, 018, 020, 023, 031, 034 — evidence: up/down/up in
  `tests/integration/platform-foundation/migrations.rs`; storage classification test of
  `integration_security_multitenancy`.
- [ ] T032-007 Ports and adapter: `crates/application/src/region.rs` (`RegionAuthorityStore`,
  `ReplicationObserver`, `PeerProbe`) and `crates/infra-postgres/src/region.rs` (read authority with
  `pg_is_in_recovery()`, replay LSN and timestamp; transitions; placement; fan-out); every transaction
  through `begin_safely` (ADR-0014). — FR-032-002, 003, 007 — evidence: real-PostgreSQL tests in
  `tests/integration/multi-region-failover/store.rs`; runtime-role test that `sagat_app` cannot write
  `region_authority` or `region_history` (`tests/integration/platform-foundation/runtime_privileges`).
- [ ] T032-008 Role watcher in `crates/application/src/region.rs` (refresh every `SAGAT_REGION_REFRESH_MS`,
  single-flight, cached for the request path, sticky `SUPERSEDED`) wired into
  `apps/control-plane/src/lib.rs` and `apps/execution-engine/src/loops.rs`. — FR-032-003 — evidence: unit
  tests with a fake store and clock for every row of the truth table, including the fail-closed rows.
- [ ] T032-009 [P] Contract test refusing `UNLOGGED`, `CREATE SEQUENCE`, `serial`/`bigserial` and identity
  columns in every migration (research D1587) — `tests/contract/multi-region-failover/schema.rs` —
  FR-032-031 — evidence: the test passes on today's migrations and fails on a planted fixture.
- [ ] T032-010 [P] Configuration: `SAGAT_CELL_ID`, `SAGAT_REGION`, `SAGAT_REGION_REFRESH_MS`,
  `SAGAT_PEER_REGION_URLS` (≤ 2, https), `SAGAT_PEER_CA_FILE`, `SAGAT_REPLICATION_MONITOR_DATABASE_URL`,
  `SAGAT_CELL_ENDPOINTS`, `SAGAT_COMMIT_DEADLINE_MS`, `SAGAT_BLOB_REPLICA_GRACE_SECS`,
  `SAGAT_REGION_RETRY_AFTER_SECS` in `apps/control-plane/src/config.rs` and
  `apps/execution-engine/src/settings.rs`, refused outside the plan's ceilings; usage text; regenerated
  `docs/reference/cli.md`. — FR-032-001, 032 — evidence: config unit tests;
  `SAGAT_DOCS_BLESS=1 cargo test -p sagat-tests --test contract_developer_experience` after reading the diff,
  then without the switch.

## Phase 3: US01 — Run a cell with a standby region and see its state (P1)

**Independent test**: harness up, `region-init`, roles, `/v1/region`, refusals, protection.

- [ ] T032-011 [US1] Write first: `tests/integration/multi-region-failover/roles.rs` — unconfigured
  cell unchanged; after init eu-west `ACTIVE`, eu-central `PASSIVE`; `/v1/ready` 503 in eu-central; every
  route of the OpenAPI document refused `REGION_PASSIVE` in eu-central with zero statements other than
  the role refresh (counted with `pg_stat_statements` or a statement-counting pool hook). — FR-032-003,
  004 — SC-032-005.
- [ ] T032-012 [US1] Control-plane role middleware `crates/api-http/src/region.rs` (refusal before
  authentication of a write, `Retry-After`, no redirect), `GET /v1/region`, readiness by role in
  `crates/api-http/src/routes/health.rs`, webhook ingress refusal. — FR-032-004, 025 — evidence: T032-011
  passes; `contract_rest_platform_foundation` live conformance of `getRegionStatus`.
- [ ] T032-013 [US1] Engine role gating in `apps/execution-engine/src/loops.rs` and
  `crates/scheduler/src/shards.rs` (no shards, no singletons, no writing loops unless `ACTIVE`; drain gives
  back), `UNAVAILABLE` with `x-sagat-error: REGION_*` in `crates/api-grpc/src/service.rs`. — FR-032-005 —
  evidence: `tests/integration/multi-region-failover/engine_roles.rs`; `/livez` stays healthy in passive.
- [ ] T032-014 [US1] `region-init` and `region-status` in `apps/control-plane/src/{cli.rs,region.rs}`
  (inputs, JSON output, exit codes of `contracts/operator-cli.md`). — FR-032-026 — evidence:
  `apps/control-plane/tests/region_cli.rs` against real PostgreSQL, including `ALREADY_INITIALIZED`,
  `IN_RECOVERY` on a standby, and the runtime role's default becoming read-only.
- [ ] T032-015 [US1] Replication monitor `crates/scheduler/src/region.rs`: lag in bytes and seconds per
  standby region, synchronous quorum membership by `application_name` convention, protection; zero gauges
  on failure (ADR-0017); metrics of the plan. — FR-032-010, 027 — evidence:
  `tests/integration/multi-region-failover/monitor.rs` (an in-region-only quorum reports `UNPROTECTED`; a
  paused standby raises lag; no monitor connection → `UNKNOWN`).
- [ ] T032-016 [P] [US1] Alerts and dashboard: `infra/observability/alerts.yaml` (the seven alerts of the
  plan) and `infra/observability/dashboards/`. — FR-032-027 — evidence: `contract_observability_operations`
  (alerts name only emitted metrics; labels are fixed).
- [ ] T032-017 [P] [US1] Helm: region values, region-local database and bucket secrets, peer URLs and CA,
  `infra/helm/examples/multi-region.yaml`; render refuses a cell with one region, a peer URL that is not
  https and an image older than this feature. — FR-032-001, 031 — evidence: `contract_scalability_ha`
  render tests extended.

## Phase 4: US02 — Planned switchover without losing acknowledged work (P1)

**Independent test**: ledger workload, drain → stop → promote → activate → route, zero missing.

- [ ] T032-018 [US2] Write first: `tests/e2e/multi-region-failover/us02_switchover.rs` with the ledger of
  acknowledgments, journal digest (`infra/dr/journal_digest.sql`) of the cut on both sides and
  `verify_run` of every run. — FR-032-012 — SC-032-001.
- [ ] T032-019 [US2] `region-drain` / `region-undrain` and the `DRAINING` behaviour (admissions refused,
  reads and reports of granted leases served, shards and singleton leases given back within the drain
  bound). — FR-032-004, 005, 012 — evidence: `tests/integration/multi-region-failover/drain.rs` (a report
  of a granted lease commits during the drain; a claim is refused; undrain restores `ACTIVE`).
- [ ] T032-020 [US2] Activation transaction in `crates/infra-postgres/src/region.rs`: CAS, not in recovery,
  planned evidence (`pg_last_wal_replay_lsn()` ≥ final LSN, NULL refused), history row with cut point,
  shard rows generation+1/expired/epoch, `engine_leases` and `scheduler_members` removal, `region_fanout`
  row, `connector_leases` removal, `TOO_SOON`. — FR-032-007, 012, NFR-032-003 — evidence: `tests/fault/multi-region-failover/activation.rs` with a
  failure injected after each statement (nothing changed), two concurrent activations (one wins,
  `EPOCH_CONFLICT`), planned activation refused below the LSN.
- [ ] T032-021 [US2] Epoch in the ADR-0020 fence and the singleton leases:
  `crates/infra-postgres/src/partitioning.rs`, `crates/scheduler/src/shards.rs`, the `engine_leases`
  statements. — FR-032-006 — evidence: `tests/fault/multi-region-failover/fence.rs` — a replica holding a
  pre-activation snapshot is refused (`region_fenced_total{kind="shard"}`); with the epoch predicate
  removed the same test fails (negative control, tried and recorded).
- [ ] T032-059 [US2] Database enforcement (D1593): connection settings `sagat.region` and the read-only
  override in `crates/infra-postgres/src/pool.rs`, pool rebuild on role or epoch change in the role
  watcher, `region-init`'s role default. — FR-032-006, 031 — evidence:
  `tests/fault/multi-region-failover/old_binary.rs` — an engine connected without the settings cannot take
  a shard, singleton, membership or connector lease (trigger) nor write (25006) in an initialized cell, in
  either region and right after promotion; a region-aware engine can; single-region cells are unchanged.
- [ ] T032-022 [US2] Runbook `docs/runbooks/multi-region-failover.md`: topology, init, switchover,
  checks after activation (shards owned, lag, fan-out), undrain. — NFR-032-006 — evidence: every command in
  it is executed by T032-018 or the drill.

## Phase 5: US03 — Unplanned failover without a split brain (P1)

**Independent test**: partition with a live former primary; acknowledgments counted per region.

- [ ] T032-023 [US3] Write first: `tests/fault/multi-region-failover/commit.rs` — region loss after COMMIT
  sent and before acknowledgment (WAL received / not received by the standby); partition during the
  synchronous wait with the 5 s statement timeout, with an explicit cancel and with a backend terminate;
  former primary restarted while commits wait; an abandoned connection with
  `client_connection_check_interval = 0` (no local commit while abandoned). Read paths after a cancelled
  wait and after a restart: a keyed replay from `command_dedupe` and from an approval/update receipt, an
  inbox `DUPLICATE`, a connector duplicate acknowledgment, a worker `already_applied`, a run status read.
  Assert no acknowledgment after the quorum is lost and that keyed retries create each operation once;
  negative controls with the replay commit and the durability lease disabled. — FR-032-009 —
  SC-032-002.
- [ ] T032-024 [US3] Commit guard (research D1566) in `crates/infra-postgres/src/pool.rs` and the
  transaction wrapper: in `sync_quorum` cells `SET LOCAL statement_timeout = 0` before COMMIT, client-side
  commit deadline that abandons the connection, `COMMIT_OUTCOME_UNKNOWN`, `db_commit_outcome_unknown_total`;
  warning detection if the driver exposes notices. — FR-032-009 — evidence: T032-023 passes; the same tests
  fail with the guard disabled (negative control).
- [ ] T032-058 [US3] Durability lease and quorum-waited acknowledgments (D1590): beacon and barrier in
  `crates/application/src/region.rs` and `crates/infra-postgres/src/region.rs`; `pg_current_xact_id()`
  and `synchronous_commit` ≥ `on` in the replay paths of `crates/infra-postgres/src/{definitions,execution,events,connectors}.rs`;
  `DURABILITY_UNCONFIRMED` in `crates/api-http/src/region.rs` and `crates/api-grpc/src/service.rs`; start
  check of `client_connection_check_interval`; `SagatRegionDurabilityUnconfirmed` and
  `SagatRegionSyncWaitersStuck`. — FR-032-009 — evidence: the read-path cases of T032-023 pass; beacon cost
  measured by T032-046.
- [ ] T032-025 [US3] Unplanned activation: attestation, reason, preflight checks of
  `contracts/operator-cli.md` (blocking vs warning, unevaluable = blocked), recorded in the history row. —
  FR-032-013, 020 — evidence: `apps/control-plane/tests/region_cli.rs` (missing attestation refused; a
  key-ring gap blocks; an unreachable OpenBao warns and is recorded).
- [ ] T032-026 [US3] `region-mode` and protection after failover: the new primary reports `UNPROTECTED`
  until a cross-region standby attaches; `SagatRegionUnprotected` fires. — FR-032-010 — evidence:
  `tests/integration/multi-region-failover/monitor.rs` scenario after promotion.
- [ ] T032-027 [US3] Peer probe client `crates/infra-egress/src/peer.rs` (configured https URLs only, CA,
  no redirect or proxy, 2 s, 4 KiB) reading `GET /region` on the peers' admin listener (D1598), the
  route itself on the control-plane and engine admin listeners, and the loop; `SUPERSEDED` sticky. — FR-032-011 — evidence:
  `tests/integration/multi-region-failover/peer.rs` (higher epoch → superseded within one interval; a
  redirect, a plain-http URL and an oversized answer are refused; a lower epoch changes nothing).
- [ ] T032-028 [US3] `tests/e2e/multi-region-failover/us03_failover.rs`: `sync_quorum` partition with live
  former primary (zero missing, zero acknowledgments from eu-west after the cut) and `async` (loss counted
  and windowed). — FR-032-013, NFR-032-001 — SC-032-002, SC-032-003, SC-032-004.
- [ ] T032-029 [US3] Former primary rejoin and failback: processes on a rewound former primary come up
  `PASSIVE`; failback is a planned switchover; runbook sections. — FR-032-014 — evidence:
  `tests/integration/multi-region-failover/rejoin.rs` with `pg_rewind` from the harness.
- [ ] T032-062 [US3] In-region failover inside a two-region cell (D1591): monitor gauge
  `region_inregion_rpo_guaranteed` (quorum includes the in-region standby, or the promotion verified the
  highest synchronous flush LSN), the in-region promotion check in the runbook. — FR-032-010 — evidence:
  `tests/e2e/multi-region-failover/inregion_failover.rs` (zero ledger loss with `ANY 2 (eu-west-2,
  eu-central-1)`; with `ANY 1 (eu-central-1)` the gauge is 0 and the verified promotion catches up before
  promoting) — SC-032-001.

## Phase 6: US04 — Work continues correctly after a failover (P1)

**Independent test**: failover at injected points; `verify_run`; provider and receiver ledgers.

- [ ] T032-030 [US4] `tests/fault/multi-region-failover/duplicates.rs`: webhook resent after a replicated
  and a non-replicated 202; Kafka/SQS connector re-read from the replicated position; outbox and
  deliveries republished with the same ids; worker report of a lease the authority lacks (`STALE_ATTEMPT`,
  re-run with the same idempotency key); timeout and signal racing across the failover (first commit
  wins, the other audited). — FR-032-008, 015, 016, 017 — SC-032-006.
- [ ] T032-031 [US4] Lost broker delivery: the former region's NATS is gone; the reconciler re-hints
  ready work in the new region; nothing depends on a former-region message. — FR-032-021 — evidence:
  `tests/fault/multi-region-failover/broker.rs`.
- [ ] T032-032 [US4] Compensation across a failover: a compensation in flight at the loss is retried with
  its key; a compensation that keeps failing stays in manual review in the new region. — FR-032-017 —
  evidence: `tests/fault/multi-region-failover/compensation.rs`.
- [ ] T032-033 [US4] Follower reset on the server: `GraphChanges.epoch`, `X-Sagat-Region-Epoch`, optional
  `epoch` on `getRuntimeGraphChanges` and `streamRunChanges` (`crates/api-http/src/routes/execution.rs`);
  the authority's event at the cursor has another epoch → `reset: true` with the whole graph; same epoch →
  no reset. — FR-032-018 — evidence:
  `tests/contract/multi-region-failover/streams.rs` and an e2e case with reused sequences (SC-032-007).
- [ ] T032-034 [P] [US4] Studio follower `frontend/apps/studio/src/api/follow.ts` sends and stores the
  epoch and replaces its graph on reset. — FR-032-018 — evidence: `follow.test.ts` cases; Playwright run
  over a failover in `frontend/e2e/tests/region-failover.spec.ts`.
- [ ] T032-035 [US4] Fan-out loop `crates/scheduler/src/region.rs` + SQL: per tenant `region_epoch_notices`,
  audit `region.activated`, `ops.region.activated` deliveries (UUIDv5 event id), cursor; resumable. —
  FR-032-016, 028, NFR-032-003 — evidence: `tests/integration/multi-region-failover/fanout.rs` (engine killed mid-batch;
  each tenant exactly one audit record; each opted-in subscription exactly one delivery) — SC-032-012.
- [ ] T032-060 [US4] Epoch downstream (C2, D1594): span ids with the event's epoch in
  `crates/application/src/telemetry.rs` (unchanged at epoch 0, golden test), the tombstone rule (a run's
  next event has a later epoch → `run_tombstone` for earlier epochs from that sequence) as a pure function
  shared with 020's projector, and `region-emit-tombstones`. — FR-032-017, 034 — evidence:
  `tests/integration/multi-region-failover/downstream.rs` with a test sink keyed `(tenant, run, seq,
  epoch)` (020's key with its epoch component): after an unplanned activation with lost events the sink
  holds the authority's events and tombstones covering the lost ones; a planned activation emits none —
  SC-032-007.
- [ ] T032-064 [US4] CDC across a failover (C12), when 021 is present: in `sync_quorum` the flush LSN is
  reported only after the quorum-waited inbox commit; under `async` the runbook states the loss and 021's
  slot re-creation/backfill; divergence report and re-ingest cover CDC inbox events. — FR-032-015 —
  evidence: `tests/fault/multi-region-failover/cdc.rs` gated on 021's connector kind; skipped and stated
  as such while 021 is absent.
- [ ] T032-036 [US4] Blob replica grace in `crates/application/src/blobs.rs` and
  `crates/infra-blobstore/src/backend.rs` (`BLOB_NOT_REPLICATED` within the grace from `activated_at`,
  `BLOB_LOST` after, digest check unchanged) and the leased verifier. — FR-032-019 — evidence:
  `tests/integration/multi-region-failover/blobs.rs` (object arrives within grace → step resumes; never
  arrives → `BLOB_LOST`; wrong bytes → `Corrupt`, never served) — SC-032-008.
- [ ] T032-037 [US4] Secrets readiness: engine start in a region checks key-ring coverage; a credential
  version missing in the region's OpenBao fails closed with the existing typed error and no fallback. —
  FR-032-020 — evidence: `tests/integration/multi-region-failover/secrets.rs` with a real OpenBao snapshot
  older than the database.
- [ ] T032-061 [US4] Key inventory (C11, D1595): fingerprint writers for the keys of research §3b in the
  control plane and engine, passive comparison and `region_key_mismatch{key}`, `SagatRegionKeyMismatch`,
  preflight checks `key_fingerprints_blocking` / `_warning` in `apps/control-plane/src/region.rs`. —
  FR-032-013, 020 — evidence: `tests/integration/multi-region-failover/keys.rs` (a different
  flow-control digest key blocks activation; a different decision-link key warns and is recorded; no key
  material in the table, logs or output) — SC-032-010.
- [ ] T032-038 [US4] Five SDKs: close pooled connections and resolve again on `REGION_*`; send the epoch on
  run streams; `sdks/SPEC.md` §5/§6.2 text (retry rules unchanged); `sdks/{typescript,go,python,dotnet,java}`.
  — FR-032-025, 018 — evidence: each SDK's unit tests and the per-SDK lints of `sdks/AGENTS.md`.
- [ ] T032-039 [US4] Conformance scenarios `tests/contract/sdk-conformance/scenarios/region-*.json` of
  `contracts/sdk-conformance.md`, run in all five languages. — FR-032-008, 009, 018, 025 — SC-032-009 —
  evidence: the seven `SAGAT_SDK_SCENARIO` runs, five languages each.

## Phase 7: US05 — Namespace placement and residency (P2)

**Independent test**: placements, residency refusals, topology refusal, cross-tenant battery.

- [ ] T032-040 [US5] Write first: `tests/integration/multi-region-failover/placement.rs` and the
  cross-tenant battery over the new routes (`tests/contract/security-multitenancy/rest/battery.rs` is
  driven by the OpenAPI document). — FR-032-022, 023 — SC-032-010.
- [ ] T032-041 [US5] Placement: domain residency rules, store, `GET`/`PUT /v1/namespaces/{id}/placement`,
  `allowedRegions` on `createNamespace`, permission `placement:manage` (admin) in
  `crates/domain/src/identity.rs`, audit `namespace.placement.changed` in the same transaction. —
  FR-032-022, 023, 028 — evidence: T032-040 passes; role-matrix test of feature 009 updated.
- [ ] T032-042 [US5] `region-topology` with residency validation under the authority row lock (≤ 100,000
  namespaces). — FR-032-023, 026 — evidence: `apps/control-plane/tests/region_cli.rs` (`RESIDENCY_CONFLICT`
  with the count; `ACTIVE_REMOVED`).
- [ ] T032-043 [US5] `region-routing-manifest` (cell endpoints and tenant slugs only). — FR-032-024 —
  evidence: CLI test asserting the closed output keys and no other tenant field.
- [ ] T032-044 [P] [US5] Studio: placement and residency in `frontend/apps/studio/src/app/NamespacesPage.tsx`
  and `CreateNamespaceDialog.tsx`; Region panel `frontend/apps/studio/src/features/operations/RegionPanel.tsx`
  under `ops:read` with a table alternative, design tokens, no drawn values without data (FR-007-010). —
  FR-032-022, 033 — evidence: Vitest + Playwright with axe (WCAG AA); `npm run check:api`.

## Phase 8: US06 — Drill and measure RPO and RTO (P2)

- [ ] T032-045 [US6] `infra/dr/region-drill.sh` (scenarios of US06 including restart of the former primary
  and in-region failover, `--old-binary`, the ledger
  `tests/performance/multi-region-failover/ledger.sh`) writing
  `tests/performance/reports/multi-region-failover.md` with environment, injected RTT, failures and the six
  RTO components; fails on acknowledged loss in `sync_quorum`. — FR-032-030 — SC-032-011.
- [ ] T032-046 [US6] Benchmark `tests/performance/multi-region-failover/` (`perf_multi_region_failover`):
  commit and start latency p50/p95/p99 and transitions per second, `sync_quorum` vs `async`, at 2, 20 and
  80 ms injected RTT, with the cost of the beacon, the replay commits and `run_events.region_epoch`
  reported separately; a level not reached is stated. — NFR-032-005 — SC-032-011.
- [ ] T032-047 [US6] Drill section of `docs/runbooks/multi-region-failover.md` and the cross-link from
  `docs/runbooks/disaster-recovery.md` (its scope line). — NFR-032-006 — evidence: the drill run recorded.

## Phase 9: US07 — Reconcile what the former region accepted (P3)

- [ ] T032-048 [US7] `region-divergence-report`: read-only on both databases, bounded per table,
  identifiers and counts, file `0600`, history row with counts; its former-only runs feed
  `region-emit-tombstones` (T032-060). — FR-032-029 — evidence:
  `tests/integration/multi-region-failover/divergence.rs` (lists 100% of the ledger's missing
  acknowledgments in the async scenario; payload and secret canaries absent; truncation reported) —
  SC-032-003, SC-032-010.
- [ ] T032-049 [US7] `region-reingest-inbox` through the normal inbox admission (dedupe, quotas, audit
  `event.reingested`), bounded per command, refusals counted by reason. — FR-032-029 — evidence:
  `tests/integration/multi-region-failover/reingest.rs` (a duplicate is absorbed; a quota refusal is
  reported, not dropped).

## Phase 10: Polish and gates

- [ ] T032-050 Merge the contract fragments into `specs/001-platform-foundation/contracts/openapi.yaml`,
  `events.asyncapi.yaml` (and `events.baseline.json` after reading the diff), `worker.proto` (and
  `worker.baseline.json`); document the reasons in the OpenAPI `Error.details` description and
  `sdks/SPEC.md` §2; regenerate Studio types (`npm run api`, `npm run check:api`). — FR-032-031, 032,
  NFR-032-004 —
  evidence: `contract_asyncapi_events_webhooks_messaging`, the worker-protocol compatibility test, OpenAPI
  lint, `contract_multi_region_failover` — SC-032-013.
- [ ] T032-051 Documentation: `docs/architecture/overview.md`, `capacity-and-topology.md`,
  `storage-and-events.md`, `observability-slo.md`, `docs/runbooks/disaster-recovery.md`; ADR-0009 status
  line after acceptance; notes in the dependents named in spec "Dependencies" (020, 021, 026). —
  FR-032-031 — evidence: review of the diff; `python3 scripts/check-agent-config.py`.
- [ ] T032-052 Independent read-only reviews (security, correctness, architecture subagents), findings and
  outcomes recorded in `quickstart.md` §Reviews. — all — evidence: the review records.
- [ ] T032-053 Security: canaries in every command output and in `/v1/region`; the peer probe refuses
  non-configured URLs; runtime-role sweep `SAGAT_TEST_AS_RUNTIME_ROLE=1` over the new suites. —
  NFR-032-002 — SC-032-010.
- [ ] T032-054 Gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `python3 scripts/check-architecture.py`, `python3 scripts/check-agent-config.py`, the new and existing
  suites (`RUST_TEST_THREADS=2 cargo test --workspace --locked --no-fail-fast`), the compose smoke test. —
  FR-032-031 — SC-032-013 — evidence: commands and outputs, including any failure explained.
- [ ] T032-055 Ceilings: refusals before work for fan-out batch, report rows, re-ingest count, residency
  entries, peer URLs, drain bound and grace outside their ranges; zero is never unlimited. — FR-032-032 —
  evidence: `tests/integration/multi-region-failover/limits.rs`.
- [ ] T032-056 Restart and recovery: engine restart during fan-out, control plane restart during a
  drain, role watcher during a database outage (not ready, no stale `ACTIVE`). — FR-032-003, 028 —
  evidence: `tests/fault/multi-region-failover/restart.rs`.
- [ ] T032-063 Governance after evidence (D1592): once the reports of T032-045 and T032-046 exist with
  their environment and failures, the owner decides acceptance; then set ADR-0067 `Accepted`, add
  "Superseded in part by ADR-0067" to ADR-0009, and open the constitution amendment pull request with the
  text kept in ADR-0067 and its Sync Impact Report (D1583). — FR-032-031 — evidence: the owner's decision
  referencing both reports.
- [ ] T032-057 Traceability: `specs/032-multi-region-failover/traceability.md` mapping every FR/NFR/SC to
  tasks and tests with their state; the coordinator updates `docs/traceability.md`. — all — evidence:
  every FR has at least one passing test or an explicit open gap.

## Dependencies and order

- T032-001 blocks everything. Phase 2 blocks every story.
- US01 (T032-011..017) precedes US02 and US03 (they need roles and `region-init`).
- US02 precedes US03 (activation transaction and fence are shared; T032-020/021 first).
- US04 needs US02 or US03 activation (T032-020) and the harness.
- US05 needs Phase 2 and T032-012 only; it can run in parallel with US02–US04.
- US06 needs US02 and US03; US07 needs US03 (T032-028 produces divergence).
- Phase 10 after the stories it covers; T032-050 before any SDK release that uses the new fields;
  T032-063 only after T032-045 and T032-046.
- T032-059 (database enforcement) before T032-028; T032-058 with T032-024; T032-060 after T032-006 and
  before 020's sink tasks that rely on tombstones; T032-064 waits for 021.

## Parallel opportunities

- Phase 1: T032-002, T032-003, T032-004 together.
- Phase 2: T032-009 and T032-010 alongside T032-005..008.
- US01: T032-016 and T032-017 alongside T032-012..015.
- US04: T032-034 (Studio) and T032-038 (SDKs) alongside the server tasks once T032-033 fixes the wire
  shape; T032-036 and T032-037 independent of each other.
- US05: T032-044 (Studio) alongside T032-041..043.

## Implementation strategy

MVP = Phase 1, Phase 2, US01, US02 and US03: a cell that can be switched over and failed over with the
epoch fence and the commit guard, proven by the e2e and fault suites. Then US04 (correctness of work
after a failover, SDKs), then US05, US06 and US07. Each increment is merged with its tests, contracts,
runbook section and observability; nothing is called supported before its drill scenario has run.
