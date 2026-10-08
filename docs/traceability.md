# Traceability of features, requirements and test evidence

> Base record to update per PR. Feature 001 is being implemented (see the evidence table below); every other feature is still only specified.

| Feature | Requirements | User stories | Planned tests | Status |
|---|---|---|---|---|
| [001 platform-foundation](../specs/001-platform-foundation/spec.md) | FR-001-001..007 | 4 | `tests/e2e/001-platform-foundation/`, `tests/fault/001-platform-foundation/` | Implementing |
| [002 durable-execution](../specs/002-durable-execution/spec.md) | FR-002-001..009 | 4 | `tests/e2e/002-durable-execution/`, `tests/fault/002-durable-execution/` | Planned |
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
| FR-001-002 REST/OpenAPI v1 for health, auth context, workspace, namespace | Done | OpenAPI lint in CI, `tests/e2e/001-platform-foundation/{us01,us02}.rs` |
| FR-001-002 … and the **definition lifecycle** | Open | Declared in `openapi.yaml`, not implemented (belongs with feature 002) |
| FR-001-003 versioned gRPC/protobuf worker protocol | Partial | `worker.proto` compiles in CI; no server, no compatibility check (`buf breaking`) yet |
| FR-001-004 idempotent migrations; development-only seed | Done | `tests/integration/001-platform-foundation/migrations.rs`, `tests/e2e/001-platform-foundation/us03.rs`, `apps/control-plane/tests/cli.rs` |
| FR-001-005 Docker Compose with PostgreSQL, NATS and optional S3-compatible store | Partial | `infra/docker/compose.yaml`, `scripts/smoke-compose.sh`; the S3-compatible store is deferred (optional in this phase) |
| FR-001-006 canonical errors without sensitive data | Partial | One envelope `{code, message, retryable, traceId, details}` proven in `tests/e2e/001-platform-foundation/`; the spec's separate `request_id` is not implemented (the trace id serves) |
| FR-001-007 OIDC/JWT, base RBAC on every route, audit trail (including tenant and administrator provisioning) | Done | `tests/contract/001-platform-foundation/auth.rs`, `tests/e2e/001-platform-foundation/{us02,us04}.rs`, `tests/fault/001-platform-foundation/provider.rs` |
| NFR-001-OBS structured logs, correlated traces, essential metrics | Partial | JSON logs and trace-id correlation (response, error body, audit) are tested; **OTLP export and metrics are not implemented** (G6) |
| NFR-001-PERF p95 < 250 ms namespace read | Open | No benchmark has been run (T001-033) |
| SC-001-001 contract/API + tenant isolation suite and reproducible local environment | Done | The suites above plus `scripts/smoke-compose.sh` |
| SC-001-002 100% of US01..US04 pass in an automated suite | Done | `tests/e2e/001-platform-foundation/us0{1,2,3,4}.rs` |
| SC-001-003 no violation of the Constitution gates | Open | `.specify/memory/constitution.md` is not in the repository (G1); cannot be asserted |
| SC-001-004 critical errors expose `code`, `trace_id` and remediation without leaking | Partial | `code` and `traceId` proven; there is no separate `remediation` field |
