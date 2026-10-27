# New connector and endpoint kinds (draft contract)

Draft for FR-021-021..029. Revised 2026-10-27 after the architecture and security reviews. Every
kind admits through `inbox_events` (ADR-0012): the key `(tenant, namespace, source slug, event id)`
deduplicates, the body digest separates a retry (`DUPLICATE`) from a reuse (`EVENT_ID_REUSED`).
Credentials are namespace secrets with OpenBao authority. Binding a new-kind connector to a secret,
or changing a host-bearing setting of such a connector, requires `secret:manage` on that secret
(D1158). The five new connector kinds are `MANAGED`: their consumers run in a managed executor under
a fenced stream lease ([integration-runner.md](integration-runner.md#managed-streams)), connect over
a socket the guarded resolver opened to a checked address (TLS over that stream, SNI and
certificate name = configured host), and admit through `AdmitConnectorEvents`. The engine supervisor
skips `MANAGED` connectors. Egress: the namespace policy intersected with the runner's deployment
policy; CDC additionally within ADR-0035's operator-approved database boundary.

## Delivery guarantees

| Kind | Placement | Acknowledgement after inbox commit | Redelivery after crash before commit | Event id |
|---|---|---|---|---|
| `POSTGRES_CDC` | managed: database runner | flush LSN in a standby status update | yes: replication restarts from the stored checkpoint | `{slot}/{commitLsn}/{index}` |
| `MQTT` (QoS 1) | managed: integration runner, single holder | `PUBACK`, in PUBLISH order | yes (persistent session) | user property `eventId`, else JSON pointer, else SHA-256(topic ‖ payload) |
| `MQTT` (QoS 0, opt-in) | managed: integration runner, single holder | none | **no — at-most-once** | as above |
| `GCP_PUBSUB` | managed: integration runner | `:acknowledge` | yes (ack deadline) | `messageId` |
| `AZURE_SERVICE_BUS` | managed: integration runner | complete (delete locked message) | yes (lock expiry); broker DLQ after its max delivery count | `MessageId`, else `seq-{SequenceNumber}` |
| `WEBSOCKET` | managed: integration runner, single holder | none | **no — best-effort** | JSON pointer, else SHA-256(frame) |
| `KAFKA` (existing, extended) | engine, partition leases (unchanged, G1137) | position row in the admission transaction | yes | existing |
| `ROUTE` endpoint | control plane ingress | `202` | caller retries | idempotency header; server-generated only on mTLS routes (non-deduplicating) |
| `EMAIL` endpoint | control plane ingress | `200` (Mailgun) / `202` (relay) | provider retries | `doc-{SHA-256(normalized document)}` |

The Studio shows this table's guarantee next to each connector and endpoint (FR-021-035).

## Stream leases (every managed kind)

A runner holding the matching role claims a stream (`database-runner` for `POSTGRES_CDC`,
`integration-runner` for the four others; a mismatch is refused). Renewal and every batch recheck
connector status (`ACTIVE`), the secret ACL of the connector's publisher, and the credential version
selected at claim; a refusal ends the lease and the runner closes the source connection
(`PAUSED`, revoked or rotated → new claim, new credential). Renewal never returns a credential. Single-
holder kinds (CDC per slot, MQTT, WebSocket) have one lease per connector; Pub/Sub and Service Bus
may have up to `concurrency` leases per connector, each a distinct stream name.

## `POSTGRES_CDC`

Settings: `host`, `port`, `database`, `tls` (017 `TlsPolicy`), `secretName` (replication role
credentials), `publication` (one identifier), `slot` (`^[a-z0-9_]{1,63}$`), `tables[] {schema,
table, correlationColumn?}` (≤ 64; optional narrowing of the publication), `lagThresholdBytes`,
`standbyIntervalMs` (1,000–60,000). Prerequisites checked at stream start (closed health classes):
`wal_level = logical` (`WAL_LEVEL`), publication exists (`NO_PUBLICATION`), slot exists with plugin
`pgoutput` (`NO_SLOT`, `WRONG_PLUGIN`), role has `REPLICATION` (`NOT_PERMITTED`). The platform runs no
DDL and never calls `pg_create_logical_replication_slot` or `pg_drop_replication_slot`. Runbook:
grant the role a database-specific `pg_hba` line, never the `replication` keyword (which would also
allow physical replication and base backups).

Protocol: replication connection, `START_REPLICATION SLOT {slot} LOGICAL {checkpoint}
(proto_version '1', publication_names '{publication}')`; the slot is validated by its pattern and
the publication is written as a quoted literal with `'` doubled (no list, no injection). `{checkpoint}`
is the stored `connector_offsets.next_offset` (or `0/0` before the first commit, letting the server
use its confirmed position). One event per change; `BEGIN`/`COMMIT` frame them; `RELATION` updates
the column description; `TRUNCATE` → `cdc.truncate` per table. Event: type
`cdc.{insert|update|delete|truncate}`, subject `{schema}.{table}` (identifiers quoted when they contain
`.`), data per [data-model.md](../data-model.md#cdc-event-data-canonical-shape). A correlation key from
`correlationColumn` is a value of customer data: the event is marked data-derived and the inbox API
returns that key only under the FR-016-026 reveal.

Admission: the runner sends batches with `expectedCheckpoint` and, on the batch containing a
transaction's `COMMIT`, `nextCheckpoint = end LSN`. The checkpoint never moves inside a transaction.
Each CDC inbox row records its commit LSN; the inbox purge skips rows whose commit LSN is above the
connector's stored checkpoint, so a transaction resent after any outage is recognised. In a
`sync_quorum` cell (032) the commit reply is sent only once the commit is durable on the quorum
standby; under `async` replication changes inside the RPO can be lost at failover (032). After the
commit reply, the runner reports `flush = write = apply = nextCheckpoint`. Keepalives with `reply
requested` are answered with the last committed checkpoint. Lag = server WAL end (from
XLogData/keepalive) − last reported flush; above `lagThresholdBytes` the connector becomes `LAGGING`
and `connector.lagging` is audited once per crossing.

## `MQTT`

Settings: `servers[]` (`mqtts://host:port`; plaintext only in development), `protocol` (`3.1.1`,
`5`), `topicFilters[]` (≤ 32, no `#` at the root unless `allowWildcardRoot`), `qos` (`1` default,
`0` opt-in), `clientIdSuffix?`, `eventIdPointer?`, `eventTypeProperty?` or `defaultEventType`,
`secretName?` (username/password) or client certificate secret. Session: `clean_session=false`
(3.1.1) or `clean_start=false` with a session expiry (5); client id `sagat-{connector id}`; manual
acknowledgements; `receive maximum` = in-flight bound. PUBACKs are sent in the order PUBLISH packets
arrived; a filtered or dead-lettered message (payload not a JSON object ≤ 240 KiB) is acknowledged in
its place after its own commit. When a message cannot be admitted (backlog, rate) the consumer sends
no further PUBACK and disconnects after a bounded wait (default 30 s), so the broker redelivers.

## `GCP_PUBSUB`

Settings: `project`, `subscription`, `secretName` (service-account key JSON), `maxMessages`,
`ackExtensionSeconds`. Calls: `POST https://pubsub.googleapis.com/v1/{subscription}:pull`,
`:acknowledge`, `:modifyAckDeadline`, with an OAuth2 JWT-bearer token from the pinned endpoint
`https://oauth2.googleapis.com/token` (the key file's `token_uri`, if present, must equal it; its
`type` must be `service_account`), kept only in runner memory for its lifetime (≤ 1 hour), never
persisted. Attributes `eventType`, `subject`, `correlationKey`, `eventId` map to the envelope like
broker headers (publisher-declared envelope, as for Kafka/NATS/SQS); `messageId` is the default id.

## `AZURE_SERVICE_BUS`

Settings: `namespaceHost` (lower-cased, must end in `.servicebus.windows.net` after IDNA and
trailing-dot normalization), `entity` (`queue` or `topic/subscriptions/name`), `auth` (`sas`: key name
+ secret key; `entra`: `tenant` as a UUID or a domain name, client id, secret), `concurrency`. The
Entra token endpoint is `https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token`, not
configurable. Calls: peek-lock `POST …/messages/head?timeout=…`, complete `DELETE
…/messages/{messageId}/{lockToken}`, renew `POST …/messages/{messageId}/{lockToken}`;
`BrokerProperties` give `MessageId`, `SequenceNumber`, `DeliveryCount`, `LockToken`. Exact semantics
are verified against the service documentation in T021-048.

## `WEBSOCKET`

Settings: `url` (`wss://`), `headers` (≤ 16; a value may name a namespace secret, resolved like
connector credentials; `Host`, `Connection`, `Upgrade`, `Sec-WebSocket-*` refused), `subprotocol?`,
`subscribeMessage?` (static, ≤ 16 KiB), `eventIdPointer?`, `defaultEventType`, `binary` (`reject`
default, or `base64`). The upgrade request is sent only over the pre-checked socket; redirects on the
upgrade are refused. Reconnect with jittered backoff up to 60 s. Text frames must be JSON objects
≤ 240 KiB. Ping/pong keeps the connection; the server cannot be asked to resend.

## Connector filter (every kind)

`filter` is an expression of the existing evaluator over the document
`{key, keyBase64, headers, value, topic?, partition?}` where `value` is the decoded JSON body (or
absent). True admits; false acknowledges/moves the position and counts `filtered`; an evaluation
error counts as not matching and is counted `filter_error`. A Kafka key is `key` when it is valid
UTF-8, otherwise `key` is null and `keyBase64` carries it.

## Broker metadata on the event (`event.message`)

`{key?, keyBase64?, topic?, partition?, offset?, timestamp?, deliveryAttempt?}` for Kafka, MQTT
(topic), Pub/Sub (`deliveryAttempt`), Service Bus (`deliveryCount` as `deliveryAttempt`). Disclosed
exactly like `data` (FR-016-026).

## `ROUTE` endpoint

URL: `{method} /v1/routes/{routePrefix}{path}`. `routePrefix` is the namespace's random prefix
(`GET/POST /namespaces/{id}/route-prefix`, rotation keeps the previous prefix valid for a bounded
overlap, default 24 h). Path: `/` + segments of `[a-z0-9._~-]` or `{param}`; ≤ 512 bytes; ≤ 16
parameters; trailing slash normalized away; matching is exact on segments, case-sensitive. Two
routes conflict when method and shape (parameters replaced by `{}`) are equal.

Order of checks. Before authentication: request line and header bounds, the global body cap
(240 KiB) and an attempt limiter keyed by a hash slot of the requested prefix (a bounded table, the
same for known and unknown prefixes). Their refusals (`413`, `431`, `429`) do not depend on whether
a route exists. Every authentication failure (unknown prefix or route, wrong method, bad credential,
stale timestamp) is the same `401`. After authentication: pause
(`503` + `Retry-After`), the route's body limit (`413`), the route's rate (`429`), a missing
idempotency header (`400 IDEMPOTENCY_KEY_REQUIRED`), then admission.

Authentication (one profile per route, ADR-0026): `HMAC` with headers `X-Octopus-Timestamp`,
`X-Octopus-Signature: v1=<hex>` over

```
"octopus-route-in-v1\n{timestamp}\n{idempotencyKey}\n{METHOD}\n{routePath as sent}\n{canonicalQuery}\n{canonicalHeaders}\n" + body
```

where `canonicalQuery` is the declared captured query parameters sorted by name, percent-encoded,
joined by `&`, and `canonicalHeaders` the declared captured headers as `lower-case-name:value`
(value trimmed, absent = empty) sorted by name, joined by `\n` then percent-encoded as one line;
`OIDC` (deployment verifier, exact subject); `MTLS` (leaf fingerprint). Timestamp window and the three
newest secret versions as ADR-0026.

Idempotency: `idempotency.header` (default `Idempotency-Key`, 1–200 printable characters) is the
event id. `idempotency: generate` (a random id per request, labelled non-deduplicating) is accepted
only on `MTLS` routes and refused at creation for `HMAC` and `OIDC` (`IDEMPOTENCY_GENERATE_NOT_ALLOWED`),
because a captured signed request or bearer could otherwise start a new run per replay. Body: JSON
object ≤ the endpoint limit, or empty for `DELETE` (data `{}`). Captures: declared query parameters and
headers only, each ≤ 1 KiB, in `event.request = {method, pathParams, query, headers}`; captured header
names exclude `Authorization`, `Cookie`, `Proxy-*` and the signature headers. Response `202` with
`EventAccepted`. The route field `response` is reserved: `mode: accept` (default) is the only value
until feature 026 adds `mode: wait` (FR-026-012); any other value is refused at creation.

## `EMAIL` endpoint

URL: `/v1/webhooks/{endpointId}` (existing random id). Authentication precedes any per-endpoint
answer, as for routes. Providers:

- `GENERIC_RELAY`: an operator relay posts the canonical e-mail document
  ([data-model.md](../data-model.md#e-mail-event-data-canonical-shape)) with an ADR-0026 profile;
  headers as `/v1/webhooks`; `202`.
- `MAILGUN` profile (enabled only after T021-055 confirms the scheme with provider fixtures): Mailgun
  JSON forward (forward URL ending in `json`), headers `X-Mailgun-Signature`, `X-Mailgun-Timestamp`;
  signature = hex HMAC-SHA256 under the namespace secret (the Mailgun webhook signing key) over the
  timestamp concatenated with the raw body, constant-time compared; timestamp within the endpoint
  skew. Form-encoded forwards (signature over timestamp and token only, body unsigned) are refused.
  Replay is bounded by the timestamp window and the inbox dedupe; no token table is kept. Responses:
  `401` on authentication failure; then `503` while paused or unavailable (provider retries for 8
  hours); `200` on admission or `DUPLICATE`; `406` on a permanent refusal (too large, invalid
  document) so the provider stops retrying. JSON field names are mapped from the fixtures.

Event id: `doc-` + SHA-256 of the normalized document (provider envelope fields such as signatures
and timestamps excluded); the sender-set `Message-ID` is data, never the id, so two messages sharing
it are two events. Event type `email.received` unless the endpoint sets another; the event
`subject` and correlation key are not set from the message (the message subject is in `data`);
dedupe window default 1 day.
