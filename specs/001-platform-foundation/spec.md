# Feature Specification: Platform foundation and contracts

**Feature Branch:** `001-platform-foundation`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Provide a foundation of tenants, namespaces, HTTP/gRPC contracts and a reproducible environment without coupling the domain to infrastructure.

**Priority:** M0–M1. **Dependencies:** none.

## User Scenarios & Testing (mandatory)

### User Story 1 — Create a workspace and namespace (P1)

**As** a user of this feature, **I want** to create a workspace and namespace, **so that** I can provide a foundation of tenants, namespaces, HTTP/gRPC contracts and a reproducible environment without coupling the domain to infrastructure.

**Why P1:** independent, verifiable value requirement; supports the M0–M1 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** an authenticated administrator; **When** they create a workspace and dev namespace; **Then** the workspace is persisted, and the scope appears in the API/Studio.

### User Story 2 — Authenticate an SDK client (P1)

**As** a user of this feature, **I want** to authenticate an SDK client, **so that** I can provide a foundation of tenants, namespaces, HTTP/gRPC contracts and a reproducible environment without coupling the domain to infrastructure.

**Why P1:** independent, verifiable value requirement; supports the M0–M1 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** a valid token with tenant scope; **When** the client queries namespaces; **Then** the API returns only the authorized resources and a trace id.

### User Story 3 — Bring up a complete local environment (P1)

**As** a user of this feature, **I want** to bring up a complete local environment, **so that** I can provide a foundation of tenants, namespaces, HTTP/gRPC contracts and a reproducible environment without coupling the domain to infrastructure.

**Why P1:** independent, verifiable value requirement; supports the M0–M1 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** the cloned repository and prerequisites installed; **When** the developer starts docker compose and runs health checks; **Then** API, PostgreSQL and NATS are running and migrations are idempotent.

### User Story 4 — Deny cross-tenant access (P1)

**As** a user of this feature, **I want** to deny cross-tenant access, **so that** I can provide a foundation of tenants, namespaces, HTTP/gRPC contracts and a reproducible environment without coupling the domain to infrastructure.

**Why P1:** independent, verifiable value requirement; supports the M0–M1 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** a user of tenant A; **When** they request a resource of tenant B; **Then** the API does not reveal tenant B data and records the attempt.

## Functional Requirements

- **FR-001-001**: The system MUST implement Tenant, Workspace, Namespace, Membership and Role entities with uniqueness constraints and per-tenant isolation.
- **FR-001-002**: The system MUST expose REST/OpenAPI v1 for health, auth context, workspace, namespace and definition lifecycle.
- **FR-001-003**: The system MUST define versioned gRPC/protobuf for the worker protocol and message compatibility.
- **FR-001-004**: The system MUST create idempotent PostgreSQL migrations and seed exclusively for development.
- **FR-001-005**: The system MUST create a development Docker Compose with PostgreSQL, NATS and an S3-compatible object store (optional in the initial phase).
- **FR-001-006**: The system MUST emit canonical errors with code, message, retryable, trace_id, request_id without sensitive data.
- **FR-001-007**: The system MUST implement OIDC/JWT authentication and base RBAC on all routes, with an audit trail.

## Edge Cases

- JWT expired/revoked or JWKS temporarily unavailable.
- same-named namespace in different tenants.
- migration interrupted and repeated.
- authorization after stale permission cache.

## Mandatory cross-cutting requirements

- **NFR-001-SEC:** tenant isolation, RBAC, least privilege, secret never in logs or in the journal.
- **NFR-001-DUR:** after commit, loss of a worker/broker cannot lose a transition; use lease/fencing where applicable.
- **NFR-001-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-001-COMP:** versioned protocol and schemas; do not break in-progress runs.
- **NFR-001-PERF:** p95 <250ms on namespace read in the baseline benchmark; authentication fails closed.

## Success Criteria (measurable)

- **SC-001-001:** Contract/API + tenant isolation suite and reproducible local environment with migrations executed twice.
- **SC-001-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-001-003:** no violation of the Constitution gates and no silent loss of persisted state.
- **SC-001-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

## Key Entities

Check `docs/architecture/overview.md`, `docs/architecture/execution-semantics.md` and `specs/001-platform-foundation/data-model.md` for the persisted entity; feature abstractions are described in the local `data-model.md`.

## Assumptions

- Workflow IRs and versions are immutable after publication.
- External integrations are at-least-once, subject to idempotency or reconciliation.
- V1 single-region HA; multi-region active-active requirements are out of scope.

## Out of Scope

- Reimplementing Temporal/Restate; implementing the entire connector market; automatic migration of arbitrary code; exactly-once physical execution of external side effects.

## Open Design Questions

- Define capacity/quota values per tier, after benchmark and security testing.
- Confirm the payload classification/retention scheme per customer and regulator.
- Adjust SDK ergonomics based on interviews and proof-of-concept.

## Traceability

Requirements `FR-001-###` and criteria `SC-001-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation.
