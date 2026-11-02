# Feature033 transport contracts

## Internal publisher configuration

| Selector | Endpoint | Behavior |
| --- | --- | --- |
| absent/empty | legacy SAGAT_NATS_URL optional | Preserve NATS or disabled; a Redis URL without explicit selector is refused |
| nats | SAGAT_NATS_URL required | NATS only; Redis variables are inactive |
| redis | SAGAT_REDIS_URL required | Redis only; leftover NATS variables are inactive |
| none | endpoints inactive | Do not claim/publish notifications; PostgreSQL outbox accumulates |

Unknown selectors fail. Redis URL uses redis:// or rediss:// with database0..15; TLS is mandatory
outside development. Deployment Redis URLs are bounded to16KiB, refuse port0, and redact credentials in Debug/errors.
SAGAT_REDIS_STREAM defaults sagat:v1:events; key1..512 ASCII alphanumeric or : . _ - { }.
SAGAT_REDIS_STREAM_MAXLEN defaults100000, range1..10000000 and uses exact MAXLEN =, not approximate
trimming. SAGAT_REDIS_TIMEOUT_SECS defaults5, range1..30, covering connect/handshake/XADD/reply.
Internal Redis entries are binary-safe field/value pairs: message-id (stable outbox/application id),
subject (unchanged sagat.v1 routing string), payload (byte-for-byte committed body).

## Redis callbacks

Outbound kind REDIS is separate from inbound ConnectorKind. Settings: server (redis:// or rediss://
host:port, no credentials/query/fragment/database path), stream (same bounded key syntax), database
(integer0..15, default0). Optional secretName is a current namespace secret reference.
Secret document is password-only or ACL username/password, optionally a caCert PEM for private PKI. A caCert-only document is allowed for anonymous TLS.
Unknown keys, wrong types, empty credentials or username without password are refused. A named Redis secret requires rediss at management time. Credentials
never cross plaintext; certificate and hostname verification cannot be disabled. Cluster, Sentinel,
auto-discovery and redirect following are excluded. Every attempt rechecks namespace egress and
resolves current credentials and pins the exact policy-approved DNS addresses, then owns a fresh connection permanently bound to DB/auth/TLS. One checked DNS address is selected per attempt; there is no parallel socket fan-out or automatic within-attempt fallback.

Fields use existing header spelling: event-id,event-type,delivery-id,attempt,schema-version,
content-type (application/json), optional traceparent, and body (raw versioned event JSON bytes). Required metadata is nonempty;
duplicate/reserved field collisions are refused. Event/delivery ids remain stable across attempts;
Redis-generated stream ids may differ. A valid canonical numeric stream id acknowledges acceptance,
not arbitrary fsync/replica/failover durability. Callback writes never trim the customer's stream.

## Cutover / mixed versions

Transport choice is deployment-wide, not a per-replica routing mechanism. Rolling mixed NATS/Redis
publishers are unsupported: quiesce and join all dispatchers, preserve outbox, switch every replica
and all interested consumers, restart and resume. Confirmed rows remain confirmed on their prior
transport. Pending/unconfirmed rows use the newly selected backend; an ambiguous earlier append
can therefore leave physical copies on both brokers with the same stable application identity.
Switching to none preserves pending rows, which publish after reenabling a transport.

Additive DDL alone does not make old binaries parse REDIS subscriptions. Keep Redis callback
creation disabled until all control-plane/engine readers and writers have been upgraded and old
processes stopped/joined. Explicit operator activation is required; do not infer safety from an
expired membership or guarded down migration. Rollback after Redis use retains compatible readers
and schema until those retained rows can be removed by supported procedures. Actual staged-rollout
proof remains an acceptance gate, not a guarantee supplied by this prose. SAGAT_REDIS_CALLBACKS_ENABLED defaults false and enables creation/update only after operator activation; existing compatible readers continue to process retained rows even when creation is disabled.

## Bounds / cancellation / delivery

A publish attempt owns at most one connection and one pending command. No shared SELECT, cross-
namespace authentication pool, cached credential version or retained timed-out connection exists.
A timeout can still mean Redis accepted the write: retry with stable application identity. The
existing PG delivery lease/attempt/rate/backoff/DEAD/redrive rules remain the only state authority.
