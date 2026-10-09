# Persistence, messaging and connectors

## PostgreSQL V1: single state authority

- Transactions with `READ COMMITTED` + row lock/CAS for the execution of a Run; raise isolation on invariants that require it (with benchmark).
- Partition `run_events` by time and `tenant_id` according to measured volume; preserve index `(tenant_id,run_id,seq)`.
- Outbox in the same transaction as the state change. Dispatcher with leasing, retry and message dedupe.
- Periodic reconciler detects READY without dispatch, expired LEASED and stuck outbox.
- Isolated connection pools for API/read vs engine/write. Read model projects events for the Studio.

## NATS JetStream V1

- Small messages containing `tenant_id`, `task_id`, `generation`, `shard`, `traceparent` and deadline; no secrets.
- JetStream durable consumers with explicit ack; ack after claim/execution per protocol. Messages may be duplicated.
- NATS confirmation never replaces the PostgreSQL commit.
- Subject per worker class/capability, not an unbounded subject per run. Backpressure and DLQ with visibility.

## Inbound events

- HTTP ingress (webhook): verify HMAC/OIDC/mTLS per connector, timestamp, replay nonce, allowlist, schema, max payload; respond `202` after persisting to the inbox.
- An event has `source`, `external_event_id`, `tenant_id`, `namespace`, `subject`, `event_type`, `schema_version`, `occurred_at`, `received_at`, `correlation`, `payload_ref`, `trace_context`.
- Inbox unique `(tenant_id, source, external_event_id)`, with dedupe/TTL policy; a duplicate responds with idempotent success.
- The matcher queries subscriptions by `(tenant,event_type,correlation_key)` and chooses wakeups with transactional lock.
- An out-of-order event gets policy `reject`, `buffer_with_ttl`, `process`; the policy is specific per trigger.

## Egress

- Outbound webhook: signed HTTP, timeout, retries with jitter, delivery status and endpoint disable/quarantine; 2xx confirms, 429/5xx allow retry, 4xx permanent configurable.
- Kafka: consume→inbox and outbox→produce adapter; offset commits after persistence in the inbox. Exactly-once between DB and Kafka is not assumed.
- SQS: long-poll, renewable visibility timeout, delete after persistence; DLQ and redrive.
- NATS: inbound/outbound adapters separate from the internal bus and with subject policies.
- HTTP action: egress proxy, retries only when idempotent/configured, TLS policy.

## Correlation

- `waitForEvent(type, key, timeout)`: registration/lease confirmed BEFORE the side effect that triggers the response; if not feasible, inbox buffer with dedupe and lookup of previously received events according to a window.
- `signal(runId, signalName, idempotencyKey)`: direct addressing and authentication by namespace.
- `trigger(event)`: fanout to activated definitions (per-tenant limits; restricted CEL filter; trigger version fixed at the moment of admission).

## Outbox / inbox contract

`event_key`, `tenant_id`, `aggregate_id`, `event_type`, `payload_ref`, `schema_version`, `created_at`, `delivered_at`, `attempt_count`, `next_attempt_at`.

Invariants: no cross-tenant correlation; at-least-once delivery; no silent loss; bounded and observable retries.

## As built (feature 004)

What `docs/runbooks/events.md` operates and `specs/004-events-webhooks-messaging/research.md` records. Differences from the text above:

- The inbox key is `(tenant, namespace, source, event_id)`, not `(tenant, source, external_event_id)`: two namespaces of a tenant may each have a source of the same name. The payload is kept inline for the deduplication window (a dedupe row that outlived its payload would not be able to tell a retry from a reuse).
- A webhook is verified against the three newest versions of its secret; the signature covers the envelope (type, subject, correlation key) as well as the body. A duplicate responds `202` with `DUPLICATE`; the same id with another payload is `409`.
- Matching is asynchronous: ingress answers `202` when the row is committed, and a dispatcher starts runs and wakes waits, recording each effect with its delivery in one transaction. An out-of-order event has one policy, `process`: a wait takes the first event with its key that reaches it, and may ask to look back (`lookbackMs`) at events that arrived before it (G74, G89).
- Kafka is read without consumer groups: positions are rows, partitions are leased (G81). SQS and NATS are acknowledged after the inbox commit. The "outbox→produce" adapters for Kafka, SQS and NATS are not built (G77); outgoing events go to HTTP subscribers.
- Outbound webhooks: 12 attempts over about three days with jitter, `Retry-After` honoured, 4xx set aside as `DEAD` (not configurable), redrive, switch-off after 20 dead in a row or 10 000 waiting.
