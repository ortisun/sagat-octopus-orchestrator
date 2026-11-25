# Implementation Plan: Elastic scaling driven by telemetry

**Branch:** `feature/036-elastic-scaling` | **Date:** 2026-10-28 | **Spec:** [`spec.md`](./spec.md)

**Input:** the specification and its five clarifications (O1703, O1706–O1709). Decisions are `D1700…`, gaps `G1700…`, owner questions `O1710…` in [`research.md`](./research.md); the design records are ADR-0072 to ADR-0074 (accepted 2026-11-24, delegate, Owner decision 2026-11-24-4).

## Readiness against `develop` (2026-11-24)

The plan was written on 2026-10-28 and approved for implementation on 2026-11-11 (Owner decision 2026-11-11-1); feature
036 is in release 0.3.0 (Owner decision 2026-11-24-1). It was read again against `develop` at `546d5aa5`. Nothing of it
is built: there is no scaling code, table, crate or setting yet; the chart's KEDA and HPA toggles are feature 010's.
Where this section and an older line below disagree, this section is right.

**Corrections.**

1. **Migration numbers.** `20261226000000` is taken (`capacity_transfer_receipts`); the highest migration on `develop` is
   `20270119000000_batch_purge`, and open branches go further (`feature/022-throughput-control` to `20270209000100`,
   `feature/041-step-logs-server` to `20270201000100`). The three migrations of this feature take the next free
   timestamps at build time (Task T036-001 re-checks every branch). Their names (`scaling_core`, the connector lag and
   members, the `connector_parallelism` quota) are unchanged.
2. **The quota restatement.** The quota CHECK has been restated since (`20261230000100_schedule_quota_resources`; on the
   022 branch `20270130000300` and `20270206000000`). This feature's quota migration restates the full list of the
   tree it lands on; whichever of 022, 041 and 036 merges later restates the others' names. There are 37 quota kinds on
   `develop` (`crates/domain/src/quota.rs`, `ALL: [Self; 37]`), not 32, and 53 permissions, not 48.
3. **The chart has no `mode` key.** It has two toggles, `engine.autoscaling.keda.enabled` and
   `engine.autoscaling.hpa.enabled`, refused together (`infra/helm/templates/_helpers.tpl`). This feature's `platform`
   autoscaling is a third toggle, `engine.autoscaling.platform.enabled`, refused together with either of the other two;
   every mention of "the `lag` and `cpu` modes" and `engine.autoscaling.mode` below means these toggles.
4. **The database budget exists.** The chart already refuses a render when (control plane replicas + the engine's
   maximum) × `database.maxConnections` exceeds `database.serverMaxConnections`. Decision D1722's budget reuses those
   values and adds only `reserveConnections`; it does not introduce `perReplicaConnections`.
5. **Mixed versions use the instance registry.** Architecture decision 0082 (accepted) gives a per-process capability
   gate and a fleet census. The keeper of balanced partitions (D1708, G1715) uses a capability
   `connector-rank-share`: the rank share is switched on only when the census shows no process without it, which
   removes the unbalanced rolling upgrade that Architecture decision 0073 names as a consequence. Design 048's rule for
   rolling upgrades applies to every migration of this feature.
6. **The operator listener needs a second audience.** Token verification holds one audience per process
   (`crates/api-http/src/auth/verifier.rs`) and so does the development identity provider. Task T036-065 adds the
   operator audience's configuration and a second issuer configuration of the development provider.
7. **Process and identifiers.** Spec Kit is gone; the transitional clause of the constitution (4.7.0) lets this
   spec, plan and tasks stand in for a design doc. References below to `/speckit-*` commands are historical. New
   prose uses the word forms of `docs/governance/identifier-scheme.md`; the old identifiers stay where tools read them.
   Tasks that depend on an owner decision now carry `Owner-gate:` lines.
8. **CI gates that now apply.** Any change under `crates/infra-postgres/` selects the budget suite
   `integration_claim_transition_efficiency`; a new integration suite needs an entry in
   `scripts/integration-suite-durations.toml`; the OpenAPI lint runs in the `contracts` job on a release;
   `check-plain-words.py`, `check-docs-first.py` and `check-docs-index.py` run on every pull request.
9. **Line drift.** Evidence lines in `research.md` that moved (for example `inbox_pending_events` at
   `crates/scheduler/src/events.rs:234`, the idle timeout at `crates/telemetry/src/metrics.rs:279`, the autoscaler
   contract test `the_autoscalers_scale_on_metrics_the_platform_emits_and_on_nothing_else`) are recorded in
   `research.md`, section "Readiness, 2026-11-24"; the facts they support still hold.
10. **Other features.** 019 is approved and not built (0 of 68); 022's throughput slice is on its branch; 010 is built
    (gaps 239, 243, 246 and 249 still open); 038's budget suite is merged; 032 is in 0.3.0 and still out of scope here;
    designs 048 (standby reads, rolling upgrades) and 049 (journal partitioning) are new and change nothing here except
    item 5.

**Decisions taken on 2026-11-24** (delegate, Owner decision 2026-11-20-6 and Owner decision 2026-11-24-4; the reasons
are in the specification's "Owner questions" and in `research.md`): O1700, O1701, O1702, O1704 and O1710 as recommended
(Owner decisions 2026-11-24-500 to 2026-11-24-504); Architecture decisions 0072, 0073 and 0074 accepted (Owner decision
2026-11-24-505); the security checklist's items settled one by one (Owner decision 2026-11-24-506, with the per-item
record in `checklists/security.md`).

**Build slices**, each off by default and shippable, in this order:

1. Foundation: T036-001 to T036-020 and T036-092 (the pure domain, the scaling tables, the fenced evaluator, the
   exporter).
2. Lag and drain state (User story 036-01): T036-021 to T036-033.
3. Balanced partitions (User story 036-02): T036-034 to T036-040, with the registry capability of item 5.
4. The closed loop (User story 036-03): T036-041 to T036-046 and T036-093.
5. Configuration (User story 036-04): T036-047 to T036-052, in parallel with slices 2 to 4.
6. Safety and explanation without the operator surface: T036-066 and T036-068 to T036-074.
7. The operator surface (User story 036-05): T036-053 to T036-065, T036-094, then T036-067.
8. Pulled sources (User story 036-13): T036-075 to T036-083.
9. Polish: T036-084 to T036-091.

## Summary

Turn telemetry into capacity decisions that an operator declares by configuration or through a management interface, and hand both the signals and the decisions to the autoscaling provider (KEDA or another). The first delivery (O1709) closes the loop for consumers: lag per connector (Kafka, NATS, SQS), balanced Kafka partitions, a policy model with a deterministic evaluator, explanation of every decision, safety rules, pulled sources, and the export to the provider.

**Technical approach.**

- A **pure domain module** (`scaling`) holds the model and one deterministic function, `evaluate`, from (policy version, samples, current state, ceilings, time) to a decision record. Rank-based partition targets are pure too. Everything else is an adapter around them.
- One **evaluator loop in the engine**, under a lease with a fence in the decision's transaction, collects signals (built-in from the database, pulled from operator-declared sources), evaluates every policy, records the decision with its inputs, and writes the desired capacity. The platform applies nothing in the orchestrator (that is the optional applier of a later delivery).
- **Every replica exports** the desired capacity and the signal states as Prometheus series read from database state, so the value persists while the evaluator is down (hold) and the chart's autoscaler follows `authority="authoritative"` series only.
- **Lag** is measured where the data already is: Kafka's fetch returns the high watermark (today discarded), NATS has consumer info, SQS has queue attributes. The connector reports why it is not draining (behind, blocked, **rate-limited**, paused, failing).
- **Balanced Kafka assignment** replaces first-come leases: replicas announce themselves in a membership table, each computes its target from its rank (floor or ceil), gives back what exceeds it and takes free partitions up to it. Positions and admission stay one commit (ADR-0002, ADR-0012).
- **Management interface**: tenant endpoints on the existing API (lag, consumption), and an **operator listener** with its own principal for policies, sources, signals, overrides and decisions (O1710, default recommended). `sagat scaling` is a client of it.

**Finding that reshaped the plan** (confirmed in code, `research.md` D1710): a connector's `ratePerSecond` already holds across replicas through a shared bucket, so a connector at its rate limit gains nothing from more readers. The drain state and the scale-out rule account for it, and the spec and `docs/runbooks/events.md` are corrected.

## Technical Context

- **Language/Version:** Rust stable pinned by `rust-toolchain.toml`; Helm templates; Python only for repository checks.
- **Primary Dependencies (existing):** Tokio, Axum, tonic, SQLx, `metrics` 0.24 with the Prometheus exporter, `reqwest` 0.13 (through `infra-egress`), `rskafka` 0.6.0 (vendored, zstd patch only), `async-nats` 0.50.0, `aws-sigv4`. **New dependencies: none in the domain** (`thiserror`, `time`, `uuid`, `zeroize` only; `proptest` is already a dev-dependency). One new adapter crate, `infra-signals`, no new third-party crate.
- **Storage:** PostgreSQL (CI uses 16). Three additive migrations at the next free timestamps at build time (corrected 2026-11-24: `20261226000000` is taken; see "Readiness against `develop`"): the scaling tables, the connector lag and members, and the `connector_parallelism` quota. NATS is not involved in scaling.
- **Testing:** `cargo test` with real PostgreSQL (Testcontainers) and real brokers (`tests/support/brokers.rs`: Kafka, ElasticMQ for SQS; `nats.rs`); `proptest` for the decision function; `helm template` contract test; kind, Cilium and KEDA end-to-end by extending `tests/e2e/scalability-ha/kubernetes/run.sh` (G247).
- **Target Platform:** Linux containers on Kubernetes; Docker Compose for development (no Kafka or SQS emulator in Compose, tests start their own).
- **Project Type:** modular Rust monorepo, Helm chart, operator CLI.
- **Performance Goals:** criteria, not promises. Overhead of the machinery is bounded and reported (NFR-036-PERF): the evaluator reads once per interval; each lag holder writes one row per connector per sampling interval; each replica reads the desired state once per sampling interval.
- **Constraints:** single region; positions and admission stay one commit; no tenant, namespace, connector, run or activity label on a metric (ADR-0017, enforced by `no_metric_is_labelled_with_something_that_grows_with_the_data`); the domain crate accepts four dependencies only (`scripts/check-architecture.py`); the operator surface is off by default; nothing here changes delivery guarantees.
- **Scale/Scope:** at most 64 signals and 32 policies (bounds the label values), 256 partitions per connector and 20 connectors per tenant (existing limits), at most 64 engine replicas (existing ceiling).

## Constitution Check

*Re-evaluated after Phase 1 design: unchanged, no violation.*

- [x] **I. State and retry semantics.** PostgreSQL holds every scaling fact; a decision is recorded with its inputs in one transaction that the evaluator's fence guards (`D1701`). No transition, timer or lease semantics change. The Kafka position and the admission of its events stay one commit (`D1707`). A stale evaluator's decision is refused, never merged.
- [x] **II. Modular domain, ports and adapters.** `crates/domain/src/scaling/` is pure; ports in `crates/application`; loops in `crates/scheduler`; adapters in `infra-postgres`, `infra-connectors`, the new `infra-signals` and `api-http`. `infra-signals` is an adapter by its `infra-` prefix, so `scripts/check-architecture.py` needs no change and enforces its layer edges (`D1700`).
- [x] **III. External effects.** No new external effect in the first delivery. Pulled sources are read-only queries. The applier (later) is idempotent by decision identity and at-least-once.
- [x] **IV. Compatibility.** Additive tables and columns; existing metrics, alerts, settings and the connector API keep their meaning; mixed versions are safe because an engine without the keeper only competes for free partition leases and the fences hold (`D1708`); the SDKs do not change (O1708). Contract changes are versioned (OpenAPI fragment, JSON Schema).
- [x] **V. Tenant boundary and secrets.** Lag is per namespace and another tenant's connector is a missing one; tenants never write a policy (O1703). Sources are operator-only, allowlisted, redirect-free, bounded and read-only; credentials are file names resolved at use, never stored (`D1716`, `D1717`). Release-blocking risks named: SSRF through a source destination, cross-tenant lag read, a credential in a record, an operator token reaching the tenant door or the reverse. Each has a security test (SC-036-013, SC-036-018, SC-036-020). The evaluator re-validates every definition against the configuration-only limits at each evaluation, so rows written by a compromised role cannot take a unit past them (`D1732`, SC-036-021). *Reviewed against `checklists/security.md` on 2026-10-28; the spec was amended (FR-036-062 to FR-036-068).*
- [x] **VI. Observability and audit.** Traces on evaluation and handover; metrics without high-cardinality labels (`contracts/metrics.md`); platform-scope changes audited in a new append-only trail because the tenant audit needs a tenant (`D1714`); typed errors.
- [x] **VII. Events are redeliverable.** Acknowledgement after the inbox is unchanged. Backpressure and dead letters are untouched; the new drain states make backpressure and rate limiting visible. Duplicates after a handover are answered by the inbox, as today.
- [x] **VIII. Failure is tested.** The test plan below lists the crash points; results will be reported as they are.
- [x] **Delivery constraints.** Technology baseline unchanged; targets are criteria; the operator listener ships off by default.

Complexity Tracking at the end justifies the three costs that need it.

## Delivery slices (O1709)

| Slice | Stories | Content |
|---|---|---|
| **1 — first delivery** | US01, US02, US03, US04, US05, US06, US10, US13 | everything in this plan marked M1–M8 |
| 2 | US07, US09 | signals and scaling for the other roles; the engine split into selectable roles; the dispatcher and delivery division and the claim path, **on benchmark evidence** |
| 3 | US08, US12, US11 | hot tunables and resource policies; the optional applier; the benchmark on separate machines |

Slices 2 and 3 are listed so that slice 1 leaves them room; their design is not fixed here. What is undecided is marked in [Pending decisions](#pending-decisions-marked-as-such).

## Architecture of slice 1

```mermaid
flowchart LR
  subgraph engine replicas
    R[connector readers] -->|lag, drain state| DB[(PostgreSQL)]
    K[partition keeper] <-->|members, leases| DB
    E[evaluator: one at a time, fenced]
  end
  S[operator-declared sources] -->|read-only queries| E
  DB -->|built-in signals| E
  E -->|decision record + desired capacity| DB
  DB -->|every replica exports| P[(Prometheus)]
  P --> KEDA[autoscaling provider] -->|replicas| D[deployments]
  OP[operator listener + sagat scaling] --> DB
  T[tenant API: lag, consumption] --> DB
```

| Piece | Where | Notes |
|---|---|---|
| Model, `evaluate`, rank-based targets, role catalogue | `crates/domain/src/scaling/` | pure; property tests |
| Ports and services (management, tenant lag, evaluator tick) | `crates/application/src/scaling.rs`; loop in `crates/scheduler/src/scaling.rs` | the monitor is the pattern |
| Stores and migrations | `crates/infra-postgres/src/scaling.rs`, `connectors.rs`; two migrations | |
| Lag probes, drain state, balanced assignment, hot parallelism | `crates/infra-connectors/src/{lag,kafka,nats,sqs,supervisor,ingest}.rs` | `ConnectorRuntime` gains methods |
| Pulled sources and the operator egress guard | new `crates/infra-signals/` | implements the `infra-egress` `Guard` seam |
| Tenant endpoints | `crates/api-http/src/routes/` + OpenAPI fragment | `x-implemented: '036'` |
| Operator listener, principal, routes | `apps/control-plane`, `crates/api-http` | off by default (`D1713`, ADR-0074) |
| `sagat scaling …` | `apps/cli` | REST client only |
| Evaluator, exporter | `apps/execution-engine/src/lib.rs`, settings | supervised as critical tasks |
| Chart | `infra/helm` | ConfigMap, env, `platform` autoscaling mode, egress |

**Evaluation order** (recorded in every decision, `contracts/evaluation.md`): usable signals → raw demand → stabilization → step → bounds with overrides → hold or freeze → role ceiling → database budget → cost ceiling → mode. Safety ceilings are last, so no override can pass them.

## Project Structure

### Documentation (this feature)

```text
specs/036-elastic-scaling/
├── spec.md              # with the five clarifications and the planning correction
├── plan.md              # this file
├── research.md          # D1700–D1739, G1700–G1718, O1710
├── data-model.md        # entities, tables, state transitions
├── quickstart.md        # runnable validation scenarios
├── contracts/
│   ├── README.md
│   ├── evaluation.md                       # formulas, clamp order, golden vectors
│   ├── signals-catalogue.md                # built-in signals of slice 1
│   ├── metrics.md                          # new series, labels, rules
│   ├── scaling-config.schema.json          # the configuration document
│   ├── tenant-scaling.openapi-fragment.yaml
│   └── operator-scaling.openapi.yaml       # the operator listener
├── checklists/requirements.md
└── tasks.md             # /speckit-tasks
```

### Source Code (repository root)

```text
crates/domain/src/scaling/                    signal, policy, decision, evaluate, assignment, role, lag
crates/application/src/scaling.rs             ports, ScalingService, ConnectorLagService
crates/scheduler/src/scaling.rs               evaluator tick
crates/scheduler/src/scaling/                 signals, context, export, config (sync), retention
crates/infra-postgres/src/scaling.rs          stores (policies, signals, decisions, desired, overrides, audit)
crates/infra-postgres/src/connectors.rs       members, release, lag rows, drain state, tuning
crates/infra-postgres/migrations/20261226000000_scaling_core.{up,down}.sql
crates/infra-postgres/migrations/20261226000100_connector_lag_members.{up,down}.sql
crates/infra-postgres/migrations/20261226000200_connector_parallelism_quota.{up,down}.sql
crates/domain/src/quota.rs, crates/application/src/quotas.rs, crates/infra-postgres/src/quotas.rs   the quota connector_parallelism (D1712)
crates/infra-postgres/storage-classification.json
crates/infra-connectors/src/lag.rs            LagProbe and the three kinds
crates/infra-connectors/src/{kafka,nats,sqs,supervisor,ingest}.rs   changed; new assignment.rs (the keeper)
crates/infra-signals/src/                     config, guard, metrics_query, broker_queue, credentials
crates/api-http/src/routes/connectors.rs      lag and consumption (tenant)
crates/api-http/src/routes/operator_scaling.rs, dto_scaling.rs, operator/   the operator listener, its principal and routes
crates/api-http/src/error.rs                  remediation entries for the scaling reasons (D1738)
apps/control-plane/src/{config,lib}.rs        operator listener, config
apps/execution-engine/src/{lib,settings}.rs   evaluator, exporter, keeper wiring
apps/cli/src/scaling.rs                       sagat scaling …
infra/helm/                                   templates, values, examples/ha.yaml
specs/001-platform-foundation/contracts/openapi.yaml   tenant fragment merged
docs/runbooks/{scaling,events,security-governance}.md  ADR index, roadmap, owner-decisions
tests/integration/elastic-scaling/            real PostgreSQL and brokers
tests/fault/elastic-scaling/                  crash points
tests/contract/elastic-scaling/               OpenAPI, chart, config schema, metrics, architecture
tests/security/elastic-scaling/               SSRF, cross-tenant, credentials
tests/e2e/elastic-scaling/                    engine processes; kind and KEDA via the G247 script
tests/performance/scalability-ha/             lag control loop section (slice 1), separate machines (slice 3)
```

**Structure Decision.** No new application crate: the engine's loops already live in `crates/scheduler` and the application ports in `crates/application`. One new adapter crate (`infra-signals`) because the operator's outbound policy is a different trust boundary from the tenant's egress policy and must not live next to it.

## Implementation phases (slice 1)

Order follows dependency; each milestone ends with the evidence that closes its tasks.

1. **M1 — Pure model and evaluator** (FR-014–017, 021, 026, 027, 039, 043–046, 007). Types, validation, `evaluate`, rank-based targets, role catalogue. Evidence: unit tests, `proptest` properties (`SC-036-007`), the golden vectors of `contracts/evaluation.md`.
2. **M2 — Persistence** (FR-018, 020, 025, 028, 031). Migration `20261226000000`, stores, retention, the platform audit trail, privilege audit and storage classification. Evidence: migrate, roll back, re-migrate; `runtime_privileges`; reruns with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
3. **M3 — Lag and drain state** (FR-001–003, 005, 006, 013). Migration `20261226000100`; `LagProbe` for the three kinds; `Outcome::Backlogged` split; the correction of the runbook. Evidence: real Kafka, NATS and ElasticMQ (`SC-036-001`).
4. **M4 — Balanced assignment** (FR-007–011). Membership, keeper, release, readers started only for assigned partitions. Evidence: 12 partitions, 1→3→1 replicas, crash and clean stop (`SC-036-002`, `SC-036-003`).
5. **M5 — Evaluator, publication, chart** (FR-004 for slice 1, 017, 029–031, 043–048, 061, 064). Evaluator under lease with fence, config sync, re-validation of every definition against the configuration-only limits at each evaluation (`D1732`), exporter, chart mode `platform`. Evidence: integration and fault tests; `helm template` contract.
6. **M6 — Management interface** (FR-019, 020, 022–025, 047, 062, 063, 065, 066, 068; tenant: FR-002, 012, 023). Operator listener and principal (token audience and lifetime, peers, rate limit, refusal audit), routes, `sagat scaling`, tenant lag and consumption, hot parallelism, the `connector_parallelism` quota (`D1712`). Evidence: contract tests per `x-implemented: '036'` operation, parity test (`SC-036-005`), isolation tests (`SC-036-013`).
7. **M7 — Pulled sources** (FR-056–060, 067). `infra-signals`, the guard, file-name credentials and their rotation, the test operation. Evidence: security tests (`SC-036-018`), source kinds (`SC-036-017`).
8. **M8 — Evidence and documentation.** The kind and KEDA end-to-end (`SC-036-019`), the lag control-loop measurement on the available environment (`SC-036-004`), runbooks, alerts (NFR-036-OPS), `docs/traceability.md`, ADR acceptance.

## Test plan (Constitution VIII)

| Failure point | Test | Kind |
|---|---|---|
| Reader killed mid-batch during a partition handover | no loss, no duplicate admitted, position forward only | fault, real Kafka |
| Clean stop and crash of a holder | partitions move within the rebalance time and the lease time | fault |
| Broker redelivery and lost acknowledgement | inbox answers `DUPLICATE`; lag converges | integration, real brokers |
| Evaluator killed between collecting and recording | no partial decision; the next holder decides | fault |
| Two evaluators at once | exactly one decision in force (`FR-036-031`) | fault, held lock |
| Telemetry outage and stale signals | hold, condition raised, no scale-in | integration, property |
| Rolling upgrade, mixed versions | old engine and new keeper coexist, no partition read twice | fault |
| Tenant floods a shared signal | cost ceiling and step hold; dominance flagged | integration |
| Config unreadable or invalid | last valid version stays | integration |
| Source: SSRF, redirect, oversize, slow, NaN, plain-text credentials | refused at declaration or first query | security |
| Cross-tenant read of lag or write of consumption | indistinguishable from missing, audited | security |
| Rate-limited and blocked connectors | no scale-out decided | integration |
| A credential is used, rotated or fails | first use and failures audited, the rest counted (FR-036-067, D1736) | integration, security |
| Timeout and signal race; failed compensation | not applicable: this feature changes no transition, timer, signal or compensation semantics (Constitution VIII) | none |

Races use held locks and barriers, never sleeps (AGENTS rule 18). A golden or generated file changes only after reading the diff.

## Data Model / Contracts

- [`data-model.md`](./data-model.md)
- [`contracts/`](./contracts/README.md)
- ADRs: [ADR-0072](../../docs/adr/ADR-0072-scaling-decisions-are-deterministic-recorded-and-published-not-applied-by-the-platform.md), [ADR-0073](../../docs/adr/ADR-0073-connector-partitions-are-shared-by-rank-and-lag-says-why-a-consumer-is-not-draining.md), [ADR-0074](../../docs/adr/ADR-0074-platform-operators-act-through-their-own-listener-and-outside-data-comes-from-allowlisted-sources.md) (proposed).

## Risks & Mitigations

- **Security risk: pulled sources are outbound calls chosen by configuration.** Operator-only; destination allowlist in configuration only (the API cannot widen it); no redirects; every resolved address checked; credentials are file names; bounded time and size; read-only. Adversarial tests first (`D1716`).
- **Security risk: the operator listener is a new authority.** Off by default; separate audience and principal; network policy peers explicit; hard ceilings are configuration-only so a compromised API cannot lift them, the evaluator re-validates every definition against them at each evaluation, tokens live at most an hour and are rate limited (`G1706`, `G1707`, `D1732`, `D1733`).
- **Operational risk: the provider reads a series that is absent or `NaN`.** Desired capacity is exported from stored state and persists; raw signals export `NaN` when unavailable; the real behaviour is proved in the kind and KEDA test, not assumed (`G1703`).
- **Functional risk: scale-out for lag that cannot be drained.** Drain state excludes blocked and rate-limited connectors; the ceiling for useful readers is recorded (`D1711`).
- **Compatibility risk: a hand-written rate statement was wrong.** Corrected in the spec and the runbook; no behaviour changes.
- **Performance risk: decision records grow.** Identical consecutive decisions coalesce into one row with a repeat count (`D1721`); 14 days retention.
- **Delivery risk: slice 1 is large (eight stories).** Milestones are independently testable; the operator surface (M6) and pulled sources (M7) are the ones to cut last if the owner must.

## Complexity Tracking

| Decision | Cost | Justification |
|---|---|---|
| New adapter crate `infra-signals` | one more crate in the workspace | the operator's outbound policy is a different trust boundary from the tenant's; putting it in `infra-egress` or `infra-connectors` would blur a security line |
| A second listener and principal for operators | new authority, new attack surface, a second OpenAPI document | there is no platform-operator principal on the REST API today and operator actions run as owner-role commands; a tenant listener must never accept an operator (`O1710` records the alternatives) |
| Credential use audited at first use and on failure, not at every read | a counter instead of a row per query | Constitution VI lists secret access among the audited actions; a row per query is one every 5 to 15 seconds per source. The stricter reading costs one platform-audit row per query and is a retention choice, not a redesign (D1736); the owner decides |
| A platform audit trail apart from the tenant audit | one more append-only table | the tenant audit requires a tenant and a pseudo-tenant would receive quotas and appear in listings |

## Pending decisions (marked as such)

These are placeholders, not designs (AGENTS rule 11):

- **O1710** — operator identity for the management interface (the default in this plan is the separate listener; the owner decides).
- **Slice 2:** how the event dispatcher and the delivery worker are divided (decided on the benchmark of FR-036-033), whether the claim path is the limit (FR-036-034), the load-aware shard share (FR-036-035), how engine roles are selected (ADR to write).
- **Slice 3:** how a connection-budget tunable is applied to a pool that is fixed at creation (`G1712`), the applier's permissions and manifest, the benchmark topology.
- **Tenant custom signals** are not published in slice 1 (`G1700`).
