# Traceability of features, requirements and test evidence

> Base record to update per PR. Features 001–012 have implementations and evidence tables below. The status rows and requirement evidence distinguish delivered scope from remaining product, decision and validation gaps. In particular, native Go and Java sandbox runners are proven on Linux AMD64 and ARM64, while arbitrary TypeScript/Python dependencies remain unsupported; feature 006 is not published; protocol 1.4 task streaming is implemented and passes five-language conformance, with its comparative performance report recorded (MIT was selected on 2026-10-11; reproducible packages and five clean-consumer smoke tests passed); feature 008 has no exemplars, pager integrations or OTLP protobuf/gRPC exporter, and its Operations menu is implemented and validated by T008-036; feature 010 has exact per-activity concurrency admission but no generic pool/queue quota, demonstrated 5 000 transitions/s result or partitioned journal. The full product/KEDA validation passed in a disposable one-node kind/Cilium cluster on 2026-10-12; node drain, cross-node rescheduling and multi-zone recovery remain unproven.

| Feature | Requirements | User stories | Planned tests | Status |
|---|---|---|---|---|
| [001 platform-foundation](../specs/001-platform-foundation/spec.md) | FR-001-001..007 | 4 | `tests/e2e/platform-foundation/`, `tests/fault/platform-foundation/` | Implementation tasks complete; explicit criterion gaps remain below |
| [002 durable-execution](../specs/002-durable-execution/spec.md) | FR-002-001..009 | 4 | `tests/e2e/durable-execution/`, `tests/integration/durable-execution/`, `tests/contract/durable-execution/` | Implemented; explicit payload, result, worker-claim and retention gaps remain below |
| [003 sagas-compensations](../specs/003-sagas-compensations/spec.md) | FR-003-001..007 | 4 | `tests/e2e/sagas-compensations/`, `tests/integration/sagas-compensations/`, `tests/contract/sagas-compensations/`, `tests/fault/sagas-compensations/` | Implemented; the SDK DSL, Studio saga operations and dashboards were subsequently supplied by features 006–008 |
| [004 events-webhooks-messaging](../specs/004-events-webhooks-messaging/spec.md) | FR-004-001..008 | 4 | `tests/e2e/events-webhooks-messaging/`, `tests/integration/events-webhooks-messaging/`, `tests/contract/events-webhooks-messaging/`, `tests/fault/events-webhooks-messaging/` | Implemented and verified, including OIDC/mTLS profiles and outbound broker producers; Studio manages inbound NATS/Kafka/SQS connectors, while outbound subscriptions and dedicated webhook/event screens remain API-only |
| [005 inline-code-sandbox](../specs/005-inline-code-sandbox/spec.md) | FR-005-001..012 | 6 | `tests/e2e/inline-code-sandbox/`, `tests/integration/inline-code-sandbox/`, `tests/contract/inline-code-sandbox/`, `tests/fault/inline-code-sandbox/` | Implemented for TypeScript, Python, C#, native Go and Java; AMD64 and ARM64 native proofs passed. Dependency installation is limited to the exact C# catalog; arbitrary TypeScript/Python dependencies remain unsupported and Go/Java reject dependency requests |
| [006 sdk-multilanguage](../specs/006-sdk-multilanguage/spec.md) | FR-006-001..008 | 4 | `tests/e2e/sdk-multilanguage/`, `tests/fault/sdk-multilanguage/`, `tests/contract/sdk-conformance/`, `tests/contract/sdk-multilanguage/`, `tests/performance/sdk-multilanguage/` | Implemented within the recorded scope; MIT packages and clean-consumer installation verified; registry publication awaits namespace/credentials; finite-credit streaming passes five-language conformance, with a recorded 30-row comparative benchmark |
| [007 workflow-studio](../specs/007-workflow-studio/spec.md) | FR-007-001..010 | 4 | `tests/e2e/workflow-studio/`, `tests/fault/workflow-studio/` | Implemented within the recorded scope, including dedicated Namespaces and inbound Integrations menus; owner approval and real-sandbox Studio CI evidence are linked below |
| [008 observability-operations](../specs/008-observability-operations/spec.md) | FR-008-001..006 | 4 | `tests/e2e/observability-operations/`, `tests/integration/observability-operations/`, `tests/contract/observability-operations/`, `tests/fault/observability-operations/`, `tests/performance/observability-operations/` | Backend implemented; exemplars, pager integrations and protobuf/gRPC export remain open. The Studio Operations menu for alerts and audited drains is implemented and validated (T008-036) |
| [009 security-multitenancy](../specs/009-security-multitenancy/spec.md) | FR-009-001..008 | 4 | `tests/e2e/security-multitenancy/`, `tests/integration/security-multitenancy/`, `tests/contract/security-multitenancy/`, `tests/fault/security-multitenancy/` | Implemented within the explicit gaps below, including integration with features 004, 005 and the Helm chart |
| [010 scalability-ha](../specs/010-scalability-ha/spec.md) | FR-010-001..007 | 4 | `tests/e2e/scalability-ha/`, `tests/integration/scalability-ha/`, `tests/contract/scalability-ha/`, `tests/fault/scalability-ha/`, `tests/fault/failover/`, `tests/performance/scalability-ha/`, `infra/dr/` | Implemented within the measured scope, including exact per-activity concurrency admission and one-node full-product KEDA validation; 5 000 transitions/s, journal partitioning, a generic pool/queue quota and multi-node disruption remain unproven |
| [011 gitops-versioning](../specs/011-gitops-versioning/spec.md) | FR-011-001..006 | 4 | `tests/e2e/gitops-versioning/`, `tests/integration/gitops-versioning/`, `tests/contract/gitops-versioning/`, `tests/fault/gitops-versioning/`, `tests/performance/gitops-versioning/`, `apps/cli/tests/` | Implemented; artifact/history retention, scripts in the manifest and a packaged binary remain open |
| [012 developer-experience](../specs/012-developer-experience/spec.md) | FR-012-001..006 | 4 | `tests/e2e/developer-experience/`, `tests/fault/developer-experience/`, `tests/contract/developer-experience/`, `tests/performance/developer-experience/`, `apps/cli/tests/`, `apps/dev-idp/tests/` | Implemented; human acceptance remains open (no person was timed for SC-012-001, and there is no packaged binary, published SDK or hosted documentation) |
| [016 payload-classification](../specs/016-payload-classification/spec.md) | FR-016-001..029 | 8 | `tests/contract/payload-classification/`, `tests/integration/payload-classification/`, `tests/fault/payload-classification/`, `tests/e2e/payload-classification/`, `tests/performance/payload-classification/`, `frontend/e2e/tests/payload-disclosure.spec.ts` (focused implementations and evidence in `tasks/project-completion/integration-notes.md`; final gates pending) | Implementation in progress |

## Evidence protocol

Feature [017 database-connections](../specs/017-database-connections/spec.md) is approved (2026-10-16), implemented and integrated with the feature-014 base of `develop` on `feature/017-database-connections-integration` (2026-10-24). FR-017-001..022 and SC-017-001..011 are tracked in its [tasks](../specs/017-database-connections/tasks.md), its [traceability](../specs/017-database-connections/traceability.md) and the "Integration into develop" section of its [verification](../specs/017-database-connections/verification.md). Deployment-gated criteria (private-network egress, three-member Raft restore, Kubernetes) keep the limits recorded there.

1. Link the PR to the FR and SC satisfied; reference the exact test (name and CI execution).
2. For durability invariants: failure scenario, journal evidence and absence of a double transition.
3. For security: tenant, secrets, sandbox and permissions tests with results.
4. For performance: workload, hardware, configuration and metrics, not just isolated throughput.
5. Update the Status column to Implementing / Tested / Delivered, with auditable evidence.

Cross-cutting HTTP error completion, 2026-10-12: the outer request-context middleware now gives every REST success or error a fresh server-generated `X-Request-Id`; error bodies carry the same `requestId` plus `code`, `traceId`, `retryable` and remediation selected from a closed payload-free table. It ignores an inbound request-id value and also synthesizes this envelope for framework and fallback errors. This supersedes the older feature quickstart statements that REST had no separate remediation field. It does not imply that every worker-gRPC or persisted background-failure representation has a remediation field.

## Evidence: 001 platform-foundation

Completion review, 2026-10-11: T001-002/030/031 are covered by static consumer fixtures, the existing compatibility baselines and the existing migration/CI gates. Focused verification passed: platform contracts 27, live REST 3, worker compatibility 2, AsyncAPI compatibility/conformance 4, and migration idempotency 1. Formatting, focused Clippy, OpenAPI lint, protobuf compilation and AsyncAPI validation passed. Exact commands and non-blocking validator notices are in `specs/001-platform-foundation/quickstart.md`. "Partial" and "Open" below describe remaining requirement gaps rather than unchecked implementation tasks.

| Requirement | State | Evidence |
|---|---|---|
| FR-001-001 tenants, workspaces, namespaces, memberships, roles; uniqueness and tenant isolation | Done | `crates/domain/tests/identity.rs`, `tests/integration/platform-foundation/{constraints,store,flows}.rs` |
| FR-001-002 REST/OpenAPI v1 for health, auth context, workspace, namespace | Done | OpenAPI lint in CI, `tests/e2e/platform-foundation/{us01,us02}.rs`, live conformance of every Identity operation in `tests/contract/platform-foundation/rest/`; static REST and TaskReady consumer fixtures in `tests/contract/platform-foundation/fixtures/`, validated by `fixtures.rs`. T007-052 subsequently adds the dedicated Studio Namespaces list/create/context flow over these contracts, with a real Chromium/API journey. |
| FR-001-002 … and the **definition lifecycle** | Done by feature 002 and extended by 011 | Draft creation/update/read, publication into immutable versions, version listing/read and active-version selection are implemented in `crates/application/src/definitions.rs` and the REST routes; `tests/contract/durable-execution/rest/` and `tests/e2e/durable-execution/stories.rs` exercise the lifecycle. Feature 011 adds approval requests, activation, promotion and pointer history without rewriting published versions. |
| FR-001-003 versioned gRPC/protobuf worker protocol | Done | `worker.proto` compiles in CI; `crates/api-grpc/tests/compatibility.rs` compares the compiled descriptor with `worker.baseline.json`; feature 002 supplies the real worker server and protocol conformance suite. The descriptor baseline provides the required compatibility gate without a second `buf breaking` implementation. |
| FR-001-004 idempotent migrations; development-only seed (and an explicit `provision-tenant` for real environments) | Done | `tests/integration/platform-foundation/migrations.rs`, `tests/e2e/platform-foundation/us03.rs`, `apps/control-plane/tests/cli.rs` |
| FR-001-005 Docker Compose with PostgreSQL, NATS and optional S3-compatible store | Partial | `infra/docker/compose.yaml`, `scripts/smoke-compose.sh`; the S3-compatible store is deferred (optional in this phase) |
| FR-001-006 canonical errors without sensitive data | Done | The envelope is `{code, message, retryable, traceId, requestId, remediation, details}`. The server generates an independent request id for every success, application/framework error and fallback response, ignores an inbound `X-Request-Id`, returns the id in `X-Request-Id`, and uses the inherited/generated trace independently (`crates/api-http/src/context.rs::requests_with_the_same_parent_get_distinct_server_ids_on_every_response`). The existing leak scans cover the closed error shape. Focused agent-run verification reports this request-context test passing. |
| FR-001-007 OIDC/JWT, base RBAC on every route, audit trail (including tenant and administrator provisioning) | Done | `tests/contract/platform-foundation/auth.rs` (token matrix, `typ` as a media type), `tests/e2e/platform-foundation/{us02,us04}.rs`, `tests/fault/platform-foundation/provider.rs`; the reserved `system:` subject namespace: `crates/domain/tests/identity.rs`, `crates/application/tests/identity_service.rs` |
| NFR-001-SEC tenant isolation, RBAC, least privilege, no secrets in logs | Done for a single instance, hardened by two review passes | Isolation: `tests/fault/platform-foundation/tenant_boundary.rs`; least privilege: `tests/integration/platform-foundation/{runtime_role,least_privilege}.rs` and the start-up refusal in `tests/e2e/platform-foundation/us03.rs`; abuse resistance (label cardinality, per-subject and per-tenant throttles, stalled bodies, readiness): `tests/fault/platform-foundation/{metrics,limits,readiness}.rs`, `crates/api-http/tests/{rate_limiter,tenant_concurrency}.rs`. Every `403` / `PERMISSION_DENIED` is audited as `access.denied.permission` (G9; `tests/integration/security-multitenancy/denials.rs`). Open and accepted items: G7, G13 (per process), G17, G18 in `research.md`; the evidence table of both passes is in `quickstart.md` |
| NFR-001-OBS structured logs, correlated traces, essential metrics | Done | JSON logs, trace-id correlation (response, error body, audit) and Prometheus metrics are tested (`tests/e2e/platform-foundation/us03.rs`); feature 008 supplies bounded OTLP/HTTP JSON span export (`crates/telemetry/tests/otlp.rs`, `tests/e2e/observability-operations/`). |
| NFR-001-PERF p95 < 250 ms namespace read | Done | Baseline `tests/performance/platform-foundation/main.rs`, report `tests/performance/reports/platform-foundation.md`: p95 15.0 ms, 0 errors (Apple M3 Max, release build, single instance) |
| SC-001-001 contract/API + tenant isolation suite and reproducible local environment | Done | The suites above plus `scripts/smoke-compose.sh` |
| SC-001-002 100% of US01..US04 pass in an automated suite | Done | `tests/e2e/platform-foundation/us0{1,2,3,4}.rs` |
| SC-001-003 no violation of the Constitution gates | Partial | Constitution v1.0.0 was explicitly ratified by the maintainer on 2026-10-11 (G1 resolved; its `Last Amended` date remains 2026-10-09). Only principle II has a dedicated automated gate (`scripts/check-architecture.py`, CI job `architecture`); the other principles are verified by the tests listed above and by review |
| SC-001-004 critical errors expose `code`, `trace_id` and remediation without leaking | Done | `crates/api-http/src/error.rs` maps the closed code/reason table to a payload-free remediation; `context.rs` puts it beside `code`, `traceId` and the independent `requestId` for application and synthesized framework errors. The request-context test checks errors, successes and fallback routes and rejects an attacker-supplied request id; the existing E2E leak scan covers secrets and payloads. |

## Evidence: 002 durable-execution

"Partial" and "Open" are stated explicitly; see `specs/002-durable-execution/research.md` for the gaps (G19-G34) and the decisions (D25-D41) behind them.

| Requirement | State | Evidence |
|---|---|---|
| FR-002-001 interpret versioned IR: Activity, Condition, Parallel/Join, Timer, WaitEvent, ChildWorkflow | Done and extended by features 003/005 | Domain: `crates/domain/tests/{workflow,execution,execution_properties}.rs`; codec: `crates/infra-ir/tests/codec.rs`; every original kind end to end: `tests/integration/durable-execution/{flows,routing,children}.rs`. `saga` and `script` are now executable through features 003 and 005; only `foreach` and `humanApproval` remain refused at publish with a stable reason (G20). |
| FR-002-002 append-only journal sequenced per run, transitions verified by compare-and-swap | Done | `run_events` is append-only by trigger (`tests/integration/durable-execution/faults.rs`), the run row is guarded by `run_version`; replay equals the projections after every random action (`execution_properties.rs`) and for stored runs (`verify_run`, `assert_consistent`) |
| FR-002-003 step result, journal, new state, ready tasks, timers/subscriptions and outbox in one transaction | Done | `PgRunTx::persist`; crash/connection-kill chaos in `tests/integration/durable-execution/faults.rs`; outbox in `outbox.rs` |
| FR-002-004 leases, heartbeat, generation/fencing, expiry with safe requeue | Done | `tests/integration/durable-execution/{scheduler,concurrency}.rs`, `tests/contract/durable-execution/grpc/protocol.rs` (stale leases), `tests/e2e/durable-execution/stories.rs::us02_*` |
| FR-002-005 backoff with jitter, max attempts, timeouts, deadline, explicit cancellation | Done | `crates/domain/tests/execution.rs`, `scheduler.rs`, `flows.rs`; operator actions in `tests/contract/durable-execution/rest/` |
| FR-002-006 reconciler for the outbox, orphaned READY tasks and expired leases | Done | `crates/scheduler` (`hints`, `leases`, dispatcher); `scheduler.rs`, `outbox.rs`, `nats.rs`; the engine runs them: `tests/e2e/durable-execution/engine.rs` |
| FR-002-007 Paused/Waiting without a dedicated process and without a NATS consumer per run | Done | A waiting run is a row (benchmark: 2.4 KiB, idle tick flat from 1k to 10k runs); `stories.rs::us03_*` |
| FR-002-008 bind runs to the immutable version; control payload/result size via blob refs | Implemented and verified | ADR-0023: immutable version pinning, lease/tenant/namespace-scoped streamed outputs and creator-owned run inputs; atomic catalog binding and completion, bounded transfers, integrity and GC. Real PostgreSQL/gRPC contract 45/45 passes; worker blob conformance passes all five SDKs. REST upload/start and forged early-201 refusal pass all five SDKs. Three mixed-version regressions prove protocol 1.2 workers retain inline compatibility and cannot lease blob-backed work. |
| FR-002-009 persist the child-run parent link and the cancellation propagation policy | Done | `tests/integration/durable-execution/children.rs` (17 tests: creation race, cancelled meanwhile, tenant isolation, recursion bound, tree budget, deadlines, every policy), `crates/domain/tests/execution.rs::children`, full stack in `tests/e2e/durable-execution/engine.rs`. The child's result (its `end.outputExpr`) becomes the parent step's output (G21, `contract_grpc_durable_execution` `run_result::*`); the API does not disclose it (ADR-0033) |
| NFR-002-SEC tenant isolation, RBAC, least privilege, no secrets in logs | Done within the documented trust model | `isolation.rs`, `tests/contract/durable-execution/grpc/protocol.rs` (authentication, role, cross-tenant, session binding, no echo of secrets), engine start-up refusal (`engine.rs`); feature 009 adds real TLS/mTLS handshakes and fail-closed plaintext-secret guards (`grpc/tls.rs`) plus worker bindings rechecked on every lease-bound use (`integration/security-multitenancy/bindings.rs`). Feature 010 adds weighted scheduler/outbox fairness; worker polling still shares the database pool and retains its feature-002 claim policy (G32/G246). |
| NFR-002-DUR after commit, losing a worker or the broker cannot lose a transition | Done | `faults.rs`, `outbox.rs` (crash between publish and confirm), `nats.rs` (broker outage through a severed connection), `engine.rs` |
| NFR-002-OBS structured logs, correlated traces, essential metrics, auditability per run/step | Done | Journal with actor and trace id per event, audit records for operator actions and child starts, metrics in the runbook; feature 008 exports live and journal-derived spans over bounded OTLP/HTTP JSON and tests trace linkage through the scheduler and worker. |
| NFR-002-COMP versioned protocol and schemas; do not break runs in progress | Done with the repository's descriptor baseline | `worker.proto` changed additively; protocol major/minor is negotiated at registration; `crates/api-grpc/tests/compatibility.rs` compares the compiled descriptor with `worker.baseline.json`. This repository uses that gate instead of a second `buf breaking` implementation. |
| NFR-002-PERF benchmark of at least 10k suspended runs and scheduling latency | Done as a baseline | `tests/performance/durable-execution/main.rs`, report `tests/performance/reports/durable-execution.md` (a noisy shared machine; known limits G22) |
| SC-002-001 deterministic crash test at each point of claim, effect, commit, publish | Done for the points reachable from the application | `faults.rs` (kill before/after commit, between statements), `outbox.rs::a_crash_between_publishing_and_confirming_*`, `stories.rs::us02_*` |
| SC-002-002 100% of US01..US04 pass in a reproducible automated suite | Done | `tests/e2e/durable-execution/stories.rs` |
| SC-002-003 no violation of the Constitution gates and no silent loss of persisted state | Partial | The crash, replay and consistency tests above prove no confirmed loss in their scope, and the v1.0.0 Constitution now exists (G1 resolved). Principle II has the automated architecture gate; the other principles rely on the named suites and review, and the maintainer ratified the Constitution on 2026-10-11. |
| SC-002-004 critical errors expose `code`, `trace_id` and remediation without leaking | Partial across both transports; REST done | All durable-execution REST errors receive the canonical payload-free remediation and independent `requestId` described above. Worker gRPC carries the stable `x-sagat-error` and trace context; blob upload/integrity failures also carry `x-sagat-remediation`, but the general gRPC status mapper does not, so the combined HTTP/gRPC criterion remains partial. |

## Evidence: 003 sagas-compensations

Gaps and decisions: `specs/003-sagas-compensations/research.md` (G51–G65, D59–D71). Validation flow and the review: `specs/003-sagas-compensations/quickstart.md`. Operating it: `docs/runbooks/sagas.md`.

| Requirement | State | Evidence |
|---|---|---|
| FR-003-001 saga boundary, forward action, compensation, reconciliation and cancellation policy in the IR | Done | `saga`, `compensateWith`, `reconcileWith`, `compensationRetry`, `compensationTimeoutMs`, `irreversible`, `highRisk` in the workflow schema; derived nodes `<id>~c` / `<id>~r` (ADR-0010); `crates/domain/tests/{workflow,sagas}.rs`, `crates/infra-ir/tests/codec.rs`, `examples/payment-saga.workflow.json`. Feature 006 supplies equivalent saga builders in all five SDKs (`tests/contract/sdk-conformance/.../saga-compensation.json`, builder golden files). |
| FR-003-002 the compensation plan persisted as a reverse DAG, respecting branch independence | Done | `saga_plans` / `saga_plan_entries` (immutable, tenant-bound) written at publish, read at `GET …/compensation-plans`; `plan::*`, `us01`, `the_plan_is_the_same_as_the_one_worked_out_the_long_way`, `what_ran_after_a_join_is_undone_before_the_branches_that_fed_it`; the plan a run follows (`run_saga_entries`) is decided from what completed |
| FR-003-003 compensation with its own retries, idempotency, limits, timeout and provider circuit breaker | Done | default five attempts / `compensationRetry` / `compensationTimeoutMs`; `compensation:{id}:{run}` keys (`us02`, `crashes`, `provider`); the breaker per tenant, namespace and activity (`breaker::*`, mutation-checked); limits are the quotas and bindings of 009 applied to the derived steps. Lookups are not behind the breaker (G56); the breaker is per activity name (G55) |
| FR-003-004 distinguish COMPENSATED, PARTIAL, COMPENSATION_FAILED, MANUAL_REVIEW and irreversible | Done | the five statuses in the OpenAPI (and a test that none is missing), `run.finished` with all terminal ones (AsyncAPI, tested), `saga` state in the graph; `an_irreversible_effect_makes_the_outcome_partial...`, `a_sibling_saga_whose_compensation_failed_is_not_hidden...`, `us04` |
| FR-003-005 pause, retry compensation and skip, skip only with explicit permission and an audited justification | Done | `RETRY_STEP` / `RETRY_COMPENSATION` (`run:operate`), `SKIP_COMPENSATION` / `RESOLVE_STEP` (`saga:override`, administrators), justification after trimming, audit with the reason; `review::*`, `us04`, `contract_rest_sagas_compensations`. Feature 008 subsequently implements `MARK_MANUAL_REVIEW` and `RERUN_AS_NEW` with contract/E2E coverage. |
| FR-003-006 propagate cancellation / deadline to sub-workflows under a configured policy | Done | `cancelPolicy` of the `child` node (002) carries a cancellation into a child whose saga then undoes its work, and the parent waits (`request_cancel`) or not (`propagate`, `detach`); a child that ends in a saga status fails its parent's step with its own code. `children::*` (8 tests) |
| FR-003-007 reconcile or mark manual for an ambiguous high-risk result | Done | `LEASE_EXPIRED` / `ATTEMPT_TIMEOUT` / `OUTCOME_UNKNOWN` look the step up (`reconcileWith`) or, if `highRisk` without a lookup, wait for a person; retrying such a step needs an administrator; `reconcile::*`, `us03`, `provider::*` |
| NFR-003-SEC tenant isolation, RBAC, least privilege, no secret in logs or in the journal | Done | tenant in every key, foreign key and statement of the new tables; breaker per namespace; `saga:override` only for administrators; no label, audit record or error carries a payload; the lookup's output goes through the secret scan; two independent reviews (D71) |
| NFR-003-DUR a transition is not lost after commit; lease and fencing | Done | `fault_sagas_compensations` (13 tests: crash before commit, restart, effect before checkpoint, duplicates, three races, a real HTTP provider, connections killed under load), the domain property tests extended with sagas, `verify_run` after every scenario |
| NFR-003-OBS logs, correlated traces, essential metrics, auditability per Run/Step | Done | Six counters taken after commit with closed labels (D69), journal events, audit records and the graph; feature 008 subsequently supplied the shared dashboards and alerts (G60 resolved). |
| NFR-003-COMP versioned protocol and schemas; do not break in-flight runs | Done | protocol 1.2 additive (baseline +15 lines, none removed; a protocol-1.0 message decodes every lease), additive reversible migration, a run without a saga has no derived nodes |
| NFR-003-PERF compensating does not block threads; throughput is the provider's | Done as a baseline | `tests/performance/reports/sagas-compensations.md`: planning 49 compensations p95 below 250 ms, 4 workers 2.8x one, a run that waits for a person costs the scheduler and the pool nothing, the breaker's predicate leaves the claim path within 2x; 16 workers over a pool of 8 do worse than 4 (G57) |
| SC-003-001 E2E reserve→submit fails→release; ambiguous provider requires reconciliation; release failure triggers manual review | Done | `e2e_sagas_compensations` us02, us03 (engine restart), us04 |
| SC-003-002 100% of US01..US04 pass in a reproducible automated suite | Done | `e2e_sagas_compensations` (20 tests) and the suites above |
| SC-003-003 no violation of the Constitution gates and no silent loss of persisted state | Partial | The fault and replay suites above prove no confirmed loss in their scope. The v1.0.0 Constitution exists; it was ratified on 2026-10-11 and only principle II has a dedicated automated architecture gate. |
| SC-003-004 critical errors expose `code`, `trace_id` and remediation without leaking | Partial across API and persisted failures; REST done | Saga REST refusals now receive the canonical payload-free remediation and independent `requestId`. Persisted/provider step failures retain stable codes, safe messages and trace correlation but do not all have an independent remediation field. |

## Evidence: 004 events-webhooks-messaging

Gaps and decisions: `specs/004-events-webhooks-messaging/research.md` (G73–G95, D78–D94). Validation flow and the review: `specs/004-events-webhooks-messaging/quickstart.md`. Operating it: `docs/runbooks/events.md`. Design: ADR-0012.

| Requirement | State | Evidence |
|---|---|---|
| FR-004-001 manual/API, webhook, event, cron, message and signal triggers | Done | triggers in the workflow schema, registered with ids that survive republishing; `dispatch::the_triggers_in_force_are_those_of_the_active_version`, `a_filter_decides_which_events_start_a_run`, `a_webhook_trigger_is_about_its_endpoint_and_an_event_trigger_about_a_type`, `cron::*` (9), `us01`, `us03`; manual start and signals are feature 002's |
| FR-004-002 a single inbox per tenant/source/event_id with configurable deduplication TTL | Done | the primary key of `inbox_events`; digest to tell a retry from a reuse; window from the database clock, at least twice the skew and a minute for a signed endpoint; `ingress::*` (21) |
| FR-004-003 event_type+correlation subscriptions with timeout and atomic competition between event and timer | Done | waits found from both directions (`event_since`, `event_scanned_at`); `dispatch::*` (22), `fault_events_webhooks_messaging` (the race with the timeout, with a second dispatcher in it) |
| FR-004-004 HMAC/timestamp/nonce on webhooks; OIDC/mTLS per profile | Implemented and independently reviewed | ADR-0026: exactly one HMAC, deployment-approved OIDC or native verified mTLS profile. All share durable replay admission, bounded verification and audited refusal. Ingress 29/29 includes gated pause/replace/delete races after asynchronous verification; transactional configuration revalidation closes the review finding. Migrations 5/5, HTTP contracts 2/2, platform authentication 27/27 and real TLS/OIDC tests pass. |
| FR-004-005 inbound/outbound adapters for NATS, Kafka and SQS with ack/offset after persistence | Implemented and verified | ADR-0025 reuses transactional fanout, lease/fence, retries and DLQ/redrive. Five broker-specific tests pass with real NATS/Kafka/SQS payloads, ambiguous acknowledgement/restart, stable delivery IDs, tenant/egress boundaries, DLQ/redrive and migration rollback. All nine broker REST operations and AsyncAPI compatibility pass. T007-053 subsequently adds Studio create/edit/activate flows for inbound NATS/Kafka/SQS connectors with write-only credentials and namespace egress controls; outbound subscriptions remain API-only. External effects remain at-least-once. |
| FR-004-006 outbound HTTP webhook with signing, retry jitter, TLS, rate limit, DLQ, redrive, endpoint status | Done | `outbound::*` (19), `us04` ×2, `sender::*`, `fault deliveries::*`; rate enforced across workers by `next_slot_at`; switch-off after 20 dead and when 10 000 wait; TLS is the URL's scheme under the namespace's egress policy |
| FR-004-007 AsyncAPI versioned for messages and a schema evolution policy | Done | `events.asyncapi.yaml` 0.2.0 (validated), `events.baseline.json`, `asyncapi/*` (4 tests), `docs/event-schema-evolution.md` |
| FR-004-008 backpressure, size limits, retention, safe filters and quotas per tenant/connector | Implemented; customer classification remains separate | Bounded inbox backlog and request bodies, namespace/source durable event-rate admission, exact retained content bytes, retention ceilings plus replay minimum, and atomic dedupe/checkpoint/ack behavior (ADR-0027; event integration 111/111). No field-level customer data classification or broader raw-value disclosure policy is inferred (G80). |
| NFR-004-SEC tenant isolation, RBAC, least privilege, no secret in logs or in the journal | Done for what exists | tenant in every statement and composite foreign key of every new table; `event:publish|read|manage`; secrets sealed and named, never shown; the receiver of a webhook cannot make the platform store its answer; URLs shown without the path to roles that cannot manage; two independent reviews (D94). Open: G76 (broker sockets), G87 (waits by source), G90 (use-time re-check) |
| NFR-004-DUR after commit, losing a worker or broker cannot lose a transition | Done | `fault_events_webhooks_messaging` (8: six of the feature and two about cutting off a `begin`, ADR-0014), the dispatcher cut off at every await, leases, fenced attempts, idempotency keys that include the admission, connections killed under load |
| NFR-004-OBS logs, correlated traces, essential metrics, auditability | Done except traces and dashboards | the counters, gauges and histograms of the runbook; audit of every change, of publishing, of refusals (collapsed) and of the platform's own switch-offs. Trace propagation from an event into a run (G79) and dashboards (G85) are open |
| NFR-004-COMP versioned protocol and schemas; do not break in-flight runs | Done | AsyncAPI only grown (checked against a baseline), OpenAPI and the workflow schema additive, additive reversible migration that backfills the waits that existed |
| NFR-004-PERF ingress p95 < 300 ms on persistence without heavy work; scales by partition/correlation key and quotas | Done as a baseline | `tests/performance/reports/events-webhooks-messaging.md` (release, clean tree): ingress p95 and under 16 providers at once, refusals cheaper than admissions, dispatchers scale within the pool, waiting costs the dispatcher and the scheduler nothing, fan-out to 50 subscriptions adds little, the rate of a subscription holds with two workers, connector ingest. One bound was restated openly (dispatcher scaling) |
| SC-004-001 signed webhooks, duplicates, retries, out-of-order and event/timeout race in integration tests | Done | `integration_events_webhooks_messaging`, `fault_events_webhooks_messaging` |
| SC-004-002 100% of US01..US04 pass in a reproducible automated suite | Done | `e2e_events_webhooks_messaging` (11 tests, with real Kafka, SQS-compatible and NATS containers) |
| SC-004-003 no violation of the Constitution gates and no silent loss of persisted state | Partial | No loss is provable by the tests above; only principle II has an automated gate; the constitution was ratified on 2026-10-11 |
| SC-004-004 critical errors expose `code`, `trace_id` and remediation without leaking | Partial across API and background deliveries; REST done | Webhook, connector and delivery REST refusals now receive the canonical payload-free remediation and independent `requestId`. Persisted background delivery failures retain stable safe reasons but do not all have an independent remediation field. |

## Evidence: 009 security-multitenancy

"Partial", "Open" and "Not built" are stated explicitly; see `specs/009-security-multitenancy/research.md` for the gaps (G35-G50) and the decisions (D42-D58) behind them, and `specs/009-security-multitenancy/quickstart.md` for the security review of the implementation, finding by finding.

| Requirement | State | Evidence |
|---|---|---|
| FR-009-001 tenant and namespace on every entity and query; granular RBAC on all APIs | Done for the APIs that exist | Composite foreign keys on every new table (`crates/infra-postgres/migrations/20261010*`); `tests/contract/security-multitenancy/rest/battery.rs` attacks every implemented REST operation (39) with a valid administrator token of another tenant, driven by the OpenAPI document so a new operation without an entry fails it; `tests/e2e/security-multitenancy/us01.rs` does the same through the worker protocol with a leaked lease token; role matrix in `rest/conformance.rs` and in each integration suite. A later feature that adds an API must add it to the battery |
| FR-009-002 policies for resources, secrets, scripts, connector scopes, publish and manual operations | Partial | Resources: seven permissions in `crates/domain/src/identity.rs`; worker bindings (`integration/.../bindings.rs`); secrets: role `secret:use` plus a per-secret access list, checked at publish and again at every resolution for the publisher (`secrets.rs`, `us02.rs`); publish and manual operations: existing roles. Scripts and connector scopes: features 005 and 004. No custom roles (G39). A secret's list controls who may reference it; delivery to workers is limited by an optional per-secret list of worker pools, enforced at the lease and again at `ResolveSecrets` and audited once per worker identity, run, node and attempt (`secret_pools.rs`, G48) |
| FR-009-003 federated OIDC/JWT identity, short-lived service tokens, rotatable secrets | Done, KMS open | OIDC/JWT from feature 001; the lease is the short-lived capability for a secret and ends with the lease (`secrets::the_capability_ends_with_the_lease...`); rotation with versions, a key ring, `control-plane reseal-secrets` (`secrets.rs`); TLS and mutual TLS on the worker API with real handshakes (`contract/.../grpc/tls.rs`). Open: KMS/HSM (G43), certificate-bound tokens (G44) |
| FR-009-004 immutable, redacted audit trail for access to sensitive resources | Done | Every governance change is audited in the same transaction as the change; governance refusals are audited off the response path, while sandbox egress denials are acknowledged only after their category-only incident commits; no row carries a secret value (`us02.rs::the_secret_reaches_no_response_log_row_or_error_on_the_way` searches every table, every log line and every response for a canary); repeats a caller controls are written once (`secrets::asking_again_within_an_attempt...`). `audit_records` is append-only by trigger (feature 001) |
| FR-009-005 scripts with cgroups, namespaces, seccomp and egress proxies in isolated pools | Partial | Built by feature 005 for TypeScript, Python, C#, Go and Java: a container per build and run with no network, read-only root, no capabilities, seccomp, memory/CPU/process/time limits, and requests only through the supervisor under the namespace's egress policy (`integration_inline_code_sandbox_containers`, `e2e_inline_code_sandbox::us04`). The live supervisor reloads namespace policy before every fetch and fails closed on policy/incident persistence errors (`e2e_security_multitenancy::us03`, `fault_security_multitenancy::egress`). The integrated Helm chart enforces role-specific egress and worker-pool ingress, rejects empty/global peers and invalid ports, and passes `contract_scalability_ha` plus `tests/security/sandbox/kubernetes-network-policy.sh` on real kind/Cilium. The earlier policy-only evidence did not prove a product/KEDA rollout; T010-038 subsequently supplied that full one-node product/KEDA proof. It still does not prove node drain, cross-node rescheduling, trust-level pools or stronger runtime isolation (G98, G99). |
| FR-009-006 quotas for API/event rate, concurrent runs, workers, builds, storage, logs, retention | Implemented and independently reviewed | ADR-0027 supplies exhaustive storage classification, exact transactional content/log/reservation counters, shared namespace/source token buckets, canonical authorized API admission and pinned retention. Migration `20261026000000` activates byte guards after terminal/stop, compensation and late-PUT cleanup proofs. Enabled-by-default suites passed: durable execution 176/176, security 95/95, scripts 54/54; event integration 111/111 and two-server HTTP admission 10/10. The new 2,000-operation quota benchmark records exact bytes and lock latency. Product tier defaults remain provisional (G42). |
| FR-009-007 supply chain, artifact verification, policy changes with cache invalidation | Partial | `cargo audit --deny warnings` and locked builds in CI; no policy is cached, so a change applies to the next call. Feature 005 signs artifacts with Ed25519 over tenant/digest/language, requires runner verification, stores a CycloneDX SBOM, and admits the C# dependency catalog only with exact hashes plus fatal NuGet vulnerability auditing (`script_signing.rs`, `integration_inline_code_sandbox_containers::csharp`). General customer-selected dependency catalogs and a policy for future disclosures remain open (G38/G97). |
| FR-009-008 webhook SSRF/replay and egress destinations | Partial | Egress destinations: default-deny per namespace, no policy can open loopback, link-local, metadata, private, multicast or reserved space, IPv4 hidden in IPv6 or numbers in disguise, and sandbox refusals commit only a closed category without hostname, URL, path or query (`integration/.../egress.rs`, `incidents.rs`). Fenced production/test incidents deduplicate per attempt, with database-clock lease expiry and historical audit lookup verified (9 PostgreSQL tests). Inbound webhook signature and replay: built in feature 004 (HMAC over the envelope, timestamp window, replay window, uniform 401; `ingress::*`); outbound webhooks and broker connectors go through the same guard (`sender::*`), with the broker caveat G76 |
| NFR-009-SEC tenant isolation, RBAC, least privilege, no secrets in logs or the journal | Done for what exists | The battery, the leak scan, `us01.rs`, `us02.rs`; the security review of the implementation found 8 groups of issues, all verified and 7 fixed, 1 (G48) closed afterwards by the worker-pool restriction (`specs/009-security-multitenancy/quickstart.md`) |
| NFR-009-DUR after commit, losing a worker or the broker cannot lose a transition | Done | `tests/fault/security-multitenancy/faults.rs` kills connections around quota-checked starts, secret writes, secret resolutions and binding replacements; every committed change has its audit record (an attempt that was handed secrets has exactly one, however often it asked) and nothing is half done |
| NFR-009-OBS structured logs, correlated traces, essential metrics, auditability | Done | Audit actions (runbook section 6); counters `security_denials_total{action}`, `quota_exceeded_total{limit}`, `secrets_handed_out_total`, `secret_resolution_denied_total`, `secret_output_detected_total`, `grpc_connections_refused_total`, `grpc_connections_closed_total`, none with a tenant, subject or secret in a label (`e2e/.../telemetry.rs` checks the page). Feature 008 supplies bounded OTLP/HTTP JSON span export. |
| NFR-009-COMP versioned protocol and schemas; do not break runs in progress | Done | `worker.proto` only grew (`ResolveSecrets`; compatibility baseline test); additive migrations with a tested `down`; OpenAPI additive; the workflow IR gained an optional field |
| NFR-009-PERF a quota decision must not depend on an eventually consistent counter | Done as a baseline | `tests/performance/reports/security-multitenancy.md`: exact counting under a lock, its cost against the namespace's active runs, the tenant-lock cost measured with a two-tenant case, the cost of the binding check, secret resolution and the output scan. The count grows linearly with a namespace's active runs (a counter row is the answer far above the default limits) |
| SC-009-001 battery of cross-tenant, SSRF, sandbox escape, secret leak, webhook replay and quota racing | Done | Built: cross-tenant, SSRF on the egress guard, secret leak, quota racing; webhook replay and forged signatures (feature 004, `ingress::*`, `fault_events_webhooks_messaging`); and sandbox escape (feature 005): the cloud metadata service, the engine, loopback and every name unreachable from the container, no capabilities or privileges, host files and the engine's socket invisible, memory/process/CPU/output/disk bombs held, execution from the scratch area refused, a secret never in the log or the result (`integration_inline_code_sandbox_containers`, `e2e_inline_code_sandbox::us04`). A kernel or runtime vulnerability is an escape this battery cannot rule out (G98) |
| SC-009-002 100% of US01..US04 pass in a reproducible automated suite | Done | US01..US04 pass (`e2e_security_multitenancy`: 7/7), including production TypeScript/Python sandbox egress. Security faults pass 10/10, including policy revocation between calls and audit commit before runner lease loss. Feature 005 separately validates C# execution. |
| SC-009-003 no violation of the Constitution gates and no silent loss of persisted state | Partial | The transactional/fault suites above prove no confirmed loss in their scope, and the v1.0.0 Constitution exists. It was ratified on 2026-10-11 and only principle II has a dedicated automated architecture gate. |
| SC-009-004 critical errors expose `code`, `trace_id` and remediation without leaking | Partial across HTTP and worker gRPC; REST done | Security and quota REST refusals now receive the canonical payload-free remediation and independent `requestId`, retaining safe `details.limit` or `details.reason`. The general worker-gRPC status mapper still has no remediation metadata, so the combined transport criterion remains partial. |

## Evidence: 005 inline-code-sandbox

Gaps and decisions: `specs/005-inline-code-sandbox/research.md` (G96–G120, D95–D118). Validation flow: `specs/005-inline-code-sandbox/quickstart.md`. Operating it: `docs/runbooks/scripts.md`. Design: ADR-0013, ADR-0021 and ADR-0024. The earlier G96/traceability statement that Go and Java were unimplemented is superseded by T005-037..045 and the native CI evidence below.

| Requirement | State | Evidence |
|---|---|---|
| FR-005-001 immutable ScriptDraft, ScriptVersion, BuildJob and ArtifactDigest per tenant/namespace | Done | `scripts`, `script_versions`, `script_build_jobs`, `script_artifacts` with composite tenant keys; a version and an artifact are immutable by trigger; a draft is a compare-and-swap; the same draft is the same version; `integration_inline_code_sandbox scripts::*`, `builds::*` |
| FR-005-002 isolated per-language builders; pinned toolchain and lockfile; SBOM; supply-chain policy | Partial only for general dependency installation | TypeScript, Python, C#, Go and Java build in isolated no-network containers with pinned image/toolchain identities in provenance and CycloneDX evidence; artifacts are signed and verified before execution. C# accepts only the exact offline Newtonsoft.Json 13.0.4 catalog closure. Go and Java deliberately reject dependency requests and non-empty lockfiles. Arbitrary dependencies and TypeScript/Python package installation remain unsupported (G97; ADR-0021/0024). |
| FR-005-003 TS/JS and Python first; C#/.NET, Go and Java with dedicated runners | Done | TypeScript, Python and C# retain their isolated runners. Go now builds a signed target-specific static v3 ELF and executes it through the sealed-memory loader; Java builds validated Java 17 class files and executes them in the trusted parent-first host. Native AMD64 and ARM64 jobs passed on CI run 38200759888, with all five production/resource tests subsequently run in both architecture jobs (run 38201472580). |
| FR-005-004 execute in the worker sandbox with cgroups/namespaces/seccomp/containers and a network policy | Done for all five languages | The existing TypeScript/Python/C# real-container regressions remain, and the Go/Java production-path tests add mediated host calls, redaction, direct-network denial, timeout/resource enforcement, target mismatch and cleanup under the unchanged sandbox profile (`integration_inline_code_sandbox_containers`, `e2e_inline_code_sandbox::us04`, native CI jobs). |
| FR-005-005 TestRun without publishing, own quota, no prod secrets | Done | `e2e_inline_code_sandbox::us01` (10), `integration_inline_code_sandbox trials::*`; no secret of the namespace ever reaches a test run; its own `script_jobs` quota; one no runner takes fails as unavailable |
| FR-005-006 input schema / typed form and output validated against JSON Schema | Done | schemas checked when written (`infra-schema`), the input checked before the script runs and the result after, violations named by rule and path and never by value, schemas inferred from what was seen (`e2e_inline_code_sandbox::us01`). The typed form is Studio's (G105) |
| FR-005-007 redacted stdout/stderr, exit status, deps, metrics and trace with configurable retention | Done for supported catalog | per-attempt execution records with the redacted bounded log, exit code, numbers and trace id, retention 1–365 days per script, purged by the scheduler (`integration_inline_code_sandbox executions::*`); C# catalog dependencies are embedded in the signed artifact and listed in its SBOM; arbitrary dependencies remain unsupported (G97) |
| FR-005-008 build caches by source hash+toolchain+deps+policy, invalidated when the policy changes | Done | `builds::an_identical_build_is_reused_and_a_change_of_source_policy_or_toolchain_is_not`, `e2e_inline_code_sandbox::us02`; the schemas are part of the key too |
| FR-005-009 bind Go artifacts to one natively proven Linux target and fail closed on mismatch/probe failure | Done for Linux AMD64 and ARM64 | Artifact v3 signs the target; `SAGAT_RUNNER_GO_TARGET` configures one homogeneous pool target; the production compile-and-execute startup probe runs before Go is advertised and can be required for readiness. Native ARM64 local proof and independent native ARM64/AMD64 CI jobs passed. A single eligible pool still cannot mix targets until claims carry the target. |
| FR-005-010 preserve seccomp/noexec while executing Go through a sealed memory file | Done | The signed static ELF is validated, decompressed under a bound and executed from a four-seal memfd with sealed input fd 3 and mediated replies; malformed gzip/ELF, executable-stack/interpreter/target mismatches, direct network and resource violations are refused before or during the isolated production path (ADR-0024; T005-038/039/042). |
| FR-005-011 fixed Java 17 build and trusted parent-first in-memory runtime | Done | `runners/java/` invokes fixed `javac --release 17 -proc:none`, accepts only validated `Script.class`/nested classes, rejects JARs, unrelated or wrong-version classes and runs the bytes through the trusted Java/Jackson host with the same mediated supervisor contract (T005-038/040/042). |
| FR-005-012 reject Go/Java dependencies and bind exact build/runtime identities into provenance and SBOM | Done | Non-empty lockfiles and dependency/package requests are rejected; compiler, ABI, Go target/compression, Java JSON library and exact builder/runtime image identities are signed in v3 provenance and represented in SBOM evidence (T005-042/044). This does not add general dependency installation. |
| NFR-005-SEC tenant isolation, RBAC, least privilege, secret never in logs or in the journal | Done for the delivered five-language runners | a `runner` role with three permissions and nothing else; tenant in every statement; the runner has no database and no signing key; a secret reaches a script only if the step declared it and never leaves through the log or the result (`us04`); native Go/Java tests cover mediation, redaction and direct-network denial. Open: stronger isolation of the runner host and trust-level pools (G98, G99) and G102. |
| NFR-005-DUR after commit, losing a worker or broker cannot lose a transition | Done | leases fenced by token hash for builds and test runs; script steps use the engine's own lease; `fault_inline_code_sandbox` (12): runner death mid-build, mid-test and mid-step, restart, engine down, cancellation |
| NFR-005-OBS logs, correlated traces, essential metrics, auditability per Run/Step | Done for the shared telemetry path; script-specific dashboard panels remain open | Runner and control-plane metrics (runbook §4), execution records with the trace id and `script.*` audit; feature 008 supplies the shared OTLP/HTTP JSON exporter and overview dashboard, while that dashboard does not currently expose the runner's `script_*` series. |
| NFR-005-COMP versioned protocol and schemas; do not break runs in progress | Done | OpenAPI remains additive; artifact v3 adds exact target/ABI/encoding metadata while existing v1/v2 fixtures and pinned digests remain valid and immutable. The v1/v2 regression suite is part of the schema/native gates. |
| NFR-005-PERF cold/warm start metrics per language; caches only if policy/digest are identical; builds separate from execution | Done as a baseline | `tests/performance/reports/inline-code-sandbox.md`, `inline-code-sandbox-csharp.md` and `inline-code-sandbox-go-java.md`; the detailed Go/Java timing table is native ARM64 only, while AMD64 has functional native CI proof. These local samples are not a production SLO or tail-latency claim. |
| SC-005-001 TS/Python/C#/Go/Java script goes from the editor to execution; escape tests and CPU/memory/network limits verified | Done within the recorded native targets | Studio and API flows retain TypeScript/Python/C# coverage; Go/Java production-path tests cover compile/run, mediated fetch/secret/log/idempotency calls, redaction, direct-network denial, timeout/resource enforcement and cleanup. Native CI proves both Linux AMD64 and ARM64; it does not permit mixed-target Go pools. |
| SC-005-002 100% of US01..US04 pass in a reproducible automated suite | Done | The original `e2e_inline_code_sandbox` 28/28, registry integration 41/41, fault 12/12 and real-container 30/30 evidence remains; US05/US06 native coverage is recorded separately under SC-005-005/006 rather than retroactively changing the historical US01..US04 counts. |
| SC-005-003 no violation of the Constitution's gates and no silent loss of persisted state | Partial | No loss is provable by the tests above; only principle II has an automated gate; the constitution was ratified on 2026-10-11 |
| SC-005-004 critical errors expose `code`, `trace_id` and remediation without leaking | Partial across REST and execution records; REST done | Script REST refusals now receive the canonical payload-free remediation and independent `requestId`. Sandbox execution records retain stable `SCRIPT_*` failure codes, safe reasons and trace correlation but do not all have an independent remediation field. |
| SC-005-005 malformed v3 gzip/ELF/class fixtures fail before execution; v1/v2 remain compatible | Done | Additive bundle/schema tests cover canonical/base64 gzip, expansion bounds, malformed members, ELF and class structure, duplicates and unknown fields; unchanged v1/v2 fixtures remain in the regression suite (T005-038/042). |
| SC-005-006 Go capability follows a successful native target probe with independent ARM64/AMD64 evidence | Done for both recorded targets | Native ARM64 production-path proof is recorded in the feature quickstart/report. The report's earlier “AMD64 pending” paragraph is historical and is superseded by native AMD64 job 114653433277 and ARM64 job 114653433252 on CI run 38200759888; run 38201472580 subsequently passed all five native production/resource tests on both architectures. |

## Evidence: 006 sdk-multilanguage

Gaps and decisions: `specs/006-sdk-multilanguage/research.md` (G121–G141, D119–D137). Validation flow: `specs/006-sdk-multilanguage/quickstart.md`. Using and operating the SDKs: `sdks/README.md`, `docs/runbooks/sdks.md`. Behaviour: `sdks/SPEC.md`. Design: ADR-0015.

| Requirement | State | Evidence |
|---|---|---|
| FR-006-001 canonical workers schema in Protobuf and REST, with versions and error codes | Done | `worker.proto` 1.0–1.4 with negotiated minor compatibility (`tests/contract/sdk-multilanguage/grpc/negotiation.rs` and the mixed 1.2/1.3 admission/rollback regressions); the OpenAPI; the 19 canonical errors in `sdks/error-catalog.json`, embedded by each SDK and compared by a test of each. Protocol 1.3 adds blobs/run inputs without taking inline compatibility from 1.2 workers; 1.4 adds finite-credit task streaming, with explicit older-minor probes and fallback tests. |
| FR-006-002 five SDK clients with Start/Get/List/Cancel/Signal/PublishEvent | Done | TypeScript, Python, Go, C# and Java; `run-lifecycle`, `signals-and-events`, `errors`, `lost-responses`; `e2e_sdk_multilanguage us01` |
| FR-006-003 typed WorkerHost and Activity registration with lease heartbeat and cancellation | Done | `worker-registration` (NOT_BOUND, restart), `secrets` (a secret read through the real `ResolveSecrets`, its value in no output; a protocol-1 worker gets `UNSUPPORTED_PROTOCOL`), `cancellation` (a handler that ignores its token), `lease-lost`, `worker-shutdown`, `worker-transport-faults`; `e2e_sdk_multilanguage us02`; `fault_sdk_multilanguage` (9) |
| FR-006-004 Step.Run, Sleep, WaitForEvent, InvokeChild, Parallel, Saga.Compensate, Reconcile in idiomatic forms | Done | the builders of the five write the same document **and the same canonical bytes** (`builders`, with edge cases for key order and number formatting); `saga-compensation` runs compensation, reconciliation and the three verdicts (applied, not applied, unknown); InvokeChild and Parallel are built and published, their execution is the engine's (002) |
| FR-006-005 W3C traceparent, correlationId, idempotency key, deadline; retries only when safe | Done | `run-lifecycle` (the trace reaches the activity; the deadline is `<createdAt+3600s>`), `lost-responses` (a lost answer or a call that never left is one run; a refusal is not repeated; a 408 and two 503 are; `faultsApplied`), `errors`; `fault_sdk_multilanguage network`; one retry rule by code in the five SDKs (D127, D133) |
| FR-006-006 one conformance suite with fixtures and fault injection for five languages | Done | `tests/contract/sdk-conformance/` now has 19 common scenarios, including blob, run-input and partial-delivery stream fault flows, executed across all five SDKs (95 final recorded executions) against a platform and real engine with faults at each SDK's transport hook. The earlier 15-scenario three-pass timings remain historical evidence for the pre-blob suite. |
| FR-006-007 C# with Microsoft.Extensions.DependencyInjection, IHostedService, CancellationToken, ILogger, ActivitySource | Done | `sdks/dotnet` (`DependencyInjectionTests`, `AspNetCoreTests`, `TelemetryTests`); `e2e_sdk_multilanguage us01` plays an ASP.NET Core service against the real platform through the `aspnet` mode (the same run for the same key, the trace the request came with) |
| FR-006-008 packages with SemVer, runtime version matrix and minimal examples | Local distribution implemented; registry publication conditional | MIT was selected and applied to the repository and all five SDK packages. T006-037 built 11 reproducible distributables plus checksums, including Maven source/Javadoc classifiers and SDK release notes; five clean consumers passed in paired same-input runs, and final smoke checks include the blob APIs (`scripts/package-sdks.sh`, `docs/runbooks/sdk-packaging.md`, CI run 38210316160). The older G123 list that says release notes and Maven classifiers are missing is superseded by this evidence. Public registry namespaces/credentials, external signing/provenance verification and durable release retention remain release prerequisites; no registry publication is claimed. |
| NFR-006-SEC tenant isolation, RBAC, least privilege, secret never in logs or in the journal | Done for what exists | `fault_sdk_multilanguage tenant`; four independent reviews (security and correctness, each twice) found defects, each read against the code and fixed failing-first (the list is in `research.md` and the `fix(sdk)` commits): a re-register storm in every SDK, a heartbeat that died on a provider exception, secrets readable after the handler, a body cap that a TLS option bypassed, credentials in every printable form, Jackson advisories, Go advisories; TLS and mTLS tested for both channels in every SDK; no SDK follows a redirect or can turn verification off (a test of each reads its sources). Remaining: G134–G136 |
| NFR-006-DUR after commit, losing a worker or broker cannot lose a transition | Done | stale reports dropped, reports idempotent (`done-<id>`), `lease-lost`, `worker-shutdown` (a stopped host does not fail a step for good), `worker-transport-faults`; local lease fencing is not done (G134) |
| NFR-006-OBS structured logs, correlated traces, essential metrics, auditability | Opt-in OTel integration verified | All five SDKs passed real OpenTelemetry in-memory exporter tests for parentage, bounded safe attributes/metrics and exporter failure isolation. TS/Python/Go/Java bridge the existing hooks; C# uses ActivitySource/Meter. Providers and exporters stay application-owned. See [SDK observability](runbooks/sdk-observability.md); final integrated conformance remains a release gate. |
| NFR-006-COMP versioned protocol and schemas; do not break runs in progress | Done | minor negotiation (ADR-0015); protocol 1.3 added blob/run-input capabilities while mixed-version tests prove accepted 1.2 workers retain inline behavior and cannot claim blob-backed work; the OpenAPI changes are additive. |
| NFR-006-PERF no busy polling; measure streaming and backpressure | Done as a baseline | `tests/performance/reports/sdk-multilanguage.md` (release build, three complete runs on 2026-10-10, five SDKs, on a loaded machine): an idle worker polls 2.0 times per second as the server asks (no busy polling), a worker holds its limit (83–92 % of the ideal throughput), a run costs 1.8–2.2 calls; H1, H3 and H4 were restated after the first runs, openly (report header). Protocol 1.4 finite-credit streaming is implemented (ADR-0029), with 95 shared conformance executions, six E2E and nine fault tests passing; the [30-row release comparison](../tests/performance/reports/sdk-streaming.md) measures all five SDKs, including backpressure, signed cross-clock latency and process-tree RSS (G125, measured within the report’s limits; final five-consumer packages pass); a heartbeat of the lease that already reported is answered OK with no deadline instead of `aborted` (G141, 2026-10-16; lost or forged leases stay `STALE_ATTEMPT`); wall times are of this machine only (G140) |
| SC-006-001 same golden flows and failure cases in TS, Go, Python, C# and Java with normalized results | Done | the matrix above |
| SC-006-002 100% of US01..US04 pass in a reproducible automated suite | Done | `e2e_sdk_multilanguage` (6) with all five languages |
| SC-006-003 no violation of the Constitution's gates and no silent loss of persisted state | Partial | no loss is provable by the tests above; only principle II has an automated gate (architecture check) |
| SC-006-004 critical errors expose `code`, `trace_id` and remediation without leaking | Done | every error has `code`, `traceId`, `retryable` and a remediation sentence in the five SDKs; the harness fails a scenario when an error repeats a token or a payload the scenario marked, and when any runner output holds a token of the run |

## Evidence: 007 workflow-studio

Gaps and decisions: `specs/007-workflow-studio/research.md` (D138–D159, G142–G168). The visual direction is implemented; the original
baseline and current restyle checks are distinguished in `specs/007-workflow-studio/quickstart.md`. Using and operating the Studio:
`docs/runbooks/studio.md`, `frontend/README.md`. Design: ADR-0016 and `specs/007-workflow-studio/design/README.md`. Benchmark:
`tests/performance/reports/workflow-studio.md` (restyled benchmarks passed).

| Requirement | State | Evidence |
|---|---|---|
| FR-007-001 canvas with React Flow and a typed node registry | Done, with notes | `model/registry.ts` (step, script, condition, parallel with its `join`, timer, wait for an event, child, saga, end; the start is a card and a webhook is a kind of trigger, D145; human approval and for-each are drawn and cannot be published, G147); `canvasModel.test.ts`, `WorkflowEditor.canvas.test.tsx`; journeys US01 and SC-007-001 |
| FR-007-002 Monaco, schema-aware input/output editors, autocomplete and lint | Done | Bundled Monaco JSON services validate and autocomplete workflow input/output schemas using independent models; the accessible plain-text fallback preserves incomplete edits across blur and mode switches. T007-047 records 31 focused tests and three real Chromium US01 journeys; the complete Studio suite now passes 1,042 tests. Script editing retains its language services and schema-aware test input. |
| FR-007-003 compile and verify the graph into the IR, no cycles or broken references | Done | `POST /workflows/{id}/validate` is the publish check as a dry run (`definition_service.rs`: 39 tests; `e2e_workflow_studio us01`), every problem names its node, the editor refuses a link that closes a cycle, and a rename follows the links and the `steps.<id>` reads of expressions (`references.ts`); one graph error at a time (G142) |
| FR-007-004 draft and published versions, diff, a warning for contract-breaking changes, no in-place change | Done for the kinds it checks | `GET /workflows/{id}/diff` (`infra-ir/tests/diff.rs`: 18 tests, including documents built to cost gigabytes), the publish dialog asks twice for a break (journey US01); a published version is immutable (001); the classifier checks a set of kinds of break and says which (G155) |
| FR-007-005 journal projected into a graph with attempts, retries, compensations, trace links and redacted payload | Done | the graph, changes and stream carry every step an event creates (`RunEvent::changed_nodes`; `e2e_workflow_studio us03`, the domain chaos test), attempts/retries (`attempts.ts`), undo/lookup edges and the journal timeline. `POST /runs/{id}/payload-inspections` and the Studio now provide explicit Admin/Operator-only `structure-only-v1` inspection: every key and scalar is removed server-side, external content is metadata-only, and the selected read plus collapsed audit commit together before projection. The real PostgreSQL/API suite passed 2/2, focused application/IR/identity suites 6/6, 4/4 and 27/27, 70 Studio tests and one real Chromium journey passed with canaries absent (T007-048, ADR-0028, G144). T007-055 closes G156 with validated runtime trace URLs, copy fallback, 15 HTTP-serving tests, 34 configuration tests and two real Chromium scenarios. |
| FR-007-006 incremental updates by stream with a polling fallback, no reload of the whole graph | Done | `GET /runs/{id}/graph/changes` and `/stream` (`e2e_workflow_studio us03`: 14 tests, among them reconnection with `Last-Event-ID`, the caps, the token's expiry, a removed member, the cost of a quiet stream), `follow.ts` and its tests, the journey US03 asserts the graph is loaded once; a quiet stream costs the read of one row per tick (G143) |
| FR-007-007 RBAC and a confirmation that explains the consequence | Done | `ops.ts` and the dialog (`operations.test.tsx`: 32 tests), `e2e_workflow_studio us04`, `fault_workflow_studio` (two operators at once, a request repeated, a stale view, an undo that fails again, another tenant), journeys US04 |
| FR-007-008 WCAG AA as a target and an alternative table/list view | Done as a target, partly proven | axe-core (WCAG 2.2 A and AA rules) on every page in the journeys, dialogs and menus driven by the keyboard alone, the table and the timeline as first-class views, the list view of the editor; T007-050 adds focused, explicit route-load recovery with 320px reflow/axe and a real CSS-404→reload→sign-in journey (24 browser journeys passed, two benchmarks skipped, US02 excluded; quickstart). **No screen reader was used by a person (G146).** |
| FR-007-009 the visual direction on every screen | Implemented; owner approved | shared tokens/themes, local Geist fonts, sidebar/top bar, wide Runs master/detail, graph cards/curves/grid and derived editor/Script/sign-in styles; `tokens.test.ts`, `Shell.test.tsx`, `RunDetailPage.test.tsx`, `styles.test.ts`; final browser suite passed after the dark Monaco correction, including all three themes, 320px reflow and system fonts; owner approval was recorded on 2026-10-11 (T007-043/044), with captures linked in the design record. T007-049 corrects initial graph scale, responsive panels, forced-colour controls and populated Scripts scrolling; 1 049 unit tests and 23 real-browser journeys passed on 2026-10-12 (two opt-in benchmarks skipped; US02 excluded), with geometry coverage and eight before/after captures in the quickstart/design record. |
| FR-007-010 no element without a data source | Implemented and unit-tested | only real navigation destinations; run facts use API fields, omit queue/worker/trigger-kind/global counts, and show unknown terminal duration without `finishedAt`; `Shell.test.tsx`, compact-list and detail regressions; authorized payload structure inspection is delivered (T007-048); signal action and per-step timeline bars remain unbuilt (G163–G168) |
| T007-052 integrated namespace management | Done | A dedicated **Namespaces** menu and `/namespaces` route list tenant namespaces/environments, select context and authorize creation through the real identity API. Ten focused tests, the 1,066-test full frontend suite and a real Chromium/API creation-selection journey are recorded in the feature quickstart; the later full-suite pass supersedes the earlier six concurrent graph-layout failures. |
| T007-053 integrated broker management | Done for inbound connectors | A dedicated **Integrations** menu lists and creates/edits/activates inbound NATS JetStream, Kafka and SQS connectors with write-only secret references and explicit namespace egress editing. Eleven focused component tests, 1,077 frontend tests and a real Chromium/API persistence/redaction journey passed. The journey used paused synthetic brokers and did not claim external connectivity; outbound event subscriptions remain a separate API without Studio UI. |
| NFR-007-SEC tenant isolation, RBAC, least privilege, no secret in logs or the journal | Done for what exists | two rounds of independent security and correctness reviews of the Studio, each confirmed finding fixed failing-first (quadratic and unbounded work on hostile documents, the file server's memory, the cost and the role of a stream, a delta that missed created steps, step reads after a rename, a decision repeated with a new key; the list is in `quickstart.md`); the strict policy of the page is asserted (`crates/api-http/tests/studio.rs`, journey SEC), the token is in memory only, text from outside is rendered as text (journey SEC), another tenant gets the answer an unknown run gets (`contract_rest_security_multitenancy` battery, `fault_workflow_studio`, `e2e_workflow_studio`) |
| NFR-007-DUR after commit, losing a worker or the broker loses no transition | Done | the Studio adds no state; a decision is made once however many ask and however often (`fault_workflow_studio`); deltas equal the whole graph (`e2e_workflow_studio us03` and an invariant of the chaos test) |
| NFR-007-OBS logs, correlated traces, essential metrics, auditability | Partial | the platform's request metrics, trace ids in every error, `run_streams_open`, `run_streams_refused_total`, `run_streams_tick_errors_total`, `http_write_stalls_total`, every decision audited with its reason; **the browser sends no telemetry (G159)**, the Studio's files have no request metrics (G158) |
| NFR-007-COMP versioned protocol and schemas, do not break runs in progress | Done | the OpenAPI changes are additive (validate, diff, changes, stream, the `begin` edge, `changesOmitted`, `operationId` derived), the TypeScript types are generated from the contract and checked in CI (`npm run check:api`) |
| NFR-007-PERF a 1 000-node DAG usable, virtualized lists for 10k+, FPS targets | Proven on the stated reference environment | Restyled `tests/performance/reports/workflow-studio.md`: both benchmarks passed, H1–H6 held in three cold rounds each with unchanged floors. Median open 687 ms for 1 000 editor nodes and 1 470 ms for 9 991 run nodes; 245 run cards, at most 29 table rows, peak heap 307 MB. **One machine, Chromium, stationary run and loopback network (G151)**. |
| SC-007-001 create → validate → publish → trigger → fail → compensate → inspect | Done | journey `sc001-chain.spec.ts`: a saga built in the Studio, published, run by a worker that fails, compensated, and its journal and trace id read from the page |
| SC-007-002 100% of US01..US04 pass in a reproducible automated suite | Proven | [Studio CI](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38156597169/job/114520746513) on `ace4060`: all 19 browser journeys passed, including real-sandbox US02; two opt-in benchmarks skipped and passed separately locally. The local partial run passed 18 journeys and excluded US02 explicitly. |
| SC-007-003 no violation of the Constitution's gates and no silent loss of persisted state | Partial | fmt, clippy, the architecture check, `cargo audit` and the workspace tests pass; only principle II has an automated gate |
| SC-007-004 critical errors expose `code`, `trace_id` and remediation without leaking | Done | `ErrorState` shows the platform's message, what to do, the code and the trace id (`ErrorState.test.tsx`, journey SEC); the journal and the graph carry no payload |
| SC-007-005 tokens, contrast and human comparison with the prototype | Proven within the recorded scope | Token/literal-colour tests, axe in light/dark/forced colours, 320px reflow and system-font fallback; [Studio CI](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38156597169/job/114520746513) passed 1 034 unit tests and 19 journeys. Marcelo explicitly approved the visual proposal on 2026-10-11. Ten captures and the reference are linked from `design/README.md`. |

## Evidence: 008 observability-operations

Gaps and decisions: `specs/008-observability-operations/research.md` (D160–D182 and G169–G192). Validation flow, the reviews of the implementation and what could not be run: `specs/008-observability-operations/quickstart.md`. Design: [ADR-0017](adr/ADR-0017-telemetry-is-derived-from-the-journal-and-conditions-are-found-by-looking.md).

| Requirement | State | Evidence |
|---|---|---|
| FR-008-001 OTel traces, metrics and structured logs with IDs correlated by Run/Step/Attempt | Done, OTLP/JSON only | one trace from the request through the scheduler and the worker to the provider (`e2e us02`, `trace_links.rs`: 6), spans derived from the journal with computed ids so a re-export is idempotent (`telemetry_derive.rs`: 17; `fault projection.rs`: 8), live spans, an own OTLP/HTTP exporter with retries and a bounded queue (`crates/telemetry/tests/otlp.rs`: 17), JSON logs that carry the trace and redact (`logs.rs`: 10, `redact.rs`: 9), the metrics the projector and the monitor make; **no exemplars (G173), OTLP over protobuf and gRPC refused (G169), the caller chooses whether its own request is sampled (G187)** |
| FR-008-002 paginated read models for timeline and DAG, with cursor, lag and projection state | Done | the timeline carries the cause, time and trace of every transition and the telemetry state `CURRENT`/`CATCHING_UP`/`BEHIND`/`NO_TRACE`/`UNTRACKED` (`e2e us01`: 5), the attempts of a run (read in batches: `telemetry_derive.rs`), cursor pages that say when there is more (`us04`), the graph's own lag stays 0 because its read models are transactional; the Studio shows cause, span and the note (`RunDetailPage.test.tsx`) |
| FR-008-003 dashboards for latency, queue lag, leases, DLQ, retry, compensation and quotas | Done, quotas as refusals | a Grafana dashboard of 34 panels and 17 Prometheus rules (`infra/observability/`), checked by cargo tests (every metric a panel draws or a rule evaluates is one the code emits; every rule points to a runbook section; no label that grows with the data: `contract_observability_operations`: 9) and by the `observability` job of CI (`promtool`, the Collector's `validate`); **the quota panel draws refusals, not usage against the limit (G180)** |
| FR-008-004 operational actions: pause, resume, cancel, retry, rerun-as-new, manual review and drain workers | Done | retry now of a step waiting for its retry, hold for review, rerun as new from an ended run, drain and undrain a worker identity, each audited with a justification (`e2e us04`: 9; `operator_actions.rs`: 10; `fault operators.rs`: 4; `integration isolation.rs`: the drain waits for the polls that looked). Existing run operations are in Studio; the dedicated Operations menu exposes worker sessions and persistent drain/resume controls; T008-036 has component, permission/context and real Chromium/API drain/resume evidence. A rerun uses the same version (G179). |
| FR-008-005 alerts and runbooks for DB, NATS, worker and compensation failures | Done for platform and namespace conditions | five conditions the engine finds and keeps as alerts, announced by webhook in the transaction that records them (`e2e us03`: 5; `fault monitor.rs`: 6; `integration monitor.rs`: 5, the bounds and the fairness between tenants), rules for the engine being down, leases, the outbox, queue lag, dead items, the telemetry pipeline, API errors and latency, the pool, compensation and quotas, and a runbook section for each (`docs/runbooks/alerts.md`, `observability.md`); **no pager or email (G175), thresholds are global (G174), the monitor's cost grows with the failure it detects (G186, G188)** |
| FR-008-006 redaction and access controls on logs and payloads, with auditing | Done | redaction by name and by the shape of a credential, newline and reserved-key forging prevented (`logs.rs`, `redact.rs`); `ops:read`, `worker:manage` and `audit:read` on the new operations (`identity.rs` matrix; the cross-tenant battery of 009 attacks every one); the audit read scrubs credential-shaped reasons, records that it was read and withholds the denials that would be an existence oracle (`us04`); **redaction by heuristic is not a proof (G170); no retention or export of the trail (G182)** |
| NFR-008-SEC tenant isolation, RBAC, least privilege, no secret in logs or the journal | Done for what exists | the isolation tests of every read and write added to the store (`integration isolation.rs`), the cross-tenant battery, a security review and a correctness review by independent agents (quickstart: 16 findings, each real one fixed failing-first; the unfixed are G186–G188 and G190–G192) |
| NFR-008-DUR after commit, losing a worker or the broker loses no transition | Done | the projector records progress only after the collector took the spans and re-exports with the same ids after a crash; a collector that is down, refuses credentials or answers late loses nothing (`fault projection.rs`, `integration projection.rs`); a note ahead of the journal is settled; the migrations go back and forth with data in the tables (`integration migration.rs`) |
| NFR-008-OBS structured logs, correlated traces, essential metrics, auditability per Run/Step | Done | as FR-008-001/002/006; audit actions `run.marked_for_review`, `run.rerun`, `audit.read`, `worker.drained`, `worker.undrained` |
| NFR-008-COMP versioned protocol and schemas, do not break runs in progress | Done | every change to the OpenAPI and AsyncAPI is additive (`x-extended-again: '008'`, the baseline regenerated), a run from before the migration reads as `UNTRACKED` and keeps working, the SDKs ignore the new field of the error envelope |
| NFR-008-PERF the projection must not block the write path; a documented and flagged lag budget | Done as a baseline | `tests/performance/reports/observability-operations.md`: the note a writer pays is about 4% of a completion, a busy projector slows the median completion by 1.04x, a backlog is projected at over a thousand runs a second and the lag under sustained writing stays at the resolution of the sample against a budget of 30 s, a monitor round at 5 000 live runs takes ~170 ms with everything firing; one machine, PostgreSQL 18 with commits that do not ask the drive to flush; **hypotheses, not promises** |
| SC-008-001 an operator diagnoses a failure in fewer than N interactions during a UX test | **Not done** | no UX test with people was run; the timeline shows cause, trace and attempts in one view |
| SC-008-002 100% of US01..US04 pass in a reproducible automated suite | Done | `e2e_observability_operations` (26), `fault_…` (18), `integration_…` (20), `contract_…` (11) |
| SC-008-003 no violation of the Constitution's gates and no silent loss of persisted state | Partial | The historical implementation run passed fmt, Clippy, architecture and workspace checks as recorded in the quickstart, and Constitution v1.0.0 was subsequently ratified on 2026-10-11 (superseding the old G1 “missing constitution” statement). Only principle II has a dedicated automated architecture gate; the other principles depend on the named suites and review. |
| SC-008-004 critical errors expose `code`, `trace_id` and remediation without leaking | Done | `remediation` in the envelope from a closed table by code, shown by the Studio (`ErrorState.test.tsx`, `errors.test.ts`), and the same battery that checks that nothing leaks |

## Evidence: 011 gitops-versioning

Gaps and decisions: `specs/011-gitops-versioning/research.md` (D183–D205 and G193–G214). Validation flow, the reviews of the implementation and what could not be run: `specs/011-gitops-versioning/quickstart.md`. Design: [ADR-0018](adr/ADR-0018-publishing-is-one-transaction-the-pointer-has-a-history-and-an-approval-is-of-a-document.md). How to use it: [`runbooks/gitops.md`](runbooks/gitops.md).

| Requirement | State | Evidence |
|---|---|---|
| FR-011-001 WorkflowDraft, ImmutableWorkflowVersion, immutable ScriptVersion and dependency digests | Done | drafts and immutable versions are feature 002's, script artifacts feature 005's; this feature records the artifacts each version pins and makes the database refuse to remove one (`workflow_version_scripts`, `ON DELETE RESTRICT`; `integration artifacts.rs`: 3), exposes them on a single version (`scripts`, `e2e us03`), and fixes the digest and the document of a version at publication (`e2e us01`, `us04`: byte for byte as they were) |
| FR-011-002 a declarative Git-friendly manifest and a CLI for validate, diff, export, publish, promote and rollback | Done, not packaged | `apps/cli` (`sagat`: also history, requests, approve, reject) and the manifest with a published JSON Schema held to its reader by a corpus (`contract_manifest_gitops_versioning`: 3; `apps/cli/tests`: 18); every command through the real API in `e2e_gitops_versioning` (15); convergent and safe to run again (`fault cli.rs`: 4); the tool validates and compares with the server's own codec; **no scripts in the manifest (G199), no release of the binary (G203), bearer tokens only (G204)** |
| FR-011-003 optimistic concurrency on drafts and approval policies for prod publish | Done | the draft's revision (feature 002) and a per-namespace policy, one approval in production by default (`integration approvals.rs`: 10; `crates/domain/tests/versioning.rs`: 12; `e2e cli.rs::publishing_a_manifest_into_a_namespace_that_asks_for_approvals_waits_and_then_converges`); neither requester nor recorded draft editor decides, a person decides once, an approval is of a document and is counted again when used (`fault requests.rs`: 5) |
| FR-011-004 versioned routing by namespace and event trigger with compatibility rules | Done for promotion and activation | promotion across namespaces of the same environment or a later one (`integration promotion.rs`: 8; `e2e us03`); the event triggers follow the pointer with ids that do not change (`integration activation.rs`, `fault events.rs`: 2); the compatibility rules are the breaking-change findings the Studio diff shows, enforced on activation and promotion (`activation.rs`, `promotion.rs`); **a direct publication is not gated by them on the server (G195)** |
| FR-011-005 an audit trail of authorship, review, approval, publisher, digest and reason | Done | requests (author, digest, revision), decisions (who, verdict, reason), the history of the pointer (who, why, the request, where a promotion came from) that the database refuses to change (`integration isolation.rs`), and audit entries for each (`publication.*`, `workflow.activated`); read back by `sagat history` and `sagat requests` (`e2e us03`, `us04`) |
| FR-011-006 block removal of artifacts used by active runs until a safe retention policy is in place | Partial | the block is built and stricter than asked (an artifact pinned by **any** version cannot be removed, because a rollback can make it active: `integration artifacts.rs`); **the retention policy that would let artifacts go is not built (G193)** |
| NFR-011-SEC tenant isolation, RBAC, least privilege, no secret in logs or the journal | Done for what exists | every new operation is in the cross-tenant battery (`contract_rest_security_multitenancy`), composite foreign keys the database enforces (`integration isolation.rs`: 6), `publication:approve`, `publication:manage` and `version:activate` in the matrix (`identity.rs`), nothing but closed sets in the metrics (`e2e telemetry.rs`), the tool never prints or sends its token elsewhere and confines the files it reads (`apps/cli/tests`, `fault cli.rs`); two independent read-only reviews (a security and a correctness one: about thirty points, none critical or high, each real one fixed with a test, the rest in G209–G214) and a code and test review, recorded in the quickstart (not a penetration test) |
| NFR-011-DUR after commit, losing a worker or the broker loses no transition | Done | the pointer, its history, the version, the triggers and the audit entry commit together (`integration activation.rs`); under three seconds of every connection being terminated the history stays one chain with one row per version (`fault storm.rs`); an order defect found by the fault suite was fixed failing-first (D198) |
| NFR-011-OBS structured logs, correlated traces, essential metrics, auditability | Done | `publication_refusals_total`, `publication_events_total`, `version_activations_total`, `promotions_total` with closed labels (`e2e telemetry.rs`); every request is already a span with a trace id that the error envelope returns; audit as FR-011-005 |
| NFR-011-COMP versioned protocol and schemas, do not break runs in progress | Done | additive OpenAPI (a tag, 12 operations, `scripts` on a version), the Studio's types regenerated and checked, the migration back and forth with data (`integration migration.rs`: 2); a run keeps the version it started on whatever moves (`e2e us02`, `us04`) |
| NFR-011-PERF publishing asynchronous in builds; the pointer switch is transactional | Done as a baseline | `tests/performance/reports/gitops-versioning.md`: a 1000-node version publishes in ~6 ms, a move of the pointer in ~1 ms, 1.3x slower run starts while it moves, 50 workflows published by `sagat` in ~0.1 s on the loopback; the machine was shared and the figures are the optimistic ones of a local database |
| SC-011-001 V2→V3 with a suspended V2 run resumes correctly; rollback only affects new runs | Done | `e2e us02` and `us04` |
| SC-011-002 100% of US01..US04 pass in a reproducible automated suite | Done | `e2e_gitops_versioning` (15), `fault_…` (15), `integration_…` (35), `contract_rest_…` (1), `contract_manifest_…` (3), `sagat-cli` (18) |
| SC-011-003 no violation of the Constitution's gates and no silent loss of persisted state | Partial | the constitution exists and the architecture check passes (20 crates, 8 rule tests); 74 focused tests pass against PostgreSQL 18.6. Docker container creation fails because its metadata store is read-only: container-dependent workspace tests and Compose validation are not proven locally; see the quickstart for gate results |
| SC-011-004 critical errors expose `code`, `trace_id` and remediation without leaking | Done | the new refusals carry a `details.reason` and a remediation from a closed table (`error.rs`), shown by `sagat` in the platform's words, and the same battery that checks that nothing leaks |

### Publication recovery regression (2026-10-11)

Full execution record: [`validation-2026-10-11.md`](../specs/011-gitops-versioning/validation-2026-10-11.md).
The focused suites passed 74 tests. The workspace run passed 1,626, failed 63 in six container-dependent targets
and left 26 existing opt-in tests ignored; SC-011-003 remains partial.

FR-011-005 / NFR-011-DUR: a committed publication remains recoverable with the original idempotency key after
saving a new, invalid draft. The fault test
`requests::a_retry_storm_with_one_key_gets_one_answer_and_later_the_same_one` failed with `SUPERSEDED` before
the fix and passed afterwards. It asserts one version, one activation, a final `PUBLISHED` request, the original
version on replay and refusal of a different key. The REST conformance test checks `created=false`, the same
version identifier, and continued tenant/permission enforcement. Contract, generated Studio types, spec,
data model and GitOps runbook describe this behavior.

### 007 visual direction — implementation and validation (2026-10-11)

FR-007-009/010 and T007-039..043/046: shared design tokens, locally licensed fonts, shell, Runs master/detail, compact graph and
derived editor/Script/sign-in screens. Latest unit result: **1 033 tests passed in 64 files**; lint/typecheck/build/API/advisory
checks and Rust fmt/clippy/architecture passed locally. Review fixed missing graph tokens, faint omitted edges, master-list state
reset, unknown terminal duration and selected-step reflow. Both restyled benchmarks passed with all six floors met in three cold
rounds each. All 18 executed final browser journeys passed, including three-theme accessibility, 320px reflow and system-font
fallback; two opt-in benchmarks were skipped in that suite and passed separately. Human visual acceptance was recorded on 2026-10-11; ten review
PNGs are linked from the design record. Exact scope is recorded in `specs/007-workflow-studio/quickstart.md`. The explicit
no-sandbox development mode excluded US02; the subsequent Studio CI job on `ace4060` passed all 19 journeys with real Docker, closing T007-044 (link in the evidence table).

Owner review recorded on 2026-10-11: Marcelo explicitly approved the visual proposal ("Aprovo a proposta visual")
after opening the local Studio. This approval covers Runs, the light theme, editor, Scripts and sign-in;
automated CI evidence for this branch is recorded separately.

## Evidence: 012 developer-experience

Gaps and decisions: `specs/012-developer-experience/research.md` (D206–D228 and G215–G238). Validation flow, the reviews of the implementation and what could not be run: `specs/012-developer-experience/quickstart.md`. Design: [ADR-0019](adr/ADR-0019-the-first-day-runs-on-the-real-engine-with-canned-answers-and-a-development-identity-provider.md). How to use it: [`quickstarts/README.md`](quickstarts/README.md); how to run and check it: [`runbooks/developer-experience.md`](runbooks/developer-experience.md).

| Requirement or criterion | State | Evidence |
|---|---|---|
| FR-012-001 the CLI: `workflow validate\|publish\|run\|inspect\|cancel`, `script test\|publish`, `worker dev` | Done, not packaged | `apps/cli` (`sagat`; also `new`, `worker bind`, `dev token`, `doctor`); every command through the real API in `e2e_developer_experience` (31 + 4 ignored); the script commands against the production sandbox runner with a stand-in for the container engine (`e2e scripts.rs`: 3; with the real engine in one test CI runs); `apps/cli/tests` (56); **no release of the binary (G216)** |
| FR-012-002 quickstarts for TS, Go, Python, C# and Java and a saga business example | Done | `docs/quickstarts/` (7 pages), `examples/hello-world/{typescript,go,python,dotnet,java}` run against a real platform by `e2e us03` (same story in all five, the digest of every output, a trace on every event), `examples/payment-saga/typescript` (`e2e saga`: a good day, a decline compensated, a lost answer reconciled with one charge); the SDKs are used from the checkout, not published (G216), and **the saga is in one language (G226)** |
| FR-012-003 compose with services and reproducible fixtures; an optional local emulator consistent with the real semantics | Done | the stack gains the development identity provider and serves the Studio, the seed leaves a namespace (`e2e dev_environment`: 7); **the emulator is the real engine served by `worker dev` (D207): nothing to diverge**; GitHub CI run [38155640785](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38155640785) at `ae3c23aa05f4423d5021dc6a5c042bc8d523fd8c` built and started the actual images, ran the seed, CLI, worker, TypeScript SDK and Studio browser journey, and passed the real script sandbox |
| FR-012-004 templates for workflows, event trigger, cron, approval, compensation and inline script | Done | `templates/` (6), `apps/cli/tests/templates.rs`, `e2e stories.rs` (each publishes as the workflow its README names; the event, approval, cron and script templates run) |
| FR-012-005 API docs and contracts generated from the schemas; examples executable in CI | Done as a map of the contract | `docs/reference/{api,workflow-document,cli}.md` are what a regeneration gives; the `sagat` commands in the pages are parsed by the tool, the code excerpts are in their files, the scripts exist, the links lead somewhere, and no example, template or page carries a secret (`contract_developer_experience`: 8); **the OpenAPI file is not rendered anywhere (G224), and the checks parse commands rather than run them (G223)** |
| FR-012-006 onboarding instrumentation and diagnostics for environment, credentials and incompatible versions | Implemented, opt-in instrumentation | `sagat doctor` (address, token, reachability, readiness, credentials, versions via `GET /v1/version`, namespace, worker endpoint, Studio, broker; `e2e doctor.rs`: 7), the errors page and the table behind it (`apps/cli/tests/diagnostics.rs`), the benchmark; opt-in OTLP/HTTP JSON command spans, completion logs and duration points plus optional local NDJSON; closed labels, no token/payload, W3C REST/gRPC correlation (`apps/cli/tests/telemetry.rs`; G238) |
| NFR-012-SEC tenant isolation, RBAC, least privilege, no secret in logs or the journal | Done for what exists | the token never in an argument, an output or a request to anywhere but the address it was given; the worker endpoint held to the same rule; the terminal filter; bindings of names only; canned answers only to a dev namespace (`fault_developer_experience`: 14; `apps/cli/tests`); the development provider's refusals (`apps/dev-idp/tests`: 15); two independent read-only reviews (security: none critical or high, about a dozen real findings, each fixed with a test; correctness: 21, the same) recorded in the quickstart (not a penetration test) |
| NFR-012-DUR after commit, losing a worker or the broker loses no transition | Done | a worker that stops and starts again loses nothing and compose does not run twice (`fault a_worker_that_stops_and_starts_again_…`); a stop at every point of the loop leaves the task to its lease; the platform's own guarantees are the earlier features' |
| NFR-012-OBS structured logs, correlated traces, essential metrics, auditability | Implemented, opt-in export | every run a tool starts has a trace the tool shows, and `e2e us03` requires every event of every language's run to be in it; the seed and the bindings are audited by the platform; the CLI emits opt-in completion logs and duration gauge points with bounded labels, correlated to command spans (G238) |
| NFR-012-COMP versioned protocol and schemas, do not break runs in progress | Done | `GET /v1/version` is additive (OpenAPI, the Studio's generated types, the REST conformance suite); no migration; the tool offers worker protocol 1.2 and `doctor` compares ranges |
| NFR-012-PERF onboarding under fifteen minutes with the dependencies installed, in UX evaluation | **Not evaluated with people** | `tests/performance/reports/developer-experience.md`: the machine's part of the first day is under a second (six commands, `worker dev` start to `SUCCEEDED` included) and each SDK's example builds in 1–6 s from a clean build with warm package caches and runs in 0.6–1.4 s; images and downloads are not in it (G215, G227) |
| SC-012-001 a newcomer runs the first runs in TS and C# from the documentation alone; CLI tests cover invalid workflows | **Not done** (the second half is) | no person was asked (G215); invalid workflows: `e2e diagnostics.rs`, `apps/cli/tests/diagnostics.rs` |
| SC-012-002 100% of US01..US04 pass in a reproducible automated suite | Done | `e2e_developer_experience` (31 plus the SDK, saga, sandbox and browser opt-in scenarios), `fault_…` (14), and `contract_…` (8); CI run [38155640785](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38155640785) passed the five-language conformance, user stories, faults and examples, payment saga, real script sandbox, native onboarding browser and Compose onboarding browser |
| SC-012-003 no violation of the Constitution's gates and no silent loss of persisted state | Done | CI run [38155640785](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38155640785) at `ae3c23aa05f4423d5021dc6a5c042bc8d523fd8c` passed formatting, full-workspace Clippy and tests, architecture rules, contracts, observability validation, dependency audit, all container-backed suites, Compose, Studio and both onboarding browser paths |
| SC-012-004 critical errors expose `code`, `trace_id` and remediation without leaking | Done | the tool shows the platform's code, reason, message and remediation (`workflow`, `script`, `worker` paths), every example prints `code (trace …): remediation` and never the payload or the token (`e2e us03`, `saga`), and the generic findings have help from a closed table |

## Evidence: 010 scalability-ha

Gaps and decisions: `specs/010-scalability-ha/research.md` (D229–D260 and G239–G270). Validation flow, the two rounds of reviews and what could not be run: `specs/010-scalability-ha/quickstart.md`. Design: [ADR-0020](adr/ADR-0020-the-scheduler-is-sharded-by-run-and-owned-by-lease-with-a-fence.md). How to run it: [`runbooks/scaling.md`](runbooks/scaling.md) and [`runbooks/disaster-recovery.md`](runbooks/disaster-recovery.md). Measurements: [`tests/performance/reports/scalability-ha.md`](../tests/performance/reports/scalability-ha.md) and [`tests/performance/reports/disaster-recovery.md`](../tests/performance/reports/disaster-recovery.md), both of one machine on one day.

| Requirement or criterion | State | Evidence |
|---|---|---|
| FR-010-001 partition ownership with lease and fencing for the scheduler | Done (by run, not by namespace) | 64 shards by the run's identifier, a lease row with a generation on the database's clock, a fence (`FOR SHARE` of the shard row, owner + generation + expiry) in every action's transaction, takeovers that never wait; `crates/domain/tests/sharding.rs` (7), `crates/scheduler` unit tests of the keeper (31 with fairness), `integration_scalability_ha` (44: leases, the fence, takeover against an action in flight and against an orphan, failover, every pass divided between two replicas), `fault_failover` (6: connections killed over and over, a replica cut off, killed mid-pass, held past its lease). Mutation checks: with a takeover that waits, a renewal that waits, a third of the lease as interval, the tests fail |
| FR-010-002 quotas per tenant/queue; weighted fairness or a measurable policy | Done as a measurable policy with exact per-activity concurrency; no generic pool/queue quota | weighted max-min fairness inside a pass, weights 1–16 set by the operator (`control-plane scheduling-weight`, audited), remains proven by `fairness.rs`, integration tests and the recorded benchmark. T010-039/040 add tenant/namespace/activity concurrency admission in the same claim transaction: a 12-poller race, busy-activity skip, expired-lease recovery, tenant isolation and ceiling/default fencing pass against real PostgreSQL; five lock-order/edit-race regressions cover ABBA prevention and policy acknowledgement. Management REST conformance passed 7/7. Forward, compensation and reconciliation work counts by advertised activity. A separate generic pool/queue quota is not built. |
| FR-010-003 instrument and optimise journal queries, timers, outbox and projections | Partial | timers, retries, leases, deadlines and the outbox are shard-first partial indexes bounded by `now()` (an idle tick costs 0.8 ms at a million waits, its plan is in the report), the outbox is divided and claimed by id, the quota of active runs is read from 16 counters instead of counted (a start no longer costs more with the waiting runs); the journal is not partitioned (G241) and the projections are not divided |
| FR-010-004 worker pools by language/capabilities and autoscaling by queue lag | Implemented; one-node live KEDA deployment validated | The [disposable kind/Cilium product deployment](../tests/performance/reports/kubernetes-validation.md) ran migrations, authenticated the API, enqueued 60 real runs and drained all 60 successfully. Real queue metrics drove workers 1→3→1; engine pod replacement, restricted admission and negative NetworkPolicy checks passed. This supersedes the older quickstart/research statement that only a policy subset had run. The report records exact tested image IDs from an uncommitted feature snapshot, so release acceptance still requires rebuilding and smoke-running final artifacts. The one-node validation does not prove node drain, cross-node rescheduling, disruption-budget behavior or multi-zone recovery. |
| FR-010-005 chaos and fault tests for DB, NATS, worker and shard failover with reports | Done | `fault_failover` (6, about 25 s), `fault_scalability_ha` (5: replicas of different versions, a deadline racing a cancel, the runtime role's privileges, what the keeper logs), `e2e_scalability_ha` (6: the four stories against real engine processes, one killed like a process) |
| FR-010-006 load harness, p50/p95/p99 and a hardware profile for 10k/100k/1M waits and 100/1000/5000 transitions per second | Partial: 5 000 not reached | `tests/performance/scalability-ha/` and `run.sh`; 10k, 100k and a million waits (≈1,060 starts/s steady, idle tick 0.8 ms, 2.8 KB per wait); 100 and 1,000 transitions/s reached (best 1,513/s), 5,000 **not reached** and not tried under conditions that could (the harness's ceiling, G270) |
| FR-010-007 backup, PITR, restore drills, RPO/RTO and a DR plan | Done for single-region HA on one machine | `infra/dr/drill.sh`, `infra/dr/journal_digest.sql`, `docs/runbooks/disaster-recovery.md`; restored to a named point in 0.9 s with the digest of the journal and of the audit trail equal and 6,000 runs replayed; `kill -9` failover in 0.1–0.2 s with no acknowledged run lost on a synchronous or an asynchronous standby (an asynchronous standby on a loopback is never behind: its RPO is not shown). Reference objectives decided 2026-10-16 (RPO 0, failover ≤ 5 min, PITR ≤ 1 h for the synchronous-standby topology); the drill enforces them and gates the `release` workflow; they are not yet measured on a real cluster |
| NFR-010-SEC tenant isolation, RBAC, least privilege, no secret in logs or the journal | Done for what exists | two rounds of security review; metrics carry no tenant, run or activity; the weights are the operator's; the counters' function cannot be hijacked (`a_temporary_table_cannot_stand_in_for_the_counters_or_borrow_the_function`); role-specific ingress and dependency-scoped egress policies passed render-time tests and the live Cilium verifier. G259 and G265 remain; G266 is resolved. |
| NFR-010-DUR after commit, losing a worker or the broker loses no transition; lease and fencing where applicable | Done | The fence, `fault_failover` and the passes' idempotence under the run's lock; the outbox claim is not fenced (G262) and a message may be published twice, never lost. `apps/execution-engine/src/supervision.rs` closes G269: any critical loop/server exit fails process health, stops its siblings and returns exit 1, while intentional shutdown drains for at most 30 seconds. Focused agent-run evidence reports all 32 engine tests and package Clippy passing; the cited CI run predates this change. |
| NFR-010-OBS logs, traces, metrics, auditability | Done | `scheduler_*`, `pool_*`, a span per keeper round, alerts and dashboard panels (`infra/observability`), the runbook anchors |
| NFR-010-COMP versioned protocol and schemas; do not break runs in progress | Done | two additive migrations with tested rollbacks, engines of the previous version keep working during an upgrade (`fault_scalability_ha`) |
| NFR-010-PERF quantifiable and reproducible; distinguish wait capacity, throughput and latency under backlog | Done | the report keeps the three apart, records hardware, configuration and load, and says where a level was not reached |
| SC-010-001 performance reports with hardware and configuration, zero confirmed run loss in fault injection and a validated DR restore | Done on one machine; the 5 000/s level not reached | the two reports above; `fault_failover` and the drill fail when a run is lost |
| SC-010-002 100% of US01..US04 pass in a reproducible automated suite | Done | `e2e_scalability_ha` (6) |
| SC-010-003 no violation of the Constitution's gates and no silent loss of persisted state | Done for the automated gates | Release CI run [38189200062](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38189200062) at `20793c2` (PR #20) passed all nine checks, including PostgreSQL 16 container suites, architecture, contracts, observability, dependency audit, compose, SDKs, Studio, Helm, the load harness and the DR drill. The later completion [CI run 38192086606](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38192086606) on `65841f33cd5b1af462ba4b6a41e92b918939c63d` passed formatting, full-workspace Clippy/tests, architecture, contracts, observability, dependency audit, the small scale harness, DR drill and Helm lint. The live Cilium policy verifier is separately recorded evidence; CI syntax-checks that script but does not create its cluster. |
| SC-010-004 critical errors expose `code`, `trace_id` and remediation without leaking | Done for what exists | no new API operation; the operator command and the engine's configuration name the variable and never echo a value |

## Evidence: 016 payload-classification

Decision: [ADR-0033](adr/ADR-0033-payload-values-are-classified-per-field.md); combined dependency
policy: [ADR-0036](adr/ADR-0036-integrated-scoped-managed-and-classified-execution.md). Implementation
is in progress. [Integration notes](../tasks/project-completion/integration-notes.md) name the actual
commands and focused results; unchecked canonical tasks require their full acceptance criteria.

| Requirement or criterion | State | Evidence and remaining gate |
|---|---|---|
| FR-016-001..005 authoring and immutable pinning | Focused gates passed | Pure domain, publication/ref security, all corpus runtime shapes and generated monotonicity; real PostgreSQL legacy, ordinary/approved publish and promotion pinning |
| FR-016-006..009, 013..015 disclosure and exact carrier selection | Registered core gate passed | 10 application tests, 96-cell REST matrix, retained run result and exact scoped journal carriers; actual script/blob/child/rerun/retry outputs |
| FR-016-010..012 permissions, grants and audit | Focused fault gates passed | 48-permission catalog, grant fences/cap/cascade, deferred audit failures and actual process death before/after commit with lost-response retry; concurrent publication/activation pinning case passed |
| FR-016-016..017 unchanged surfaces and value-free telemetry | Focused byte and canary gates passed | 22 key/value classes across API/CLI and captured logs, OTLP, metrics and audit; ten actual HTTP surfaces matched immutable pre-classification response bytes (`compat::unchanged_surfaces_match_actual_pre_classification_response_bytes`); broader combined workspace gate remains |
| FR-016-018..019 Studio and CLI | Focused browser and real CLI passed | 1149 Studio tests, typecheck/lint/build, actual four-role Chromium disclosure journey and actual CLI grant/disclose/revoke journey; broader release matrix remains |
| FR-016-020 five SDK builders | Focused builders and canonical scenario passed | All five emitted the same 1,474-byte carrier document; full integrated runtime protocol matrix remains pending |
| FR-016-021..025 compatibility, errors, boundaries, bounds and diff | Named compatibility and boundary gates passed | Actual v0.1.0 control plane/engine, archived Studio in Chromium and five released SDK sources passed named cases; legacy extension regression reproduced before the fix and passed afterward with current-process pause/resume, immutable bytes/pins and legacy opacity. Frozen contract checksum, 87 gRPC/security cases, numeric guards/diff and bounded benchmark passed. See the [compatibility report](../tests/integration/payload-classification/compatibility-report.md); complete current SDK/workspace gates remain separate |
| FR-016-026..029 event and dead-letter values | Actual HTTP canary core passed | Six roles × grant × reveal across three reads; captured event telemetry/audit, empty-page authority, default structure and mandatory reveal audit; original event suites and full tenant release matrix remain |
| NFR-016-001..006 and SC-016-001..010 | Complete acceptance not yet established | Measured 720 disclosure projections, actual byte compatibility, prior-release processes/browser/SDKs, complete named API/CLI audience matrix and executable quickstart passed on recorded checkpoints. The validator cache charges schema weight, not actual compiled heap. Complete current SDK/workspace and broader Studio/Compose release gates remain |


## Completion follow-up — 2026-10-12

T007-054 closes incomplete namespace listing: all cursor pages are loaded, with no partial success
after failure or cancellation (three focused tests). T007-056 closes G167: authorized, explicit
signal confirmation, JSON/UTF-8 size validation and stable retries; nine component tests and one
real Chromium/API/engine run passed, including the persisted `wait.satisfied` event. T007-055 trace
links have the runtime serving/configuration and browser evidence above. Local integrated gates and corrected fixture evidence are recorded in `tasks/todo.md`;
PR #22 records the final Linux CI result. ADR-0029/T006-040..042 define the implemented finite-credit task transport. Server contracts,
concurrency, authority, cancellation and five-language conformance pass. The 30-row release
comparison is recorded; final packages from `517c657` pass all five clean consumers. The named `stream_telemetry` contract also proves
bounded metric labels, open-stream cleanup, trace propagation and absence of secret canaries in logs.

T008-036 closes G181 with the Operations menu: namespace alerts, live sessions and persistent identity drains, explicit audited reason, permission boundaries and late-response protection. The isolated real Chromium/API drain/resume journey passed on 2026-10-12.

Final local checkpoint (2026-10-13): full Rust execution recorded 2,253 passes and one reproduced
observability-fixture failure; the corrected affected target passed 26/26 at two-thread and default
parallelism. Namespace-dialog fixture isolation passed all six focused and 1,105 Studio tests.
Full-workspace Clippy, integrated 0.4.0 builds, readiness probes and four real Chromium journeys pass.
Neither the initial failed runs nor the external publication/human-acceptance gates are marked green.

## 014 — Workflow primitives and expanded SDK examples (updated 2026-10-17)

[Specification](../specs/014-workflow-primitives/spec.md),
[implementation tasks](../specs/014-workflow-primitives/tasks.md), and
[local validation evidence](../specs/014-workflow-primitives/validation-report.md). Decisions and
gaps (D/G/O numbers below) are in the feature's [research.md](../specs/014-workflow-primitives/research.md);
the SDK version and capability matrix is [sdk-compatibility.md](sdk-compatibility.md).

| Requirement / criterion | Status | Evidence / remaining work |
|---|---|---|
| FR-014-001, FR-014-002, SC-014-001 — practical examples and parity | Implemented and locally verified | `examples/order-processing/`: three workflows in all five SDK languages. `e2e_developer_experience` filter `order_processing::` passed 25 workflow runs and 10 configuration checks on a real disposable platform. The common CI example job includes this filter. |
| FR-014-003, FR-014-009, FR-014-014 — additive v1 identity and bounded switch | Switch implemented and locally verified; broader identity scope partial | The v1 schema, typed root/scoped identity and compatible PostgreSQL migration are implemented. A switch choice is journaled with step projection and outbox in one transaction; PostgreSQL fault tests cover replay, recovery and rollback. WP-07 uses one reviewed scenario across all five SDKs. Restricted scoped collection and loop execution now has public worker 1.5 evidence; broader bodies remain gated. See the [validation report](../specs/014-workflow-primitives/validation-report.md). |
| FR-014-010–011 — declared Query/Update | Run-addressed subset implemented and locally verified | Public v1 publication accepts bounded declared state with existing basic nodes. Real PostgreSQL and HTTP tests cover revision, replay, authorization, audit, rollback and a response lost after commit; WP-08/09 runs in all five SDKs. Chain Query/Update have focused HTTP/PostgreSQL evidence, including head fencing and replay; a direct public continuation transfers declared state. The `chain-query-handoff` scenario passes successor Query, Update, exact replay and stale expected-head Query in all five SDKs. Timer race, secret-manager capability canaries, broader handoff faults and performance limits remain open. |
| FR-014-004–008, FR-014-012–013, FR-014-015–017 — collections, loops, approval, continuation and cross-cutting rules | Partially implemented; public runtime incomplete | A bounded root approval can be published, started, listed and decided through the public API; PostgreSQL integration/fault tests and the five-language `approval-decisions` runtime scenario cover its available slice. Approval combined with parallel execution, declared state or another structured primitive remains disabled. A restricted continuation graph can create a chain publicly and hand off with fenced PostgreSQL transactions; focused handoff and HTTP tests cover success/FINAL, CHAIN_LIMIT/FINAL, replay, tenant opacity and direct declared-state transfer. The five-language `chain-query-handoff` scenario distinguishes segment and chain waits. The five-language `chain-commands` scenario proves a signal-controlled wait after an entry activity, exact signal/cancel replay, stale-head fences, tenant opacity and final cancellation. A `waitEvent` entrypoint (the successor is created already waiting; five-language `chain-root-wait`), a parent cancelled mid-chain (`chain-cancel-parent`) and `RERUN_AS_NEW` (`chain-rerun`) are covered; bodies, triggers and wider graphs remain open. A restricted root collection chain with one-activity/return bodies can be published and executed through HTTP, worker 1.5 and the fenced scheduler after audited tenant capacity provisioning; public PostgreSQL tests cover two scopes, ordered checkpoints, terminal rollback/retry and replay. Broader collection bodies, nested scopes and compensation/reconciliation remain closed. A restricted public loop has PostgreSQL restart/rollback and five-SDK `loop-runtime` evidence; mapped collection results have source-bound transactional storage and five-SDK `collection-runtime` evidence. Broader loop graphs and the remaining failure matrix stay open. The separate IR revision was superseded by [ADR-0032](adr/ADR-0032-additive-workflow-ir.md); documents retain v1. |
| SC-014-002–006 and NFR-014-001–004 — primitive release guarantees | **Not fully verified** | The latest consolidated checkpoint passed 139 real-PostgreSQL integration tests, 26 contracts and 25 fault tests; a separate real HTTP/PostgreSQL E2E route scenario passed. The scheduler finalizes a deterministic continuation transfer failure with `EXPRESSION_ERROR`; fault tests verify rollback and retry after outbox failures, including signal-controlled handoff. All 36 active shared scenarios pass in five SDKs with explicitly pinned historical worker 1.4 fixtures and 1.5 primitive fixtures. The missing WP cases, full crash/race/tenant/upgrade suites and measured resource ceilings remain open. Exact commands and limits are in the validation report. |
| FR-014-017 — primitive telemetry and published hints | Query/Update and hint contract verified locally | Declared Query/Update emit the contracted `workflow.query`/`workflow.update` spans, `workflow_operation_duration_seconds` and `workflow_primitive_rejections_total` with closed labels; `observability.rs` also proves trace/request correlation and fail-closed audit. All 17 outbox hint types are in `events.asyncapi.yaml` 0.3.0 and `hint_contract.rs` validates the real rows of 11 scenarios. Runs finished by a loop (and by `finalize_chain`) now reach `run.finished` subscribers; both regressions failed first. See the [validation report](../specs/014-workflow-primitives/validation-report.md). |
| FR-014-017 — projection compatibility | Partial | The PostgreSQL graph/telemetry integration target covers scoped identities, switch routes and continuation links. A future journal event is skipped by the current decoder while its persisted sequence advances the telemetry watermark in one projector round; the real-PostgreSQL regression failed before the fix and passed after it. Studio unknown-node card tests passed 76/76. Explicit unsupported telemetry representation for future definition node kinds remains open. |


Classification completion checkpoint (2026-10-17, local integration): T016-049 now has the
768-REST/96-stream role/guarded-payload matrix, 96 actual CLI-process cases, expanded Chromium
identity/navigation/storage checks and ten exact old/new HTTP body pairs. T016-056 has complete
checked-in examples and an actual CLI/curl/HTTP/gRPC/PostgreSQL walkthrough (1/1, 21.00 s), plus
prerequisite, component and reference gates. Commands/manifests/limits are in
`tasks/project-completion/integration-notes.md`. Full workspace/advisory/Compose integration
(T016-054 / T014-043), remaining accepted feature-014 runtime and large-workload evidence remain
open; this checkpoint is not remote CI or deployment evidence.


### Integrated bounded execution evidence (feature 014)

FR-014-005/014/015: shared bounded/full effect and attempt policies are covered by
`crates/domain/tests/bounded_path_planning.rs` and `scoped_compensation.rs`; the real runtime-role
path writer is covered by `byte_ceilings::a_bounded_reconciliation_persists_its_companion_and_manual_review_status`.
Run status preserves other-path review/live facts, outputs and lookup hints are committed with
the journal, and full replay verifies persisted results. Existing event telemetry vocabulary is
reused; no payload/secret fields are added to logs or metric labels. Internal work-summary/index
contracts are in feature 014's data model and migrations 20261207000000/20261208000000.
Exact commands, initial failures, performance phase exclusions and unproven full worker/control/
root-finalization criteria remain recorded in `tasks/project-completion/integration-notes.md`.


### Focused scoped BlobOutput checkpoint — 2026-10-17

FR-014-004/005/014/015/016: ordinary forward iteration outputs may use the existing claim-check
blob protocol. `scoped_blob_completion.rs` traces exact identity/current authority, stable replay,
cancellation effect accounting, catalog/output/journal rollback and live quota recovery;
`collection-blob-output` and `blobs` provide all-five SDK result equality. Contracts:
`specs/014-workflow-primitives/contracts/conformance.md`; existing worker.scoped.report spans,
blob upload/read auditing and quota refusal counters remain. Reproduce happy/failure cases with
the commands in `docs/runbooks/workflow-primitives.md`. Exact runtime/static evidence is recorded
in `specs/014-workflow-primitives/validation-report.md` and its scoped-blob manifest. This is a
partial checkpoint; broader/nested bodies and complete feature/release tasks remain unchecked.

### Admission lock checkpoint — 2026-10-18

FR-014-012/014/015: `chains::grandchild_handoff_and_root_child_admission_do_not_deadlock_on_the_root_foreign_key`
and `chains::handoff_and_new_root_admission_do_not_invert_quota_and_payload_locks` exercise real
PostgreSQL tree/FK and active-quota/payload competition. The correction preserves transactional
journal/audit/outbox boundaries and existing scheduler failure telemetry. Contract details are
in feature014's spec/plan/data model; reproduction is in the workflow-primitives runbook.
The validation report and `tasks/project-completion/evidence/child-root-fk-20261018/` retain
failed fixtures separately from behavioral RED and subsequent GREEN. B0's complete lock graph,
nested capacity and canonical release tasks remain open.


### Public publication fixture checkpoint — 2026-10-18

FR-014-003/015/016, T014-004: the 47 native primitive contracts and 10 public HTTP draft/publication
rejections pass. Scope errors retain named-body provenance; ordinary root dangling edges retain
GRAPH. Unsupported object literals and 1024-byte effective expression failures use EXPRESSION,
including schema-valid multibyte strings above the legacy wrapper limit. Trace/header correlation
and independent no-version SQL assertions accompany every HTTP case; the byte-limit diagnostic
contains no expression content. The public reproduction command and operator remediation are in
`docs/runbooks/workflow-primitives.md`. Three bounded review findings and exact failures/source/
results are in `tasks/project-completion/evidence/publication-scope-20261018/manifest.json`.
Together with existing five-SDK instance-limit evidence and fresh ordered10,000-reference
materialization in the379-test runtime-role regression,
this closes the fixture task; broader runtime and shared release tasks remain unchecked.


### Child-chain settlement checkpoint — 2026-10-18

FR-014-012–015: parent cancellation/final notification updates its child-chain link without
mixing legacy child flag UPDATEs. Three real PostgreSQL trigger regressions failed before
correction and passed afterward; exact cancellation audits and one parent completion journal
event accompany consistency checks. Full fault 92 and durable 178 targets pass, including legacy
child policies and ordered cancellation/handoff races. The strict ADR metadata-guard question,
remaining B0 prefixes and canonical broader tasks stay open. Reproduction/remediation are in
`docs/runbooks/workflow-primitives.md`; exact evidence is in
`tasks/project-completion/evidence/child-link-only-20261018/manifest.json`.


## 2026-10-18 — Stable parent identity guard (ADR-0037)

The existing handoff keeps chain→parent KEY SHARE→own head→incoming link. ADR-0037 permits
this narrow identity protection under immutable initial/head/link ancestry; it grants no parent
business-state or mutation authority. Parent commands still require explicit own-run FOR UPDATE.
No executable production change, migration, SDK/API change or nested activation is introduced.

Removing only the early guard as a reversible negative control made the existing ordered race
fail 0/1: handoff committed but parent cancellation returned Unavailable. That is an observed
public error, not proof of PostgreSQL SQLSTATE/deadlock cause. Original source bytes were restored
exactly and the same two-order race passed 1/1 (3.64s). Added positive ancestry assertions passed
for three direct child segments/link (3.05s) and a depth-two handoff with distinct parent/root
(2.85s). Two independent scoped reviews resolved an unstated ancestry precondition, then found
no remaining concrete defects. This is a current-writer invariant, not a stronger FK guarantee.

Exact controls, hashes and review: tasks/project-completion/evidence/parent-fk-guard-20261018/.
The complete B0 graph, B1/group persistence, compatibility fencing and production resource/release
gates remain open. No canonical checkbox closes from this clarification.


### Exact-path storage prefix — 2026-10-18

FR-014-014/015, ADR-0030§18: begin_current_path/begin_path retain their applicable tenant chain
guard before the own-run lock. Four storage barrier regressions fail before and pass after;
eight bounded fence tests pass, with actual chain blocking, no premature run lock and guard
lifetime through commit. This is a trusted internal composition fixture, not new public nesting
or chain-finality support. Exact commands/source/review: tasks/project-completion/evidence/
path-chain-prefix-20261018/. Remaining B0 and canonical runtime/release tasks stay unchecked.


FR-014-014 / ADR-0030§18: direct run-addressed declared Update now takes its membership chain
before the exact run. HTTP successor barrier, transaction-lifetime probes, unchanged predecessor,
one receipt/event/outbox plus two audited apply/replay calls are recorded in
`tasks/project-completion/evidence/direct-update-chain-prefix-20261018/`. Existing terminal,
permissions and tenant HTTP regressions remain; no canonical task closes from this narrow fix.


FR-014-014/015, current scheduler authority: three unchanged-tuple barriers reproduced legacy
handoff, evaluated continuation and finalization committing after authority expired during a
row-lock wait. Shared transaction-typed post-lock validation makes all three reject with no
persisted effects; fresh authority/replay and generation-error compatibility pass (4/4). Private
chain fixtures now select execution_pool when runtime-role mode is requested. Historical owner
fixture results do not imply runtime privilege coverage from the environment flag alone. Exact
source/commands/reviews: tasks/project-completion/evidence/chain-post-wait-fence-20261018/.


### Collection/loop authority expiry during lock wait (2026-10-18)

Refs:T014-040, FR-014-014/015. The collection and loop adapters retain their scheduler SHARE
guard in the owning transaction and validate expiry using a database clock sampled after
acquiring it. Wrong-shard and generation-conversion errors keep their existing precedence.
A rejected entry changes no projection, payload, quota, timer, journal, outbox or audit. A supplied
collection entry audit is inserted once through finish_version, atomically with the transition.
Private runtime-role entry barriers reproduce the stale commits and verify rejection, fresh
authority and replay. This is not a continuous wall-clock-until-commit guarantee or acceptance
of nested scopes. Approval's separate check and B0 ordering remain pending. Existing fenced
metrics use kind=collection/loop without tenant/run labels; the port tests do not test counters.
Evidence: tasks/project-completion/evidence/scope-post-wait-fence-20261018/.


### Approval scheduler authority after lock waits (2026-10-18)

Refs:T014-040, FR-014-008/014. Open/expire retain a scheduler SHARE guard in their owning
transaction and validate authority against a database clock sampled after acquiring it. An
expired authority returns the existing typed fence error before request/timer/projection, journal,
outbox, audit or telemetry queue effects. Oversized-generation storage errors keep precedence;
decide uses its current-user authorization. No business-deadline clock or opening replay contract
changes. Real runtime-role barriers cover both opening and genuinely due expiry; the atomic
checkpoint includes the telemetry queue and approval tables. Existing fenced metrics use
approval_open/approval_expire kinds without tenant/run labels; these port tests do not assert
counters. This does not enable nested scopes or complete B0.
Evidence: tasks/project-completion/evidence/approval-post-wait-fence-20261018/.


### Approval member-chain writer guard (2026-10-18)

Refs:T014-040, FR-014-008/014. Approval open/expire/decide acquire a tenant/namespace-scoped
applicable membership chain before the exact own run, retaining both guards through commit.
The shared transaction helper is the existing declared Run Update guard; it never forwards or
grants head authority. Initial membership commits with new-run creation and later identity is
immutable; live reparent/migration requires its own protocol. Decision authorization, typed
errors, terminal/replay and audit/outbox boundaries remain unchanged. Native wait/final-audit
barriers verify ordering and guard lifetime; no new public composition or B0 completion is
inferred. Existing outcome metrics and sensitive-data-free logs keep their current mapping.
Evidence: tasks/project-completion/evidence/approval-chain-prefix-20261018/.


### Collection/loop member-chain serialization (2026-10-18)

Refs:T014-040, FR-014-004/006/014. Collection entry/drain and loop writer helpers retain an
applicable scoped membership chain before own-run UPDATE in their owning transaction. Existing
CAS, fences, quota/deadline/drain rules, typed errors, audit/outbox and telemetry mapping stay
unchanged. Native app-role barriers prove chain wait precedes own-run acquisition; fresh/replay
controls cover persisted writes. Shared guard lifetime is separately proven by the approval
final-audit barrier. These trusted storage fixtures grant no new public composition, head
authority or nested eligibility; complete B0/group and resource gates remain pending.
Evidence: tasks/project-completion/evidence/scope-chain-prefix-20261018/.


### Production incident lock prefix (ADR-0038,2026-10-18)

Refs:T014-040, FR-009-004/005/008, FR-014-014. Production attribution now takes applicable
member chain→exact own run→binding EXCLUSIVE→own step→audit deduplication, refining ADR-0022's
reproduced opposite report/binding edge. Current tenant/namespace/fence/expiry/binding checks,
redacted closed categories, deduplication and commit-before-ACK remain unchanged. No execution
state/outbox or new capability is introduced; test-run attribution is unchanged. Native runtime
barriers cover the actual report storage cycle (PostgreSQL deadlocks0→1 before correction) and
a genuinely published/leased script chain. Completion/journal consistency are positive controls.
Reproduce with runtime option and external DB unset:
`cargo test --locked -p sagat-tests --test integration_security_multitenancy incident_lock_order_`.
Existing script_incidents_total/log mappings remain after commit; port tests do not assert counters.
Exact source, review and regression limits: tasks/project-completion/evidence/incident-lock-order-20261018/.
Complete B0/group and release gates remain open.


## Worker session renewal freshness — 2026-10-18

Refs:T014-040/044, FR-009-001/003, FR-014-014. WorkerStore.use_session locks only the matching
tenant/id/token row, then samples database clock and renews in the same transaction if live.
Missing/expired sessions retain the existing unauthenticated mapping. No session expired during
an unchanged-row wait may extend its TTL or claim work. Existing TTL, caller-subject check and
wire remain unchanged; this sample is not continuous authority through commit. No run/group
lock or new API is added. Separate full/path claim-room and managed-database checks stay open.

Actual public-poll RED renewed an expired session and leased1 task (0/1,5.10s). GREEN1/1,5.82s
observes an actual sagat_app session wait while live, releases only after database expiry and
compares one atomic checkpoint of expiry/run version/events/outbox/active leases. A fresh
session claims successfully; old authority remains refused. No token/hash/task debug is logged.
The entire security integration passes126/126 (81.09s); strict locked workspace/all-target
Clippy (80.91s), format/diff and24-crate architecture pass. Two bounded plan/code reviews ended
with no concrete defects; reviewers ran no tests. Owner's current-review selection applies,
with no external CLI. Existing worker authentication/transport error and trace mapping is used;
these integration tests do not assert telemetry counters. No all-workspace/SDK58/Compose/advisory/
resource/nesting acceptance or canonical checkbox closure is claimed. Exact source/commands/
limits: tasks/project-completion/evidence/session-renewal-postwait-20261018/manifest.json.


## Full/exact-path claim-room session freshness — 2026-10-18

Refs:T014-040/044, FR-009-001/003, FR-014-014. Both claim-time ports now share a private
transaction-typed matching tenant/namespace/id/token/subject session guard. Sample DB clock
after row acquisition; expired/missing yields0, otherwise preserve active LEASED/RUNNING count,
conversion and saturation. Keep row guard on owning execution TX through count/lease commit;
no renewal or new lock/API/TTL policy. Preserve root run/drain→session and scoped run/drain/
binding→session separately; there is no new root binding guard. This sample does not promise
continuous validity until commit. Separate managed-database SHARE guard/B0/group remain open.

Both native-port RED cases returned8 after expiry (0/2,5.58s); GREEN2/2,7.02s. Actual known
blocker and sagat_app wait are observed before expiry, unchanged guard rolls back after DB
expiry, and zero room leaves21-table execution checkpoint and session expiry unchanged. Fresh
public registration supplies8, old remains0, verify_run passes. These are claim-time port
barriers, not publicpoll expired-lease acceptance: earlier publicpoll renewal is separate.
Regression: scoped_worker18/18 (15.62s), bounded_fences10/10 (14.04s), security126/126 (85.39s).
The10 include the focused2. Strict locked workspace/all-target Clippy113.29s, fmt/diff and
24-crate architecture pass. Two bounded reviews ended with no concrete issues after correcting
the prompt's root/scoped prefix wording; reviewers ran no tests. Existing worker.scoped.claim
trace and outcome/error mapping stay intact; port tests do not assert counters. No new full
workspace/SDK58/Compose/advisory/resource/nested acceptance or canonical checkbox closure.
Exact source/commands/limits: tasks/project-completion/evidence/session-room-postwait-20261018/manifest.json.


## Managed owner session freshness after SHARE wait — 2026-10-18

Refs:T014-040/044, FR-017-012/019, FR-014-014. Existing database_owner now retains matching
session SHARE then samples DB clock; missing/expired supplies no owner authority. Keep exact
tenant/namespace/ownrun/rootnode/subject/minor>=6 filters and binding→session→namespaceSecrets
quota→current dedicated membership order. No TTL/wire/provider/role change or new run lock.
Validity is at that postlock sample, not continuous through later catalog waits or commit.

Real PostgreSQL/OpenBao aggregate creates credential/catalog/published database query/dedicated
runner and obtains an actual public lease. Unchanged-row barrier observes known blocker and
actual sagat_app JOINsession waiter whilelive, rolls back only after DBexpiry. RED aggregate
0/1,17.20s: public heartbeat accepted and extended lease after session expiry. This proves
heartbeat authorization failure, not credential disclosure. GREEN whole connections target3/3,
28.78s: heartbeatNotBound and atomic runversion/events/outbox/audits/pins/sessionexpiry/leaseexpiry
unchanged, verify_run; public same-name reconnect preserves ID, replaces token and permits
heartbeat before continuing all prior aggregate cases. No owner SQL revives positive authority.

Fresh isolated database-runner process integration1/1,28.49s (latency measurement filtered),
with rebuilt binary; strict locked workspace/all-target Clippy67.59s, fmt/diff and24-crate
architecture pass. Two fresh bounded plan/code reviews found no actionable issues; no tests
run by reviewers or external CLI. Existing database.resolve/worker heartbeat metrics, closed
errors and after-commit audit mappings remain; native barriers do not assert counters.
Complete B0/group, fullworkspace/SDK58/Compose/advisory/resource acceptance remain pending,
with no canonical checkbox closure. Exact sources/commands/limits:
tasks/project-completion/evidence/database-session-postwait-20261018/manifest.json.


## Managed report receipt binding order — 2026-10-18

Refs:T014-040/044, FR-017-019, FR-014-014. Existing managed report replay now retains binding S
before namespace Secrets quota X and dedicated membership S. Permission, durable attempt owner,
fingerprint and current binding/role remain required, including after session removal. Live/
missing-receipt and ordinary paths retain their semantics. No new provider I/O, execution/pin/
resolution-audit/outbox write or public field. This is a focused existing017 dependency of B0.

Actual real PostgreSQL/OpenBao public receipt replay RED aggregate0/1 (17.62s) retains Secrets X
while blocked on Binding S. This ordered-prefix failure is not a three-party deadlock proof.
GREEN full connections3/3 (27.25s): known blocker/app-role wait, free Secrets probe, session
cleared, exact already-applied receipt/status, unchanged atomic runversion/events/outbox/audit/
pincounts and verify_run; public reconnect and next managed claim succeed.
Fresh isolated runner integration1/1 (28.85s, benchmark filtered), rebuilt binary; strict locked
workspace/all-target Clippy85.74s, formatting/diff and24-crate architecture pass. Sol/high
provided a read-only test draft; root inspected, extended/applied and ran all tests. Fresh
Astra/xhigh bounded code review found no concrete issues, no tests run by reviewer. Owner's
current-review selection applies, no external CLI. Existing typed errors, database/worker
metrics and committed audit mapping remain; these barriers do not assert counters.
No fullworkspace/SDK58/Compose/advisory/resource/nested/group acceptance or task closure.
Exact source/commands/limits: tasks/project-completion/evidence/managed-replay-prefix-20261018/manifest.json.


## Rerun source identity admission — 2026-10-18

Refs:T014-040/044, FR-014-012–015, T017-012/024. ADR-0039 advances exact tenant/source KEY SHARE
after existing receipt replay and before retained definition, managed catalog, active-run and
payload admission. Identity only; missing reference stays MissingReference/NotFound. Cleanup
FOR UPDATE SKIP LOCKED is unchanged. Current reruns create unrelated roots and child writers
have no rerun source: documented writer invariant, not a schema/type guarantee. No source-state
or group authority, wire field, provider I/O or external-effect guarantee is added.

The runtime-role real PostgreSQL/OpenBao barrier actually reproduced the old source/catalog
cycle: deadlocks0→1; rerun succeeded and native pin cleanup returned Unavailable. Corrected
full connections initially3/3 (27.30s). Missing source leaves atomic counts unchanged; cancelling
the source-blocked request rolls back before same-key retry; cleanup skips guarded source and
later full cleanup releases it. A real new rerun claims, resolves, completes and releases its pin;
strengthened metadata control requires the current rotated credential and rejects source version.
The first strengthened-control run hit an INT4/i64 fixture decode error, retained separately;
only test SELECT casts changed. Final exact results are in the manifest below.

Segment rerun/independent-chain regression1/1 (1.95s). Three fresh Astra/xhigh bounded plan/code
reviews; actionable SKIP/error/runtime-role/writer-invariant and stale-credential-test findings
incorporated, final no concrete issues. Sol/high read-only draft, root sole writer/test executor;
owner's current review applies, no external CLI. Existing run.rerun trace/typed error and committed
RunRerun audit/outbox/metrics mappings remain; native barriers do not assert metric counters.
No fullworkspace tests/SDK58/100k/Compose/advisory/S3/nested/group acceptance or canonical closure.
Evidence: tasks/project-completion/evidence/rerun-source-identity-20261018/manifest.json.

Final stronger-control full connections3/3 (27.36s), strict locked all-target workspace Clippy
passes2.87s, formatting4.11s, architecture24crates and diff check pass.


## Pure capacity header restoration — 2026-10-18

Refs:T014-014/017/040. Checked CapacityGroup::restore preserves identities/revision/epoch/count/
possession, validates nonzero/bounded counters and exact Active holder consistency, and rejects
malformed snapshots with InconsistentSnapshot. Maximum counters restore without reset; mutation
overflow is inert. No registry hydration or new infrastructure/domain dependency.

Test-first missing API compilation is retained, not reported as behavioral RED. Focused model9/9
(0.22s),1024histories seed3714 roundtrip after every accepted independent-registry transition,
15invalid header/fact tuples, explicit maximum preservation; internal overflow2/2. Strict domain
all-target Clippy1.00s, fmt4.05s, architecture24crates. Fresh adversarial Astra/xhigh code review
found no substantive issues; only root ran tests. No external reviewCLI, owner's current selection.
Supplied consistency is not actual database count/tenant/unique history/own-member authority. SQL
port/atomic journal/audit/outbox/OTel, mixed writers, eligibility/resource/SDK/nested acceptance
remain pending adapter/integration work; no canonical checkbox closure. Evidence:
tasks/project-completion/evidence/capacity-header-restore-20261018/manifest.json.


## Dormant capacity Finish vocabulary — 2026-10-18

Refs:T014-014/017/040. Truthful engine audit capacity.member.finished / capacity_group, without
generic repeat collapse; native receipt dedup proof remains pending. Canonical closed AsyncAPI
hint contains reference identities/result revision/epoch/retired/optional nullable trace only.
Only new baseline message added manually; all prior parsed shapes and SDK goldens unchanged.
Audit7/7, synthetic hint1/1, AsyncAPI evolution/actual existing outbound conformance4/4 (2.02s).
Initial API/fixture compile errors retained separately; corrected hint RED0/1 (0.04s) on missing
channel. Actual new producer/outbox/receipt behavior remains B2d/e; no broker delivery claim.
Pinned CLI validates0errors/0warnings plus1version info; strict locked workspace/all-target
Clippy83.89s, fmt4.04s,24-crate architecture pass. Fresh Astra/xhigh review's only finding was an
overclaiming test name, corrected and audit rerun. No external reviewCLI. Native schema/port,
SDK58/fullworkspace/resource/old-writer/nested acceptance and canonical tasks remain pending.
Evidence: tasks/project-completion/evidence/capacity-finish-vocabulary-20261018/manifest.json.


## Dormant native capacity Finish checkpoint — 2026-10-18

Refs:T014-014/017/040, FR-014-015. Local A1/B1a/B1b.1/B2a-e deliver only existing ChildRoot
Finish in owner-seeded metadata. Actual current physical child FINAL and fence precede atomic
member/header/receipt/audit/outbox; current head cannot be inferred from CONTINUED history.
Schema constraints, narrow runtime column privileges and retained tombstones are tested; permanent
chain references currently pin history and require C0 retention compatibility before activation.
G Busy consumes the transaction, duplicate commands serialize, postwait expiry rejects first
apply and replay, injected statement/deferred failures leave zero effects, and actual unresolved
COMMIT response loss reconciles both real server outcomes with the retained original command.
Handoff queue ordering, exact stale-head rejection and same-tenant two-namespace membership with
both holder placements have native tests. Execution/retained usage snapshots remain unchanged;
actual capacity.member.finished hints validate against the closed canonical AsyncAPI message.

B2f adds workflow.capacity.finish and finite returned attempt metrics; see the exact native
observation contract in specs/014-workflow-primitives/contracts/README.md. Only acknowledged first
commit increments workflow_capacity_member_finished_total. Replay and unknown acknowledgement do
not; the durable receipt remains authoritative. No identifier metric labels, payloads, scheduler
credentials or arbitrary error strings are introduced by these observations. An untraced direct
call emits metrics without an OTLP span; a future production caller must establish trace context.
This dormant port has no production caller, creation/admission/eligibility, scoped-path Finish,
retention rollout or nested execution authority. Full B1/B2, canonical tasks and broad runtime/
SDK/resource/release gates remain open. Exact tests/reviews/source provenance are recorded under tasks/project-completion/evidence:
capacity-finish-schema-20261018, capacity-native-finish-20261018, capacity-finish-barriers-20261018,
capacity-finish-commit-loss-20261018, capacity-finish-races-20261018 and
capacity-finish-observations-20261018.
