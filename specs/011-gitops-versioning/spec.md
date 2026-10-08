# Feature Specification: GitOps, versioning and definition promotion

**Feature Branch:** `011-gitops-versioning`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Maintain Git pipelines, auditing, immutable versions and safe deployment of workflows/scripts across environments.

**Priority:** M4–M5. **Dependencies:** 001, 002, 005, 007, 009.

## User Scenarios & Testing (mandatory)

### User Story 1 — Publish immutable version (P1)

**As** a user of this feature, **I want** to publish an immutable version, **so that** I can maintain Git pipelines, auditing, immutable versions and safe deployment of workflows/scripts across environments.

**Why P1:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** a validated draft; **When** the author publishes workflow v3; **Then** the IR/deps digest is fixed and new runs can use v3.

### User Story 2 — Keep old run (P1)

**As** a user of this feature, **I want** to keep the old run, **so that** I can maintain Git pipelines, auditing, immutable versions and safe deployment of workflows/scripts across environments.

**Why P1:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** run v2 is waiting for an event; **When** v3 is published; **Then** run v2 resumes with v2 semantics/artifacts.

### User Story 3 — Promote version across environments (P1)

**As** a user of this feature, **I want** to promote a version across environments, **so that** I can maintain Git pipelines, auditing, immutable versions and safe deployment of workflows/scripts across environments.

**Why P1:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** a manifest versioned in Git; **When** the pipeline promotes dev→stage→prod; **Then** history, approvals and dependencies remain traceable.

### User Story 4 — Perform safe rollback (P1)

**As** a user of this feature, **I want** to perform a safe rollback, **so that** I can maintain Git pipelines, auditing, immutable versions and safe deployment of workflows/scripts across environments.

**Why P1:** independent, verifiable value requirement; supports the M4–M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** version v4 shows a regression; **When** the operator switches the active pointer to v3; **Then** new runs use v3 without rewriting v4's history.

## Functional Requirements

- **FR-011-001**: The system MUST implement WorkflowDraft, ImmutableWorkflowVersion, immutable ScriptVersion and dependency digests.
- **FR-011-002**: The system MUST create a declarative Git-friendly manifest and a CLI for validate, diff, export, publish, promote and rollback.
- **FR-011-003**: The system MUST apply optimistic concurrency control on drafts and approval policies for prod publish.
- **FR-011-004**: The system MUST allow versioned routing by namespace and event trigger with compatibility rules.
- **FR-011-005**: The system MUST record an audit trail of authorship, review, approval, publisher, digest and reason.
- **FR-011-006**: The system MUST block removal of artifacts used by active Runs until a safe retention policy is in place.

## Edge Cases

- draft edit conflict.
- rollback during a Run.
- artifact changed under the same name.
- offline dependency disappeared.
- duplicate trigger after promotion.

## Mandatory cross-cutting requirements

- **NFR-011-SEC:** tenant isolation, RBAC, least privilege, secrets never in logs or in the journal.
- **NFR-011-DUR:** after commit, loss of a worker/broker cannot lose a transition; use lease/fencing where applicable.
- **NFR-011-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-011-COMP:** versioned protocol and schemas; do not break in-flight runs.
- **NFR-011-PERF:** Asynchronous publishing in builds; the active pointer switch must be transactional.

## Success Criteria (measurable)

- **SC-011-001:** V2→V3 with a suspended V2 run resumes correctly; rollback only affects new runs.
- **SC-011-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-011-003:** no violation of the Constitution gates and no silent loss of persisted state.
- **SC-011-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

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

Requirements `FR-011-###` and criteria `SC-011-###` must be mapped to tasks and tests in `tasks.md` before each implementation starts.
