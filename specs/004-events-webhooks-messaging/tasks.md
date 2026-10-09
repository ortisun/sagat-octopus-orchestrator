# Tasks: Triggers, webhooks, messaging and correlation

**Input:** `specs/004-events-webhooks-messaging/spec.md`, `plan.md`, `research.md`, `data-model.md`, 001 contracts.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [x] T004-001 Check FR-004-001..FR-004-008, SCs and ADRs; record gaps in `specs/004-events-webhooks-messaging/research.md`. — gaps G73–G95 and decisions D78–D94 in `research.md`; the review of the implementation is in `quickstart.md` and D94.
- [x] T004-002 [P] Define fixtures and contract validation in `tests/contract/004-events-webhooks-messaging/fixtures/`. — there is no `fixtures/` directory: the bodies of the requests are built inline from the signing scheme and validated against the OpenAPI and AsyncAPI documents (`tests/contract/004-events-webhooks-messaging/{rest,asyncapi}/`, with the compatibility baseline `events.baseline.json`).
- [x] T004-003 [P] Document types, transitions and invariants in `specs/004-events-webhooks-messaging/data-model.md`. — `data-model.md` (tables, state machines, invariants, locks).

## Phase 2: Foundation
- [x] T004-004 Create the necessary domain contracts/interfaces in `crates/events/src/inbox.rs` and invariant tests. — `crates/domain/src/{events,triggers,outbound,connectors}.rs` and `crates/application/src/signing.rs` with their invariant tests; there is no `crates/events` (G73, ADR-0012).
- [x] T004-005 Implement ports/use cases in the appropriate module based on `specs/004-events-webhooks-messaging/plan.md`. — `crates/application/src/{events,dispatch,outbound,connectors}.rs` (services and ports).
- [x] T004-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`. — `crates/infra-postgres/migrations/20261012000000_events.{up,down}.sql`: additive, reversible, checked by the rollback test of feature 001 (which counts it) and by the backfill test.
- [x] T004-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/004-events-webhooks-messaging/`. — `tests/integration/004-events-webhooks-messaging/` (84 tests, tenant scoping in every module).

## Phase 3: US01 — Start a workflow via webhook (P1)
**Independent test:** Given an endpoint with HMAC configured; When a provider sends a valid event; Then the inbox confirms 202 and a Run is created exactly-once as a logical admission.
- [x] T004-008 [P] [US01] Write Given/When/Then test for starting a workflow via webhook in `tests/e2e/004-events-webhooks-messaging/us01.rs`. — `us01.rs`.
- [x] T004-009 [US01] Implement domain path and error handling for starting a workflow via webhook in `crates/events/src/inbox.rs`. — `crates/domain/src/events.rs`, `crates/application/src/{events,signing}.rs` (there is no `crates/events/src/inbox.rs`, G73).
- [x] T004-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/events.rs`, `crates/infra-postgres/src/events.rs`, `crates/api-http/src/routes/events.rs`.
- [x] T004-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`. — counters taken after commit with closed labels (`events_received_total`, `webhook_rejected_total`, `events_dispatched_total`, `runs_started_total{source="trigger"}`), refusals audited once per endpoint and reason, no payload, signature or secret anywhere (`telemetry.rs`); `crates/telemetry` itself needed no change.
- [x] T004-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/004-events-webhooks-messaging/`. — `ingress::*`, `fault ingress::*`, `dispatch::events_never_cross_a_tenant_boundary`, the chaos and race tests above.

## Phase 4: US02 — Wake a suspended Run on an event (P1)
**Independent test:** Given a persisted waitForEvent subscription; When the correlated event arrives; Then the scheduler releases the Step once and stores the evidence.
- [x] T004-013 [P] [US02] Write Given/When/Then test for waking a suspended run on an event in `tests/e2e/004-events-webhooks-messaging/us02.rs`. — `us02.rs`.
- [x] T004-014 [US02] Implement domain path and error handling for waking a suspended run on an event in `crates/events/src/matcher.rs`. — the matcher is `crates/scheduler/src/events.rs` with the queries of `crates/infra-postgres/src/dispatch.rs` (there is no `crates/events/src/matcher.rs`, G73).
- [x] T004-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`. — `crates/infra-postgres/src/{dispatch,execution}.rs` (`record_delivery`, `event_since`, `event_scanned_at`) and `crates/application/src/execution.rs`.
- [x] T004-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`. — `event_wakeups_total{outcome}`, `trigger_events_total{outcome}`, `event_dispatch_lag_seconds`, `inbox_pending_events`; the wake is audited as `run.signalled` with the event named by id only.
- [x] T004-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/004-events-webhooks-messaging/`. — `dispatch::*`, `fault dispatch::*`.

## Phase 5: US03 — Integrate Kafka and SQS (P1)
**Independent test:** Given an authenticated adapter; When messages are redelivered and out of order; Then inbox dedupe, DLQ and offset checkpoints work.
- [x] T004-018 [P] [US03] Write Given/When/Then test for integrating Kafka and SQS in `tests/e2e/004-events-webhooks-messaging/us03.rs`. — `us03.rs`.
- [x] T004-019 [US03] Implement domain path and error handling for integrating Kafka and SQS in `crates/connectors/src/webhook.rs`. — `crates/infra-connectors/src/{ingest,nats,kafka,sqs,supervisor,credentials,guard}.rs` (there is no `crates/connectors/src/webhook.rs`; the outbound webhook is `crates/scheduler/src/deliveries.rs` with the guarded sender in `crates/infra-connectors/src/webhook.rs`, G73).
- [x] T004-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/connectors.rs`, `crates/infra-postgres/src/connectors.rs`, `crates/api-http/src/routes/connectors.rs`.
- [x] T004-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`. — `connector_messages_total`, `connector_failures_total`, `connectors_running`, `connector_dead_letters_dropped_total`; credentials are a secret of the namespace, redacted in `Debug`, never sent without TLS.
- [x] T004-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/004-events-webhooks-messaging/`. — `connectors::*`, `us03`, `fault_004`.

## Phase 6: US04 — Deliver an outbound webhook (P1)
**Independent test:** Given a completed run event; When an endpoint responds with a temporary 503; Then the system retries with backoff and records all attempts.
- [x] T004-023 [P] [US04] Write Given/When/Then test for delivering an outbound webhook in `tests/e2e/004-events-webhooks-messaging/us04.rs`. — `us04.rs`.
- [x] T004-024 [US04] Implement domain path and error handling for delivering an outbound webhook in `crates/connectors/src/kafka.rs`. — `crates/scheduler/src/deliveries.rs` and `crates/domain/src/outbound.rs` (not `kafka.rs`: the outbound webhook is HTTP; outbound broker producers are G77).
- [x] T004-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/outbound.rs`, `crates/infra-postgres/src/outbound.rs` (`fan_out` in the transaction of `run.finished`), `crates/api-http/src/routes/outbound.rs`.
- [x] T004-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`. — `webhook_attempts_total`, `webhook_deliveries_total`, `webhook_delivery_lag_seconds`, `webhook_pending_deliveries`, `webhook_subscriptions_switched_off_total`; the receiver's answer is never read or stored.
- [x] T004-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/004-events-webhooks-messaging/`. — `outbound::*`, `fault deliveries::*`, `sender::*`.

## Phase: Engineering design details (story-linked hardening)

- [x] T004-028 [US01] Persist webhook inbox with unique tenant+source+event_id in `crates/events/src/inbox.rs`. — the primary key of `inbox_events` and `admit_within` in `crates/infra-postgres/src/events.rs` (not `crates/events/src/inbox.rs`, G73).
- [x] T004-029 [US01] Apply signature verification + clock skew/nonce in `crates/connectors/src/webhook.rs`. — `crates/application/src/signing.rs` and `EventService::receive` (not `crates/connectors/src/webhook.rs`, G73): HMAC over the envelope, timestamp window, nonce as the event id with a replay window of twice the skew and a minute.
- [x] T004-030 [US01] Test timer x signal race under `SELECT FOR UPDATE` in `tests/fault/004-events-webhooks-messaging/`. — `dispatch::the_event_and_the_timeout_racing_resolve_to_exactly_one_of_them` and `fault dispatch::the_event_the_timeout_and_a_second_dispatcher_race_and_exactly_one_outcome_wins_every_time`.
- [x] T004-031 [US01] Implement Kafka/SQS consumers with offset/delete only after persistence in `crates/connectors/src/`. — `crates/infra-connectors/src/{kafka,sqs,nats,ingest}.rs`: position, delete and ack only after the inbox commit (not `crates/connectors/src/`, G73).

## Final Phase: Hardening / rollout
- [x] T004-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/004-events-webhooks-messaging/`. — `contract_rest_004`, `contract_asyncapi_004`; Redocly lint (valid, the 10 warnings of the baseline) and the AsyncAPI validation in CI.
- [x] T004-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`. — clippy `-D warnings` clean, `scripts/check-architecture.py` (16 crates, no cycle, the domain imports no adapter), `cargo audit`.
- [x] T004-034 Run the benchmark defined in `specs/004-events-webhooks-messaging/plan.md` and publish the report in `tests/performance/reports/004-events-webhooks-messaging.md`. — `tests/performance/004-events-webhooks-messaging/main.rs` and `tests/performance/reports/004-events-webhooks-messaging.md` (measured on a clean tree).
- [x] T004-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/004-events-webhooks-messaging/quickstart.md`. — two independent reviews (D94), `docs/runbooks/events.md`, `README.md`; evidence in `quickstart.md`.
- [x] T004-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`. — `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: 001, 002.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
