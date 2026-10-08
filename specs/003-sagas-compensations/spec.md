# Feature Specification: Native sagas and compensations

**Feature Branch:** `003-sagas-compensations`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Orchestrate long-running business transactions, with compensations, controlled reprocessing and reconciliation.

**Priority:** M2. **Dependencies:** 001, 002.

## User Scenarios & Testing (mandatory)

### User Story 1 — Define compensation on publish (P1)

**As** a user of this feature, **I want** to define compensation on publish, **so that** I can orchestrate long-running business transactions, with compensations, controlled reprocessing and reconciliation.

**Why P1:** independent, verifiable value requirement; supports the M2 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** a reserve action with compensate release; **When** the author publishes a Saga; **Then** the plan records dependencies and durable compensation IDs.

### User Story 2 — Compensate after permanent failure (P1)

**As** a user of this feature, **I want** to compensate after permanent failure, **so that** I can orchestrate long-running business transactions, with compensations, controlled reprocessing and reconciliation.

**Why P1:** independent, verifiable value requirement; supports the M2 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** reserve completed and submit failed permanently; **When** the Saga Coordinator triggers recovery; **Then** release executes idempotently and the run indicates COMPENSATED.

### User Story 3 — Reconcile an uncertain effect (P1)

**As** a user of this feature, **I want** to reconcile an uncertain effect, **so that** I can orchestrate long-running business transactions, with compensations, controlled reprocessing and reconciliation.

**Why P1:** independent, verifiable value requirement; supports the M2 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** the provider accepted the payment but the response was lost; **When** the engine restarts; **Then** a reconciliation step queries the outcome before retrying or compensating.

### User Story 4 — Resolve a compensation failure (P1)

**As** a user of this feature, **I want** to resolve a compensation failure, **so that** I can orchestrate long-running business transactions, with compensations, controlled reprocessing and reconciliation.

**Why P1:** independent, verifiable value requirement; supports the M2 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** compensation failed after retries; **When** the operator reviews and intervenes; **Then** the state and the reason are audited without erasing previous events.

## Functional Requirements

- **FR-003-001**: The system MUST incorporate SagaBoundary, ForwardAction, CompensationAction, ReconcileAction and a cancellation policy into the Workflow IR.
- **FR-003-002**: The system MUST persist the compensation plan as a topological reverse DAG, respecting branch independence.
- **FR-003-003**: The system MUST execute compensation with its own retries/idempotency, limits, timeout and provider circuit breaker.
- **FR-003-004**: The system MUST distinguish COMPENSATED, PARTIAL, COMPENSATION_FAILED, MANUAL_REVIEW and irreversible action.
- **FR-003-005**: The system MUST expose pause, retry compensation and skip operations, with skip allowed only with explicit permission and an audited justification.
- **FR-003-006**: The system MUST propagate cancellation/deadline to subworkflows under a configured policy.
- **FR-003-007**: The system MUST require reconcile or mark-manual for operations with an ambiguous high-risk result.

## Edge Cases

- compensation fails after external success.
- parallel saga with independent compensations.
- double manual command on compensation.
- irreversible effect already settled.
- cancellation while execution is active.

## Mandatory cross-cutting requirements

- **NFR-003-SEC:** tenant isolation, RBAC, least privilege, secret never in logs or in the journal.
- **NFR-003-DUR:** after commit, loss of a worker/broker must not lose a transition; use lease/fencing where applicable.
- **NFR-003-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-003-COMP:** versioned protocol and schemas; do not break in-flight runs.
- **NFR-003-PERF:** The compensation process does not block threads; throughput is subject to provider quotas and business policies.

## Success Criteria (measurable)

- **SC-003-001:** E2E reserve→submit fails→release; ambiguous provider requires reconciliation; release failure triggers manual review.
- **SC-003-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-003-003:** no violation of the Constitution gates and no silent loss of persisted state.
- **SC-003-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

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

Requirements `FR-003-###` and criteria `SC-003-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation.
