# Redis Streams for internal notifications and outbound callbacks

Feature033; release0.3.0; owner-authorized2026-10-27. Branch feature/033-redis-streams.
Owner chose: “Alternativa ao NATS e transporte adicional para callbacks, selecionável por configuração”.
Dependencies001/002/004/009/017 are approved by this request. Existing feature014 work remains independent.

## User stories

US01: an operator selects Redis Streams for internal outbox notifications while existing deployments
keep their NATS configuration. Workers continue through the same gRPC lease protocol.
US02: a namespace administrator configures a Redis Stream callback destination using a namespace
secret reference, receives versioned run/alert events and manages attempts/DLQ/redrive identically
to existing webhook and broker subscriptions.
US03: operators recover after publisher crashes or broker outage without losing PostgreSQL state,
changing delivery identity, exposing secrets or crossing namespace policy.

## Requirements

- FR-033-001: internal publishing is selectable NATS/Redis/disabled. Absent selection preserves
  legacy SAGAT_NATS_URL behavior. Selecting Redis requires valid settings and publishes only there.
  PostgreSQL remains authoritative; worker gRPC and workflow semantics are unchanged.
- FR-033-002: Redis internal entries carry stable outbox id, the existing versioned subject and
  payload, written by the existing transactional-outbox dispatcher. A bounded configured internal
  MAXLEN may trim old hints; PostgreSQL reconciliation remains the recovery authority.
- FR-033-003: Redis is an additional outbound subscription destination with host/port/TLS, stream
  key and database settings. Management uses existing namespace permissions, quotas and audits.
  Inbound Redis connectors and direct SDK Redis task consumption are outside this delivery.
- FR-033-004: callback delivery uses the existing PostgreSQL obligations, lease/attempt fences,
  retry/dead-letter/redrive and health accounting. XADD acknowledgement is required before success.
  Redis-generated stream ids are transport ids; deliveryId/eventId remain stable application ids.
- FR-033-005: broker settings never contain credentials. Optional namespace secret references use
  existing OpenBao authority and current resolution. Authenticated callback connections require TLS;
  namespace egress policy is checked before connecting. Errors/logs/metrics contain no credentials.
- FR-033-006: duplicate physical entries after ambiguous acknowledgement are allowed and retain the
  stable application id. No exactly-once external effect or Redis failover durability is promised.
- FR-033-007: publish work, input sizes, metadata and timeouts are bounded. Callback producers do not
  trim customer streams. Internal trim and Redis AOF/replication/retention tradeoffs are documented.
- FR-033-008: repository Compose declares a pinned Redis image, loopback-only port, persistent AOF
  volume and healthcheck. Tests own disposable Redis/PostgreSQL fixtures and require no hand-started service.
- FR-033-009: canonical REST/AsyncAPI, Studio generated types, deployment settings, examples,
  traceability and compatibility guidance ship with executable tests.

## Acceptance

SC-033-001: real Redis receives internal committed outbox notifications with correct identity;
legacy NATS settings/tests remain valid and invalid/missing/contradictory selection fails closed.
SC-033-002: public Redis callback management→real run completion→XADD→DELIVERED preserves body,
metadata, namespace/tenant ownership, audit and existing callback retry/attempt behavior.
SC-033-003: real outage, wrong-type/denied/auth/TLS/timeout, duplicate acknowledgement and restart
controls prove retry/DLQ/redrive and stable identity. Secret-bearing diagnostics remain redacted.
SC-033-004: migration forward/down guards preserve historical rows, HTTP callbacks regressions pass,
required focused/static/contract gates run with real outputs. Release0.3 inclusion is not a waiver
of the other feature014 or integrated release gates.
