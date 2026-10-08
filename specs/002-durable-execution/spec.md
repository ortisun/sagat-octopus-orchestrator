# Feature Specification: Durable execution engine and scheduler

**Feature Branch:** `002-durable-execution`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Ensure DAG progress, checkpoints, suspension, retries and failure recovery without re-executing commits.

**Priority:** M1. **Dependencies:** 001.

## User Scenarios & Testing (mandatory)

### User Story 1 — Execute a DAG and persist checkpoints (P1)

**As** a user of this feature, **I want** to execute a DAG and persist checkpoints, **so that** I can ensure DAG progress, checkpoints, suspension, retries and failure recovery without re-executing commits.

**Why P1:** independent, verifiable value requirement; supports the M1 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** a published definition with A→B; **When** the client starts the run; **Then** A and B are executed, and checkpoints and journal record progress.

### User Story 2 — Recover after a worker crash (P1)

**As** a user of this feature, **I want** to recover after a worker crash, **so that** I can ensure DAG progress, checkpoints, suspension, retries and failure recovery without re-executing commits.

**Why P1:** independent, verifiable value requirement; supports the M1 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** a running step and a registered lease; **When** the worker dies and the lease expires; **Then** another worker resumes without reusing the stale generation.

### User Story 3 — Wait without consuming compute (P1)

**As** a user of this feature, **I want** to wait without consuming compute, **so that** I can ensure DAG progress, checkpoints, suspension, retries and failure recovery without re-executing commits.

**Why P1:** independent, verifiable value requirement; supports the M1 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** a Run enters a one-hour timer; **When** no worker remains allocated; **Then** the scheduler creates a new task when the timer expires.

### User Story 4 — View a redelivered attempt (P1)

**As** a user of this feature, **I want** to view a redelivered attempt, **so that** I can ensure DAG progress, checkpoints, suspension, retries and failure recovery without re-executing commits.

**Why P1:** independent, verifiable value requirement; supports the M1 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** NATS delivers a duplicate task; **When** two workers contend for the claim; **Then** only one generation and commit are accepted.

## Functional Requirements

- **FR-002-001**: The system MUST interpret versioned Workflow IR with Activity, Condition, Parallel, Join, Timer, WaitEvent and ChildWorkflow (the latter may be progressive).
- **FR-002-002**: The system MUST maintain an append-only journal sequenced per Run with transitions verified by CAS.
- **FR-002-003**: The system MUST write step result, journal, new state, ready tasks, timers/subscriptions and outbox in the same transaction.
- **FR-002-004**: The system MUST use leases, heartbeat, generation/fencing and expiration with safe requeue.
- **FR-002-005**: The system MUST implement exponential backoff with jitter, max attempts, timeouts, deadline and explicit cancellation.
- **FR-002-006**: The system MUST add a reconciler for the outbox, orphaned READY tasks and expired leases.
- **FR-002-007**: The system MUST allow Paused/Waiting without a dedicated process and without a fixed NATS consumer per Run.
- **FR-002-008**: The system MUST bind all runs to the immutable version and control payload/result size via blob refs.
- **FR-002-009**: The system MUST persist the child-run parent link and the cancellation propagation policy.

## Edge Cases

- crash after external effect and before the checkpoint.
- double complete with the same idempotency_key.
- stale complete after a new lease.
- race between timer and cancellation.
- invalid loop/unbounded fanout.
- fanout re-executed after restart.

## Mandatory cross-cutting requirements

- **NFR-002-SEC:** tenant isolation, RBAC, least privilege, secret never in logs or in the journal.
- **NFR-002-DUR:** after commit, loss of a worker/broker cannot lose a transition; use lease/fencing where applicable.
- **NFR-002-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-002-COMP:** versioned protocol and schemas; do not break in-progress runs.
- **NFR-002-PERF:** Benchmark of at least 10k suspended runs and scheduling latency; 100k/1M as later experiments.

## Success Criteria (measurable)

- **SC-002-001:** Deterministic crash test at each point of claim→effect→commit→publish, with an intact journal and no double transition.
- **SC-002-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-002-003:** no violation of the Constitution gates and no silent loss of persisted state.
- **SC-002-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

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

Requirements `FR-002-###` and criteria `SC-002-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation.
