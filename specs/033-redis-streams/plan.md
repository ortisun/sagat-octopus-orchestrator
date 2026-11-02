# Implementation plan — feature033

Read spec.md/tasks.md before implementation. Base origin/develop@70f09435. Target release0.3.0.

Reuse EventPublisher/OutboxDispatcher and the existing shared outbound delivery pipeline.
Redis protocol implementation lives in infra-connectors; domain contains only typed destination
validation. Separate outbound BrokerKind from inbound ConnectorKind to avoid advertising an
unimplemented Redis inbound connector. No new scheduler semantics or SDK worker transport.

Use Redis XADD with auto-generated transport id and stable outbox/delivery application id fields.
Internal hints may use bounded MAXLEN; callbacks do not trim. XADD reply acknowledges acceptance,
not arbitrary replication/failover persistence. All retries remain at-least-once.

Internal configuration selects a typed NATS/Redis transport; existing NATS env behavior stays valid.
Redis callback settings and credential references extend existing API/database representation.
One additive discriminator migration has guarded down behavior and does not rewrite history.
Redis library async/TLS features are pinned through Cargo.lock; image pinned in Compose/test fixtures.

Constitution: PG transactions/inbox/outbox remain authoritative; no domain Redis/JSON/framework
imports; at-least-once effects; existing gRPC/wire semantics; namespace policy/secrets/audit;
closed errors/OTel metrics; actual crash/duplicate/outage evidence. ADR-0069 extends transport choice
and ADR-0025 destinations, while retaining the established defaults and transaction boundaries.

Implementation sequence: specification/contracts → failing adapter/config/domain tests → first
real internal Redis path → callback domain/API/storage path → real guarded publisher/recovery →
operator/contract/static/integrated verification. Each checkbox requires named output evidence.
No deployment or release is performed until its complete gates pass.
