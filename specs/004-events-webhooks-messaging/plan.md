# Implementation Plan: Triggers, webhooks, messaging and correlation

**Branch:** `004-events-webhooks-messaging` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Receive and send events with dedupe, persistence and delivery controls, waking durable workflows correctly.

**Technical approach:** Event Gateway with Inbox/Outbox in Postgres; subscription matcher partitioned by tenant+type+key; external Kafka/SQS adapters with idempotent checkpoints.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** Ingress p95 <300ms on persistence without heavy work; scales by partition/correlation key and quotas.
- **Constraints:** single-region HA; state loss after ACK forbidden; side effects at-least-once; security-by-default.
- **Scale/Scope:** scale horizontally without keeping live processes for waits.

## Constitution Check

- [x] State and retry semantics documented (`I`).
- [x] Modularity/ports and adapters planned (`II`).
- [x] Saga and reversibility applied where relevant (`III`).
- [x] Compatibility and version pinning (`IV`).
- [x] Code/tenant boundary and secrets (`V`).
- [x] OTel and auditable journal (`VI`).
- [x] Redeliverable events and inbox/outbox (`VII`).
- [x] Fault injection/contract plan (`VIII`).

These checks indicate conformance **of the design**, not approval of the implementation.

## Project Structure

```text
crates/events/src/inbox.rs
crates/events/src/matcher.rs
crates/connectors/src/webhook.rs
crates/connectors/src/kafka.rs
crates/connectors/src/sqs.rs
crates/connectors/src/nats.rs
crates/api-http/src/routes/webhooks.rs
tests/contract/
tests/integration/
tests/fault/
tests/e2e/events-webhooks-messaging/
```

**As built.** The structure above was a proposal. The inbox and the matcher are not a crate: the rules are in the domain, the services in the application and the SQL in the persistence adapter, as for every other feature; the broker clients are adapters in a crate of their own (decision D78, ADR-0012, gap G73).

```text
crates/domain/src/{events,triggers,outbound,connectors}.rs   envelope, limits, verdicts, backoff, triggers, connector settings (no I/O)
crates/domain/src/workflow.rs                  triggers and the event wait (`lookbackMs`) in the definition
crates/application/src/signing.rs              HMAC-SHA256 `v1=` signing and verification, shared by in and out
crates/application/src/{events,dispatch,outbound,connectors}.rs   services and ports: ingress, publish, endpoints; the dispatcher's store; subscriptions and deliveries; connectors and their runtime port
crates/infra-postgres/src/{events,dispatch,outbound,connectors}.rs   inbox, dispatcher claims, deliveries, connector state
crates/infra-postgres/migrations/20261012000000_events.{up,down}.sql
crates/scheduler/src/{events,deliveries}.rs    the dispatcher (events -> runs and wakes, cron) and the delivery worker
crates/infra-connectors/                       NATS, Kafka (rskafka), SQS (SigV4 + XML) readers, the guarded sender, the supervisor
crates/infra-ir/src/cron.rs                    the calendar behind cron triggers (croner, chrono-tz)
crates/api-http/src/routes/{events,outbound,connectors}.rs   the public webhook route and the management routes
apps/execution-engine/src/{lib,loops,settings}.rs   the three loops (dispatcher, deliveries, connectors) and their settings
tests/{integration,e2e,fault,contract,performance}/004-events-webhooks-messaging/
```

Outbound broker producers extend the same subscription and delivery aggregate as webhooks (ADR-0025).
The physical `webhook_*` table names remain for migration compatibility, while a destination kind and
typed settings select webhook, NATS, Kafka or SQS. The delivery transaction, lease/fence, rate limit,
retry, dead-letter and redrive paths are shared; protocol clients remain in `infra-connectors`.

## Implementation phases

1. Domain contracts and types, invariant validation + tests.
2. Persistence and port adapters + migration and integration tests.
3. APIs/command processing + versioning + authorization and observability.
4. Happy path flow with e2e tests.
5. Fault injection, redelivery, timeouts, concurrency and regression.
6. Security/performance review, documentation and release gate.

## Data Model / Contracts

- [`data-model.md`](./data-model.md)
- Canonical contracts in `specs/001-platform-foundation/contracts/` with reviewed migrations.
- Applicable ADRs in `docs/adr/`.

## Risks & Mitigations

- **Functional risk:** event arrives before the wait is registered. Mitigate via adversarial scenario with fixture.
- **Operational risk:** throughput below target; perf profiling and scaling driven by metrics, never a promise without a benchmark.
- **Compatibility risk:** schema/SDK diverges; schema evolution and golden fixtures.
- **Security risk:** tenant/sandbox boundary; adversarial tests and fail closed.

## Complexity Tracking

| Decision | Cost | Justification |
|---|---|---|
| Rust modular monolith | Learning curve | Centralized invariants and gradual evolution |
| DB transactional outbox | Extra reads/writes | Avoid losing a wakeup after commit |
| Multi-SDK via proto | More CI and tests | Real parity and low coupling |

## Delivery Gate

Signed webhooks, duplicates, retries, out-of-order and event/timeout race in integration tests.

## Webhook identity profile completion

ADR-0026 adds deployment-approved OIDC verifiers and native mutual TLS client identity. Endpoint profiles are persisted and audited, and use the existing admission path. No endpoint-controlled discovery or forwarded certificate trust is introduced.


## Approved workflow-primitives dependency (2026-10-14)

[ADR-0030](../../docs/adr/ADR-0030-structured-instances-and-continuation-chains.md) and
[ADR-0031](../../docs/adr/ADR-0031-declared-operation-result-disclosure.md) accept the feature-014
architecture and separate declared-result disclosure policy.
[ADR-0032](../../docs/adr/ADR-0032-additive-workflow-ir.md) keeps the workflow IR at v1 and
supersedes the earlier separate-revision allocation. Inbox consumers, subscriptions, timers and outbox references may carry full scoped instance identity. Signals remain accepted only by a waiting step and consumed in the same transaction, with no buffering or implicit forwarding across continuation.

This approves the described dependency for implementation, not a runtime guarantee. Canonical
contracts, migration/replay, tenant/fault tests and applicable five-SDK evidence remain the
feature-014 release gates; existing feature behavior stays supported.
