# Feature Specification: Security, isolation and multi-tenant governance

**Feature Branch:** `009-security-multitenancy`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Protect tenant workloads, secrets and untrusted code execution before offering inline scripts.

**Priority:** Cross-cutting, gate for 005. **Dependencies:** 001 and 002 for platform identity and
worker protocols; the completion evidence composes the integrated 005 sandbox and reuses the
published 010 Helm chart rather than creating a second runner or deployment contract.

## User Scenarios & Testing (mandatory)

### User Story 1 — Prevent cross-tenant access (P1)

**As** a user of this feature, **I want** to prevent cross-tenant access, **so that** I can protect tenant workloads, secrets and untrusted code execution before offering inline scripts.

**Why P1:** independent, verifiable value requirement; supports the Cross-cutting, gate for 005 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** tenant A with a valid token; **When** it tries to access tenant B's Run; **Then** reading, manipulation and discovery of existence are denied.

### User Story 2 — Grant a capability to a script (P1)

**As** a user of this feature, **I want** to grant a capability to a script, **so that** I can protect tenant workloads, secrets and untrusted code execution before offering inline scripts.

**Why P1:** independent, verifiable value requirement; supports the Cross-cutting, gate for 005 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** an author with access to a staging secret; **When** they publish a Script; **Then** the worker receives only a temporary capability and authorized secrets.

### User Story 3 — Block unauthorized egress (P1)

**As** a user of this feature, **I want** to block unauthorized egress, **so that** I can protect tenant workloads, secrets and untrusted code execution before offering inline scripts.

**Why P1:** independent, verifiable value requirement; supports the Cross-cutting, gate for 005 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** a script tries to access a forbidden endpoint; **When** the sandbox executes; **Then** the network is blocked and the incident is audited.

### User Story 4 — Enforce quotas per namespace (P1)

**As** a user of this feature, **I want** to enforce quotas per namespace, **so that** I can protect tenant workloads, secrets and untrusted code execution before offering inline scripts.

**Why P1:** independent, verifiable value requirement; supports the Cross-cutting, gate for 005 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** a namespace exhausts its limit of builds and tasks; **When** new requests arrive; **Then** they return quota exceeded without affecting other tenants.

## Functional Requirements

- **FR-009-001**: The system MUST enforce tenant_id and namespace on all entities and queries, and granular RBAC on all APIs.
- **FR-009-002**: The system MUST define policies for resources, secrets, scripts, connector scopes, publish and manual operations.
- **FR-009-003**: The system MUST use federated OIDC/JWT identity, short-lived service tokens and rotatable secrets.
- **FR-009-004**: The system MUST emit an immutable, redacted audit trail for access to sensitive resources.
- **FR-009-005**: The system MUST run scripts with cgroups, namespaces, seccomp and egress proxies in isolated pools.
- **FR-009-006**: The system MUST implement quotas for API/event rate, concurrent runs, workers, builds, storage, logs and retention.
- **FR-009-007**: The system MUST cover dependency supply chain, artifact verification and policy changes with cache invalidation.
- **FR-009-008**: The system MUST implement controls for webhook SSRF/replay and egress destinations.

## Edge Cases

- tenant token spoofing.
- secret in stack trace.
- container with undue privileges.
- confused deputy across namespaces.
- ACL race when renewing token.
- quota with concurrent replicas.

## Mandatory cross-cutting requirements

- **NFR-009-SEC:** tenant isolation, RBAC, least privilege, secret never in logs or in the journal.
- **NFR-009-DUR:** after commit, loss of worker/broker must not lose a transition; use lease/fencing where applicable.
- **NFR-009-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-009-COMP:** versioned protocol and schemas; do not break in-flight runs.
- **NFR-009-PERF:** Quota approval on the critical path must not depend on an eventually consistent counter for a security limit.

## Success Criteria (measurable)

- **SC-009-001:** Battery of cross-tenant, SSRF, sandbox escape, secret leak, webhook replay and quota racing; fail the release if any critical finding exists.
- **SC-009-002:** 100% of the US01..US04 scenarios pass in a reproducible automated suite.
- **SC-009-003:** no violation of the Constitution's gates and no silent loss of persisted state.
- **SC-009-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

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

Requirements `FR-009-###` and criteria `SC-009-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation.

## Remaining durable quota contract

ADR-0027 defines completion of FR-009-006: exact tenant retained-content and log-byte accounting,
durable namespace API/event rate admission, and operator ceilings on new event/script retention
choices. `tenant_content_bytes` counts classified logical tenant content and external object
reservations; it does not claim physical disk, index, WAL, replica or backup accounting. Every
variable-size tenant column must be classified and covered by migration tests. Bounded, payload-free
audit/journal/delivery control records remain protected so a full content/log quota cannot prevent
terminal evidence. Log exhaustion truncates with persisted evidence. Rejected asynchronous content
ends with a fenced typed failure and repeatable acknowledgement. Lowering retention policy does not
rewrite existing expiry or authorize deletion. These are conservative engineering limits; quota
tiers and regulatory retention classification remain the open product questions above.


## Approved workflow-primitives dependency (2026-10-14)

[ADR-0030](../../docs/adr/ADR-0030-structured-instances-and-continuation-chains.md) and
[ADR-0031](../../docs/adr/ADR-0031-declared-operation-result-disclosure.md) accept the feature-014
architecture and separate declared-result disclosure policy.
[ADR-0032](../../docs/adr/ADR-0032-additive-workflow-ir.md) keeps the workflow IR at v1 and
supersedes the earlier separate-revision allocation. Add the accepted built-in approval/read/decision and run/query/update grants, current authorization on receipt replay, tenant-scoped instance/chain references and atomic quotas/audit. ADR-0031 permits only explicitly declared bounded business results; ordinary structural inspection retains ADR-0028 and no custom/per-operation roles are invented.

This approves the described dependency for implementation, not a runtime guarantee. Canonical
contracts, migration/replay, tenant/fault tests and applicable five-SDK evidence remain the
feature-014 release gates; existing feature behavior stays supported.
## Approved dependent delivery: database connections

Feature [017](../017-database-connections/spec.md) and [ADR-0035](../../docs/adr/ADR-0035-database-connections-and-managed-execution.md) were approved on 2026-10-16. Its [plan](../017-database-connections/plan.md) defines the bounded extension to shared contracts, admission/leases, effects, SDKs, Studio, telemetry, secret-provider/security and immutable revisions. Existing transactional, tenant and general payload-inspection guarantees remain binding. Track implementation and new runtime evidence in feature 017; this dependency note does not mark its requirements complete.
