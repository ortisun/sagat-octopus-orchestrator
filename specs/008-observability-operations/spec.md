# Feature Specification: Distributed observability and operations

**Feature Branch:** `008-observability-operations`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Diagnose and operate runs, steps, external integration, resources and Sagas from reliable data.

**Priority:** M4–M5. **Dependencies:** 001, 002, 003, 004, 007.

## User Scenarios & Testing (mandatory)

### User Story 1 — Inspect timeline and attempt (P1)

**As** a user of this feature, **I want** to inspect timeline and attempt, **so that** I can diagnose and operate runs, steps, external integration, resources and Sagas from reliable data.

**Why P1:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** a Run with 3 attempts; **When** the operator opens the Timeline; **Then** each transition has a timestamp, cause and contextualized trace.

### User Story 2 — Correlate external calls (P1)

**As** a user of this feature, **I want** to correlate external calls, **so that** I can diagnose and operate runs, steps, external integration, resources and Sagas from reliable data.

**Why P1:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** an HTTP activity with trace propagation; **When** the operator opens the step's span; **Then** the trace crosses ingress, scheduler, worker and instrumented provider.

### User Story 3 — Receive alerts for stuck runs (P1)

**As** a user of this feature, **I want** to receive alerts for stuck runs, **so that** I can diagnose and operate runs, steps, external integration, resources and Sagas from reliable data.

**Why P1:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** outbox age/expired leases above the threshold; **When** the monitor detects the condition; **Then** an alert with runbook and correct namespace is emitted.

### User Story 4 — Execute an audited operational action (P1)

**As** a user of this feature, **I want** to execute an audited operational action, **so that** I can diagnose and operate runs, steps, external integration, resources and Sagas from reliable data.

**Why P1:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** an operator with retry permission; **When** they execute a retry with justification; **Then** new attempts appear without erasing the previous attempt.

## Functional Requirements

- **FR-008-001**: The system MUST emit OTel traces/metrics/structured logs with IDs correlated by Run/Step/Attempt.
- **FR-008-002**: The system MUST project paginated read models for timeline and DAG, with cursor, lag and projection state.
- **FR-008-003**: The system MUST create dashboards for latency, queue lag, leases, DLQ, retry, compensation and quotas.
- **FR-008-004**: The system MUST expose operational actions pause, resume, cancel, retry, rerun-as-new, manual review and drain workers.
- **FR-008-005**: The system MUST configure exemplary alerts and runbooks for DB/NATS/worker/compensation failures.
- **FR-008-006**: The system MUST apply redaction and access controls to logs and payloads with auditing.

## Edge Cases

- trace missing but journal present.
- lagging read-model.
- metric with exploding cardinality.
- manual retry racing with auto retry.
- payload expired due to retention.

## Mandatory cross-cutting requirements

- **NFR-008-SEC:** tenant isolation, RBAC, least privilege, secrets never in logs or in the journal.
- **NFR-008-DUR:** after commit, loss of a worker/broker must not lose a transition; use lease/fencing where applicable.
- **NFR-008-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-008-COMP:** versioned protocol and schemas; do not break in-flight runs.
- **NFR-008-PERF:** Projection must not block the write-path; documented and flagged lag budget.

## Success Criteria (measurable)

- **SC-008-001:** Operator diagnoses a failure in fewer than N interactions during a UX test, with run journal and trace correlated.
- **SC-008-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-008-003:** no violation of the Constitution's gates and no silent loss of persisted state.
- **SC-008-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

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

Requirements `FR-008-###` and criteria `SC-008-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation.
