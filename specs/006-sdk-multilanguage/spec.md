# Feature Specification: TypeScript, Go, Python, C# and Java SDKs

**Feature Branch:** `006-sdk-multilanguage`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Allow definition, triggers, activities and Sagas in five languages with common execution semantics.

**Priority:** M3–M4. **Dependencies:** 001, 002, 003, 004.

## User Scenarios & Testing (mandatory)

### User Story 1 — Create an application run in C# (P1)

**As** a user of this feature, **I want** to create an application run in C#, **so that** it allows definition, triggers, activities and Sagas in five languages with common execution semantics.

**Why P1:** independent, verifiable value requirement; supports the M3–M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** an ASP.NET Core service; **When** it registers a client via DI and calls StartAsync; **Then** Run is created with idempotencyKey and trace context propagated.

### User Story 2 — Register workers in any SDK (P1)

**As** a user of this feature, **I want** to register workers in any SDK, **so that** it allows definition, triggers, activities and Sagas in five languages with common execution semantics.

**Why P1:** independent, verifiable value requirement; supports the M3–M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** a developer with a TS/Go/Python/C#/Java activity; **When** they register a worker and receive a task; **Then** the server validates capabilities and completes the step via the common protocol.

### User Story 3 — Write a typed Saga (P1)

**As** a user of this feature, **I want** to write a typed saga, **so that** it allows definition, triggers, activities and Sagas in five languages with common execution semantics.

**Why P1:** independent, verifiable value requirement; supports the M3–M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** an activity with compensate/reconcile defined; **When** a workflow fails; **Then** all SDKs receive the same semantics and canonical errors.

### User Story 4 — Migrate the SDK version (P1)

**As** a user of this feature, **I want** to migrate the SDK version, **so that** it allows definition, triggers, activities and Sagas in five languages with common execution semantics.

**Why P1:** independent, verifiable value requirement; supports the M3–M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** a compatible previous minor version; **When** the worker connects to the updated server; **Then** protocol negotiation supports progressive upgrade.

## Functional Requirements

- **FR-006-001**: The system MUST maintain the canonical workers schema in Protobuf and REST, with versions and error codes.
- **FR-006-002**: The system MUST publish TypeScript, Go, Python, C#/.NET and Java SDK clients with Start/Get/List/Cancel/Signal/PublishEvent.
- **FR-006-003**: The system MUST provide typed WorkerHost and Activity registration in each SDK with lease heartbeat and cancellation.
- **FR-006-004**: The system MUST expose the primitives Step.Run, Sleep, WaitForEvent, InvokeChild, Parallel, Saga.Compensate and Reconcile in idiomatic forms.
- **FR-006-005**: The system MUST propagate W3C traceparent, correlationId, idempotency key and deadline; implement retries only when safe.
- **FR-006-006**: The system MUST implement a single conformance suite with fixtures and fault injection for the five languages.
- **FR-006-007**: The system MUST ensure C# integrates with Microsoft.Extensions.DependencyInjection, IHostedService, CancellationToken, ILogger and ActivitySource.
- **FR-006-008**: The system MUST publish packages with SemVer versioning, runtime version matrix and minimal examples.

## Edge Cases

- SDK worker dies without heartbeat.
- newer SDK with older server.
- payload without schema.
- decimal/UTC serialization across languages.
- Go context canceled.
- CancellationToken expires.
- Python async cancellation.

## Mandatory cross-cutting requirements

- **NFR-006-SEC:** tenant isolation, RBAC, least privilege, secret never in logs or in the journal.
- **NFR-006-DUR:** after commit, loss of a worker/broker cannot lose a transition; use lease/fencing where applicable.
- **NFR-006-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-006-COMP:** versioned protocol and schemas; do not break runs in progress.
- **NFR-006-PERF:** Worker protocol must tolerate high concurrency without busy polling; measure streaming and backpressure.

## Success Criteria (measurable)

- **SC-006-001:** Same golden flows and failure cases executed in TS, Go, Python, C# and Java with normalized results.
- **SC-006-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-006-003:** no violation of the Constitution's gates and no silent loss of persisted state.
- **SC-006-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

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

Requirements `FR-006-###` and criteria `SC-006-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation.
