# Tasks: Performance, scalability and high availability

**Input:** `specs/010-scalability-ha/spec.md`, `plan.md`, `research.md`, `data-model.md`, 001 contracts.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [x] T010-001 Check FR-010-001..FR-010-007, SCs and ADRs; record gaps in `specs/010-scalability-ha/research.md`. — `research.md`: decisions D229–D260 and gaps G239–G270; ADR-0020.
- [x] T010-002 [P] Define fixtures and contract validation in `tests/contract/scalability-ha/fixtures/`. — `tests/contract/scalability-ha/fixtures/` (`base.yaml`, `refused.yaml`) and `infra/helm/examples/ha.yaml`, read by `contract_scalability_ha` (12 tests, which render the chart with `helm template`).
- [x] T010-003 [P] Document types, transitions and invariants in `specs/010-scalability-ha/data-model.md`. — `data-model.md`: the lease table and its transitions, the members, the weights, the counters of active runs, the indexes, the in-memory types, the invariants, the migrations.

## Phase 2: Foundation
- [x] T010-004 Create the necessary domain contracts/interfaces in `crates/scheduler/src/shards.rs` and invariant tests. — `crates/domain/src/sharding.rs` (shard, set, fair share, `plan`; `crates/domain/tests/sharding.rs`: 7 tests incl. a property test) and `crates/scheduler/src/shards.rs` (keeper, placement, snapshot; 9 tests with a store that has a clock the test moves).
- [x] T010-005 Implement ports/use cases in the appropriate module based on `specs/010-scalability-ha/plan.md`. — `crates/application/src/{shards,maintenance}.rs` (the `ShardStore` port, `Window`, `RunTx::holds_shard`, the outbox's `due_outbox`/`claim_outbox_ids`), `crates/scheduler/src/{passes,fairness,outbox}.rs`.
- [x] T010-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`. — `migrations/20261017000000_scheduler_shards` and `20261017000100_active_run_counts`, additive, each with a `.down.sql`; `integration platform-foundation migrations.rs` rolls back to empty and forward again (29 tables, both functions gone after the rollback).
- [x] T010-007 Create PostgreSQL integration and per-tenant isolation tests in `tests/integration/scalability-ha/`. — `tests/integration/scalability-ha/` (44 tests): leases, the fence, failover, fairness, the outbox, the lag gauges, the operator's weights, the counters, every pass divided between two replicas; tenant isolation in `fairness.rs` and `shards.rs`.

## Phase 3: US01 — Scale workers horizontally (P1)
**Independent test:** Given the queue grows and quota allows; When worker pools scale; Then throughput increases and no invalid claim exists.
- [x] T010-008 [P] [US01] Write a Given/When/Then test for scaling workers horizontally in `tests/e2e/scalability-ha/us01.rs`. — `tests/e2e/scalability-ha/us01.rs`: eight workers finish the same queue in less than three quarters of the time of two, and no task goes to two of them.
- [x] T010-009 [US01] Implement domain path and error handling for scaling workers horizontally in `crates/scheduler/src/shards.rs`. — `crates/scheduler/src/shards.rs` (the keeper) and `passes.rs` (the passes of a replica, concurrent, fenced).
- [x] T010-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`. — `crates/infra-postgres/src/{partitioning,maintenance,execution}.rs`, `crates/application/src/shards.rs`.
- [x] T010-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`. — metrics `scheduler_*`, `pool_*` (no tenant, run or activity as a label: `contract_observability_operations`), a span per keeper round and per action, the handover logs, the alerts and dashboard panels of `infra/observability`; implemented in `crates/scheduler` where the passes are, not in `crates/telemetry`.
- [x] T010-012 [US01] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US01 in `tests/fault/scalability-ha/`. — `tests/fault/scalability-ha/` (5 tests: replicas of different versions, a deadline racing a cancel, the runtime role's privileges, what the keeper logs) and `tests/fault/failover/` (6 chaos tests).

## Phase 4: US02 — Sustain wait volume (P1)
**Independent test:** Given 100k runs await timers/events; When idle workers are drained; Then processes are not dedicated per run.
- [x] T010-013 [P] [US02] Write a Given/When/Then test for sustaining wait volume in `tests/e2e/scalability-ha/us02.rs`. — `tests/e2e/scalability-ha/us02.rs`: 1 500 suspended runs add no task to the runtime, no connection, no lease and no worker; the benchmark takes the story to a million.
- [x] T010-014 [US02] Implement domain path and error handling for sustaining wait volume in `crates/scheduler/src/fairness.rs`. — the mechanism of US02 is the shard-first indexes, the bound by `now()` and the counters of active runs (`migrations`, `passes.rs`, `quotas.rs`); `crates/scheduler/src/fairness.rs` is US04's (10 tests).
- [x] T010-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`. — `crates/infra-postgres/src/{maintenance,quotas}.rs`.
- [x] T010-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`. — as T010-011.
- [x] T010-017 [US02] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US02 in `tests/fault/scalability-ha/`. — `tests/fault/scalability-ha/` and `tests/integration/scalability-ha/counts.rs` (the quota exact when starts race).

## Phase 5: US03 — Survive an engine node failure (P1)
**Independent test:** Given a scheduler shard owner goes down; When another takes over the lease; Then timers and work are resumed without inconsistency.
- [x] T010-018 [P] [US03] Write a Given/When/Then test for surviving an engine node failure in `tests/e2e/scalability-ha/us03.rs`. — `tests/e2e/scalability-ha/us03.rs`: two engines, one killed like a process (its runtime dropped); the other holds the 64 shards within a lease and every timer fires once; a clean stop is taken over within a keeper round.
- [x] T010-019 [US03] Implement domain path and error handling for surviving an engine node failure in `crates/infra-postgres/src/partitioning.rs`. — `crates/infra-postgres/src/partitioning.rs`.
- [x] T010-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`. — as T010-010.
- [x] T010-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`. — as T010-011.
- [x] T010-022 [US03] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US03 in `tests/fault/scalability-ha/`. — `tests/fault/failover/` (a replica cut off from the database stops by itself and rejoins; a replica killed mid-pass; a replica held up past its lease is refused; the broker down while a dispatcher changes hands).

## Phase 6: US04 — Isolate noisy neighbors (P1)
**Independent test:** Given tenant A fires a burst; When tenant B has ready tasks; Then fairness and quotas prevent starvation beyond an explicit budget.
- [x] T010-023 [P] [US04] Write a Given/When/Then test for isolating noisy neighbors in `tests/e2e/scalability-ha/us04.rs`. — `tests/e2e/scalability-ha/us04.rs`: a tenant of fifteen timers behind a burst of three hundred is served in the first pass (the burst has not half fired); the negative control, with fairness off, shows the quiet tenant served last.
- [x] T010-024 [US04] Implement domain path and error handling for isolating noisy neighbors in `infra/helm/values.yaml`. — `infra/helm/values.yaml`: the engine, the pools, the autoscalers and what limits them; the weights are the operator's command (`control-plane scheduling-weight`).
- [x] T010-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`. — as T010-010; the weights: `crates/infra-postgres/src/quotas.rs`.
- [x] T010-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`. — as T010-011.
- [x] T010-027 [US04] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US04 in `tests/fault/scalability-ha/`. — as T010-017 and `tests/integration/scalability-ha/fairness.rs` (the weights are audited, bounded, and a tenant cannot reach another's runs).

## Phase: Engineering design details (story-linked hardening)

- [x] T010-028 [US01] Implement scheduler owner leasing with generation in `crates/scheduler/src/shards.rs`. — `crates/scheduler/src/shards.rs`, `crates/infra-postgres/src/partitioning.rs`.
- [x] T010-029 [US01] Create a 1M WAITING synthetic load and p50/p95/p99 collection in `tests/performance/scenarios/`. — `tests/performance/scalability-ha/main.rs` and `run.sh` (the path of the task was a placeholder; the harness sits with the feature's other tests): 10 000, 100 000 and 1 000 000 suspended runs made through the platform's services, start and idle-tick p50/p95/p99, bytes per wait; the result is in `tests/performance/reports/scalability-ha.md`.
- [x] T010-030 [US01] Validate DB/broker/shard failover under chaos in `tests/fault/failover/`. — `tests/fault/failover/` (database connections killed over and over under load; the database cut off; a crash mid-pass; a replica held past its lease; the broker down with a dispatcher change; three workers in ten lost). The target is `fault_failover` (25 s).
- [x] T010-031 [US01] Run PITR restore and journal integrity hash verification, documented in `docs/runbooks/`. — `infra/dr/drill.sh` and `infra/dr/journal_digest.sql` (counts and sha256 of the journal and of the audit trail before and after, and a replay of every run against its projections), `docs/runbooks/disaster-recovery.md`, and the drill's report `tests/performance/reports/disaster-recovery.md` (restored to a named point in 0.9 s, digests equal, 6 000 runs replayed).

## Final Phase: Hardening / rollout
- [x] T010-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/scalability-ha/`. — no REST, gRPC or AsyncAPI change: the contract of this feature is the chart, the configuration and the metrics, checked by `contract_scalability_ha` (12) and `contract_observability_operations` (alerts and dashboards name only metrics the code emits).
- [x] T010-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`. — clippy with `-D warnings`, fmt, `scripts/check-architecture.py` (21 crates, no cycles).
- [x] T010-034 Run the benchmark defined in `specs/010-scalability-ha/plan.md` and publish the report in `tests/performance/reports/scalability-ha.md`. — `tests/performance/reports/scalability-ha.md`: four scenarios with hardware, configuration, load average and commit, and a section that reads them (what grows with what, what was not reached: 5 000 transitions per second, G270). The benchmark found three defects of the platform or of the harness that were fixed and re-measured (D247, D249/D260 and the dead index entries between policies).
- [x] T010-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/010-scalability-ha/quickstart.md`. — two rounds of two independent reviews (security and correctness), recorded in `quickstart.md` with what each finding became; the second round found a high one (the counters' `SECURITY DEFINER` function could be hijacked through a temporary table) and the control-plane probes that would have kept the chart's pods from starting.
- [x] T010-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`. — `docs/traceability.md`, "Evidence: 010 scalability-ha": each FR, NFR and SC with its state and its tests; FR-010-003 and FR-010-006 are partial and say why.

## Dependencies & independent delivery

- This feature requires: 001, 002, 003, 004, 005, 006, 008, 009.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.

## Integrated requirement completion

- [x] T010-037 Close G269 by supervising critical engine loops; test unexpected exit/panic and requested shutdown (NFR-010-DUR, NFR-010-OBS).
- [x] T010-038 Close G247 with a full product Helm deployment and real KEDA scaling/recovery in a disposable cluster (FR-010-004). — `tests/e2e/scalability-ha/kubernetes/run.sh`; live report: `tests/performance/reports/kubernetes-validation.md`.

T010-037 evidence: `apps/execution-engine/src/supervision.rs` tests unexpected return,
panic, graceful stop and forced bounded cleanup. Focused engine tests passed 32/32
and focused Clippy passed (implementation-agent execution). Integrated gates follow.

## Activity admission completion

- [x] T010-039 Enforce tenant/namespace/activity concurrency with transactional admission and audited namespace overrides (FR-010-002). Real PostgreSQL tests prove the 12-poller race, busy-activity skip, expired lease recovery, tenant isolation and reduced platform ceiling; management REST contract suite passes 7/7. Migration rollback/forward covers 31 tables; scoped clippy passes with `-D warnings`.
