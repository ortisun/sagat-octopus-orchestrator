# Tasks: Triggers, webhooks, messaging and correlation

**Input:** `specs/004-events-webhooks-messaging/spec.md`, `plan.md`, `research.md`, `data-model.md`, 001 contracts.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [ ] T004-001 Check FR-004-001..FR-004-008, SCs and ADRs; record gaps in `specs/004-events-webhooks-messaging/research.md`.
- [ ] T004-002 [P] Define fixtures and contract validation in `tests/contract/004-events-webhooks-messaging/fixtures/`.
- [ ] T004-003 [P] Document types, transitions and invariants in `specs/004-events-webhooks-messaging/data-model.md`.

## Phase 2: Foundation
- [ ] T004-004 Create the necessary domain contracts/interfaces in `crates/events/src/inbox.rs` and invariant tests.
- [ ] T004-005 Implement ports/use cases in the appropriate module based on `specs/004-events-webhooks-messaging/plan.md`.
- [ ] T004-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [ ] T004-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/004-events-webhooks-messaging/`.

## Phase 3: US01 — Start a workflow via webhook (P1)
**Independent test:** Given an endpoint with HMAC configured; When a provider sends a valid event; Then the inbox confirms 202 and a Run is created exactly-once as a logical admission.
- [ ] T004-008 [P] [US01] Write Given/When/Then test for starting a workflow via webhook in `tests/e2e/004-events-webhooks-messaging/us01.rs`.
- [ ] T004-009 [US01] Implement domain path and error handling for starting a workflow via webhook in `crates/events/src/inbox.rs`.
- [ ] T004-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T004-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`.
- [ ] T004-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/004-events-webhooks-messaging/`.

## Phase 4: US02 — Wake a suspended Run on an event (P1)
**Independent test:** Given a persisted waitForEvent subscription; When the correlated event arrives; Then the scheduler releases the Step once and stores the evidence.
- [ ] T004-013 [P] [US02] Write Given/When/Then test for waking a suspended run on an event in `tests/e2e/004-events-webhooks-messaging/us02.rs`.
- [ ] T004-014 [US02] Implement domain path and error handling for waking a suspended run on an event in `crates/events/src/matcher.rs`.
- [ ] T004-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T004-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`.
- [ ] T004-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/004-events-webhooks-messaging/`.

## Phase 5: US03 — Integrate Kafka and SQS (P1)
**Independent test:** Given an authenticated adapter; When messages are redelivered and out of order; Then inbox dedupe, DLQ and offset checkpoints work.
- [ ] T004-018 [P] [US03] Write Given/When/Then test for integrating Kafka and SQS in `tests/e2e/004-events-webhooks-messaging/us03.rs`.
- [ ] T004-019 [US03] Implement domain path and error handling for integrating Kafka and SQS in `crates/connectors/src/webhook.rs`.
- [ ] T004-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T004-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`.
- [ ] T004-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/004-events-webhooks-messaging/`.

## Phase 6: US04 — Deliver an outbound webhook (P1)
**Independent test:** Given a completed run event; When an endpoint responds with a temporary 503; Then the system retries with backoff and records all attempts.
- [ ] T004-023 [P] [US04] Write Given/When/Then test for delivering an outbound webhook in `tests/e2e/004-events-webhooks-messaging/us04.rs`.
- [ ] T004-024 [US04] Implement domain path and error handling for delivering an outbound webhook in `crates/connectors/src/kafka.rs`.
- [ ] T004-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T004-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`.
- [ ] T004-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/004-events-webhooks-messaging/`.

## Phase: Engineering design details (story-linked hardening)

- [ ] T004-028 [US01] Persist webhook inbox with unique tenant+source+event_id in `crates/events/src/inbox.rs`.
- [ ] T004-029 [US01] Apply signature verification + clock skew/nonce in `crates/connectors/src/webhook.rs`.
- [ ] T004-030 [US01] Test timer x signal race under `SELECT FOR UPDATE` in `tests/fault/004-events-webhooks-messaging/`.
- [ ] T004-031 [US01] Implement Kafka/SQS consumers with offset/delete only after persistence in `crates/connectors/src/`.

## Final Phase: Hardening / rollout
- [ ] T004-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/004-events-webhooks-messaging/`.
- [ ] T004-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
- [ ] T004-034 Run the benchmark defined in `specs/004-events-webhooks-messaging/plan.md` and publish the report in `tests/performance/reports/004-events-webhooks-messaging.md`.
- [ ] T004-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/004-events-webhooks-messaging/quickstart.md`.
- [ ] T004-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: 001, 002.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
