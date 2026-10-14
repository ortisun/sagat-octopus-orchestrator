# ADR-0025 — Outbound brokers share the durable delivery pipeline

Status: **Accepted** | Date: 2026-10-12 | Feature: 004

Builds on ADR-0002 (PostgreSQL is authoritative), ADR-0004 (external effects are at least once)
and ADR-0012 (outgoing events are durable delivery rows). It closes G77 without changing the
inbound connector or webhook authentication decisions.

## Context

Feature 004 already writes a delivery in the transaction that records `run.finished`, then claims,
sends, records attempts, retries, sets aside and redrives outbound webhooks. FR-004-005 also requires
outbound NATS, Kafka and SQS adapters. A separate broker outbox would duplicate the transaction,
lease, fencing, rate, backlog, dead-letter and audit rules and could let the four destinations drift.

Publishing to a broker cannot be exactly once. A process can receive the broker acknowledgement and
die before recording it. The next process has to publish again. Broker deduplication windows help,
but do not replace a stable application idempotency key.

## Decision

1. **A subscription has one typed destination.** `WEBHOOK`, `NATS`, `KAFKA` and `SQS` are variants
   of the outbound subscription aggregate. Settings contain addresses, topic, subject, queue and
   region, never credential values. Credentials remain an optional namespace secret binding.
   Existing webhook APIs remain compatible; broker subscriptions have additive management routes.
2. **One transaction creates every obligation.** The transaction that records an outbound event
   inserts one delivery for every active matching subscription, whatever its destination. The
   `(subscription, event_id)` key makes repeated fanout idempotent. A commit therefore contains both
   the fact and every delivery it owed, or neither.
3. **One worker owns rate, lease, retry and dead-letter semantics.** All destination kinds use the
   existing per-subscription rate claim, delivery lease and attempt-number fence. The attempt, next
   delivery state, subscription health and an automatic-disable audit record commit together.
   Twelve failed attempts make a delivery `DEAD`; it remains visible and redrivable. Twenty
   consecutive dead deliveries or 10,000 pending deliveries disables the subscription.
4. **A broker acknowledgement is the success boundary.** NATS succeeds after a JetStream PubAck,
   Kafka after the produce acknowledgement and assigned offset, and SQS after a successful
   `SendMessage` answer. A timeout, broken connection, unavailable credential, policy denial or any
   ambiguous outcome is retried. Known-invalid settings are rejected by management before storage;
   a broker-side rejection is retried to permit repair, then becomes `DEAD` at the attempt limit.
5. **The delivery id is stable across every attempt.** Each broker message carries `eventId`,
   `eventType`, `deliveryId`, `attempt`, `schemaVersion` and optional `traceparent`; the body is the
   same versioned event JSON used by webhooks. NATS also sends `Nats-Msg-Id: deliveryId`. Kafka uses
   `deliveryId` as its key and chooses a partition by a stable hash over the current partition set.
   Adding partitions can change that choice for a later retry, so no cross-repartition ordering is
   promised. SQS FIFO uses `deliveryId` as `MessageDeduplicationId` and the subscription id as its
   default `MessageGroupId`; standard queues remain duplicate-capable.
6. **Bounds and egress policy are checked before every effect.** Payload and metadata sizes are
   bounded by the contract. HTTP and SQS use the guarded HTTP client. NATS and Kafka apply the
   broker guard before their libraries connect, with G76's documented second-resolution limitation.
   Credentials are never sent over plaintext, persisted in settings, logged or labeled.
7. **The adapter boundary remains strict.** Domain code owns typed settings and retry verdicts;
   application code owns store and sender ports; PostgreSQL owns durable state; the scheduler owns
   the worker; broker clients and credential parsing live only in `infra-connectors`.
8. **Historical table names remain physical implementation detail.** The additive migration adds a
   destination discriminator and settings to the existing `webhook_subscriptions`,
   `webhook_deliveries` and `webhook_attempts` tables. Keeping those names avoids a high-risk rename;
   code and public contracts treat them as generic outbound rows. The down migration refuses while
   broker rows exist, rather than deleting customer deliveries.

## Verification gates

Real NATS JetStream, Apache Kafka and ElasticMQ tests must prove the published body and metadata,
acknowledgement before `DELIVERED`, retry with the same delivery id after an acknowledgement whose
database record is lost, restart recovery, tenant isolation, egress denial, bounded retry to `DEAD`
and redrive. Contract compatibility, forward/rollback migration, webhook regressions, fixed-label
metrics and secret-free logs are mandatory. A mock publisher alone is not evidence for G77.

## Consequences

At-least-once delivery is explicit. Consumers that need one logical effect keep `deliveryId` or use
their broker's idempotency facility. The shared pipeline makes webhook and broker recovery identical,
but the historical PostgreSQL names are less descriptive. A future table rename may be operational
cleanup; it does not alter correctness and is outside this change.

## Acknowledgement hardening evidence

The publish deadline covers guard/DNS resolution, client construction, network I/O and broker
acknowledgement. NATS rejection logs use the closed error category, never the broker-controlled
description. SQS accepts only a bounded 64 KiB Query success envelope containing exactly one
nonempty direct MessageId; malformed XML, HTML, error envelopes and arbitrary HTTP 2xx are rejected.
The adapter suite passes 11 tests. Real JetStream error-canary, false SQS success, stalled-guard
cancellation and NATS/Kafka/ElasticMQ positive integration tests pass; independent review found
and verified closure of the malformed-envelope edge cases.
