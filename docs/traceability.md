# Traceability of features, requirements and test evidence

> Base record to update per PR. Features 001 and 002 are implemented (see the evidence tables below); every other feature is still only specified.

| Feature | Requirements | User stories | Planned tests | Status |
|---|---|---|---|---|
| [001 platform-foundation](../specs/001-platform-foundation/spec.md) | FR-001-001..007 | 4 | `tests/e2e/001-platform-foundation/`, `tests/fault/001-platform-foundation/` | Implementing |
| [002 durable-execution](../specs/002-durable-execution/spec.md) | FR-002-001..009 | 4 | `tests/e2e/002-durable-execution/`, `tests/integration/002-durable-execution/`, `tests/contract/002-durable-execution/` | Implementing (see the evidence table) |
| [003 sagas-compensations](../specs/003-sagas-compensations/spec.md) | FR-003-001..007 | 4 | `tests/e2e/003-sagas-compensations/`, `tests/fault/003-sagas-compensations/` | Planned |
| [004 events-webhooks-messaging](../specs/004-events-webhooks-messaging/spec.md) | FR-004-001..008 | 4 | `tests/e2e/004-events-webhooks-messaging/`, `tests/fault/004-events-webhooks-messaging/` | Planned |
| [005 inline-code-sandbox](../specs/005-inline-code-sandbox/spec.md) | FR-005-001..008 | 4 | `tests/e2e/005-inline-code-sandbox/`, `tests/fault/005-inline-code-sandbox/` | Planned |
| [006 sdk-multilanguage](../specs/006-sdk-multilanguage/spec.md) | FR-006-001..008 | 4 | `tests/e2e/006-sdk-multilanguage/`, `tests/fault/006-sdk-multilanguage/` | Planned |
| [007 workflow-studio](../specs/007-workflow-studio/spec.md) | FR-007-001..008 | 4 | `tests/e2e/007-workflow-studio/`, `tests/fault/007-workflow-studio/` | Planned |
| [008 observability-operations](../specs/008-observability-operations/spec.md) | FR-008-001..006 | 4 | `tests/e2e/008-observability-operations/`, `tests/fault/008-observability-operations/` | Planned |
| [009 security-multitenancy](../specs/009-security-multitenancy/spec.md) | FR-009-001..008 | 4 | `tests/e2e/009-security-multitenancy/`, `tests/fault/009-security-multitenancy/` | Planned |
| [010 scalability-ha](../specs/010-scalability-ha/spec.md) | FR-010-001..007 | 4 | `tests/e2e/010-scalability-ha/`, `tests/fault/010-scalability-ha/` | Planned |
| [011 gitops-versioning](../specs/011-gitops-versioning/spec.md) | FR-011-001..006 | 4 | `tests/e2e/011-gitops-versioning/`, `tests/fault/011-gitops-versioning/` | Planned |
| [012 developer-experience](../specs/012-developer-experience/spec.md) | FR-012-001..006 | 4 | `tests/e2e/012-developer-experience/`, `tests/fault/012-developer-experience/` | Planned |

## Evidence protocol

1. Link the PR to the FR and SC satisfied; reference the exact test (name and CI execution).
2. For durability invariants: failure scenario, journal evidence and absence of a double transition.
3. For security: tenant, secrets, sandbox and permissions tests with results.
4. For performance: workload, hardware, configuration and metrics, not just isolated throughput.
5. Update the Status column to Implementing / Tested / Delivered, with auditable evidence.

## Evidence: 001 platform-foundation

Last updated with the control-plane branch. "Partial" and "Open" are stated explicitly; see
`specs/001-platform-foundation/research.md` for the gaps behind them.

| Requirement | State | Evidence |
|---|---|---|
| FR-001-001 tenants, workspaces, namespaces, memberships, roles; uniqueness and tenant isolation | Done | `crates/domain/tests/identity.rs`, `tests/integration/001-platform-foundation/{constraints,store,flows}.rs` |
| FR-001-002 REST/OpenAPI v1 for health, auth context, workspace, namespace | Done | OpenAPI lint in CI, `tests/e2e/001-platform-foundation/{us01,us02}.rs`, live conformance of every Identity operation in `tests/contract/001-platform-foundation/rest/` |
| FR-001-002 … and the **definition lifecycle** | Open | Declared in `openapi.yaml`, not implemented (belongs with feature 002) |
| FR-001-003 versioned gRPC/protobuf worker protocol | Partial | `worker.proto` compiles in CI; no server, no compatibility check (`buf breaking`) yet |
| FR-001-004 idempotent migrations; development-only seed (and an explicit `provision-tenant` for real environments) | Done | `tests/integration/001-platform-foundation/migrations.rs`, `tests/e2e/001-platform-foundation/us03.rs`, `apps/control-plane/tests/cli.rs` |
| FR-001-005 Docker Compose with PostgreSQL, NATS and optional S3-compatible store | Partial | `infra/docker/compose.yaml`, `scripts/smoke-compose.sh`; the S3-compatible store is deferred (optional in this phase) |
| FR-001-006 canonical errors without sensitive data | Partial | One envelope `{code, message, retryable, traceId, details}` proven in `tests/e2e/001-platform-foundation/`; the spec's separate `request_id` is not implemented (the trace id serves) |
| FR-001-007 OIDC/JWT, base RBAC on every route, audit trail (including tenant and administrator provisioning) | Done | `tests/contract/001-platform-foundation/auth.rs` (token matrix, `typ` as a media type), `tests/e2e/001-platform-foundation/{us02,us04}.rs`, `tests/fault/001-platform-foundation/provider.rs`; the reserved `system:` subject namespace: `crates/domain/tests/identity.rs`, `crates/application/tests/identity_service.rs` |
| NFR-001-SEC tenant isolation, RBAC, least privilege, no secrets in logs | Done for a single instance, hardened by two review passes | Isolation: `tests/fault/001-platform-foundation/tenant_boundary.rs`; least privilege: `tests/integration/001-platform-foundation/{runtime_role,least_privilege}.rs` and the start-up refusal in `tests/e2e/001-platform-foundation/us03.rs`; abuse resistance (label cardinality, per-subject and per-tenant throttles, stalled bodies, readiness): `tests/fault/001-platform-foundation/{metrics,limits,readiness}.rs`, `crates/api-http/tests/{rate_limiter,tenant_concurrency}.rs`. Open and accepted items: G7, G9, G13 (per process), G17, G18 in `research.md`; the evidence table of both passes is in `quickstart.md` |
| NFR-001-OBS structured logs, correlated traces, essential metrics | Done | JSON logs, trace-id correlation (response, error body, audit) and Prometheus metrics are tested (`tests/e2e/001-platform-foundation/us03.rs`); OTLP span export is owned by feature 008 (G6) |
| NFR-001-PERF p95 < 250 ms namespace read | Done | Baseline `tests/performance/001-platform-foundation/main.rs`, report `tests/performance/reports/001-platform-foundation.md`: p95 15.0 ms, 0 errors (Apple M3 Max, release build, single instance) |
| SC-001-001 contract/API + tenant isolation suite and reproducible local environment | Done | The suites above plus `scripts/smoke-compose.sh` |
| SC-001-002 100% of US01..US04 pass in an automated suite | Done | `tests/e2e/001-platform-foundation/us0{1,2,3,4}.rs` |
| SC-001-003 no violation of the Constitution gates | Open | `.specify/memory/constitution.md` is not in the repository (G1); cannot be asserted |
| SC-001-004 critical errors expose `code`, `trace_id` and remediation without leaking | Partial | `code` and `traceId` proven; there is no separate `remediation` field |

## Evidence: 002 durable-execution

"Partial" and "Open" are stated explicitly; see `specs/002-durable-execution/research.md` for the gaps (G19-G30) and the decisions (D25-D36) behind them.

| Requirement | State | Evidence |
|---|---|---|
| FR-002-001 interpret versioned IR: Activity, Condition, Parallel/Join, Timer, WaitEvent, ChildWorkflow | Done for these kinds | Domain: `crates/domain/tests/{workflow,execution,execution_properties}.rs`; codec: `crates/infra-ir/tests/codec.rs`; every kind end to end: `tests/integration/002-durable-execution/{flows,routing,children}.rs`. `saga`, `script`, `foreach`, `humanApproval` are refused at publish with the reason (G20) |
| FR-002-002 append-only journal sequenced per run, transitions verified by compare-and-swap | Done | `run_events` is append-only by trigger (`tests/integration/002-durable-execution/faults.rs`), the run row is guarded by `run_version`; replay equals the projections after every random action (`execution_properties.rs`) and for stored runs (`verify_run`, `assert_consistent`) |
| FR-002-003 step result, journal, new state, ready tasks, timers/subscriptions and outbox in one transaction | Done | `PgRunTx::persist`; crash/connection-kill chaos in `tests/integration/002-durable-execution/faults.rs`; outbox in `outbox.rs` |
| FR-002-004 leases, heartbeat, generation/fencing, expiry with safe requeue | Done | `tests/integration/002-durable-execution/{scheduler,concurrency}.rs`, `tests/contract/002-durable-execution/grpc/protocol.rs` (stale leases), `tests/e2e/002-durable-execution/stories.rs::us02_*` |
| FR-002-005 backoff with jitter, max attempts, timeouts, deadline, explicit cancellation | Done | `crates/domain/tests/execution.rs`, `scheduler.rs`, `flows.rs`; operator actions in `tests/contract/002-durable-execution/rest/` |
| FR-002-006 reconciler for the outbox, orphaned READY tasks and expired leases | Done | `crates/scheduler` (`hints`, `leases`, dispatcher); `scheduler.rs`, `outbox.rs`, `nats.rs`; the engine runs them: `tests/e2e/002-durable-execution/engine.rs` |
| FR-002-007 Paused/Waiting without a dedicated process and without a NATS consumer per run | Done | A waiting run is a row (benchmark: 2.4 KiB, idle tick flat from 1k to 10k runs); `stories.rs::us03_*` |
| FR-002-008 bind runs to the immutable version; control payload/result size via blob refs | Partial | Version pinning and size limits are enforced and tested; blob refs are not implemented (G19) |
| FR-002-009 persist the child-run parent link and the cancellation propagation policy | Done | `tests/integration/002-durable-execution/children.rs` (17 tests: creation race, cancelled meanwhile, tenant isolation, recursion bound, deadlines, every policy), `crates/domain/tests/execution.rs::children`, full stack in `tests/e2e/002-durable-execution/engine.rs`. The child's result is not passed to the parent (G21) |
| NFR-002-SEC tenant isolation, RBAC, least privilege, no secrets in logs | Done for a single instance | `isolation.rs`, `tests/contract/002-durable-execution/grpc/protocol.rs` (authentication, role, cross-tenant, session binding, no echo of secrets), engine start-up refusal (`engine.rs`); review evidence in `quickstart.md`. Open: TLS/mTLS on the worker API (G27) |
| NFR-002-DUR after commit, losing a worker or the broker cannot lose a transition | Done | `faults.rs`, `outbox.rs` (crash between publish and confirm), `nats.rs` (broker outage through a severed connection), `engine.rs` |
| NFR-002-OBS structured logs, correlated traces, essential metrics, auditability per run/step | Done except span export | Journal with actor and trace id per event, audit records for operator actions and child starts, metrics in the runbook; OTLP spans belong to feature 008 |
| NFR-002-COMP versioned protocol and schemas; do not break runs in progress | Partial | `worker.proto` changed additively (heartbeat identifies the step); protocol version `1` negotiated at registration; no `buf breaking` check yet |
| NFR-002-PERF benchmark of at least 10k suspended runs and scheduling latency | Done as a baseline | `tests/performance/002-durable-execution/main.rs`, report `tests/performance/reports/002-durable-execution.md` (a noisy shared machine; known limits G22) |
| SC-002-001 deterministic crash test at each point of claim, effect, commit, publish | Done for the points reachable from the application | `faults.rs` (kill before/after commit, between statements), `outbox.rs::a_crash_between_publishing_and_confirming_*`, `stories.rs::us02_*` |
| SC-002-002 100% of US01..US04 pass in a reproducible automated suite | Done | `tests/e2e/002-durable-execution/stories.rs` |
| SC-002-003 no violation of the Constitution gates and no silent loss of persisted state | Partial | No loss is provable by the tests above; the Constitution is still not in the repository (G1) |
| SC-002-004 critical errors expose `code`, `trace_id` and remediation without leaking | Partial | `code` (and the gRPC `x-sagat-error`) and trace ids proven; no separate `remediation` field |
