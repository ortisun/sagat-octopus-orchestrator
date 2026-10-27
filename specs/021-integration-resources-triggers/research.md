# Feature 021 — Baseline, sources, decisions, gaps and owner questions

Baseline: worktree `feature/018-032-parity-specs` at `70f09435` (release 0.2.0), read on 2026-10-26.
Identifiers of this feature: decisions D1120–D1159, gaps G1120–G1159, owner questions O1120–O1159.
Used: D1120–D1159, G1120–G1143, O1120–O1123, and the coordinator's overflow numbers D1620–D1624. Nothing below is implemented.

## Repository evidence (verified paths)

- **Connection catalog (017).** `crates/domain/src/database.rs` (`ConnectionConfig::Postgres`,
  `ConnectionRef {name, revision}`, `ConnectionStatus`, `MANAGED_DATABASE_PREFIX = "sagat.database."`),
  `crates/application/src/database.rs`, migration
  `crates/infra-postgres/migrations/20261123000000_database_connections.up.sql` (identity, immutable
  revisions with an ALWAYS trigger, deferred head FK, `run_database_connections`, retained-byte
  triggers, `sagat_app` grants) and `20261124000000_database_executor.up.sql`
  (`step_database_credentials`). REST: `/namespaces/{namespaceId}/connections[/{connectionName}[/probe]]`,
  `/database-executions`, `/database-drivers` in `specs/001-platform-foundation/contracts/openapi.yaml`.
  These are the pattern to reuse and the data that must not move (D1120).
- **Managed executor.** `apps/database-runner/src/{runner,rpc,slots,capability,attempt,config}.rs`;
  protocol 1.6 `ResolveDatabase` in `specs/001-platform-foundation/contracts/worker.proto`; contract
  `specs/017-database-connections/contracts/managed-executor.md`; machine role `database-runner`
  (`crates/domain/src/identity.rs`, seven built-in roles, 48 permissions).
- **OpenBao.** Port `crates/application/src/secret_provider.rs` (`SecretProvider`, closed
  `ProviderError`), adapter in `crates/infra-secrets`, contract
  `specs/017-database-connections/contracts/openbao.md`; secret service
  `crates/application/src/{secrets,managed_secrets}.rs`.
- **Connectors.** `crates/domain/src/connectors.rs` (`ConnectorKind` = NATS, KAFKA, SQS; reserved
  slugs `api`, `cron`), `crates/application/src/connectors.rs` (ack after inbox, `Checkpoint`,
  `ConnectorRuntime::admit` with checkpoint CAS, dead letters at most 1,000 × 64 KiB),
  `crates/infra-connectors/src/{supervisor,ingest,kafka,nats,sqs,guard,credentials}.rs`. The Kafka
  reader builds `RawMessage` from headers and value only: **the record key is dropped today**
  (`kafka.rs`), so key filters need a new field (D1147).
- **Supervisor placement.** `apps/execution-engine/src/lib.rs` starts one supervisor per engine
  replica with the namespace-scoped secret reader; `guard.rs` applies the egress policy to broker
  names and addresses before a library connects (G76 of feature 004).
- **Inbox and triggers.** `crates/infra-postgres/migrations/20261012000000_events.up.sql`
  (`event_endpoints` with `profile`, `inbox_events` PK `(tenant, namespace, source, event_id)`,
  240 KiB payload, `event_connectors`, `connector_offsets`, `connector_leases`,
  `connector_dead_letters`, `workflow_triggers`); `crates/domain/src/triggers.rs` (kinds manual,
  event, webhook, message, cron; a webhook names an endpoint slug, a message names a connector slug);
  filter evaluation in `crates/scheduler/src/events.rs` through `ExpressionEvaluator::event_filter`
  (`crates/application/src/execution.rs`), document `{event, data}`.
- **Egress.** `crates/domain/src/egress.rs` (private ranges need an explicit network rule; metadata,
  loopback and link-local never allowed) and `crates/infra-egress/src/lib.rs` (resolve inside the
  connector's resolver, no redirects, no environment proxies).
- **Blobs.** `crates/infra-blobstore` over `object_store` 0.14.2 (workspace `Cargo.toml`); ADR-0023
  claim check; managed database leases refuse generic `ReadBlob` (017 managed-executor contract).
- **Classification.** FR-016-002 fixes the carriers (workflow input/output, node `outputSchema` on
  activity, script, waitEvent, child); FR-016-026 makes event `data` PII as a whole; ADR-0036
  decision 6 keeps the database node outside the carriers.
- **Activities.** The IR `activity` node already has `compensateWith`, `reconcileWith`,
  `idempotencyTemplate`, `irreversible`, `highRisk`; `TaskLease.idempotency_key`,
  `task_kind`, `target_idempotency_key` exist in `worker.proto`.
- **Errors.** `Error.code` is closed (`BAD_REQUEST` … `INTERNAL`); refinements are
  `details.reason` values (OpenAPI `Error` schema; `sdks/error-catalog.json`).
- **Studio.** `frontend/apps/studio/src/features/{connections,integrations,secrets}`.

## External semantics consulted (2026-10-26)

- Windmill resources and resource types (JSON Schema types, secrets as variable references,
  version history): <https://www.windmill.dev/docs/core_concepts/resources_and_types> (read).
  Windmill trigger list (HTTP routes, WebSocket, Postgres CDC, Kafka, NATS, SQS, MQTT, GCP, Azure
  Event Grid, e-mail over its own SMTP): <https://www.windmill.dev/docs/triggers>,
  <https://www.windmill.dev/platform/triggers> (search summaries; the detailed pages render
  client-side and were not readable). Differences chosen here: no SMTP server (D1146), Azure
  Service Bus rather than Event Grid (brief), no in-platform preprocessor scripts.
- PostgreSQL logical decoding: slots retain WAL and catalog rows while unconsumed; a slot emits each
  change once in normal operation but can resend after a crash; clients may record the last LSN and
  request decoding from it:
  <https://www.postgresql.org/docs/current/logicaldecoding-explanation.html> (read). Streaming
  replication protocol and standby status updates:
  <https://www.postgresql.org/docs/current/protocol-replication.html>; `max_slot_wal_keep_size`:
  <https://www.postgresql.org/docs/current/runtime-config-replication.html> (to be confirmed in T021-040).
- Mailgun inbound forward: fields `signature`, `timestamp`, `token` (form posts) or
  `X-Mailgun-Signature`/`X-Mailgun-Timestamp` with the body in place of the token (JSON posts);
  `200` success, `406` rejected without retry, other codes retried for 8 hours:
  <https://documentation.mailgun.com/docs/mailgun/user-manual/receive-forward-store/receive-http>
  (read); signature verification and token caching:
  <https://documentation.mailgun.com/docs/mailgun/user-manual/webhooks/securing-webhooks> (search summary).
- Google Cloud Pub/Sub `POST /v1/{subscription}:pull`, `ackId`, `deliveryAttempt`, deprecated
  `returnImmediately`:
  <https://docs.cloud.google.com/pubsub/docs/reference/rest/v1/projects.subscriptions/pull> (read);
  `acknowledge` and `modifyAckDeadline` on the same resource (to be confirmed in T021-047).
- Azure Service Bus peek-lock REST `POST …/messages/head` with Entra or SAS auth:
  <https://learn.microsoft.com/rest/api/servicebus/peek-lock-message-non-destructive-read> (search
  summary); complete/renew/unlock pages to be confirmed in T021-048.
- MQTT client `rumqttc` 0.25.1 `MqttOptions::set_manual_acks`, `set_clean_session`:
  <https://docs.rs/rumqttc/latest/rumqttc/struct.MqttOptions.html> (read); MQTT 5.0 QoS 1 and
  session state: <https://docs.oasis-open.org/mqtt/mqtt/v5.0/mqtt-v5.0.html>.
- Slack `chat.postMessage`: about one message per second per channel, `chat:write` scope, no
  documented idempotency argument: <https://docs.slack.dev/reference/methods/chat.postMessage> (read).
- `object_store` `HttpConnector` (custom HTTP client factory, 0.14.2):
  <https://docs.rs/object_store/latest/object_store/client/trait.HttpConnector.html> (read).
- Others relied on, not re-read today: RFC 6749 §4.4 (client credentials), RFC 6455 (WebSocket),
  RFC 5322/5321 (message format, SMTP), GraphQL over HTTP
  (<https://graphql.github.io/graphql-over-http/draft/>), S3 conditional writes
  (<https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html>) and additional
  checksums (`x-amz-checksum-sha256`). Their exact behaviour is verified by tests, not assumed.
- Temporal has no counterpart for resources or these triggers; nothing here depends on code replay.

## Decisions

- **D1120 — 017 stays the database authority; resources share its rules, not its tables.** ADR-0055 §1.
- **D1121 — Resource types are platform-provided, versioned and shipped with the server.** A type
  version is immutable, its schemas are checked by `crates/infra-schema`, and new versions only add
  optional settings or auth schemes. Custom types wait for O1122. The listing names every database
  driver of 017 and 024 (`postgresql`, `mysql`, `mariadb`, `sqlserver`, `bigquery`, `snowflake`,
  `duckdb`, from `specs/024-data-engines-pipelines/contracts/engines.md`) as `managedBy: connections`
  and the resource API refuses all seven (architecture review S7).
- **D1122 — Resource names do not share the connection namespace.** References are typed
  (`connectionRef` vs `resource`); the Studio shows the kind next to the name.
- **D1123 — Slots accept only OpenBao-authority secrets**, as 017 connections do; the same
  intent/activation and partial-save flows apply.
- **D1124 — Revision per node per run, credential version per attempt** (017 FR-017-002/008).
- **D1125 — A separate `apps/integration-runner` with role `integration-runner`.** ADR-0055 §3. Its
  network policy reaches the public internet and operator-approved private ranges only; it never
  inherits the database runner's database destinations.
- **D1126 — One additive node `resourceOperation`, root-only first.** Mirrors ADR-0036 decision 3;
  credential pins are keyed by the full scoped step key (path arrays) so that G1120 needs no
  migration.
- **D1127 — Safe/effect classification is fixed per operation, not authored.**
- **D1128 — The effect key is the existing step idempotency key** (or `idempotencyTemplate`), not a
  new derivation; it is opaque to providers (no tenant or run id in clear) by hashing
  `sagat-effect-v1`, tenant, run, path and node with SHA-256.
- **D1129 — Outputs are classification carriers.** ADR-0055 §8; requires an additive amendment of
  FR-016-002 (T021-064).
- **D1130 — The runner builds auth; authors cannot set auth or routing headers.**
- **D1131 — OAuth2 tokens are fetched per attempt and never cached across attempts** (FR-017-008
  "no plaintext cache between attempts"). Cost is measured (T021-071).
- **D1132 — No silent truncation; blob output is scanned before it exists.** Over-bound inline output
  fails `RESULT_LIMIT`; blob mode is explicit. ADR-0023 makes uploads from secret-bearing steps fail
  because no cross-chunk scan exists; the integration runner adds a bounded streaming scanner (every
  slot value in raw, JSON-escaped, base64 at three alignments, base64url and percent-encoded forms,
  with an overlap of the longest pattern across chunks) before `UPLOADED`, and the server rescans
  before `COMMITTED`. Blob mode stays refused at publication until that scanner's tests pass (G1143).
- **D1133 — The S3 client comes from an `infra-blobstore` factory.** `crates/infra-blobstore`
  exposes a narrow `CustomerBucket` (get/put/head/list/delete streams, no `object_store` type in its
  signature) built over `object_store` with a guarded `HttpConnector`, so ADR-0023 decision 1 (only
  `infra-blobstore` depends on a storage client) holds. `crates/infra-integrations` uses that factory.
  `scripts/check-architecture.py` gains a per-crate client allowlist (T021-016). Fallback in G1122.
- **D1134 — SMTP requires TLS; `Message-ID` is `<{effect key}@{resource message-id domain}>`.**
- **D1135 — Slack `429` means not posted and is retryable; any other ambiguity is unknown outcome.**
- **D1136 — Variables are `public|internal`, statically referenced, pinned at admission, and absent
  from filters, resource settings and ambient worker context.**
- **D1137 — No new trigger kinds in the IR.** Routes and e-mail endpoints are named by `webhook`
  triggers, new connectors by `message` triggers.
- **D1138 — CDC is a managed stream in the database runner; renewal is a revocation point.** ADR-0055
  §6. `connector:stream` is granted to `database-runner` for `POSTGRES_CDC` only. Every
  `RenewConnectorStream` and `AdmitConnectorEvents` rechecks connector status, the secret ACL and the
  credential version; a refusal ends the lease and the runner closes the replication connection.
  Renewal never returns a credential; a new credential needs a new claim (security S2, architecture
  S6).
- **D1139 — CDC identity, checkpoint and retention.** Event id `{slot}/{commitLsn}/{index}`;
  checkpoint in `connector_offsets` (stream = slot, `next_offset` = end LSN as a signed 64-bit value;
  an LSN above 2^63−1 is refused as unsupported); replication restarts from the stored checkpoint;
  protocol version 1 without streaming of in-progress transactions, so only committed changes are
  decoded. A CDC inbox row records its commit LSN and is not purged until the stored checkpoint
  passes it, so a transaction resent after a long outage is still recognised (architecture S1).
  `slot` matches `^[a-z0-9_]{1,63}$` and `publication` is one identifier, quoted as a literal in the
  replication command (security M1).
- **D1140 — Slot lag is lag in bytes between the server's reported WAL end and the platform's
  reported flush.** Over threshold: `LAGGING` and alert `CONNECTOR_LAGGING`; the slot is never
  dropped or advanced past unadmitted changes.
- **D1141 — MQTT single holder, in-order acknowledgement.** A persistent session is bound to one
  client id (`sagat-<connector id>`), so one stream lease elects the holder in the integration runner.
  PUBACKs follow the order of PUBLISH packets (MQTT §4.6); a filtered or dead-lettered message is
  acknowledged in its place; when admission cannot proceed the consumer stops acknowledging and
  disconnects after a bounded wait so that the broker redelivers (architecture minor).
- **D1142 — WebSocket is best-effort and says so.**
- **D1143 — Pub/Sub over REST through the guarded HTTP client in the integration runner.** The
  OAuth2 token endpoint is pinned to `https://oauth2.googleapis.com/token`; the key file's `token_uri`
  must equal it (security M2).
- **D1144 — Service Bus over REST peek-lock through the guarded HTTP client in the integration
  runner**, not AMQP, in this feature; `namespaceHost` must end in `.servicebus.windows.net` after
  normalization and the Entra `tenant` is a UUID or a verified domain name (security M2); throughput
  is measured before AMQP is considered.
- **D1145 — Routes live under `/v1/routes/{routePrefix}/…` and authenticate first.** The prefix is
  random per namespace and rotatable, so URLs do not reveal tenant or namespace names. Before
  authentication only route-independent answers are given (global caps, an attempt limiter keyed by a
  hash slot of the requested prefix, the same for known and unknown prefixes, and the uniform `401`);
  pause, route size, route rate and a missing idempotency header are answered after it (security S4).
  The HMAC string `octopus-route-in-v1` covers method, path, canonical query and the declared
  captured headers (architecture S4).
- **D1146 — No MIME parsing and no SMTP server.** The provider or relay delivers JSON; the Mailgun
  JSON forward signs the body with the timestamp (in form mode only timestamp and token are signed,
  so form mode is refused). Replay is then bounded by the timestamp window and the inbox dedupe
  (ADR-0012 rule), with no token table. Dedupe window default one day, longer than Mailgun's 8-hour
  retry span; whether retries are re-signed is G1140. The event id is `doc-` plus the SHA-256 of the
  normalized document (provider envelope fields excluded), never the sender-set `Message-ID`, so a
  sender cannot suppress another message (security S3); the event `subject` is not copied from the
  message (security S5). The Mailgun adapter ships only after T021-055 confirms the scheme with
  provider fixtures; the relay does not depend on it.
- **D1147 — Connector filters run before admission for every kind; `event.message` is additive.**
- **D1148 — Preview is evaluation, not disclosure, except over retained data**, where a boolean
  over PII is an oracle and requires the reveal grant.
- **D1149 — `event.message`, `event.request` and data-derived correlation keys are disclosed exactly
  like `data`** (FR-016-026). A CDC event whose correlation key comes from `correlationColumn` is
  marked data-derived; the inbox API returns that key only under the reveal grant (security S5).
- **D1150 — `resource:effect` is granted to developer by default** (O1121 recommended default),
  unlike `database:write`; publication policy (011) remains the control for protected namespaces.
- **D1151 — Protocol minor, migration timestamps and permission order are provisional** (next minor
  after 1.6, provisionally 1.7; migrations after `20261220000000`).
- **D1152 — Probes are fixed per type and run as durable runs with origin `RESOURCE_PROBE`.**
- **D1153 — Shared managed-runner plumbing moves to `crates/infra-managed`** with no behaviour change,
  guarded by `tests/integration/database-runner`.
- **D1154 — A type version names, per operation, the executor role that may resolve it.** All v1
  operations name `integration-runner`; this lets 025 add LLM provider types resolved by its own
  runner, and 024 add a data-read operation on `s3_bucket` for its runner, without a second catalog.
- **D1155 — Routes reserve `response.mode`.** Only `accept` (asynchronous `202`) exists here; 026's
  FR-026-012 adds `wait` additively. Any other value is refused, so no route silently changes
  behaviour when 026 ships.
- **D1156 — The four new connector kinds are managed streams in the integration runner**
  (coordinator decision on architecture B1 and security B2). MQTT, Pub/Sub, Service Bus and WebSocket
  are claimed by `integration-runner` (granted `connector:stream` for these kinds only) through the
  stream lease RPCs, connect over sockets the guarded resolver opened, and admit through
  `AdmitConnectorEvents`. The engine keeps only Kafka, NATS and SQS (G1137). This removes the
  contradiction with ADR-0055 option 3 and the rebinding exposure of G1125.
- **D1157 — `idempotency: generate` only on mTLS routes** (security B1). An HMAC request or an OIDC
  bearer can be replayed by whoever captures it within its validity; with a caller key the replay is
  a `DUPLICATE`, without one it would start a new run. mTLS binds the request to a TLS session the
  attacker cannot reproduce. Chosen over recording signature digests because it needs no new table
  and is testable at creation time.
- **D1158 — Binding a slot to a destination requires `secret:manage`** (security S1). Creating a
  resource, changing a host-bearing setting, or changing a slot's secret requires `secret:manage` on
  that secret; `secret:use` lets a user revise only settings that keep every slot's destination. The
  same rule applies to secrets of the new connector kinds; existing kinds are G1141.
- **D1159 — 021's locks follow ADR-0050 decision 7** (architecture S5, coordinator C4). ADR-0050
  (018) owns the merged order. 021's rows: the namespace variable fence in slot 4a (writers only),
  resource heads then variable heads after 017 connection heads in slot 4b, slot secrets and their
  ACLs in slot 5, run pins in slot 6. See [plan.md](plan.md#lock-order-adr-0050-decision-7). 024
  cites ADR-0050 directly.
- **D1620 — The placement rule (a)–(d) of ADR-0055 decision 11** (coordinator C3, cross-feature B3).
  Tenant-authored request content only in a managed executor; non-database fixed-shape deliveries to
  tenant destinations may stay in the engine through the guarded client (ADR-0025 precedent); no
  engine or control-plane connection to a customer database whatever the protocol, including HTTP
  interfaces of analytical databases such as ClickHouse — database sinks, sources and probes go
  through `apps/database-runner`; the four new connector kinds in the integration runner. It narrows
  the rationale of ADR-0055 option 3 so that 019, 020, 025, 026 and 028 can cite it; owner question
  O1123. 026 channels do not use 021 resources.
- **D1621 — Reset rules** (coordinator C1). 021 adds no journal event type; `run_resource_bindings`
  and `run_variable_bindings` are `REMATERIALIZE` side tables (same revisions, current authority
  rechecked, refusal on a paused/deleted resource or a deleted variable); credential pins are not
  copied. Registered in 018's registry by T021-079.
- **D1622 — Resources adopt 029's folder contract** (coordinator C10). One placement per resource in
  029's `item_placements`; when 029 is enabled its evaluator decides visibility and use; `runner`
  confers `resource:use` (and `resource:effect` while developer holds it); `resource:manage` is
  sensitive. Variables are namespace configuration and are not placed.
- **D1623 — CDC flush waits for the cell's durability** (coordinator C12). In a `sync_quorum` cell
  (032) `AdmitConnectorEvents` replies, and the flush is reported, only after the inbox commit is
  durable on the quorum standby; under `async` the loss of changes inside the RPO is 032's to state.
- **D1624 — Alignment with ADR-0050 decision 7** (018 gap G1605). The earlier 021 rank table put 024
  asset heads after secrets and called the variable fence part of an "authority slot". Both are
  corrected: asset heads are in 4b before secrets (slot 5), and the variable fence is a 4a fence after
  the 017 catalog lock. No lock of 021 moved relative to another 021 lock.

## Gaps

| Gap | Description | Status |
|---|---|---|
| G1120 | Operations inside collection/loop bodies need per-instance capability proofs from 014. | Open; refused at publication |
| G1121 | `tokio-postgres` 0.7.18 (pinned for 017) has no logical-replication client; choose a maintained replication client or a minimal CopyBoth implementation in `crates/infra-database` with the 017 frame budget. | Open; decided in T021-036 with evidence |
| G1122 | Prove that `object_store`'s `HttpConnector` lets the guarded resolver choose addresses; otherwise implement a minimal SigV4 client over `crates/infra-egress`. | Open |
| G1123 | No ordering across events of new sources (extends G74 of 004); CDC changes of one transaction are admitted in order but dispatched without an order guarantee. | Accepted, documented |
| G1124 | WebSocket and MQTT QoS 0 lose in-flight messages at a crash; inherent to the sources. | Accepted, labelled |
| G1125 | MQTT and WebSocket libraries must connect over a socket the guarded resolver opened (TLS over a pre-connected stream); if a library cannot accept one, it is not used. Required by D1156; proven by the rebinding tests of T021-029/T021-050. | Open until those tests pass |
| G1126 | E-mails over 240 KiB and attachments as blobs need an inbox-owned blob class in ADR-0023. | Open |
| G1127 | AWS SES/SNS (certificate-signed notifications), SendGrid and Postmark adapters. | Out of scope |
| G1128 | Provider-specific HMAC profiles for routes (GitHub, Stripe, Shopify) need an ADR-0026 extension. | Out of scope |
| G1129 | OAuth2 authorization-code/refresh flows need a durable token lifecycle. | Out of scope |
| G1130 | SMTP and Slack have no provider-side deduplication; unknown outcomes always need a person. | Accepted |
| G1131 | Variables and resources in 011 manifests and promotion are not designed here. | Open |
| G1132 | Resources consumed by ordinary activities and scripts (typed resource arguments). | Out of scope |
| G1133 | Namespace-defined resource types. | Waits for O1122 |
| G1134 | Field-level classification of event data waits for event type schemas (G282 of 016). | Open (016) |
| G1135 | Physical convergence of 017 tables into resources would need a new ADR. | Not planned |
| G1136 | Workload identity federation (GCP) and managed identity (Azure) instead of stored keys. | Out of scope |
| G1137 | The pre-existing Kafka, NATS and SQS clients run in the engine process with platform credentials (G76 of 004); the four new kinds do not (D1156). | Open, pre-existing |
| G1138 | Ingress throttling for routes is per replica (G78 of 004). | Accepted, documented |
| G1139 | No latency, throughput or lag figures exist; all are benchmark criteria. | Open until T021-071 |
| G1140 | Whether Mailgun re-signs a retried forward with a fresh timestamp is not documented in the pages read; a retry hours later with the original timestamp would fail the window (the existing skew ceiling is one hour). Verified with provider fixtures or a sandbox account in T021-055. | Open |
| G1141 | Existing endpoints, connectors and outbound subscriptions bind namespace secrets with `secret:use` only, so a holder can point one at a host they control (the D1158 exposure for pre-existing kinds). 021 does not change them. | Open, pre-existing |
| G1142 | The existing webhook ingress answers pause, size and rate before verification (`crates/application/src/events.rs`, `receive`), an existence oracle mitigated by random endpoint ids. 021 routes and e-mail endpoints authenticate first (D1145); the webhook path is unchanged. | Open, pre-existing |
| G1143 | The bounded cross-chunk streaming scanner for blob outputs (D1132) does not exist; blob mode is refused at publication until its tests (T021-076) pass. | Open |

## Owner questions

- **O1120 — Which inbound e-mail providers first?** Options: Mailgun JSON forward plus a generic
  relay (recommended: signed body, documented retry semantics, no MIME parsing); add AWS SES via SNS
  (needs certificate fetch and verification, G1127); SendGrid/Postmark (weaker authentication).
  Default assumed: Mailgun + generic relay.
- **O1121 — Should developers hold `resource:effect` by default?** Recommended: yes, because sending
  requests and messages is the purpose of these resources and publication policy already gates
  protected namespaces; alternative: admin only, like `database:write`. Default assumed: yes.
- **O1122 — Should namespaces define custom resource types in this feature?** Recommended: no;
  platform-provided types only, revisited with 028's curated template hub (the constitution excludes
  a connector marketplace in V1). Default assumed: no.
- **O1123 — Accept the placement rule of ADR-0055 decision 11 for every parity draft?** (a)
  tenant-authored requests only in managed executors; (b) non-database fixed-shape deliveries to
  tenant destinations may be sent by the engine through the guarded client, holding only that
  destination's credential; (c) no engine or control-plane connection to a customer database,
  whatever the protocol (ClickHouse over HTTP included); (d) the new connector
  kinds in the integration runner. Alternative: every customer-credentialed call in a managed
  executor, which would move ADR-0025 deliveries, 019 push, 020 sinks, 026 channels and 028 Git sync
  out of the engine. Recommended: accept (a)–(d); it matches ADR-0025's accepted practice and keeps
  the stricter rule where tenant-authored content or customer databases are involved.

## Review resolution (2026-10-27)

Independent architecture and security reviews (round 1) and the coordinator's cross-feature
decisions (round 2). Every Blocking and Should-fix finding is fixed; Minor ones as listed.

| Finding | Resolution |
|---|---|
| Arch B1 / Sec B2 — new clients in the engine; rebinding | Fixed: managed streams in the integration runner over pre-checked sockets (D1156, ADR-0055 §5–6); SC-021-007 covers every new kind; Kafka/NATS/SQS stay (G1137) |
| Sec B1 — replay of HMAC routes with `generate` | Fixed: `generate` refused on HMAC and OIDC routes (D1157); tests in T021-033 |
| Arch S1 — CDC dedupe after the window; plan/contract contradiction | Fixed: purge guard on CDC inbox rows; plan matrix aligned (D1139) |
| Arch S2 — `object_store` outside `infra-blobstore` | Fixed: `CustomerBucket` factory; per-crate client allowlist (D1133, T021-016) |
| Arch S3 — blob outputs from secret-bearing steps | Fixed: streaming scanner, blob mode refused until proven (D1132, G1143, T021-076) |
| Arch S4 — captured headers not signed | Fixed: `canonicalHeaders` in `octopus-route-in-v1` (D1145) |
| Arch S5 — no combined lock order | Fixed, then re-homed: 021's rows sit in ADR-0050 decision 7 slots 4a, 4b, 5 and 6 (D1159, D1624, C4, 018 G1605) |
| Arch S6 / Sec S2 — CDC lease keeps its credential after revocation | Fixed: renewal and batches recheck, end the lease, no credential on renewal (D1138) |
| Arch S7 — only `postgresql` listed | Fixed: seven drivers `managedBy: connections` (D1121) |
| Sec S1 — `secret:use` sends a secret to the user's host | Fixed: `secret:manage` to bind a slot to a destination (D1158); pre-existing kinds G1141 |
| Sec S3 — `Message-ID` dedupe suppression | Fixed: id = digest of the normalized document (D1146) |
| Sec S4 / coordinator item 6 — answers before authentication | Fixed for routes and e-mail (D1145); pre-existing webhook path G1142 |
| Sec S5 — user values in envelope fields | Fixed: no e-mail subject in the envelope; data-derived CDC correlation keys disclosed like `data` (D1146, D1149) |
| Arch minors: paused `503` before auth; preview label sets; MQTT ack order; Mailgun conditional; one tenant per token | Fixed (D1145; plan Observability; D1141; FR-021-028; integration-runner contract) |
| Arch minor: ADR-0022 citation | No change needed: 021 cites only ADR-0012 §5 for acknowledgement after commit |
| Sec M1 identifiers, M2 token endpoints, M3 header names | Fixed (D1139, D1143, D1144, resource-types contract) |
| 024 S6 — role count | Fixed on the 021 side: the role set is listed and tests assert the set (FR-021-031) |
| C1 reset rules | Fixed: no new journal event type; side-table rules (D1621, T021-079) |
| C3 placement rule | Fixed: ADR-0055 decision 11 + O1123 (D1620); the claim that 026 uses 021 resources is removed |
| C4 lock order | Fixed: cites ADR-0050, 021 sub-order only (D1159) |
| C6 FR-016-002 carriers | Fixed: spec "Amendment to feature 016" + T021-064 |
| C7 encoded fields | Fixed: `ENCRYPTED_VALUE` replaced by 031's `ENCODING_READ_BY_EXPRESSION`, registry cited |
| C10 folders | Fixed: folder placement and 029 evaluator (D1622, T021-078) |
| C12 CDC and failover | Fixed: flush after quorum durability (D1623) |
| C16 reason naming | Fixed: `RESOURCE_DESTINATION_DENIED`, `CONNECTOR_DESTINATION_DENIED` |
| C18 draft test runs | Noted in Dependencies (022 owns the rule) |
| Final verification: rule 11(c) wording; engine linking of managed kinds | Fixed: 11(b) says non-database deliveries, 11(c) forbids any protocol to a customer database (ClickHouse over HTTP included); per-kind cargo features and a `cargo tree` CI assertion (T021-080, ADR-0055 Enforced by) |
| C21 new process | Fixed: Constitution II "Justified" with 017 `database-runner` precedent in Complexity Tracking |

