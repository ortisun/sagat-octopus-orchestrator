# Feature 021 — Baseline, sources, decisions, gaps and owner questions

Baseline: worktree `feature/018-032-parity-specs` at `70f09435` (release 0.2.0), read on 2026-10-26.
Identifiers of this feature: decisions D1120–D1159, gaps G1120–G1159, owner questions O1120–O1159.
Used: D1120–D1159, G1120–G1143, O1120–O1123, and the coordinator's overflow numbers D1620–D1624. Nothing below is implemented.
From 2026-11-24 (readiness review): the decision range D1120–D1159 is full and the shared overflow 1600–1674 is
fragmented across drafts, so new decisions take the feature-scoped form `D021-NNN` (written "Decision 021-NNN" in prose),
which no other feature can take; gaps continue at G1144 and owner questions at O1124. Used on 2026-11-24: D021-001 to
D021-021, G1144 to G1152. See [Readiness review (2026-11-24)](#readiness-review-2026-11-24).

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

Every question below was decided on 2026-11-24 under the owner's delegation, "delegate (OD-2026-11-24-4)", which widens
Owner decision 2026-11-20-6: each option was weighed and the one with the most architectural and business advantage was
chosen, never the simplest alone. The original text of each question is kept under its decision.

| ID | Decision | Decided by | Status |
|---|---|---|---|
| O1120 | Generic relay and Mailgun JSON forward first; SES through SNS, SendGrid and Postmark stay Gap 1127 | delegate (OD-2026-11-24-4) | Decided |
| O1121 | Developer holds `resource:effect` by default; 029 custom roles and 011 publication policy narrow it | delegate (OD-2026-11-24-4) | Decided |
| O1122 | No namespace-defined resource types; the catalog gains a provenance extension point that design 047's curated hub fills with signed type versions specialising the closed operation kinds | delegate (OD-2026-11-24-4) | Decided |
| O1123 | Placement rule (a) to (d) of Architecture decision 0055 decision 11 accepted, with an added clause (e) for the OAuth token broker of design 047 | delegate (OD-2026-11-24-4) | Decided |

**Owner question 1120, reasoning.** Options: (a) a generic relay plus the Mailgun JSON forward; (b) (a) plus AWS SES
through SNS; (c) SendGrid or Postmark first. The generic relay makes every provider reachable on day one through an
operator relay that posts the canonical document under an existing signing profile, so the platform carries no
provider-specific parsing for the long tail; Mailgun is the one provider whose JSON forward signs the body, so its replay
bound is the timestamp window plus the inbox dedupe with no token table. SES through SNS needs certificate fetching and
verification of SNS messages (a new trust path to a provider-chosen URL) and is a larger change than its adoption
justifies before the relay exists; SendGrid and Postmark authenticate weakly (basic credentials or IP lists) and would
lower the bar the endpoint keeps. Chosen: (a). SES, SendGrid and Postmark remain Gap 1127; each can be added as a
profile later without changing the endpoint kind. This is a server feature, not a hub item: a profile verifies a
signature in the control plane, which design 047's hub, which runs nothing in the server, cannot carry.

**Owner question 1121, reasoning.** Options: (a) developer holds `resource:effect` by default; (b) admin only, like
`database:write`; (c) no default holder, granted per namespace. Sending requests and messages is the purpose of these
resources, and Windmill's model lets authors call what the resource admin configured; (b) would make every effectful
workflow wait for an admin and push authors back to hand-written workers holding raw secrets, which is less safe, not
more. The protection that (b) offers is kept where it matters: binding a secret to a destination still needs
`secret:manage` (Decision 1158), publication policy (011) still gates protected namespaces, and 029's custom roles and
restricted folders can remove `resource:effect` from developers per folder. `database:write` stays admin-only because a
database write is broader (arbitrary SQL on customer data) than a typed operation on a configured endpoint. Chosen: (a).

**Owner question 1122, reasoning.** Options: (a) platform-provided types only, no extension; (b) platform types plus
curated type versions installed from the signed hub of design 047 (constitution 5.0.0 wording: a curated integration and
template hub whose only sources are the release's signed index and operator-pinned indexes, whose items are imported
under the same publication, signature, secret and isolation rules as authored content, and which runs nothing in the
server); (c) namespace-defined types with authored JSON Schema settings and authored operations. Option (c) would make a
tenant the author of a type's host binding, auth placement and request shape: every guarantee that the catalog gives
today (fixed probe, egress profile, executor role per operation, the host-bearing settings that Decision 1158 protects,
the refused-header list, the effect class) would become tenant-authored data that the integration runner must trust,
and it adds nothing that an `http_api` resource with an authored `http.request` cannot already do. Option (a) leaves the
catalog at five types and loses the breadth that Windmill's resource-type library gives. Option (b) gives that breadth
(GitHub, Stripe, Google Sheets and similar as named types with fixed hosts and auth) while every hub type version is a
specialisation of one of 021's closed operation kinds (`http.request`, `graphql.execute`, the S3 set, `smtp.send`,
`slack.postMessage`), so the runner executes nothing new. A hub-installed type is not namespace-defined: it is authored
by the hub publisher, verified by the pinned key and pinned by version like a platform type. Chosen: (b). 021 builds the
extension point (a type version records its provenance, `platform` or `hub`, and a hub type key lives under a reserved
prefix; validation of a specialisation against its operation kind; refusal of any namespace-authored type) and ships
only `platform` types; design 047 builds installation, the index and the hub type versions. Gap 1133 is closed by this
decision (Decision 021-004).

**Owner question 1123, reasoning.** Options: (a) accept clauses (a) to (d) of decision 11 as written; (b) every
customer-credentialed call in a managed executor, which would move ADR-0025 deliveries, 019 push, 020 sinks, 026
channels and 028 Git sync out of the engine; (c) accept (a) to (d) and add a clause for design 047's token broker. Since
the question was written, design 047 places an OAuth token broker in the control plane that sends token requests
(authorization-code exchange, refresh, client credentials) to provider token endpoints with client secrets and refresh
tokens that never leave it. Under clauses (a) to (d) as written that call has no home: it is neither tenant-authored
content (clause a) nor an engine delivery (clause b). Option (b) would multiply managed executors for fixed-shape calls
whose content no tenant authors, with no gain in isolation, and would give refresh tokens to a process that runs
tenant-authored requests, which is the exposure the rule exists to prevent. Chosen: (c). Clause (e): a token request of
a fixed, standard shape (RFC 6749 grants) to a token endpoint fixed by an OAuth provider definition or an operator- or
tenant-registered OAuth client MAY be sent by the platform token broker in the control plane through the guarded egress
client; it carries no tenant-authored content, and the broker hands executors only a current access token
(Decision 021-002). Recorded in Architecture decision 0055 decision 11.

Original text of the questions (2026-10-26):

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

Note (2026-11-15, from feature 041): the owner gave worker-protocol minor 1.7 to feature 041 (step logs, OD-2026-11-15-8).
Where this feature names 1.7 for its own addition, it takes the next free minor of the register in `sdks/SPEC.md` §9 when
its contract merges.

## Readiness review (2026-11-24)

Baseline: `docs/windmill-parity-readiness` at `2c94064f` (today's `origin/develop`), read on 2026-11-24 for build. Every
fact below was read in the code or the documents at that commit; nothing of this feature is built. The draft was written
on 2026-10-26 against release 0.2.0; features 018, 033, 035, 038, 041, 042 (collection slices), 043 and the instance
registry (Architecture decision 0082) have been built or specified since.

### What changed under the draft (verified facts)

| Area | Fact at `2c94064f` | Where | Effect on 021 |
|---|---|---|---|
| Worker protocol | the code serves minor 1.6 (`PROTOCOL_MINOR` and `DATABASE_PROTOCOL_MINOR` are 6); the register in `sdks/SPEC.md` gives 1.7 to step logs (041, not built); branch `docs/delivery-acknowledgement` reserves 1.8 for delivery acknowledgement (Architecture decision 0099) | `crates/application/src/worker.rs`, `sdks/SPEC.md` | Decision 021-005 |
| Migrations | the newest is `20270119000000_batch_purge`; the draft said "after `20261220000000`" | `crates/infra-postgres/migrations/` | Decision 021-006 |
| `runs.origin` | values `AD_HOC_STEP`, `AD_HOC_WORKFLOW`, `CONNECTION_PROBE` (`20261125000000_database_ad_hoc`); 017 probes run as a retained platform-built definition | `crates/application/src/runs/ad_hoc.rs` | Decision 021-015 |
| Shared runner plumbing | `crates/infra-managed` does not exist; the lake runner of feature 035 is a profile of `apps/database-runner` (module `lake/`) and shares its lease, heartbeat and report code inside that crate | `apps/database-runner/src/{lib,lease,runner}.rs` | Decision 021-007 |
| Claim path | a managed step is a `Managed` / `ManagedKind` variant (`crates/domain/src/workflow.rs`, `crates/domain/src/database.rs`), checked at every site by `crates/application/tests/managed_kind_sites.rs`; managed steps are root-only and leased only above their minor; lease input is `{}`; admission is `database_claim` inside the claim transaction; design 038's budget test fails when statements per lease move (Requirement 038-001) | `crates/application/src/worker.rs` (`claim_root_run`), `crates/infra-postgres/src/execution.rs` | Decision 021-008 |
| Concurrency | the claim counts only `activity_concurrency` and `activity_quota_limits` (per activity) today; Architecture decisions 0088 and 0089 (accepted) forbid the claim to read or write a semaphore or a window: limits are granted by the scheduler; 022's throughput slice is in release 0.3.0 but not on `develop` | `crates/infra-postgres/src/workers.rs`, ADR-0088 decision 5, ADR-0089 decision 1 | Decision 021-009 |
| Run input validation | `RunInputGate` is built; design 043 lists thirteen creators; row 9 (event, webhook and message triggers) is the dispatcher path that every 021 source reaches; row 13 covers ad hoc database execution and the connection probe; Architecture decision 0090 makes every creator pass the gate before its transaction | `crates/application/src/run_input.rs`, `crates/scheduler/src/events.rs`, `docs/designs/043-run-input-validation.md` section "1. The creators" | Decision 021-010 |
| Instance registry | `Capability` holds `schedules-v1`, `access-narrowing-v1`, `payload-encoding-v1`, `run-input-validation-v1`; only control plane and engine register; a new capability widens three CHECKs and is switched on by an audited operator activation (Requirement 018-052); the IR has its own per-process publication flags | `crates/application/src/instances.rs`, `crates/infra-ir/src/capabilities.rs` | Decision 021-011 |
| IR node kinds | no registry table: `ROOT_NODE_TYPES` and `PRIVATE_NODE_TYPES` in `crates/infra-ir/src/schema.rs`, dispatch in `convert.rs`; Architecture decision 0080 (proposed) keeps platform-only nodes out of the public schema | `crates/infra-ir/src/` | Decision 021-015 |
| Step logs | design 041 (approved) and Architecture decision 0084 decision 5: the database and lake runners capture no log; only `worker:execute` sessions at 1.7 may write | `docs/designs/041-step-logs.md` | Decision 021-012 |
| Notification transport | NATS JetStream or Redis Streams (Architecture decision 0069) behind `EventPublisher`; Redis is an outbound destination only, no inbound Redis connector | `crates/application/src/maintenance.rs`, `specs/033-redis-streams/spec.md` | Decision 021-013 |
| Stream operators | design 042 puts operator chains between the inbox and a run, per trigger, in a later release than 0.3.0 by its own text (Owner decision 2026-11-24-1 now brings slices 2 to 5 into 0.3.0); its non-goals leave connector admission to Requirement 021-029; it has no preview | `docs/designs/042-stream-operators.md` | Decision 021-014 |
| Database drivers | `GET /database-drivers` lists `postgresql` and the lake drivers `iceberg`, `object_storage`, `clickhouse` (feature 035) | `specs/001-platform-foundation/contracts/openapi.yaml` | Decision 021-016 |
| Connector leases | `connector_leases (connector_id, stream, holder, expires_at)` is unchanged since `20261012000000_events`; Architecture decision 0073 (rank-shared partitions, feature 036) is proposed and not built | migrations | Decision 021-017 |
| Kafka key | still dropped: `RawMessage` has `position`, `fallback_id`, `headers`, `body` only | `crates/infra-connectors/src/kafka.rs`, `crates/application/src/connectors.rs` | Requirement 021-029 stands |
| Webhook ingress | pause, size and attempt rate are still answered before verification | `crates/application/src/events.rs` (`receive`) | Gap 1142 stands |
| Trigger run identity | `start_key(event)` and scope `trigger:{id}`; no business identifier for event-started runs | `crates/application/src/dispatch.rs`, `crates/scheduler/src/events.rs` | Gap 1151 |
| OpenBao | in the development overlay `infra/docker/compose.database.yaml` (pinned image), not in `compose.yaml` | `infra/docker/` | Decision 021-018 |
| Design 047 | being written on 2026-11-24; owns OAuth credentials, the token broker, client registration, the curated hub and resources for ordinary activities and scripts | coordinator's boundary | Decisions 021-001 to 021-004 |

### Decisions (2026-11-24)

Each decision below that settles an open design question was taken under "delegate (OD-2026-11-24-4)": the options were
weighed and the one with the most architectural and business advantage was chosen.

- **D021-001 — The OAuth scheme references a design 047 credential; the runner never holds a client secret.** The
  `oauth2_client_credentials` scheme of `http_api` and `graphql` (with its `tokenUrl`, `clientId`, slot `clientSecret`)
  is replaced by the scheme `oauth`, whose one slot (shape `oauth_credential`) names an OAuth credential of design 047
  (authorization code or client credentials). `ResolveResource` asks 047's token broker, between its two transactions
  and with none open, for the credential's current access token and returns it as that slot's value; the refresh token
  and client secret never leave the broker. Until 047's broker capability is switched on, a resource revision or a
  publication that uses the scheme is refused (`OAUTH_BROKER_UNAVAILABLE`), so 021 builds and ships without 047.
  Options weighed: (a) keep the runner's per-attempt client-credentials request (the draft, Decision 1131): it puts a
  client secret in the process that runs tenant-authored requests and cannot serve authorization-code credentials at
  all; (b) the runner asks the broker over its own channel: a second capability path to secure and audit; (c) the
  server's `ResolveResource` obtains the token: one capability path, one audit, the same fences. Chosen (c). This
  supersedes Decision 1131 for OAuth (the runner sends no token request and keeps no token across attempts; the broker's
  token lifetime is 047's), the token-request half of Decision 1130 (the runner still builds the `Authorization` header
  from the slot value) and closes Gap 1129 here (moved to design 047). Host binding: the token endpoint is fixed by the
  047 credential, so `tokenUrl` is no longer a host-bearing setting of 021; binding an `oauth` slot to a destination
  (creating the resource, changing `baseUrl` or the slot's credential) requires the manage permission that design 047
  defines on its credential, as Decision 1158 requires `secret:manage` for a secret.
- **D021-002 — Placement clause (e) for the token broker.** Owner question 1123 above; recorded in Architecture
  decision 0055 decision 11(e).
- **D021-003 — Out of Scope points at design 047.** "OAuth2 authorization-code/refresh flows", "resources consumed by
  ordinary activities and scripts" and "a connector marketplace" are replaced by pointers to design 047 and to the
  constitution 5.0.0 hub wording; Gaps 1129 and 1132 move to design 047. 021 keeps nothing of those that would need 047
  to be built.
- **D021-004 — The type catalog has a provenance extension point; no namespace defines a type.** Owner question 1122.
  A type version records `provenance` (`platform`, or `hub` with the index, publisher key and item digest that design
  047 pins) and a hub type key lives under the reserved prefix `hub.` (`hub.<publisher>.<name>`); a hub type version
  MUST name only operation kinds of the platform catalog (`http.request`, `graphql.execute`, the S3 set, `smtp.send`,
  `slack.postMessage`), with each operation's fields narrowed (fixed `baseUrl` host, fixed auth scheme, fixed or bounded
  paths and methods) and never widened; its effect class, executor role, egress profile and probe are those of the
  operation kind it specialises. 021 builds the validator and the `platform` source; the `hub` source is installed by
  design 047. Any request to create a type from a namespace is refused (`RESOURCE_TYPE_NOT_AUTHORABLE`). Closes Gap 1133.
- **D021-005 — Protocol minor: the next free one at merge, provisionally 1.9.** 1.7 is step logs (041), 1.8 is reserved
  for delivery acknowledgement (feature 006, Architecture decision 0099, on its branch). 021's integration-runner RPCs and
  stream RPCs take one minor; the database runner's CDC profile needs the same minor. The register in `sdks/SPEC.md` is
  updated when 021's contract merges, and a host of a later minor also carries the 1.8 procedure (as that branch states).
- **D021-006 — Migration timestamps after the newest on `develop` at merge** (`20270119000000` on 2026-11-24), allocated
  by the coordinator; the draft's `20261220000000` is stale. Decision 1151 stands with this baseline.
- **D021-007 — The integration runner is its own binary; the shared plumbing is extracted from both profiles of the
  database runner.** Options weighed: (a) a third profile of `apps/database-runner`, as the lake runner is (feature 035):
  one binary, separate deployments; (b) a separate `apps/integration-runner` over a new `crates/infra-managed`. The lake
  profile works because it keeps the database runner's role and boundary (Architecture decision 0071 decision 1); the
  integration runner has another role (`integration-runner`), another egress (public internet, brokers, cloud APIs) and
  other client libraries. One binary would link HTTP-provider, SMTP, MQTT and WebSocket clients into the process that
  reaches private customer databases, which defeats the per-crate client allowlist (Task 021-016) and the engine feature
  check (Task 021-080), and one vulnerable client would be reachable from a database deployment by configuration alone.
  Chosen (b). `crates/infra-managed` takes the session, poll, lease and heartbeat, report receipt and slot code that the
  SQL and lake profiles share today (`apps/database-runner/src/{runner,rpc,lease,slots}.rs`), without behaviour change;
  the guard suites are the database-runner, database-connections and lake suites of feature 035 (Decision 1153 widened).
- **D021-008 — A managed integration step is a new `ManagedKind`, admitted inside the claim within design 038's
  budget.** `ManagedKind::Integration` (with the reserved activities `sagat.integration.*`) joins `Managed`; every site
  of `crates/application/tests/managed_kind_sites.rs` decides it; it is leased only to a session at 021's minor holding
  role `integration-runner`; lease input is `{}`; its admission (pinned binding present, current resource status) is one
  call beside `database_claim` in the same claim transaction. The change that adds it moves design 038's recorded claim
  budget in the same pull request (Requirement 038-001), and a candidate with nothing to lease still costs at most one
  statement after the run lock (Requirement 038-002).
- **D021-009 — Concurrency: namespace and tenant limits are activity quotas; the per-resource limit is a 022 concurrency
  key.** The draft counted per-resource, namespace and tenant integration concurrency at the claim. Since then
  Architecture decisions 0088 and 0089 forbid the claim to read or write a semaphore or a window. Options weighed: (a)
  new counters at the claim (the draft): contradicts Architecture decision 0089 and adds claim statements; (b) per-runner
  slots only: no durable or fair limit and no tenant bound; (c) the namespace and tenant bound through the existing
  `activity_quota_limits` and `activity_concurrency` rows for the reserved `sagat.integration.*` activities (already
  checked in the claim, no new statement), and the per-resource bound as a durable concurrency-key permit
  `resource:{resourceId}` of Architecture decision 0056 granted by the scheduler's gated-step routine (Architecture
  decision 0089), so a step over the limit waits outside `READY` instead of failing. Chosen (c): it reuses the platform's
  one admission port and one grant routine, the limit is durable, fair and visible like every 022 limit, and nothing is
  added to the claim. Until 022's gate is switched on, a revision that sets `limits.concurrency` is refused
  (`CONCURRENCY_GATE_UNAVAILABLE`); the namespace and tenant quotas work without it. The quota names
  `integration_concurrency` and `tenant_integration_concurrency` of the draft are dropped; the failure code `CAPACITY`
  is not produced by the server.
- **D021-010 — Every 021 run creator passes the `RunInputGate`.** Routes, e-mail endpoints and the five new connector
  kinds start runs only through `webhook` and `message` triggers, so they are design 043's row 9 (the dispatcher's
  `start`, which already calls the gate with `Creator::Trigger`; a refusal is a delivery without a run, audited once as
  `run.input_refused`). The resource probe is a new creator beside row 13 (connection probe): it builds a retained
  platform definition with input `{}` and an `inputSchema` that accepts exactly `{}`, and passes the gate with a new
  `Creator::ResourceProbe` before its transaction opens. 021 asks design 043's owner to add the probe to the table of
  creators and a type that refuses an unchecked input applies to it. A route answers `202` once its event is in the inbox;
  an input refusal is asynchronous (a delivery outcome), and feature 026's `response.mode: wait` is where a caller can
  receive it synchronously. The admission pins of Requirement 021-004 and Requirement 021-019 are taken in the
  transaction that the gate's checked input enters, for every creator of the table.
- **D021-011 — Rollout through the instance registry.** Five capabilities join `Capability` (with the three CHECKs, a
  `NOT VALID` widening and its validate migration, and a down migration that refuses while a value is recorded):
  `integration-resources-v1` (resource catalog writes, `resourceOperation` publication and admission, probes),
  `namespace-variables-v1` (`vars` in expressions), `inbound-endpoints-v1` (`ROUTE` and `EMAIL` endpoint kinds),
  `managed-streams-v1` (the five `MANAGED` connector kinds and the stream RPCs; every engine must skip a `MANAGED`
  connector before one exists) and `connector-filters-v1` (connector filters, `event.message`, the filter preview). Each
  is switched on by the audited operator activation of Requirement 018-052, and until then the feature refuses its own
  use with its own reason, as Requirement 018-052 asks: `503 UNAVAILABLE` with `details.reason =
  INTEGRATION_CAPABILITY_NOT_ACTIVATED` and `details.capability` naming the capability (a process-level
  `BINARY_CAPABILITY_MISSING` stays the registry's). Runners do not register (only control plane and engine do);
  the server gates runners by minor and role.
- **D021-012 — The integration runner captures no step log in release 0.3.0.** Design 041 states that managed runners
  capture none and Architecture decision 0084 decision 5 lets only `worker:execute` sessions write. Options weighed: (a)
  no log, as the database and lake runners; (b) a platform-written operation log (request line, status, timing): useful
  for debugging, but it would carry provider text and paths that Requirement 021-017 and Requirement 021-032 keep out of
  every record, and needs a value-redaction proof per encoding like the output scanner. Chosen (a); the typed result
  envelope, closed failure codes, spans and metrics are the diagnosis. A step-log append from an `integration-runner`
  token is refused like a database-runner one (Gap 1145).
- **D021-013 — "Lost NATS hint" becomes "lost transport hint".** Hints travel on NATS JetStream or Redis Streams
  (Architecture decision 0069); the reconciler finds ready integration tasks and unclaimed streams whichever transport
  is configured. No inbound Redis connector is added (feature 033 leaves it out).
- **D021-014 — Connector filters stay stateless booleans before admission; operator chains stay after the inbox.**
  Design 042 decides which events of a trigger become a run, after the inbox, per trigger; its non-goals leave connector
  admission to Requirement 021-029. Options weighed: (a) both as specified, each in its place; (b) operator chains on
  connectors as well: stateful operators before the inbox would drop or merge events that no record keeps, against
  constitution Principle VII; (c) no connector filter, only trigger chains: every unwanted record of a busy topic
  would cost an inbox row and its retention. Chosen (a). The filter preview covers trigger `filter` and connector
  `filter`; its `scope` is an open enumeration so that design 042 may add the stateless prefix of a chain; 021 adds no
  operator.
- **D021-015 — The probe is a retained platform-built definition.** As 017's connection probe (`RunOrigin::ConnectionProbe`
  in `crates/application/src/runs/ad_hoc.rs`), a resource probe admits a retained platform-built definition with origin
  `RESOURCE_PROBE` (an additive `runs.origin` value beside `AD_HOC_STEP`, `AD_HOC_WORKFLOW`, `CONNECTION_PROBE`). Its node
  is a `resourceOperation` whose operation is the type's fixed probe (`probe`), which no author can write: if
  Architecture decision 0080 is accepted it is a private node type (`resourceProbe`, in `PRIVATE_NODE_TYPES`, parsed only
  for an `internal` workflow row); otherwise the probe operation name is refused by the public codec. `resourceOperation`
  itself joins `ROOT_NODE_TYPES` and the dispatch of `crates/infra-ir/src/convert.rs`.
- **D021-016 — The listing names the lake drivers too.** `GET …/resource-types` lists every connection-catalog driver
  that `GET /database-drivers` lists (`postgresql`; the lake drivers `iceberg`, `object_storage`, `clickhouse` of feature
  035) and the 024 drivers (`mysql`, `mariadb`, `sqlserver`, `bigquery`, `snowflake`, `duckdb`) as `managedBy:
  connections`; the resource API refuses all ten (`MANAGED_BY_CONNECTIONS`). The list is read from the connection
  catalog's own driver registry, not copied, so a driver added later joins it without a 021 change. An `s3_bucket`
  resource and an `object_storage` lake connection may point at the same bucket: the first serves typed operations in
  workflows, the second is a lake storage location (feature 035); neither reads the other's credentials.
- **D021-017 — Stream leases extend `connector_leases` and leave rank sharing to Architecture decision 0073.** 021 adds
  `generation`, `holder_role`, `credential_version` and `state` to `connector_leases`, whose shape has not changed since
  feature 004. Architecture decision 0073 (proposed, feature 036) would share engine partitions by rank; it governs the
  engine's Kafka placement only. When it is accepted, the migration that lands second keeps both column sets
  (rank columns NULL for `MANAGED` leases) and the multi-lease kinds (Pub/Sub, Service Bus) keep distinct stream names.
- **D021-018 — Tests use the OpenBao of the development overlay.** `infra/docker/compose.database.yaml` already carries
  a pinned OpenBao; 021's fixtures start their own disposable OpenBao container as 017's tests do (AGENTS.md rule 16),
  and the quickstart starts the overlay. Feature 015 (production OpenBao and OpenTelemetry) is not a prerequisite.
- **D021-019 — Hub items run nothing new.** A hub resource-type version, OAuth provider definition or script template
  (design 047, design 046 for script package dependencies) executes only through the paths this feature and the existing
  script runtime define; 021 adds no hook, script or interpreter for hub content.
- **D021-020 — Build slices.** The tasks are cut into slices that two builders run in parallel on disjoint crates
  ([plan.md](plan.md#build-slices-2026-11-24)).
- **D021-021 — One canary scanner crate.** The runner's scan of every resolved slot value (Requirement 021-016) and the
  server's rescan of inline output and of blobs before `COMMITTED` must be the same code, or a form one of them misses
  becomes a leak. The server cannot depend on `crates/infra-integrations` without linking the provider clients into the
  control plane. The scanner is therefore a dependency-free crate, `crates/output-scan` (bytes in, match or no match
  out, the encodings and cross-chunk overlap of Decision 1132), used by `crates/infra-integrations` and by the server's
  completion path; the per-crate allowlist of Task 021-016 lets every crate depend on it.
- **D021-022 — The integration runner's disclosure depends on a declared set Principle V does not list yet
  (2026-11-25, delegate (OD-2026-11-24-4)).** The runner receives the values a resource's slots bind (secrets, and for an
  `oauth` slot an access token) for an operation the step declares. Principle V's executor-read exception lists two sets
  (ordinary declared secrets and the managed database capability). Design 047's Technical Plan 12 proposes a third set,
  "a declared resource", bound like managed database delivery for the integration runner; this feature's capability
  resolution depends on that wording being in the constitution before it ships, which the coordinating session is asked
  to carry into the 5.0.0 amendment or the next one.

### Gaps (2026-11-24)

| Gap | Description | Status |
|---|---|---|
| G1144 | The `oauth` scheme needs design 047's token broker and credential API; refused until 047's broker capability is on (Decision 021-001). | Open until design 047 is built |
| G1145 | No step log for integration operations (Decision 021-012); a redacted operation log needs its own design. | Accepted for 0.3.0 |
| G1146 | Architecture decision 0080 (private platform-only nodes) is proposed; the probe node's privacy follows it or the codec refusal of Decision 021-015. | Open until Architecture decision 0080 is decided |
| G1147 | The per-resource concurrency limit needs 022's concurrency-key gate (Architecture decisions 0056 and 0089), which is not on `develop`; until it is, the setting is refused (Decision 021-009). | Open until 022's throughput slice merges |
| G1148 | Design 043's table of creators does not list the resource probe; its owner adds the row (Decision 021-010). | Open, requested |
| G1149 | The protocol minor (provisionally 1.9) depends on the delivery-acknowledgement branch merging 1.8 first (Decision 021-005). | Open, allocated at merge |
| G1150 | Architecture decision 0073 (rank-shared connector partitions) and 021's lease columns must be reconciled by whichever migration lands second (Decision 021-017). | Open until Architecture decision 0073 is decided |
| G1151 | Event-started runs carry no business identifier (`start_key` only); a route that maps a captured value to a business identifier (feature 018) is not designed. | Open, later |
| G1152 | Design 047 depends on Tasks 021-005, 021-012, 021-017, 021-023 and 021-085 (catalog, `ResolveResource` and the provenance extension point); its own build waits for them. | Open, ordering |

### Status of earlier gaps after the review

- Gap 1120: unchanged; feature 014 closed on root collections whose bodies hold only effect-free ordinary activities, and
  nesting moved to feature 039, so an operation in a body still needs per-instance capability proofs.
- Gap 1129 and Gap 1132: moved to design 047 (Decision 021-003).
- Gap 1133: closed by Owner question 1122 (Decision 021-004).
- Gap 1137 and Gap 1142: unchanged in the code at `2c94064f`.
- Decision 1131: superseded for the OAuth scheme by Decision 021-001.
