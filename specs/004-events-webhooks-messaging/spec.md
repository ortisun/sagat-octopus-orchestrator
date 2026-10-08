# Feature Specification: Triggers, webhooks, messaging and correlation

**Feature Branch:** `004-events-webhooks-messaging`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Receive and send events with dedupe, persistence and delivery controls, waking durable workflows correctly.

**Priority:** M2. **Dependencies:** 001, 002.

## User Scenarios & Testing (mandatory)

### User Story 1 — Start a workflow via webhook (P1)

**As** a user of this feature, **I want** to start a workflow via webhook, **so that** I can receive and send events with dedupe, persistence and delivery controls, waking durable workflows correctly.

**Why P1:** independent, verifiable value requirement; supports the M2 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** an endpoint with HMAC configured; **When** a provider sends a valid event; **Then** the inbox confirms 202 and a Run is created exactly-once as a logical admission.

### User Story 2 — Wake a suspended Run on an event (P1)

**As** a user of this feature, **I want** to wake a suspended run on an event, **so that** I can receive and send events with dedupe, persistence and delivery controls, waking durable workflows correctly.

**Why P1:** independent, verifiable value requirement; supports the M2 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** a persisted waitForEvent subscription; **When** the correlated event arrives; **Then** the scheduler releases the Step once and stores the evidence.

### User Story 3 — Integrate Kafka and SQS (P1)

**As** a user of this feature, **I want** to integrate Kafka and SQS, **so that** I can receive and send events with dedupe, persistence and delivery controls, waking durable workflows correctly.

**Why P1:** independent, verifiable value requirement; supports the M2 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** an authenticated adapter; **When** messages are redelivered and out of order; **Then** inbox dedupe, DLQ and offset checkpoints work.

### User Story 4 — Deliver an outbound webhook (P1)

**As** a user of this feature, **I want** to deliver an outbound webhook, **so that** I can receive and send events with dedupe, persistence and delivery controls, waking durable workflows correctly.

**Why P1:** independent, verifiable value requirement; supports the M2 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** a completed run event; **When** an endpoint responds with a temporary 503; **Then** the system retries with backoff and records all attempts.

## Functional Requirements

- **FR-004-001**: The system MUST implement manual/API, webhook, event, cron, message topic/queue and direct signal triggers.
- **FR-004-002**: The system MUST create a single inbox per tenant/source/event_id with configurable deduplication TTL.
- **FR-004-003**: The system MUST create event_type+correlation subscriptions with timeout and atomic competition between event and timer.
- **FR-004-004**: The system MUST validate HMAC/timestamp/nonce on webhooks and support OIDC/mTLS according to the connector profile.
- **FR-004-005**: The system MUST implement inbound/outbound adapters for NATS, Kafka and SQS with ack/offset after persistence.
- **FR-004-006**: The system MUST offer outbound HTTP webhook with signing, retry jitter, TLS, rate limit, DLQ, redrive, endpoint status.
- **FR-004-007**: The system MUST define versioned AsyncAPI for messages and a schema evolution policy.
- **FR-004-008**: The system MUST impose backpressure, size limits, retention, safe filters and quotas per tenant/connector.

## Edge Cases

- event arrives before the wait is registered.
- valid HMAC but repeated nonce.
- Kafka offset commit lost.
- timeout expires while the signal arrives.
- webhook returns 2xx but the connection dropped.
- delivery to multiple listeners.

## Mandatory cross-cutting requirements

- **NFR-004-SEC:** tenant isolation, RBAC, least privilege, secret never in logs or in the journal.
- **NFR-004-DUR:** after commit, loss of a worker/broker must not lose a transition; use lease/fencing where applicable.
- **NFR-004-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-004-COMP:** versioned protocol and schemas; do not break in-flight runs.
- **NFR-004-PERF:** Ingress p95 <300ms on persistence without heavy work; scales by partition/correlation key and quotas.

## Success Criteria (measurable)

- **SC-004-001:** Signed webhooks, duplicates, retries, out-of-order and event/timeout race in integration tests.
- **SC-004-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-004-003:** no violation of the Constitution gates and no silent loss of persisted state.
- **SC-004-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

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

Requirements `FR-004-###` and criteria `SC-004-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation.
