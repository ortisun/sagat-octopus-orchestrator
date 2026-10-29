# ADR-0055: Integration resources reuse the managed-connection model, run in a separate managed executor, and new triggers are kinds of the existing inbox sources

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26); revised 2026-10-27 after independent architecture and security reviews and the coordinator's decisions C3, C4, C12 and C21; the placement rule of decision 11 is an owner question (O1123)
**Refines / supersedes:** Refines ADR-0012 (new connector and endpoint kinds, a managed stream placement), ADR-0023 (decision 1 kept through a factory; the refusal of secret-bearing uploads is lifted only for managed integration steps behind a streaming scan), ADR-0026 (a route signing string and one provider profile for inbound e-mail), ADR-0035 (its connection decisions 1, 3, 5, 6 and 7 are applied to non-database resources without changing feature 017) and ADR-0036 decision 6 (the output of a resource operation is a classification carrier). Supersedes none.
**Enforced by:** planned, not yet existing: a per-crate client allowlist in `scripts/check-architecture.py` (`object_store` only in `crates/infra-blobstore`; HTTP-provider, SMTP and GraphQL clients only in `crates/infra-integrations`; broker, MQTT, WebSocket and cloud clients only in `crates/infra-connectors`; the replication client only in `crates/infra-database`; T021-016); one cargo feature per connector kind in `crates/infra-connectors` and a CI assertion that `cargo tree -e features -p sagat-execution-engine` enables none of the `MANAGED` kinds' features (`mqtt`, `pubsub`, `servicebus`, `websocket`), so the engine cannot link them (T021-080); the contract compatibility tests over `openapi.yaml`, `workflow.schema.json`, `events.asyncapi.yaml` and `worker.proto`; `tests/{integration,fault,security}/integration-resources/` listed in `specs/021-integration-resources-triggers/tasks.md`. Until those exist this is review only.

## Context

Feature 021 adds generic resources (HTTP API, GraphQL, S3-compatible bucket, SMTP, Slack bot),
typed managed operations on them, namespace variables, seven inbound source kinds and two filter
capabilities. Three choices are hard to reverse because they fix table shapes, wire contracts and
process boundaries:

1. Feature 017 shipped in 0.2.0 a PostgreSQL connection catalog with immutable revisions, run pins
   (`run_database_connections`), credential pins, the `database` IR node, five SDK builders, Studio
   screens and quota policies; feature 024 adds six more drivers to it. A generic resource catalog
   either absorbs that data or lives next to it.
2. A managed operation resolves a customer credential and sends a request built from workflow
   data. ADR-0035 put database I/O in `apps/database-runner`, a separate process with no platform
   database credentials. HTTP egress already exists in the engine for outbound webhooks
   (`crates/scheduler/src/deliveries.rs`), so running operations there would be cheaper.
3. ADR-0012 admits every inbound event through `inbox_events` and acknowledges a broker only after
   the commit (§5). The existing consumers run in the engine's connector supervisor
   (`crates/infra-connectors/src/supervisor.rs`). New sources bring customer credentials (service
   account keys, SAS keys, replication roles), customer-directed sockets and parsing; PostgreSQL CDC
   also needs ADR-0035 decision 4's operator-approved private-network boundary. Broker libraries that
   resolve a name a second time defeat the egress check (G76 of feature 004).

## Decision

1. **Feature 017 stays the authority for database connections.** Resources MUST be stored in their
   own tables (`resources`, `resource_revisions`, `run_resource_bindings`,
   `step_resource_credentials`). The rules shared with 017 (immutable revisions under optimistic
   concurrency, request receipts, pause and tombstone, pin at admission, credential version per
   attempt, OpenBao-only secret slots, fixed probes as durable runs) MUST be implemented once in the
   domain and application layers and used by both catalogs. 017 tables, routes, IR, SDK builders and
   quota resources MUST NOT change. The resource-type listing MUST show every 017/024 database driver
   with `managedBy: connections`, and the resource API MUST refuse to create any of them.
2. **Resource types are platform-provided and versioned.** A type version is immutable; a resource
   revision pins exactly one; each operation of a type version names the one executor role that may
   resolve it. A namespace cannot define a type in this feature.
3. **Managed operations run only in `apps/integration-runner`.** It is a separate process with a new
   machine role `integration-runner` (permissions `worker:execute`, `integration:execute`,
   `connector:stream`), one tenant per token, no platform database URL, no OpenBao token and no
   signing key. It claims reserved `sagat.integration.*` tasks over the worker protocol, resolves its
   capability in two phases like `ResolveDatabase` (fences, no transaction during provider I/O,
   fences again, audit and credential pin commit before the value is returned) and connects only
   through the guarded egress resolver. The control plane, scheduler and engine MUST NOT send a
   managed operation. Ordinary workers, sandbox runners and database runners MUST NOT claim these
   tasks or resolve these capabilities.
4. **External effects stay at-least-once.** An effect operation (HTTP methods other than GET/HEAD,
   GraphQL mutation, S3 put/delete, SMTP send, Slack post) gets one automatic attempt unless the
   author declares an idempotency mechanism or a reconciliation; an unknown outcome is reconciled or
   waits for manual review. The platform MUST NOT claim that a provider honours a declared
   idempotency header.
5. **New inbound sources are kinds of the existing aggregates and are managed streams.** MQTT,
   Google Cloud Pub/Sub, Azure Service Bus, WebSocket and PostgreSQL CDC MUST be `event_connectors`
   kinds with placement `MANAGED`; custom HTTP routes and inbound e-mail MUST be `event_endpoints`
   kinds. Existing `message` and `webhook` triggers name them unchanged. The engine supervisor MUST
   NOT consume a `MANAGED` connector; Kafka, NATS and SQS keep their existing engine placement
   (G1137). Each acknowledges its source only after the inbox commit, or says that its source has no
   acknowledgement.
6. **Managed streams are fenced, revocable and admitted in one transaction.** CDC streams are held by
   a database runner (inside ADR-0035's operator-approved database boundary); MQTT, Pub/Sub, Service
   Bus and WebSocket streams by an integration runner; a stream kind claimed by the other role MUST be
   refused. The holder opens every source socket through the guarded resolver and runs TLS, the
   WebSocket upgrade or MQTT over that pre-connected stream. It admits a batch through
   `AdmitConnectorEvents`, whose inbox rows, dead letters and checkpoint commit in one transaction,
   and acknowledges the source only after that commit. Renewal and every batch MUST recheck connector
   status, secret ACL and credential version and end the lease on refusal; renewal MUST NOT resend a
   credential. A CDC inbox row MUST NOT be purged before the stored checkpoint passes its commit LSN.
   In a `sync_quorum` cell (feature 032) the `AdmitConnectorEvents` reply, and therefore the CDC
   flush report, MUST wait until the inbox commit is durable on the quorum standby; under `async`
   replication, changes inside the RPO can be lost at failover and need slot re-creation or backfill
   (stated by 032). The engine MUST NOT open a connection to a customer database (rule 11c). The platform MUST NOT create
   or drop publications or replication slots.
7. **Routes and e-mail authenticate first, with ADR-0026 profiles plus one provider profile.** Before
   authentication only answers that do not depend on the route are given; pause, route size, route
   rate and a missing idempotency header are answered after it. A route's HMAC signing string MUST
   cover method, path, canonical query and declared captured headers in addition to the body
   (`octopus-route-in-v1`). A server-generated event id (`idempotency: generate`) MUST be refused on
   HMAC and OIDC routes. The `MAILGUN` profile verifies Mailgun's HMAC over timestamp and body (JSON
   forwards only) under a namespace secret, within the endpoint's timestamp window, and answers
   `200`/`406` as that provider expects; an e-mail event id is the digest of the normalized
   document, never the sender-set `Message-ID`.
8. **Outputs are classified carriers, scanned before retention.** A `resourceOperation` node MAY
   declare `outputSchema`; an undeclared field is `pii` (ADR-0033). No dedicated result endpoint is
   added. A resolved credential found in an output fails the attempt before retention. Blob output
   MUST pass a bounded cross-chunk streaming scan before `UPLOADED` and a server rescan before
   `COMMITTED`; until that scanner is proven, blob mode MUST be refused at publication.
9. **Storage clients stay in `infra-blobstore`.** The S3 client for customer buckets MUST be built by
   a `crates/infra-blobstore` factory (`CustomerBucket`) whose signature exposes no `object_store`
   type, over a guarded `HttpConnector`; ADR-0023 decision 1 is kept.
10. **A secret is sent only where its manager allowed.** Creating a resource or new-kind connector,
    changing one of its host-bearing settings, or changing a slot's secret MUST require
    `secret:manage` on that secret; `secret:use` allows only revisions that keep every destination.
11. **Placement rule for customer-credentialed calls (proposed for every parity draft; owner
    question O1123).** This narrows the rationale of decisions 3, 5 and 6 and of option 3 below to
    what they actually protect against — tenant-authored request content and customer databases in a
    process that holds platform credentials — and is cited by 019, 020, 025, 026 and 028:
    (a) a request whose content is authored by a tenant in a workflow (HTTP method, path or body, SQL,
        GraphQL, model prompts, tool calls, resource operations) MUST be sent only by a managed
        executor holding only that attempt's capability;
    (b) a non-database delivery of a fixed, platform-defined shape to a tenant-configured
        destination (ADR-0025 events, 019 push envelopes, 020 non-database sink batches, 026
        notifications and channel messages, 028 Git sync) MAY be sent by the engine through the
        guarded egress client, holding only that destination's credential, as ADR-0025 already does;
    (c) no engine or control-plane process opens a connection to a customer database, whatever the
        protocol (including HTTP interfaces of analytical databases such as ClickHouse) (ADR-0035);
        database sinks, sources and probes go through `apps/database-runner`;
    (d) the new inbound connector kinds MQTT, Pub/Sub, Service Bus and WebSocket run as managed
        streams in `apps/integration-runner` (decision 5); the pre-existing Kafka, NATS and SQS
        consumers keep their engine placement (G1137).
    026 notification channels keep their own channel credentials, like webhook subscriptions, and do
    not use 021 resources.

## Considered options

1. **Shared rules, separate stores, separate executor, managed streams for every new kind (chosen)**
   — keeps a released catalog and its pins untouched, gives non-database egress its own network
   policy and fault domain, keeps every new customer credential and socket out of the engine, and
   reuses inbox, dedupe, dead letters, quotas and triggers without an IR change.
2. **Migrate 017 connections into one generic `resources` table** — one catalog, but rewrites
   immutable revisions, composite foreign keys from `run_database_connections` and credential pins
   of a release that has live runs, and couples database engines (024) to non-database types. The
   down migration would have to refuse while pins exist.
3. **Run managed operations in the engine next to outbound webhooks** — no new process, but puts
   tenant-authored request content, customer credentials and response parsing in the process that
   holds platform database credentials, contradicting ADR-0035 decision 3. The objection is to
   tenant-authored content (rule 11a), not to the fixed-shape deliveries that ADR-0025 already sends
   from the engine (rule 11b).
4. **Add operations to `apps/database-runner`** — one fewer binary, but that process's network
   policy reaches private customer databases; HTTP/SMTP/Slack egress would inherit it.
5. **Run the new connector kinds (including CDC) in the engine supervisor like Kafka** — simpler and
   matches today's consumers, but has option 3's exposure (service-account keys, SAS secrets,
   replication credentials, frame parsing in the engine), repeats G76's second-resolution weakness
   for MQTT and WebSocket, and bypasses ADR-0035's database boundary for CDC. Rejected for the same
   reason as option 3; the existing Kafka/NATS/SQS placement is a recorded pre-existing gap (G1137).

## Consequences

- Two catalogs remain. The Studio shows them together; a physical merge would be a new ADR with a
  migration proven against live pins (G1135).
- One more machine role, binary, image, NetworkPolicy and protocol minor (provisionally 1.7,
  allocated at merge) to operate. Constitution II splits a process out only after measurement; this
  split is made for isolation of tenant-authored effects (principle V), as 017's `database-runner`
  was (ADR-0035), and is recorded in the plan's Complexity Tracking; the integration runner also carries broker and cloud egress for
  the four stream kinds. The shared session/poll/heartbeat code of the database runner moves into
  `crates/infra-managed` without behaviour change; the database-runner suites guard that move.
- Permission catalog grows by six entries and the built-in roles by one machine role (the set is
  admin, developer, operator, viewer, worker, runner, database-runner, integration-runner); feature
  029 depends on the new catalog; tests assert the set, not a count.
- WebSocket and MQTT QoS 0 sources cannot be redelivered; the platform states it rather than hiding
  it. Existing connectors, endpoints and subscriptions still bind secrets with `secret:use` (G1141),
  and the existing webhook ingress still answers before authenticating (G1142).
- Blob outputs of operations are unavailable until the streaming scanner exists (G1143).
- Rollback: disable the new capabilities (publication and admission refuse them), drain
  `sagat.integration.*` tasks and stream leases, keep tables and readers; migrations refuse to drop
  rows that pins or retained runs reference.

## Evidence

None yet: the decision is a proposal. Required before acceptance: real PostgreSQL/OpenBao tests of
pinning and two-phase resolution; ordinary-worker refusal; egress refusal of metadata, loopback and
DNS rebinding for every operation and every new connector kind; crash after send and before
completion; a real logical-replication crash between inbox commit and flush report, also after the
dedupe window; stream revocation at renewal; lost MQTT PUBACK and in-order acknowledgement; duplicate
Pub/Sub and Service Bus delivery; replayed HMAC route requests and refused `generate`; replayed and
unsigned-body Mailgun requests; slot destination changes without `secret:manage`; the cross-chunk
scanner; the 021 cases of ADR-0050's lock-order harness; CDC flush only after quorum durability in
a `sync_quorum` cell; five-SDK conformance. See
[spec](../../specs/021-integration-resources-triggers/spec.md),
[research](../../specs/021-integration-resources-triggers/research.md) and
[plan](../../specs/021-integration-resources-triggers/plan.md).
