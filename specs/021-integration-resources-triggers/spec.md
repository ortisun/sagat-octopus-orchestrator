# Feature Specification: Integration resources, managed operations, variables and triggers

**Feature Branch**: `feature/021-integration-resources-triggers`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Input**: Generic, versioned resource types and resources (Windmill resources) that generalize the
feature-017 connection catalog without breaking it: JSON-Schema'd settings, secret slots backed by
OpenBao and an auth scheme, with a first platform-provided set (HTTP API with bearer, basic, API-key
header and OAuth2 client credentials; GraphQL endpoint; S3-compatible bucket; SMTP; Slack bot),
revisions, pause, probe and pinning at run admission like 017 connection revisions. Typed managed
operations on those resources (HTTP request, GraphQL operation, S3 get/put through the blob claim
check, SMTP send, Slack post) executed by a managed executor under lease, never in the API or the
scheduler, behind the egress guard, at-least-once with idempotency guidance. Non-secret, typed,
versioned, classified namespace variables readable in expressions. New triggers through the inbox:
PostgreSQL CDC, custom HTTP routes (asynchronous `202`; the synchronous variant belongs to 026),
WebSocket client, MQTT, Google Cloud Pub/Sub, Azure Service Bus, inbound e-mail through a provider
webhook, Kafka key/value filters and a filter preview in the Studio.

## Context and delivery boundary

Authors reach external systems today through their own SDK workers, inline scripts whose requests
go through the sandbox supervisor, or the PostgreSQL step of 017; configuration and credentials are
repeated per worker. Inbound events come from signed webhooks and NATS, Kafka and SQS connectors
(004). This feature adds a **resource catalog** for non-database integrations, a closed set of
**managed operations** run by a new isolated executor, **variables**, seven inbound source kinds and
two filter capabilities. It changes nothing in how connections, activities, scripts, outbound
subscriptions or existing connectors behave. Architecture:
[ADR-0055](../../docs/adr/ADR-0055-integration-resources-and-inbox-connector-kinds.md) (Proposed);
decisions D1120–D1159 and D1620–D1624, gaps G1120–G1143, owner questions O1120–O1123 and the
resolution of the 2026-10-27 reviews: [research.md](research.md).
Per-kind detail is normative in [contracts/](contracts/). Nothing described here exists.

## Dependencies and ownership

Reused unchanged in meaning: 001 identity, contracts, error envelope; 002 admission, leases, fencing,
receipts, blob claim check (ADR-0023); 003 sagas, reconciliation, manual review (ADR-0010); 004
inbox, dispatcher, triggers, connectors, endpoints (ADR-0012, ADR-0025, ADR-0026); 006 conformance
(ADR-0015); 007 Studio (ADR-0016); 008 telemetry (ADR-0017); 009 RBAC, secrets, egress, quotas
(ADR-0027); 011 publication; 014 additive IR and scoped identity (ADR-0032); 016 classification
(ADR-0033); 017 OpenBao, managed execution, connections, database runner (ADR-0035, ADR-0036).

New features: **018** owns the merged admission lock order (ADR-0050 decision 7; 021's rows sit in slots
4a, 4b, 5 and 6) and the reset-rule registry (021's rules: [Reset of runs that use resources](#reset-of-runs-that-use-resources-018)).
**022** owns standalone (ad hoc) operations; here operations run only inside published workflows and
fixed probes; in 022 draft test runs a `resourceOperation` must be pinned or mocked. **024** adds
database engines in the 017 model; every 017/024 driver is listed here as `managedBy: connections`.
**025** adds LLM provider types on this catalog, executed by its own runner (a type version names
its executor role per operation). **026** owns synchronous routes (its FR-026-012 adds
`response.mode: wait` to the route defined here; 021 reserves that field and refuses it until 026
ships the waiter), notification channels and channel approvals; 026 channels keep their own channel
credentials and do not use 021 resources (ADR-0055 rule 11b). **029** owns custom roles over the
permissions added here and the folder contract that resources adopt (FR-021-031). **031**: an encoded
field is opaque to `vars`, operation expressions and filters; 021's positions (`resourceOperation`
inputs and outputs, filters) are entries of 031's registry and use its codes. **032**: in a
`sync_quorum` cell the CDC flush position waits for quorum durability (FR-021-022); under `async`
replication 032 states the loss of changes inside the RPO.

## Domain vocabulary

- **Resource type**: platform-provided, versioned definition (settings schema, secret slots, auth
  schemes, operations, probe, egress profile): `http_api`, `graphql`, `s3_bucket`, `smtp`, `slack_bot`.
- **Resource**: a namespace instance of one type version with non-secret settings and slot bindings,
  immutable revisions and a status (`ACTIVE`, `PAUSED`, `DELETED`).
- **Managed operation**: a typed request on a resource, authored as a `resourceOperation` node
  (`http.request`, `graphql.execute`, `s3.getObject|putObject|headObject|listObjects|deleteObject`,
  `smtp.send`, `slack.postMessage`). **Safe** operations change nothing at the provider; **effect**
  operations may.
- **Variable**: a non-secret, typed, versioned namespace value of level `public` or `internal`.
- **Connector kind / endpoint kind**: variants of the existing inbound aggregates (a route is an
  endpoint kind; MQTT is a connector kind). **Connector filter**: a condition applied before admission.

## User Scenarios & Testing *(mandatory)*

### US01 — Register an HTTP API resource with an OpenBao credential and probe it (Priority: P1)

An administrator registers `billing-api` (`http_api`, OAuth2 client credentials), creates its client
secret in OpenBao from the same form, saves and probes it.

**Why this priority**: every operation needs a resource.

**Independent Test**: REST and Studio against real OpenBao and local OAuth2/HTTP test servers;
reload; probe; edit with a stale revision.

**Acceptance Scenarios**:

1. **Given** an administrator of namespace A, **When** they save `billing-api` with a new client
   secret, **Then** the value is in OpenBao, the resource stores the slot's logical secret name, and
   reads return settings and metadata only.
2. **Given** a saved resource, **When** they probe it, **Then** a durable probe run executes the
   type's fixed probe in the integration runner and reports a closed outcome, time and tested
   revision; saving alone is never shown as healthy.
3. **Given** a wrong secret, a TLS name mismatch, a destination outside the egress policy or an
   unavailable OpenBao, **When** probed, **Then** each yields a distinct typed category without
   provider text, and denials are audited.
4. **Given** a stale `expectedRevision`, **When** saved, **Then** `409 CONFLICT` with
   `REVISION_CONFLICT` and no revision is written.
5. **Given** namespace B or another tenant, **When** it reads, uses or probes `billing-api`, **Then**
   it gets the opaque `404` and the attempt is audited.

### US02 — Call an HTTP or GraphQL API from a workflow (Priority: P1)

An author adds a `resourceOperation` node (`http.request` POST `/invoices` on `billing-api`) in the
Studio or any SDK; a later step reads the typed response.

**Why this priority**: calling an API without writing a worker is the main value.

**Independent Test**: publish the same definition from the Studio and each SDK; run it against a
recording HTTP server; restart the runner and the engine mid-run.

**Acceptance Scenarios**:

1. **Given** a published workflow, **When** a run is admitted, **Then** the resource revision is
   pinned for that node in the admission transaction and a later edit changes no retry of that run.
2. **Given** an effect with `idempotency: {header: "Idempotency-Key"}`, **When** the runner crashes
   after sending and before reporting, **Then** the retry sends the same key and body and the step
   records one completion.
3. **Given** an effect without an idempotency declaration, **When** its outcome is unknown, **Then**
   it is reconciled if `reconcile` is declared, otherwise the run waits in `MANUAL_REVIEW`.
4. **Given** a response above the inline bound, **When** `response.mode` is `blob`, **Then** the body
   becomes a blob descriptor; with `json`, **Then** the step fails `RESULT_LIMIT` (no truncation).
5. **Given** a provider that echoes the bearer token, **When** reported, **Then** the attempt fails
   `SECRET_IN_OUTPUT` and nothing is retained.
6. **Given** a GraphQL mutation declared `kind: query`, **When** executed, **Then** it fails
   `OPERATION_MISMATCH` before any request is sent.

### US03 — Start a workflow from a custom HTTP route (Priority: P1)

An integrator exposes `POST /orders/{orderId}/paid` with OIDC; a `webhook` trigger starts a run per call.

**Why this priority**: most third-party callers cannot produce the Octopus webhook envelope.

**Independent Test**: valid and invalid tokens, a repeated `Idempotency-Key`, a changed body under the
same key.

**Acceptance Scenarios**:

1. **Given** the route, **When** a valid request carries `Idempotency-Key: k1`, **Then** `202` after
   the inbox commit, and the run receives the body as `data` and `event.request.pathParams.orderId`.
2. **Given** the same key and body, **Then** `202 DUPLICATE` and no second run; same key, other body,
   **Then** `409` with `EVENT_ID_REUSED`.
3. **Given** an unknown route, wrong method, invalid token or stale HMAC timestamp, **Then** the same
   `401`; only an authenticated caller learns that the route is paused (`503` with `Retry-After`),
   too large (`413`), throttled (`429`) or missing its idempotency header (`400`).
4. **Given** an HMAC or OIDC route, **When** it is created with `idempotency: generate`, **Then** it is
   refused; on an mTLS route `generate` is allowed and a client retry after a lost response may start
   two runs, as the route states.
5. **Given** a captured HMAC request replayed inside its timestamp window, **Then** it carries the same
   idempotency key and is a `DUPLICATE`; a replay with a changed captured header fails verification.

### US04 — Start workflows from PostgreSQL changes (CDC) (Priority: P2)

A `POSTGRES_CDC` connector reads the customer's `orders` publication through a slot the customer
created; a `message` trigger starts a run per insert.

**Why this priority**: replaces polling, but needs customer setup and the private-network boundary.

**Independent Test**: real PostgreSQL (`wal_level=logical`); kill the runner between inbox commit and
flush report; stop the platform past the lag threshold.

**Acceptance Scenarios**:

1. **Given** a transaction inserting three rows, **Then** three events `cdc.insert`, subject
   `public.orders`, ids `{slot}/{commitLsn}/{index}` are admitted and the flush position is reported
   only after their commit.
2. **Given** a runner killed after the commit and before the flush report, **When** it restarts,
   even after the source's dedupe window has passed, **Then** the resent transaction is recognised
   and no second run starts (CDC inbox rows are kept until the stored checkpoint passes them).
3. **Given** lag above the threshold, **Then** health is `LAGGING` with bytes, a namespace alert fires,
   and the platform neither drops nor advances the slot.
4. **Given** a missing publication or slot, `wal_level` not `logical` or a role without `REPLICATION`,
   **Then** health shows a closed class and nothing is created on the customer database.
5. **Given** a paused connector or a revoked secret ACL, **When** the next lease renewal or batch
   arrives, **Then** it is refused, the lease ends and the runner closes the replication connection.

### US05 — Move files and notify people (Priority: P2)

An author copies an S3 export into a blob, transforms it, uploads the result, e-mails a report and
posts to Slack.

**Why this priority**: the most common effects after HTTP, each with a distinct recovery profile.

**Independent Test**: MinIO, an SMTP test server, a Slack stub; drop connections after the SMTP
end-of-data marker and after a Slack request is written.

**Acceptance Scenarios**:

1. **Given** `s3.getObject` of 20 MiB, **Then** the bytes stream into a platform blob and the output is
   its descriptor with ETag, size and content type.
2. **Given** `s3.putObject` with an unknown outcome, **Then** the platform compares the object's
   SHA-256 checksum and records success only on a match; otherwise the run waits for review.
3. **Given** `smtp.send` and a drop after transmission and before the reply, **Then** the step is
   `OUTCOME_UNKNOWN`, the run waits in `MANUAL_REVIEW`, and the message carried a deterministic
   `Message-ID`.
4. **Given** Slack `429` with `Retry-After`, **Then** the retry waits at least that long; an unknown
   outcome after sending is never retried automatically.
5. **Given** CR or LF in an e-mail header value, **Then** it is refused before any connection.

### US06 — Consume MQTT, Pub/Sub, Service Bus and WebSocket, and filter Kafka by key (Priority: P2)

An operator adds four connectors and narrows a Kafka connector to keys starting with `eu-`.

**Why this priority**: broadens parity; each source's guarantee must be stated, not assumed.

**Independent Test**: Mosquitto, the Pub/Sub emulator, a Service Bus REST stub, a WebSocket server,
real Kafka; crash the integration runner's consumers before and after the inbox commit; point each
connector at a name that re-resolves to metadata space.

**Acceptance Scenarios**:

1. **Given** MQTT QoS 1, **When** the consumer crashes before the commit, **Then** the broker redelivers
   (persistent session) and one event results; PUBACK follows the commit.
2. **Given** a slow inbox commit, **Then** the Pub/Sub ack deadline is extended; a redelivery with the
   same `messageId` is a duplicate.
3. **Given** a Service Bus lock that expires before completion, **Then** the redelivered message is a
   duplicate in the inbox.
4. **Given** a WebSocket consumer crash after a frame and before the commit, **Then** that frame is
   lost, and the connector is labelled best-effort in the API and the Studio.
5. **Given** `filter: key.startsWith("eu-")`, **When** a record with key `us-1` arrives, **Then** no
   event is admitted, the position moves and `filtered` grows; triggers may read `event.message.key`.
6. **Given** any new connector kind whose host name resolves to a public address at check time and to
   `169.254.169.254` later, **Then** the integration runner connects only over the socket it checked;
   no handshake or credential reaches the second address.

### US07 — Use namespace variables in expressions (Priority: P2)

An administrator defines `region` (`public` string) and `approvalThreshold` (`internal` decimal);
workflows read `vars.region`.

**Why this priority**: removes copied constants without misusing secrets.

**Independent Test**: publish, admit, change variables, retry a step; read as viewer and developer.

**Acceptance Scenarios**:

1. **Given** a run admitted at revision 3, **When** the variable moves to revision 4 and a step is
   retried, **Then** the retry uses revision 3; a new run uses 4.
2. **Given** `vars.missing`, **Then** publication fails `VALIDATION_FAILED` with
   `VARIABLE_NOT_FOUND`; after a later deletion, admission of a new run fails the same way.
3. **Given** a viewer, **Then** an `internal` variable shows metadata only; a developer sees the value.
4. **Given** level `pii` or `secret`, **Then** creation is refused with a reason pointing to secrets.

### US08 — Start a workflow from an inbound e-mail (Priority: P3)

Mailgun forwards each message for an address to an `EMAIL` endpoint; a `webhook` trigger filters on
the subject.

**Why this priority**: provider-specific and dependent on owner choice O1120.

**Independent Test**: signed Mailgun fixtures; replays; forged signatures; oversized messages.

**Acceptance Scenarios**:

1. **Given** a valid signed JSON forward, **Then** `200` after the commit and the event holds the
   normalized e-mail document.
2. **Given** the same signed request replayed inside the timestamp window, **Then** it is a
   `DUPLICATE` (`200`) and starts nothing; outside the window or with another body, **Then** `401`.
3. **Given** a message over the bound or an invalid document, **Then** `406` (the provider stops
   retrying) and the refusal is counted and audited; two different messages that share a sender-set
   `Message-ID` are two events (the id is the digest of the normalized document).

### US09 — Preview a trigger or connector filter in the Studio (Priority: P3)

An author checks a filter against a pasted sample, or against retained events they may reveal.

**Why this priority**: shortens filter authoring; it must not become a data oracle.

**Independent Test**: preview with a sample and with retained events, with and without the reveal grant.

**Acceptance Scenarios**:

1. **Given** a sample, **Then** each answer is `MATCHED`, `NOT_MATCHED` or `ERROR` with a closed
   category and no value from the document.
2. **Given** retained events without `payload:reveal-pii`, **Then** `403` with
   `PII_REVEAL_NOT_GRANTED`; with it, each event's evaluation is audited as a reveal first.

### Edge Cases

- A paused or deleted resource refuses new admissions; an admitted run's next attempt fails
  `RESOURCE_UNAVAILABLE`; nothing claims to recall an in-flight request.
- A slot secret rotated between attempts: each attempt resolves and pins the current authorized
  version; the resource revision is unchanged.
- An OAuth2 token endpoint slower than the step deadline: `TIMEOUT` before any business request; no
  token is kept for the next attempt.
- DNS rebinding (public at check, metadata later) and redirects to other hosts: the runner connects
  only to checked addresses and follows no redirect (`PROVIDER_REJECTED`).
- Two routes with the same method and path shape conflict at creation.
- A CDC transaction with a million changes is admitted under backpressure; the flush position moves
  only at transaction boundaries; a `RELATION` change applies to later events only.
- Identical MQTT payloads without an id inside the dedupe window collapse (documented).
- A Pub/Sub exactly-once subscription is still deduplicated in the inbox.
- A Mailgun retry after the dedupe window is a new event (ADR-0012 rule).
- An HTTP resource whose `baseUrl` is edited by a holder of only `secret:use` on its slot secret: the
  revision is refused (D1158); the credential is never sent to the new host.
- A preview over a source with no retained events returns an empty list.
- A variable deleted during a chain: the current segment keeps its pin; the successor's admission
  fails `VARIABLE_NOT_FOUND`.
- An encoded field (031) in a `vars` comparison, an operation expression or a filter is refused with
  031's `ENCODING_READ_BY_EXPRESSION` (at publication where the position is known, otherwise at
  evaluation); it is never decrypted. The positions are listed in 031's registry.

## Requirements *(mandatory)*

### Functional Requirements

**Resource types and resources**

- **FR-021-001 — Resource-type catalog**: The platform MUST publish a read-only catalog of immutable,
  versioned resource types, each declaring a closed bounded settings schema, secret slots, auth
  schemes, operations, a fixed probe, an egress profile and, per operation, the managed executor role
  allowed to resolve it (`integration-runner` for the first set), with the first set and auth schemes
  of [resource-types.md](contracts/resource-types.md). The listing MUST show every database driver of
  017 and 024 (`postgresql`, `mysql`, `mariadb`, `sqlserver`, `bigquery`, `snowflake`, `duckdb`) with
  `managedBy: connections`.
- **FR-021-002 — Resource catalog**: Namespace-scoped create, list, detail at a revision, update,
  pause and delete MUST provide a unique name, stable id, immutable revisions, `expectedRevision`
  concurrency, `requestId` receipts (same actor/key/body replays; changed body conflicts) and
  tombstones retaining referenced revisions. Settings MUST validate against the pinned type version;
  unknown settings, credentials in URLs or auth-like header settings, and over-bound values are refused.
- **FR-021-003 — Secret slots**: A slot MUST bind to a namespace secret with OpenBao authority by
  logical name; a legacy local secret requires the existing migration (FR-017-009). Binding a slot to
  a destination — creating the resource, or changing any host-bearing setting (`baseUrl`, `tokenUrl`,
  `endpointPath` host, S3 `endpoint`/`bucket`, SMTP `host`, Slack `apiBaseUrl`) or the slot's secret —
  MUST require `secret:manage` on that secret; `secret:use` suffices only for revisions that keep every
  slot's destination. The resource form offers create/rotate/select with 017's write-only,
  partial-save and reconciliation behaviour (FR-017-004, FR-017-007). No slot value reaches a
  response, log, audit or the journal.
- **FR-021-004 — Pinning and current authority**: Admission MUST pin the revision of every
  `resourceOperation` node (and its `compensate`/`reconcile`) in the admission transaction; every
  attempt uses it; an explicit `revision` is honoured. Publication, admission and capability use MUST
  each check current resource status, the publisher's and initiator's permissions and slot ACLs. The
  credential version is resolved and pinned (metadata only) per attempt.
- **FR-021-005 — Compatibility with 017**: Connections, their routes, the `database` node, SDK
  builders, Studio screens, pins and quota resources MUST keep their contracts and data; both catalogs
  MUST share one implementation of revision, receipt, pin and lifecycle rules; creating a resource of
  any database driver type MUST fail with `MANAGED_BY_CONNECTIONS`.
- **FR-021-006 — Probe**: An authorized user MUST be able to admit the type's fixed probe as a durable
  run (`origin: RESOURCE_PROBE`) through the operation path (executor, egress, credentials, quotas). A
  probe sends no business data, e-mail or Slack message, accepts no request content, refuses a paused
  resource and records its tested revision; an edit makes the displayed result belong to the old one.

**Managed operations**

- **FR-021-007 — `resourceOperation` node**: The IR MUST gain the additive closed node of
  [resource-operation.schema.json](contracts/resource-operation.schema.json); dynamic values are
  `{literal}` or `{expression}` under the existing evaluator. Publication MUST check the enabled
  capability, the operation/type match and root placement (bodies refused with `UNSUPPORTED_NODE`
  until G1120 closes). An ordinary activity MUST NOT use the reserved prefix `sagat.integration.`.
- **FR-021-008 — Integration executor**: Operations MUST run only in a separate integration runner
  with a dedicated machine role, under lease, fencing, heartbeat, cancellation and report receipts.
  Its capability (pinned settings, server-evaluated request, slot values, effective limits, deadline)
  is resolved in two phases with no transaction open during provider I/O, the audit and credential
  pin committing before anything is returned ([integration-runner.md](contracts/integration-runner.md)).
  It holds no platform database URL, OpenBao token or signing key; ordinary workers, sandbox runners
  and database runners cannot claim or resolve these tasks.
- **FR-021-009 — Egress**: Every connection of an operation or probe, including OAuth2 token
  requests, MUST pass the namespace egress policy intersected with the deployment policy, resolve
  once, check every address, connect to a checked address verifying the TLS name, refuse loopback,
  link-local, metadata and platform state endpoints, follow no redirect and ignore environment
  proxies. TLS verification is mandatory outside an explicit development mode.
- **FR-021-010 — HTTP and GraphQL**: `http.request` and `graphql.execute` MUST implement
  [resource-types.md](contracts/resource-types.md): base-URL-relative path templates with
  percent-encoded parameters (no absolute URL, `..` or host change), refused auth/routing/framing
  headers, runner-built auth, JSON/text/blob bodies, success statuses, response modes and status
  mapping; GraphQL refuses a declared/actual kind mismatch before sending.
- **FR-021-011 — S3**: `s3.getObject` MUST stream into a platform blob and output its descriptor;
  `s3.putObject` streams a blob named by the operation (or bounded inline content) with a SHA-256
  checksum and optional create-only condition; head, list (bounded) and delete complete the set. A
  managed lease reads only the blob ids its server-evaluated operation names; sizes follow ADR-0023.
- **FR-021-012 — SMTP**: `smtp.send` MUST require TLS (plaintext only in development), restrict
  `from` to the resource's allowed senders, refuse CR, LF and NUL in every header value, parse
  addresses, bound recipients, bodies and blob attachments, and set `Message-ID` from the effect key.
  It is irreversible.
- **FR-021-013 — Slack**: `slack.postMessage` MUST post text and/or bounded blocks to a channel id
  (optionally a thread), treat `429` as retryable after `Retry-After`, and output channel and
  timestamp. It is irreversible.
- **FR-021-014 — Operation contract parity**: Each operation's fields, bounds, result envelope and
  error mapping MUST be exactly those of [resource-types.md](contracts/resource-types.md) for every
  author (Studio and five SDKs) and every runner build; a field outside the contract is refused at
  publication.
- **FR-021-015 — Effects, idempotency and recovery**: The class (safe/effect) is fixed per operation.
  Safe operations follow the node retry policy. An effect gets one automatic attempt unless it
  declares `idempotency.header` (the effect key is sent on every attempt) or uses the built-in S3
  checksum reconciliation. An unknown outcome MUST be reconciled (declared `reconcile` on the same
  pinned resource, or built-in S3) or wait in `MANUAL_REVIEW`, never retried blindly. The effect key
  is the existing stable step idempotency key (or `idempotencyTemplate`). A declared `compensate`
  runs as a derived compensation step (ADR-0010); `irreversible` ends a compensating saga `PARTIAL`.
  The platform MUST NOT claim a provider honours an idempotency header or that cancellation undid a
  sent request.
- **FR-021-016 — Output**: Output is the operation's result envelope, inline within the payload limit
  or as a blob in blob mode, counted against retained-content quotas. The runner and, for inline
  output, the server MUST scan for every resolved slot value (raw, JSON-escaped, base64 at the three
  alignments, base64url, percent-encoded) and fail `SECRET_IN_OUTPUT` before retention. Blob output
  (`s3.getObject`, HTTP blob mode) MUST pass a bounded cross-chunk streaming scan before the blob is
  `UPLOADED`, and the server rescans before `COMMITTED`; until that scanner's tests pass, blob mode is
  refused at publication (refines ADR-0023's refusal of secret-bearing uploads, ADR-0055 §8). The
  node's `outputSchema` is a classification carrier; undeclared fields are `pii`; no dedicated
  result endpoint is added.
- **FR-021-017 — Typed failures**: Failures MUST use the closed vocabulary and retryability of
  [error-reasons.md](contracts/error-reasons.md), never provider text, URLs, headers or values.

**Variables**

- **FR-021-018 — Variable catalog**: Namespace-scoped create, list, detail, update and delete MUST
  provide a unique identifier-shaped name, an immutable type (`string`, `integer` and `decimal` as
  strings, `boolean`, bounded `json`), an immutable level (`public` or `internal`; `pii` and `secret`
  refused), immutable revisions, `expectedRevision`, `requestId` receipts, tombstones and value bounds.
- **FR-021-019 — Variables in expressions**: Expressions MAY read `vars.<name>` with static names
  only. Publication MUST extract and validate the names and record them with the version; admission
  MUST pin each current revision in the admission transaction; every evaluation in the run, retries
  included, uses the pins. Variables are unavailable in trigger and connector filters, in resource
  settings, and to workers except through a step's input expressions.
- **FR-021-020 — Variable disclosure**: Metadata and `public` values require `namespace:read`;
  `internal` values require `payload:read-internal`. A value mapped into another carrier follows that
  carrier (no taint tracking, ADR-0036 decision 6). Values never enter logs, spans, metrics or audit.

**Triggers and inbound sources**

- **FR-021-021 — Common connector contract**: MQTT, Pub/Sub, Service Bus, WebSocket and CDC MUST be
  connector kinds with the existing slug space, inbox deduplication, acknowledgement after the inbox
  commit (or a stated absence of one), bounded dead letters with redrive and discard, rate, backlog
  backpressure, pause, health, quotas, OpenBao credentials and egress policy, with the guarantees of
  [connector-kinds.md](contracts/connector-kinds.md#delivery-guarantees); `message` triggers name them
  unchanged. They MUST be `MANAGED` streams: CDC held by a database runner, the other four by an
  integration runner, each under a fenced stream lease; the engine opens no socket for them. Every
  connection goes over a socket the guarded resolver opened to a checked address (TLS over that
  stream). Lease renewal and every batch admission MUST recheck connector status, current secret ACL
  and credential version, and end the lease on refusal; the runner then closes the source connection.
  Renewal never resends a credential. Kafka, NATS and SQS keep their engine placement (G1137).
- **FR-021-022 — PostgreSQL CDC**: A CDC connector MUST be consumed by a database runner holding a
  fenced stream lease; turn each committed change into an event as specified; move the stored
  checkpoint only in the admission transaction of a transaction's last change; report the flush
  position only after that commit is durable — in a `sync_quorum` cell (032) only after it is durable
  on the quorum standby; restart replication from the stored checkpoint; keep each CDC inbox row from
  purge until the stored checkpoint passes its commit LSN, so a resent transaction is recognised
  whatever the dedupe window; validate `slot` and `publication` as single identifiers; never create,
  alter or drop publications or slots; measure lag and mark `LAGGING` with a namespace alert above
  the threshold.
- **FR-021-023 — MQTT**: MQTT 3.1.1/5 over TLS with QoS 1 persistent sessions, one lease holder per
  connector and PUBACK only after the commit MUST be supported; PUBACKs are sent in the order the
  PUBLISH packets arrived (a dead-lettered or filtered message is acknowledged in its place); when a
  message cannot be admitted (backlog, rate) the consumer stops acknowledging and disconnects after a
  bounded wait so that the broker redelivers. QoS 0 only when explicitly chosen and labelled
  at-most-once; the event-id rule is as in the contract.
- **FR-021-024 — Google Cloud Pub/Sub**: REST pull through the guarded HTTP client, acknowledge after
  the commit, ack-deadline extension while waiting, `messageId` as event id.
- **FR-021-025 — Azure Service Bus**: REST peek-lock through the guarded HTTP client, complete after
  the commit, lock renewal, `MessageId` or sequence number as event id; the broker's own dead-letter
  queue is documented as outside the platform.
- **FR-021-026 — WebSocket client**: A `wss` client with optional upgrade headers (values may come
  from namespace secrets) and a static subscribe message, one lease holder, frames as events,
  connecting only over a pre-checked socket; labelled best-effort.
- **FR-021-027 — Custom HTTP routes**: A route endpoint kind MUST bind method (POST, PUT, PATCH,
  DELETE) and normalized path with bounded `{param}` segments under a random per-namespace prefix,
  one ADR-0026 profile (HMAC with a route signing string that covers method, path, canonical query and
  declared captured headers; OIDC; or mTLS), a fixed event type, declared captures and an
  idempotency header. `idempotency: generate` MUST be refused for HMAC and OIDC routes and is allowed
  only on mTLS routes, shown as non-deduplicating. Before authentication only answers that do not
  depend on the route are given (global size caps, an attempt limiter keyed independently of route
  existence, the uniform `401`); pause (`503`), route body limit (`413`), route rate (`429`) and a
  missing idempotency header (`400`) are answered only after authentication. A route answers `202`
  after the commit with the body as `data` and captures as `event.request`. GET and HEAD are not
  supported.
- **FR-021-028 — Inbound e-mail**: An `EMAIL` endpoint kind MUST accept a normalized e-mail document
  from an operator relay authenticated by an ADR-0026 profile (`GENERIC_RELAY`) and, once the provider
  fixtures of T021-055 confirm the signature scheme, from the `MAILGUN` adapter (JSON forward, HMAC
  over timestamp and body under a namespace secret within the endpoint's window, `200` on admission,
  `406` on permanent refusal; unsigned-body form forwards refused). Authentication precedes any
  per-endpoint answer. The platform parses no MIME and runs no SMTP server. The event id is the digest
  of the normalized document (the sender-set `Message-ID` is data, never the id); the event subject is
  not copied from the message; size is bounded by the inbox limit.
- **FR-021-029 — Connector filters and broker metadata**: Every connector kind MAY declare a filter
  over `{key, headers, value}` evaluated before admission with the existing evaluator; a non-match is
  acknowledged (or its position moved) without an inbox row and counted; an evaluation error counts
  as not matching, as for trigger filters. Kafka MUST expose key, topic, partition, offset and
  timestamp additively as `event.message`.
- **FR-021-030 — Filter preview**: The platform MUST evaluate a draft trigger or connector filter
  server-side against a caller-supplied sample (`event:read` and `definition:write`) or a bounded
  number of retained events of one source (plus the event-data reveal of FR-016-026, audited per
  event), returning only `MATCHED`, `NOT_MATCHED` or `ERROR` with a closed category per item.

**Cross-cutting**

- **FR-021-031 — Authorization and folders**: Add `resource:manage`, `resource:use`,
  `resource:effect`, `variable:manage`, `integration:execute`, `connector:stream` and the machine role
  `integration-runner` with the plan's matrix; the built-in roles become admin, developer, operator,
  viewer, worker, runner, database-runner and integration-runner (tests assert the set, not a count).
  `connector:stream` is held by `database-runner` (CDC only) and `integration-runner` (the four other
  new kinds only). Every call checks current membership; foreign or missing objects are an opaque
  `404`. `event.message`, `event.request` and any correlation key derived from source data (CDC
  `correlationColumn`) are disclosed exactly like event `data` (FR-016-026). Resources adopt 029's
  folder contract: at most one placement, names unique per namespace whatever the folder, and — when
  029 is enabled — visibility and use decided by the 029 evaluator, the `viewer` level conferring
  resource visibility and `runner` conferring `resource:use` (and `resource:effect` while developer
  holds it, O1121); a resource in a restricted folder is usable by a published workflow only if its
  publisher or initiator is listed at `runner` or above. `resource:manage` stays a sensitive item
  permission. Variables are namespace configuration and are not placed in folders.
- **FR-021-032 — Observability and audit**: Emit the plan's spans and fixed-label metrics; audit
  catalog changes, probe admissions, credential resolutions, denials, reveals, lag threshold crossings
  and variable changes in the decision's transaction. No path, query, header, body, address, value,
  credential or provider message enters a log, span attribute, metric label or audit reason.
- **FR-021-033 — Finite ceilings**: Every new resource and operation has the default and hard ceiling
  of the plan; effective = min(request, namespace, deployment); zero refuses; publication and
  admission fail before work; existing run, content, event-rate and connector quotas still apply.
- **FR-021-034 — SDKs**: The five SDKs MUST emit the same canonical `resourceOperation` nodes and
  `vars` expressions, invent no retry, auth or serialization semantics, and pass shared conformance.
- **FR-021-035 — Studio**: Resources (types, resources, probes, revisions; connections listed with a
  link), Variables, forms for the new kinds showing their delivery guarantee, a `resourceOperation`
  inspector and the filter preview, with design tokens, WCAG 2.2 AA, a list alternative and no sample
  values for absent data (FR-007-010).
- **FR-021-036 — Delivery**: Pinned images and dependencies; a nonroot read-only runner image;
  Compose and Helm profiles with default-deny egress; runbooks (resources, integration runner, each
  new source, CDC slot lag); the [quickstart](quickstart.md) demo; and the draft contracts merged into
  the canonical documents in the same delivery.

### Amendment to feature 016

FR-016-002 closes the list of classification carriers. This feature adds exactly one carrier: the
optional `outputSchema` of a `resourceOperation` node (FR-021-016), with FR-016-003's rules (an
undeclared field is `pii`) and no change to the classification rules version. The amendment is
opened by T021-064 together with the other parity drafts' carrier additions (coordinator C6); the
consolidated list is kept in `SPECIFICATION-INDEX.md` by the coordinator. Event data, the new
envelope extensions and variables are not carriers (FR-021-020, FR-021-031).

### Reset of runs that use resources (018)

021 adds **no journal event type**: `resourceOperation` steps, their compensations and
reconciliations use the existing `step.*` events, whose reset rule 018 owns (`COPY`). It adds
run-scoped side tables, declared for 018's reset-rule registry:

| Side table | Rule | Behaviour in the reset transaction |
|---|---|---|
| `run_resource_bindings` | `REMATERIALIZE` | copied from the source run at the same revisions, after rechecking current resource status, publisher/initiator permissions and slot ACLs; a paused or deleted resource or a revoked permission refuses the reset with `RESOURCE_UNAVAILABLE` |
| `run_variable_bindings` | `REMATERIALIZE` | copied at the same revisions so the restored prefix and later evaluations agree; a deleted variable refuses the reset with `VARIABLE_NOT_FOUND` |
| `step_resource_credentials` | not copied | attempt evidence of the source; the reset run's attempts resolve their own credentials |

### Non-functional requirements

- **NFR-021-001 (security)**: zero tested cross-tenant/namespace access and zero canary leakage of slot
  values, tokens, variable values or event data; capability, egress and ingress-authentication
  refusals block release.
- **NFR-021-002 (durability)**: pins, inbox rows, checkpoints and transitions commit with outbox and
  audit as the plan states; nothing acknowledged after its inbox commit is lost; no exactly-once claim.
- **NFR-021-003 (compatibility)**: existing workflows, connections, connectors, endpoints, protocol
  clients and AsyncAPI consumers keep working; contracts only grow; older workers never get
  integration tasks.
- **NFR-021-004 (observability)**: every accepted operation and admitted event has persisted
  correlation and audit; export failures never decide a commit.
- **NFR-021-005 (performance)**: admission, resolution, operation and ingress latency (p50/p95/p99),
  connector throughput and CDC lag recovery are benchmark criteria on a declared environment; no
  figure is promised before measurement.

### Key Entities

ResourceType/ResourceTypeVersion; Resource/ResourceRevision; RunResourceBinding;
StepResourceCredential; Variable/VariableRevision/RunVariableBinding; Route and EmailEndpoint (endpoint
kinds); connector kinds MQTT, Pub/Sub, Service Bus, WebSocket, CDC; ConnectorStreamLease;
FilterPreview (not stored beyond audit). See [data-model.md](data-model.md).

## Success Criteria *(mandatory)*

All evidence below is required future evidence; none exists.

| Criterion | Required automated proof | Requirements |
|---|---|---|
| SC-021-001 | Browser and REST create an `http_api` resource with an OpenBao secret, reload without values, conflict on a stale revision, one probe success and five distinct probe failures | FR-021-001..003,006,017,035 |
| SC-021-002 | In 50 seeded edit/retry interleavings no retry uses a later revision; revoking the slot's ACL stops the next resolution; a 018 reset copies resource and variable bindings at the same revisions or is refused as specified | FR-021-004,008,019 |
| SC-021-003 | 017 regression suites pass unchanged; creating a resource of each of the seven database driver types is refused with `MANAGED_BY_CONNECTIONS` | FR-021-001,005 |
| SC-021-004 | Studio and five SDKs publish byte-identical `resourceOperation` documents and get equal results and errors | FR-021-007,010..014,034 |
| SC-021-005 | For every operation kind: crash after send, stale lease, lost hint, timeout vs late completion, duplicate report and restart leave one completion or an explicit unknown outcome | FR-021-008,015 |
| SC-021-006 | A failing HTTP compensation inside a saga ends in review or `COMPENSATION_FAILED` per policy; an irreversible SMTP send ends `PARTIAL` | FR-021-015 |
| SC-021-007 | Egress refuses loopback, link-local, metadata, platform endpoints, DNS rebinding and redirects for every operation, the token request and every new connector kind (MQTT, Pub/Sub, Service Bus, WebSocket, CDC), including their token endpoints and upgrade handshakes | FR-021-009,021 |
| SC-021-008 | Canaries in slots, tokens, variables and event data never appear in outputs, logs, spans, metrics, audit or errors; an echoed token fails `SECRET_IN_OUTPUT` | FR-021-016,020,032 |
| SC-021-009 | Variable pins hold across retries and change for new runs; disclosure by role matches FR-021-020 | FR-021-018..020 |
| SC-021-010 | CDC: no duplicate run after a crash between commit and flush report, also when the restart comes after the dedupe window; in a `sync_quorum` cell no flush is reported before quorum durability; pause and ACL revocation end the lease and close the replication connection at the next renewal or batch; the lag alert fires; nothing is created on the customer database | FR-021-021,022 |
| SC-021-011 | MQTT, Pub/Sub, Service Bus admit once after a crash before commit and dedupe after one before acknowledgement; WebSocket loss is bounded to in-flight frames and labelled | FR-021-021,023..026 |
| SC-021-012 | Routes: profiles, same-key replay, changed-body conflict, path collision, prefix rotation; `generate` refused on HMAC/OIDC routes; a replayed HMAC request with a changed captured header fails; an unauthenticated caller gets the same `401` for unknown, paused, oversized and header-less requests | FR-021-027 |
| SC-021-013 | Relay and Mailgun fixtures: valid, forged, replay inside window, replay outside window, form-encoded, oversized → `200`, `401`, `200` (`DUPLICATE`), `401`, `401`, `406`; two messages sharing a `Message-ID` give two events; the event subject is never the message subject | FR-021-028 |
| SC-021-014 | The Kafka key filter admits only matches and moves the position for the rest; `event.message.key` reaches triggers; AsyncAPI only grew | FR-021-029 |
| SC-021-015 | Preview never returns a value, needs the reveal grant for retained events and audits each | FR-021-030 |
| SC-021-016 | Two tenants and two namespaces cannot manage, use, probe, resolve, consume or preview each other's objects; `event.message`, `event.request` and data-derived correlation keys follow FR-016-026; a holder of only `secret:use` cannot bind a slot to a new destination; with 029 enabled, a resource in a restricted folder is hidden and unusable for unlisted subjects exactly as a missing one | FR-021-003,031 |
| SC-021-017 | Every ceiling of the plan is refused at its bound before work; zero refuses | FR-021-033 |
| SC-021-018 | The quickstart happy and failure demos run from a clean checkout on a declared environment with recorded timings | FR-021-036, NFR-021-005 |

## Assumptions

- Customers create and own CDC publications, slots and replication roles (runbook SQL) and set
  `max_slot_wal_keep_size` to protect their disks.
- Emulators or stubs stand in where no local service exists (Slack, Service Bus); real-provider runs
  stay gaps until credentials are supplied.
- The operator approves private ranges for runners as in ADR-0035; namespaces can only narrow them.
- The coordinator allocates the protocol minor, migration timestamps and permission order across
  018–032; numbers here are provisional (D1151).

## Out of Scope

Namespace-defined resource types (O1122); resources consumed by ordinary activities and scripts;
OAuth2 authorization-code/refresh flows; SES/SNS, SendGrid, Postmark; an SMTP server; MIME parsing;
provider-specific route signatures (GitHub, Stripe, Shopify); synchronous routes (026); standalone
operations (022); database engines (024); operations inside bodies (G1120); a connector marketplace;
cross-resource distributed transactions.

## Owner questions

O1120 (e-mail providers), O1121 (default `resource:effect` grant), O1122 (custom resource types) and
O1123 (the placement rule of ADR-0055 decision 11), with recommended defaults, are in
[research.md](research.md#owner-questions); this spec assumes the defaults.

## Boundaries

**Always**: pin at admission, resolve per attempt under a fenced lease, admit before acknowledging,
refuse before work, audit in the decision's transaction, state each source's guarantee.

**Never**: run an operation or a new connector kind in the API, scheduler or engine; put a credential in IR, settings,
journal, logs or responses; create objects in a customer database; follow redirects; retry an
unknown effect without a declared mechanism; promise exactly-once; present a pending guarantee as
shipped. See the [requirements checklist](checklists/requirements.md).
