# ADR-0069 — Redis Streams share outbox and delivery authority

Status: Accepted for implementation; owner transport choice2026-10-27. Feature033, release0.3.0.
Extends ADR-0002 and ADR-0025. Implementation/release guarantees require actual named tests.

The owner requested Redis Streams for internal communications and callbacks and chose an
alternative to NATS plus an additional callback transport, selected through configuration.
NATS remains supported with legacy default behavior; PostgreSQL remains the execution authority.

Use the existing EventPublisher port/outbox transaction for internal Redis notifications and the
existing outbound subscription/delivery/attempt transaction for Redis callbacks. Redis acceptance
requires a successful bounded XADD reply. Stream entry ids are Redis transport identifiers;
stable outbox id or deliveryId/eventId fields are the application idempotency identities.
An uncertain reply or crash after Redis acknowledgement but before PG confirmation can duplicate
physical entries. Consumers must deduplicate; no exactly-once provider effect is promised.

Clients and credential parsing stay in adapters. Callback credentials remain namespace secret
references resolved under existing OpenBao authority; TLS is mandatory with credentials and
current egress policy applies before every effect. No arbitrary endpoint credentials are stored.
Internal deployment credentials are redacted and protected by production TLS settings.

Internal MAXLEN is bounded and explicitly permits losing old hints because PG reconciliation
recovers ready work. Callback producers never trim customer streams; operators own their retention,
AOF and replication policy. A Redis reply does not by itself prove fsync/replica failover durability.

Redis is an outbound destination only in this increment. Do not widen inbound ConnectorKind or
SDK worker protocols to imply otherwise. Management contracts/migration add the Redis variant;
guarded down refuses existing Redis rows instead of deleting customer obligations. Canonical
contracts, fixtures, telemetry/tenant/secrets and real broker recovery tests are release gates.

Detailed wire/config/security/cutover semantics are in feature033/contracts.md. Selection is
cluster-wide with explicit dispatcher quiescence; process-local flags do not fence old binaries.
Redis callback activation is separate from migration and waits for all readers/writers to upgrade.
Fresh per-attempt connections bind logical DB/authentication immutably; shared SELECT and discovery
are excluded. Required rollout/activation tests remain open until actual evidence exists.
