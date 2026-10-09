# Data Model: Triggers, webhooks, messaging and correlation

## Context

Receive and send events with dedupe, persistence and delivery controls, waking durable workflows correctly.

Data ownership follows the bounded contexts of `docs/architecture/overview.md`, with the reference physical schema in `specs/001-platform-foundation/data-model.md` and illustrative SQL in `contracts/schema.sql`.

## Entities and observable state

- **Definition/Version/Artifact:** immutable after publication; tenant and namespace required.
- **Run/Step/Attempt:** stable identifiers, version, status, timestamps, deadlines, parent/correlation.
- **JournalEvent:** append-only seq per Run, event_type, payload_ref, causation/correlation and actor.
- **Inbox/Outbox:** uniqueness for dedupe and delivery; next_attempt_at; last_error without PII.
- **Lease/Subscription/Timer:** fencing generation, validity, deadline and transition status.
- **SagaEntry/Compensation:** state, dependency order, idempotency key, reason and outcome.

## Invariants

- Every tenant-scoped record validates the security context.
- No in-place update of a published version.
- Every operational state change records a sequenced event and information for reconstruction.
- Large payloads go to the blob store with digest and non-sensitive references.

## Migrations

Additive migrations first; expand/contract for existing schemas; `CONCURRENTLY` indexes where applicable on large databases; idempotent and measured backfill. See the feature plan and ADRs.

## As built (feature 004)

Migration `crates/infra-postgres/migrations/20261012000000_events.up.sql` (and `.down.sql`): additive, reversible, checked by the rollback test of feature 001. Every table carries `tenant_id`; the ones that belong to a namespace carry a composite foreign key `(tenant_id, namespace_id) → namespaces`. Nothing here stores a secret value: endpoints, subscriptions and connectors hold the **name** of a secret of the namespace, opened with the key ring when it is used.

### Tables

| Table | One row per | Key | What matters |
|---|---|---|---|
| `event_endpoints` | place a provider posts to | `id` (random UUID: it is the URL), unique `(tenant, namespace, slug)` | `secret_name`, `default_event_type`, `dedupe_ttl_seconds` (60 s to 30 days), `skew_seconds` (1 s to 1 h), `max_body_bytes` (1 KiB to 240 KiB), `rate_per_second` (1 to 10 000), `status` `ACTIVE`/`PAUSED`, `profile` (`HMAC` only: G75) |
| `inbox_events` | event admitted | `(tenant, namespace, source, event_id)` — **the deduplication** | `payload` inline (≤ 240 KiB) and its SHA-256 `payload_digest` (separates a retry from a reuse), `received_at`, `expires_at` (window end, database clock), `status` `PENDING`/`DISPATCHED`/`DEAD`, `attempts`, `next_attempt_at` (the dispatcher's lease and backoff), `last_error` |
| `event_deliveries` | thing an event started or woke | `(tenant, namespace, source, event_id, consumer)` | `consumer` is `trigger:{id}` or `wait:{run}/{node}`; written in the **same transaction** as the run created or the step woken, so a repeat finds it and does nothing; cascades with the event |
| `workflow_triggers` | trigger of the active version of a workflow | `id` = hash of workflow and what the trigger is about (stable across versions; the filter is not part of it) | `kind` `event`/`webhook`/`message`/`cron`, `match_source`, `match_type`, `filter`, `cron`, `timezone`, `next_fire_at`, `last_fire_at` |
| `steps` (columns added) | — | — | `event_since` (wait start − `lookbackMs`), `event_scanned_at` (the inbox has been searched for this wait) |
| `webhook_subscriptions` | receiver of outgoing events | `id`, unique `(tenant, namespace, name)` | `url`, `secret_name`, `kinds`, `rate_per_second` (1 to 100), `timeout_ms`, `status` `ACTIVE`/`PAUSED`/`DISABLED`, `consecutive_dead`, `disabled_reason`, `next_slot_at` (the cluster-wide rate gate) |
| `webhook_deliveries` | event owed to a subscription | `id`, unique `(subscription, event_id)` | `payload` (the exact bytes signed and sent, ≤ 64 KiB), `status` `PENDING`/`DELIVERED`/`DEAD`, `attempts`, `attempts_before_redrive`, `next_attempt_at`, last outcome and HTTP status. **Never the receiver's answer** |
| `webhook_attempts` | attempt | `(delivery, attempt)` | outcome (`DELIVERED`, `REJECTED`, `SERVER_ERROR`, `THROTTLED`, `TIMEOUT`, `NETWORK`, `DENIED`, `UNSIGNED`), status, duration, a class of failure written by the platform |
| `event_connectors` | broker reader | `id`, unique `(tenant, namespace, slug)` | `kind` `NATS`/`KAFKA`/`SQS`, `settings` (no secret), `secret_name`, `default_event_type`, `rate_per_second`, `status`, health (`last_error`, `last_message_at`, counters) |
| `connector_offsets` | stream (Kafka partition) | `(connector, stream)` | `next_offset`, moved in the transaction that admits the message (compare-and-set) |
| `connector_leases` | stream | `(connector, stream)` | `holder`, `expires_at`: only the holder reads it |
| `connector_dead_letters` | message that could not become an event | `id`, unique `(connector, position)` | `reason` (`NO_EVENT_TYPE`, `BAD_ENVELOPE`, `NOT_A_JSON_OBJECT`, `TOO_LARGE`, `EVENT_ID_REUSED`), headers, body (≤ 64 KiB, `truncated`) |
| `quota_limits`, `quota_ceilings` (constraints widened) | — | — | `event_endpoints`, `webhook_subscriptions`, `event_connectors` |

The source of an event is the slug of an endpoint or of a connector, `api` or `cron`; the last two are reserved, and a slug is unique across the endpoints and connectors of a namespace (checked when one is created).

### State machines

```
inbox event:      PENDING --dispatched--> DISPATCHED --window ends--> (purged)
                     |  ^                        
                     |  '-- failure: attempts+1, next_attempt_at = backoff (1 s doubling to 10 min, +25 % jitter)
                     '--- 12 failures --> DEAD --window ends--> (purged)
                  (a PENDING event is never purged with its window)

delivery:         PENDING --2xx--> DELIVERED --retention (14 d)--> (purged)
                     |  '-- 408/425/429/5xx/timeout/network/denied: attempts+1, next = schedule (5 s .. 24 h) or Retry-After
                     '--- 4xx/3xx, or 12 attempts --> DEAD --redrive--> PENDING (attempt numbers continue)

subscription:     ACTIVE <--> PAUSED
                  ACTIVE --20 DEAD in a row--> DISABLED --set ACTIVE--> ACTIVE (failures forgotten, DEAD redriven)

connector:        ACTIVE <--> PAUSED   (a reader runs per ACTIVE connector; Kafka: per leased partition)

cron trigger:     next_fire_at --fire (run started under key cron-{epoch})--> CAS next_fire_at to the next tick after now
```

### Invariants (each has a test; see `docs/traceability.md`)

- **One inbox row per `(tenant, namespace, source, event id)`**; the same id with the same digest is `DUPLICATE`, with another digest is `EVENT_ID_REUSED` and writes nothing. A window never ends while the event is `PENDING`.
- **A dedupe window is at least twice the signature skew**, so a captured request cannot be replayed as a new event inside the time it would still verify.
- **An effect and its record are one transaction**: a run created for `trigger:{id}` / a step woken for `wait:{run}/{node}` and the `event_deliveries` row. Redispatching an event repeats nothing that was recorded.
- **A wait receives an event at most once**, and never one from before `event_since`. An event that arrives while the matcher was still looking is found from the other side (`event_scanned_at`).
- **A delivery row for each subscription that wants an event is inserted in the transaction that commits `run.finished`**: no committed event without its deliveries, no delivery of an event that was rolled back.
- **A subscription is offered to one worker at a time, at most its rate**: `next_slot_at` moves by `taken / rate` seconds with each claim.
- **The position of a Kafka partition moves with the message that advanced it** (compare-and-set on the expected offset); a consumer that no longer holds the lease cannot move it.
- **A dead letter exists once per broker position**; at most 1 000 per connector.
- **No row, log line, metric label or audit record holds a payload, a signature, a secret or a receiver's answer.** Errors written by the platform are classes (`last_error` on an event is 500 characters of the platform's own text).

### Locks

Admission takes no row lock outside its own insert (`ON CONFLICT DO NOTHING`, then a read). The dispatcher claims events with `FOR UPDATE SKIP LOCKED`; a wake locks the run (the order of feature 002: run, then steps) and then writes the delivery. The delivery worker locks a subscription row (`FOR UPDATE SKIP LOCKED`) to take its slot. Nothing takes a lock on `inbox_events` and a run in the opposite order, so there is no cycle.
