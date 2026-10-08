# Feature Specification: Visual Studio, Monaco and runtime graph

**Feature Branch:** `007-workflow-studio`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Priority:** M4. **Dependencies:** 001, 002, 003, 004.

## User Scenarios & Testing (mandatory)

### User Story 1 — Create a workflow without programming (P1)

**As** a user of this feature, **I want** to create a workflow without programming, **so that** I can offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Why P1:** independent, verifiable value requirement; supports the M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** an author in the Studio; **When** they drag activity/condition/parallel/saga and configure parameters; **Then** the validator displays errors and saves the IR draft.

### User Story 2 — Edit code in a node (P1)

**As** a user of this feature, **I want** to edit code in a node, **so that** I can offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Why P1:** independent, verifiable value requirement; supports the M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** a selected Script node; **When** the user edits TS/Python/C# and tests; **Then** Monaco shows the result in a sandbox and the version is only published after validation.

### User Story 3 — Analyze a real execution (P1)

**As** a user of this feature, **I want** to analyze a real execution, **so that** I can offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Why P1:** independent, verifiable value requirement; supports the M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** a Run with retries and an untraversed branch; **When** the operator opens the graph; **Then** attempts, duration, states, compensations and timeline reflect the journal.

### User Story 4 — Investigate and act on a failure (P1)

**As** a user of this feature, **I want** to investigate and act on a failure, **so that** I can offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Why P1:** independent, verifiable value requirement; supports the M4 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** a failed run and an authorized operator; **When** they open the trace, adjust the operational decision and perform a retry; **Then** the action is audited and does not overwrite the history.

## Functional Requirements

- **FR-007-001**: The system MUST implement Workflow Canvas with React Flow and typed node registry (Trigger, Step, Condition, Parallel, Join, Timer, Wait, Saga, Webhook, Child).
- **FR-007-002**: The system MUST implement Monaco editor, schema-aware input/output editors, autocomplete and lint.
- **FR-007-003**: The system MUST compile/verify the graph into Workflow IR with versioned node IDs, with no illegal cycles or broken references.
- **FR-007-004**: The system MUST implement Draft/Published versions, diff, and a warning for contract-breaking changes, with no in-place modification of the published version.
- **FR-007-005**: The system MUST project the journal into an execution graph with attempts, retries, compensations, trace links, redacted payload.
- **FR-007-006**: The system MUST offer SSE/WebSocket incremental updates and polling fallback, avoiding full DAG reload.
- **FR-007-007**: The system MUST enforce operational actions via RBAC and confirmation explaining the consequences of retry/compensation.
- **FR-007-008**: The system MUST implement a WCAG AA accessibility target and an alternative table/list view.

## Edge Cases

- graph with 50k nodes.
- run started on an unpublished version.
- signal between snapshots.
- worker without OTel trace.
- two simultaneous draft edits.
- circular validation error.
- redacted payload in history.

## Mandatory cross-cutting requirements

- **NFR-007-SEC:** tenant isolation, RBAC, least privilege, secrets never in logs or in the journal.
- **NFR-007-DUR:** after commit, loss of a worker/broker must not lose a transition; use lease/fencing where applicable.
- **NFR-007-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-007-COMP:** versioned protocol and schemas; do not break in-flight runs.
- **NFR-007-PERF:** 1k-node DAG usable in the reference browser; virtualized lists for 10k+; FPS targets per UX benchmark.

## Success Criteria (measurable)

- **SC-007-001:** E2E create→validate→publish→trigger→fail→compensate→inspect journal/traces.
- **SC-007-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-007-003:** no violation of the Constitution's gates and no silent loss of persisted state.
- **SC-007-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

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

Requirements `FR-007-###` and criteria `SC-007-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation.
