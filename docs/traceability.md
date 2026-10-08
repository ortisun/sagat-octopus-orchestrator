# Traceability of features, requirements and test evidence

> Base record to update per PR; no requirement is implemented in this package.

| Feature | Requirements | User stories | Planned tests | Status |
|---|---|---|---|---|
| [001 platform-foundation](../specs/001-platform-foundation/spec.md) | FR-001-001..007 | 4 | `tests/e2e/001-platform-foundation/`, `tests/fault/001-platform-foundation/` | Planned |
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
