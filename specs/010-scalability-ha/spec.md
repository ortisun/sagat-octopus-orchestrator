# Feature Specification: Performance, scalability and high availability

**Feature Branch:** `010-scalability-ha`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Scale scheduler, workers and millions of waits without state loss, and quantify the real limits of the architecture.

**Priority:** M5. **Dependencies:** 001, 002, 003, 004, 005, 006, 008, 009.

## User Scenarios & Testing (mandatory)

### User Story 1 — Scale workers horizontally (P1)

**As** a user of this feature, **I want** to scale workers horizontally, **so that** I can scale scheduler, workers and millions of waits without state loss, and quantify the real limits of the architecture.

**Why P1:** independent, verifiable value requirement; supports the M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** the queue grows and quota allows; **When** worker pools scale; **Then** throughput increases and no invalid claim exists.

### User Story 2 — Sustain wait volume (P1)

**As** a user of this feature, **I want** to sustain wait volume, **so that** I can scale scheduler, workers and millions of waits without state loss, and quantify the real limits of the architecture.

**Why P1:** independent, verifiable value requirement; supports the M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** 100k runs await timers/events; **When** idle workers are drained; **Then** processes are not dedicated per run.

### User Story 3 — Survive an engine node failure (P1)

**As** a user of this feature, **I want** to survive an engine node failure, **so that** I can scale scheduler, workers and millions of waits without state loss, and quantify the real limits of the architecture.

**Why P1:** independent, verifiable value requirement; supports the M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** a scheduler shard owner goes down; **When** another takes over the lease; **Then** timers and work are resumed without inconsistency.

### User Story 4 — Isolate noisy neighbors (P1)

**As** a user of this feature, **I want** to isolate noisy neighbors, **so that** I can scale scheduler, workers and millions of waits without state loss, and quantify the real limits of the architecture.

**Why P1:** independent, verifiable value requirement; supports the M5 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** tenant A fires a burst; **When** tenant B has ready tasks; **Then** fairness and quotas prevent starvation beyond an explicit budget.

## Functional Requirements

- **FR-010-001**: The system MUST implement partition ownership with lease and fencing for the scheduler per namespace/shard.
- **FR-010-002**: The system MUST offer quotas per tenant/queue, weighted round robin fairness or a measurable policy.
- **FR-010-003**: The system MUST instrument and optimize journal queries, timer wheel/buckets, outbox and projections.
- **FR-010-004**: The system MUST support worker pools by language/capabilities and autoscaling by queue lag.
- **FR-010-005**: The system MUST run chaos/fault tests for DB/NATS/worker/shard failover with reports.
- **FR-010-006**: The system MUST build a load harness, p50/p95/p99 reports and a hardware profile for 10k/100k/1M waits and 100/1000/5000 step transitions/s.
- **FR-010-007**: The system MUST define backup, PITR, restore drills, RPO/RTO and a DR plan for single-region HA.

## Edge Cases

- shard split-brain.
- failover with stale outbox.
- hot partitions by correlation.
- slow NATS consumer.
- DB disk full.
- backpressure on external provider.
- scheduler starvation.

## Mandatory cross-cutting requirements

- **NFR-010-SEC:** tenant isolation, RBAC, least privilege, secret never in logs or in the journal.
- **NFR-010-DUR:** after commit, loss of worker/broker must not lose a transition; use lease/fencing where applicable.
- **NFR-010-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-010-COMP:** versioned protocol and schemas; do not break in-flight runs.
- **NFR-010-PERF:** Quantifiable and reproducible targets; distinguish wait capacity, active throughput and latency under backlog.

## Success Criteria (measurable)

- **SC-010-001:** Performance reports with hardware/configuration, zero confirmed Run loss in fault injection and validated DR restore.
- **SC-010-002:** 100% of the US01..US04 scenarios pass in a reproducible automated suite.
- **SC-010-003:** no violation of the Constitution's gates and no silent loss of persisted state.
- **SC-010-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

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

Requirements `FR-010-###` and criteria `SC-010-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation.
