# Contract: built-in signals of slice 1

A signal has a stable name, a unit, a scope, a freshness after which a sample is stale, and a documented meaning (FR-036-005). The catalogue is listed by `GET /scaling/catalogue` on the operator listener, and a test fails if a built-in signal lacks any of the four. Later slices add signals; a new inbound connector kind or transport must add its lag signals before it ships (FR-036-005, checked by a conformance test that every `ConnectorKind` implements `LagProbe`).

Freshness defaults to three sampling intervals (30 s at the default 10 s). A signal that cannot be measured is `UNAVAILABLE`, never 0.

## Ingestion (from `connector_lag` and `event_connectors`)

| Signal | Unit | Scope | Meaning |
|---|---|---|---|
| `ingest.lag.backlog` | messages | operator | sum of the backlog of connectors that are **drainable** (`BEHIND` or `CAUGHT_UP`) with fresh rows. Coverage is `Σ partitions_covered ÷ Σ partitions_total` of those connectors |
| `ingest.lag.backlog_all` | messages | operator | the same over connectors in every state; for a policy that wants the raw total |
| `ingest.lag.age_max` | seconds | operator | largest age of the oldest unread message among drainable connectors whose broker can state it (Kafka, NATS); `UNAVAILABLE` when none can |
| `ingest.lag.excluded_connectors` | count | operator | connectors in `BLOCKED` or `RATE_LIMITED`; explains why a high lag did not scale |
| `ingest.readers_ceiling` | count | operator | the ceiling of useful readers: the largest partition count among drainable Kafka connectors with a backlog; unbounded (absent) when a drainable NATS or SQS connector has one (`G1701`) |

## Events, outbox, deliveries

| Signal | Unit | Scope | Meaning |
|---|---|---|---|
| `events.pending` | count | operator | undispatched events in the inbox (capped at 100,000 as today) |
| `events.oldest_age` | seconds | operator | age of the oldest undispatched event |
| `outbox.backlog` | count | operator | undelivered outbox rows |
| `outbox.oldest_age` | seconds | operator | age of the oldest undelivered outbox row |
| `deliveries.pending` | count | operator | pending outbound deliveries (webhooks and brokers), capped at 100,000 |
| `deliveries.oldest_age` | seconds | operator | age of the oldest pending delivery (new query) |

## Scheduler and worker pools (from the monitor's sample)

| Signal | Unit | Scope | Meaning |
|---|---|---|---|
| `scheduler.lag.max` | seconds | operator | largest lateness over the scheduler's passes |
| `pool.<name>.ready_steps` | count | operator | ready steps for the activities of an operator-named pool, across tenants |
| `pool.<name>.queue_lag_seconds` | seconds | operator | wait of the oldest ready step of that pool |

`<name>` is a pool named in the configuration; at most 32.

## From a declared source or published

| Signal | Unit | Scope | Meaning |
|---|---|---|---|
| `source.<name>` | the source's unit | operator | one number from a declared external source (`FR-036-056`) |
| `<custom>` | the definition's unit | operator | an operator-published sample with a time to live (`FR-036-024`) |

## Namespace-visible facts (not signals of a policy)

A tenant reads, for each of its connectors, the lag, the drain state, the retry time and the limits that bound it (`PARTITION_BOUND`, `RATE_LIMITED`), through the tenant endpoint (`contracts/tenant-scaling.openapi-fragment.yaml`). Platform signals stay operator-only and are indistinguishable from missing for a tenant.

## Slice 2 and 3 additions (not in slice 1)

Per-role CPU and memory, worker-facing interface and ingress saturation, database connections in use against the budget.
