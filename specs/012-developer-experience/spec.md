# Feature Specification: CLI, quickstarts, local testing and documentation

**Feature Branch:** `012-developer-experience`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Enable safe adoption by product, backend and operations teams without manually configuring complex infrastructure.

**Priority:** M4–M5. **Dependencies:** 001, 002, 003, 004, 005, 006, 007.

## User Scenarios & Testing (mandatory)

### User Story 1 — Start first workflow quickly (P1)

**As** a user of this feature, **I want** to start the first workflow quickly, **so that** I can enable safe adoption by product, backend and operations teams without manually configuring complex infrastructure.

**Why P1:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** a developer follows the quickstart; **When** they run compose + CLI + SDK; **Then** a two-step workflow executes and is visible in Studio.

### User Story 2 — Test Saga locally (P1)

**As** a user of this feature, **I want** to test a saga locally, **so that** I can enable safe adoption by product, backend and operations teams without manually configuring complex infrastructure.

**Why P1:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** a developer has a payment saga; **When** they inject a failure into the mock provider; **Then** they see reconciliation and compensation in the development emulator.

### User Story 3 — Generate examples per SDK (P1)

**As** a user of this feature, **I want** to generate examples per SDK, **so that** I can enable safe adoption by product, backend and operations teams without manually configuring complex infrastructure.

**Why P1:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** a new developer uses Go or C#; **When** they copy the example and run it; **Then** the example produces a contract/trace consistent with the other SDKs.

### User Story 4 — Diagnose publishing errors (P2)

**As** a user of this feature, **I want** to diagnose publishing errors, **so that** I can enable safe adoption by product, backend and operations teams without manually configuring complex infrastructure.

**Why P2:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** a manifest has an invalid node; **When** the CLI validate runs; **Then** they receive errors with path, remediation and docs links.

## Functional Requirements

- **FR-012-001**: The system MUST provide the CLI `df workflow validate|publish|run|inspect|cancel`, `df script test|publish`, `df worker dev`.
- **FR-012-002**: The system MUST create TS, Go, Python, C# and Java quickstarts and a saga business example.
- **FR-012-003**: The system MUST offer docker-compose with services and reproducible fixtures, with an optional local emulator consistent with the real semantics.
- **FR-012-004**: The system MUST provide templates for workflows, event trigger, cron, approval, compensation and inline script.
- **FR-012-005**: The system MUST generate API docs and contracts from the schemas and examples executable in CI.
- **FR-012-006**: The system MUST instrument onboarding and offer diagnostics for env, credentials and incompatible versions.

## Edge Cases

- client without Docker.
- emulator diverges from the engine.
- incompatible SDK version.
- example with a hardcoded secret.
- NATS DNS errors.
- offline inline build.

## Mandatory cross-cutting requirements

- **NFR-012-SEC:** tenant isolation, RBAC, least privilege, secrets never in logs or in the journal.
- **NFR-012-DUR:** after commit, loss of a worker/broker cannot lose a transition; use lease/fencing where applicable.
- **NFR-012-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-012-COMP:** versioned protocol and schemas; do not break in-flight runs.
- **NFR-012-PERF:** Onboarding with dependencies installed: target <15min in UX evaluation, without promising a universal time.

## Success Criteria (measurable)

- **SC-012-001:** A new team member executes their first runs in TS and C# following only the documentation; CLI tests cover invalid workflows.
- **SC-012-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-012-003:** no violation of the Constitution gates and no silent loss of persisted state.
- **SC-012-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

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

Requirements `FR-012-###` and criteria `SC-012-###` must be mapped to tasks and tests in `tasks.md` before each implementation starts.
