# Implementation Plan: 021 Integration resources, managed operations, variables and triggers

**Branch**: `feature/021-integration-resources-triggers` | **Date**: 2026-10-26 | **Spec**: [spec.md](spec.md)

Status: Proposed — not started.

## Summary

Add a revisioned resource catalog that applies feature 017's connection rules to five
platform-provided types, a closed set of managed operations executed by a new isolated
`integration-runner`, namespace variables pinned at admission, five connector kinds, two endpoint
kinds, connector filters with Kafka key metadata, and a server-side filter preview. Database
connections keep their tables and contracts (ADR-0055 §1). Decisions D1120–D1159 and D1620–D1624,
gaps G1120–G1143 and owner questions O1120–O1123 are in [research.md](research.md); entities and tables in [data-model.md](data-model.md);
draft wire shapes in [contracts/](contracts/).

## Technical Context

**Language/Version**: Rust (pinned `rust-toolchain.toml`), TypeScript Studio, five SDKs (TS, Go, Python, C#, Java).
**Primary Dependencies**: existing Tokio, Axum, tonic, SQLx, `reqwest` 0.13 behind `crates/infra-egress`,
`object_store` 0.14.2, `rskafka`, `async-nats`; new and to be pinned in T021-002: an SMTP client
(`lettre` class), an MQTT client (`rumqttc` 0.25.x), a WebSocket client (`tokio-tungstenite` class),
a GraphQL parser, a PostgreSQL logical-replication client (G1121). Only `crates/infra-integrations`,
`crates/infra-connectors` and `crates/infra-database` may depend on them.
**Storage**: PostgreSQL (authority); OpenBao KV v2 for slot values (ADR-0035); blob store for S3 bytes.
**Testing**: `sagat-tests` targets with real PostgreSQL, OpenBao, MinIO, Mosquitto, Kafka, the Pub/Sub
emulator, an SMTP test server, a WebSocket server and HTTP/OAuth2/Slack/Service Bus stubs;
`tests/contract/sdk-conformance`; Studio Vitest and Playwright.
**Target Platform**: Linux containers (Compose, Helm).
**Project Type**: modular monolith plus managed executors.
**Performance Goals**: none promised; benchmark criteria in "Verification plan" (G1139).
**Constraints**: finite ceilings below; no operation in API/scheduler/engine; additive contracts.
**Scale/Scope**: 36 FRs, 9 user stories, 80 tasks.

Revised 2026-10-27 after the independent architecture and security reviews (findings and their
resolution: [research.md](research.md#review-resolution-2026-10-27)).

## Constitution Check

*Re-evaluated after design. A check marks the design as conforming, not the implementation.*

| Principle | Result | How |
|---|---|---|
| I. State and retry semantics durable | Pass | Pins, inbox rows, checkpoints, transitions, outbox and audit commit per "Transaction boundaries"; stale attempts and stale stream holders are fenced; a terminal run never reopens; probes and reconciliations are new steps or runs. |
| II. Modular monolith, ports and adapters | Justified | A new process, `apps/integration-runner`, is split out for isolation (principle V), not after a performance measurement: see Complexity Tracking (coordinator C21). Rules in `crates/domain`, services and ports in `crates/application`, SQL in `crates/infra-postgres`, protocol clients only in `crates/infra-integrations`, `crates/infra-connectors`, `crates/infra-database`; no serde_json business representation in the domain (type schemas are opaque documents checked by `crates/infra-schema`); no empty mappers (resource and connection catalogs share one rule set, D1120). |
| III. At-least-once, sagas compensate | Pass | FR-021-015: one automatic attempt for effects without a declared mechanism, reconcile or manual review, compensations as derived steps (ADR-0010), no exactly-once claim. |
| IV. Versioned and pinned | Pass | Type versions immutable; resource and variable revisions pinned at admission; IR, OpenAPI, AsyncAPI and protocol grow additively; five SDKs share conformance. |
| V. Isolation, tenant, secrets | Pass | Operations and the MQTT, Pub/Sub, Service Bus and WebSocket streams only in `apps/integration-runner`; CDC only in the database runner; the engine opens no new customer socket; slot values resolved per attempt under a fenced capability; binding a slot to a destination needs `secret:manage` (D1158). The pre-existing Kafka/NATS/SQS placement is unchanged (G1137). |
| VI. Observability and audit | Pass | Spans, fixed-label metrics and audit actions below; audit in the decision transaction. |
| VII. Inbox/outbox contract | Justified | Every source admits through `inbox_events` before acknowledging. Justified: WebSocket and MQTT QoS 0 have no acknowledgement, so in-flight loss is stated and labelled (D1142, G1124). |
| VIII. Failure tested honestly | Pass | Rule-9 tests per story in [tasks.md](tasks.md); unproven items stay gaps. |

## Architecture and crate placement

| Place | Change |
|---|---|
| `crates/domain/src/resources.rs` (+ `resources/{catalog,operations,failure}.rs`) | Type versions, resource config, slots, status, operation model, safe/effect classification, failure vocabulary, limits. |
| `crates/domain/src/managed_catalog.rs` | Revision/receipt/lifecycle rules extracted from `database.rs` and shared (D1120). |
| `crates/domain/src/variables.rs` | Variable types, levels, value bounds, static reference rule. |
| `crates/domain/src/{connectors,events,triggers,identity,audit,quota}.rs` | New connector/endpoint kinds, envelope extension, permissions, role, audit actions, quota resources. |
| `crates/application/src/{resources,variables}.rs`, `worker/resources.rs`, `connector_streams.rs`, `filter_preview.rs` | Services, ports, two-phase resolution, stream admission, preview. |
| `crates/infra-postgres/src/{resources,variables,connector_streams}.rs` + migrations | Stores and SQL. |
| `crates/infra-ir` | `resourceOperation` codec, `vars` reference extraction, capability gates. |
| `crates/infra-schema` | Validation of settings against type-version schemas. |
| `crates/infra-integrations` (new) | HTTP, OAuth2, GraphQL, S3 (through the `infra-blobstore` factory), SMTP, Slack clients; inline and streaming canary scanners. |
| `crates/infra-blobstore` | `CustomerBucket` factory over `object_store` with a guarded `HttpConnector`; no `object_store` type crosses the crate (ADR-0023 decision 1, D1133). |
| `crates/infra-managed` (new) | Session-per-namespace polling, heartbeat, report receipts, slots — moved from `apps/database-runner` (D1153). |
| `crates/infra-connectors/src/{mqtt,pubsub,servicebus,websocket,filter}.rs` | New consumers, each behind its own cargo feature (`mqtt`, `pubsub`, `servicebus`, `websocket`) enabled only by `apps/integration-runner`; a CI assertion over `cargo tree -e features -p sagat-execution-engine` fails if the engine enables any of them (T021-080); Kafka key exposure in `kafka.rs`; filters before admission; the engine supervisor skips `MANAGED` connectors. |
| `crates/infra-database/src/replication.rs` | Logical replication client with frame budget (G1121). |
| `crates/api-http/src/routes/{resources,variables,http_routes,filter_previews}.rs` | Management, ingress `/v1/routes/…`, e-mail profile in `routes/events.rs`, preview. |
| `crates/api-grpc/src/{resources,connector_streams}.rs` | `ResolveResource`; stream lease RPCs. |
| `crates/scheduler` | Derived compensation/reconciliation steps for `sagat.integration.*` tasks; no I/O. |
| `apps/integration-runner` (new) | Executes managed operations; holds MQTT, Pub/Sub, Service Bus and WebSocket stream leases (`src/streams.rs`). One tenant per runner token. |
| `apps/database-runner/src/cdc.rs` | Consumes CDC streams under a stream lease. |
| `frontend/apps/studio/src/features/{resources,variables,integrations}` | Pages, forms, node inspector, preview. |
| `sdks/*` | Builders and conformance (T021-062/063). |

## Transaction boundaries

Each line is one PostgreSQL transaction; nothing else commits with it; provider or network I/O is
never inside it.

1. **Resource write** — namespace secret/catalog lock → membership rows → namespace → resource head
   → slot secrets and ACL rows (017 order); insert revision, move head, receipt, audit
   `resource.changed`. Secret provisioning stays the separate 017 intent/activation flow.
2. **Variable write** — namespace variable lock → membership → head; revision, head, receipt,
   audit `variable.changed`.
3. **Publication** — existing publication transaction plus validation of resource references,
   operation/type match, `resource:use`/`resource:effect` and slot ACLs, and `vars` names; records the
   names with the version; no pin.
4. **Run admission** (API, trigger, child, rerun, probe) — existing admission transaction plus
   `run_resource_bindings` for every operation node and its compensation/reconciliation, and
   `run_variable_bindings` for every referenced variable, with run, journal, initial tasks, outbox,
   quotas, audit and receipt, under [ADR-0050 decision 7](#lock-order-adr-0050-decision-7). A crash before
   commit leaves no run and no pin.
5. **Claim** — existing lease transaction plus per-resource, namespace and tenant integration
   concurrency counts; requires current role `integration-runner`, binding and protocol minor.
6. **Resolve phase 1** — fences (lease, generation, session SHARE guard with post-lock DB clock,
   binding), pinned revision, current status, publisher/initiator rights, slot ACLs, provider
   metadata; commits nothing. Then OpenBao reads with no transaction open.
7. **Resolve phase 2** — repeats every fence, compares the selected versions, inserts
   `step_resource_credentials` and the bounded `secret.resolved` audit, then returns.
8. **Completion / failure** — existing `CompleteStep`/`FailStep` transaction: receipt replay, output
   bounds, blob commit (ADR-0023), step transition, journal, next tasks, timers, outbox. The canary
   scan runs before this transaction in the runner and is repeated by the server for inline output.
   `OUTCOME_UNKNOWN` takes the existing manual-review or reconciliation transition.
9. **Connector admission (engine, existing Kafka/NATS/SQS only)** — inbox row (or dead letter) plus
   checkpoint CAS when the platform keeps the position; acknowledgement after commit.
10. **Filtered message** — checkpoint-only CAS (Kafka); other kinds acknowledge after an in-memory
    count; `filtered_total` is flushed in bounded batches and is not an authority.
11. **Managed stream batch** (`AdmitConnectorEvents`, every managed kind) — stream lease fence
    (holder, generation, expiry on the DB clock), current connector status, secret ACL and credential
    version, inbox rows (CDC rows carry their commit LSN), dead letters and checkpoint CAS for one
    batch; returns only after the commit is durable (in a `sync_quorum` cell, on the quorum standby,
    032); only then does the runner acknowledge the source (CDC flush LSN,
    MQTT PUBACK in order, Pub/Sub acknowledge, Service Bus complete). A refused recheck ends the lease.
12. **Route / e-mail ingress** — after authentication (before it, only route-independent bounds,
    D1145): endpoint configuration lock and comparison (ADR-0026), per-route pause/size/rate checks,
    inbox row, bounded refusal audit; `202`/`200` after commit.
13. **Filter preview over retained events** — reads at most the bounded events and commits one
    `payload.pii_revealed` audit per event before returning booleans; changes no state.
14. **Lag threshold crossing** — health update and `connector.lagging` audit; the namespace alert
    is found by the existing monitor (ADR-0017).
15. **Stream renewal** (`RenewConnectorStream`) — lease row, connector status, secret ACL and
    credential version rechecked; extends the lease or ends it; never returns a credential.
16. **Inbox purge** (existing loop) — additionally skips CDC rows whose commit LSN is above their
    connector's stored checkpoint (D1139).

### Lock order (ADR-0050 decision 7)

The merged admission lock order is
[ADR-0050](../../docs/adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md)
decision 7 (feature 018, coordinator decision C4). Every admission takes its slots in increasing
order and never returns to a lower slot. 021 adds rows only in these slots (D1159, D1624):

| ADR-0050 slot | 021 rows, in the decision-7 order |
|---|---|
| 4a — namespace fences and authority | 017 namespace secret/catalog lock → **021 namespace variable fence** (variable writers only) → 024 data-catalog fence (writers) → memberships `FOR SHARE` → 029 access fences → namespace row |
| 4b — catalog heads and pins (`FOR SHARE` on admission) | 019 fences → 017 connection heads → **021 resource heads** → **021 variable heads** → 024 asset heads → 025 / 030 heads; ascending id within each kind |
| 5 — secrets | slot and connector-credential secret rows (ascending id) → their ACL rows |
| 6 — new run rows | after the run insert: **021 pins** `run_resource_bindings`, `run_variable_bindings` (with 017 connection, 019 deployment and 024 asset pins) |
| 8b — usage | quota and usage rows and the deferred content flush, last |

Writers follow the same slots: a resource write takes 4a (catalog lock, memberships, namespace),
its head in 4b `FOR UPDATE`, then its slot secrets and ACLs in 5; a variable write takes 4a (the
variable fence, memberships, namespace) and its head in 4b `FOR UPDATE`; neither takes a run row.
Transitions on an existing run (claim, resolve, completion, stream batches) start at slot 3R with the
run (chain before run), or with the stream lease row for streams, take 017's managed guard order
(binding → session → namespace secret/catalog → dedicated membership), then 4b and 5, and end with 8b.
T021-075 runs the 021 cases inside ADR-0050's harness (T018-012).

## Authorization

| Action | Permissions | admin | developer | operator | viewer | machine |
|---|---|:-:|:-:|:-:|:-:|---|
| List types, resources, variables (metadata), public variable values | `namespace:read` | ✓ | ✓ | ✓ | ✓ | — |
| Read an `internal` variable value | `payload:read-internal` | ✓ | ✓ | ✓ | — | — |
| Create a resource, or change a host-bearing setting or a slot's secret | `resource:manage` + `secret:manage` on each affected slot secret (D1158) | ✓ | — | — | — | — |
| Revise other settings, pause, delete a resource | `resource:manage` + slot secret use ACL | ✓ | — | — | — | — |
| Create, revise, delete a variable | `variable:manage` | ✓ | ✓ | — | — | — |
| Publish a workflow using a resource | `definition:publish` + `resource:use` (+ `resource:effect` for effect operations) + slot ACLs | ✓ | ✓ (O1121) | — | — | — |
| Start a run that uses a resource | human initiator: `run:start` + `resource:use` (+ `resource:effect`) + slot ACLs; the publisher: the same, current; triggered runs check the publisher only (017 rule) | ✓ | ✓ | — | — | `system:engine` grants nothing |
| Probe a resource | `run:start` + `resource:use` + slot ACLs | ✓ | ✓ | — | — | — |
| Manage routes, e-mail endpoints, connectors | existing `event:manage` (+ `secret:use` ACL) | unchanged grants | | | | — |
| Preview a filter with a sample | `event:read` + `definition:write` | ✓ | ✓ | — | — | — |
| Preview over retained events | above + `payload:read-internal` + namespace reveal grant (FR-016-026) | grant | grant | — | — | — |
| Claim/resolve `sagat.integration.*` tasks | `worker:execute` + `integration:execute` + binding | — | — | — | — | `integration-runner` only |
| Claim/renew/admit a CDC stream | `worker:execute` + `connector:stream` + binding | — | — | — | — | `database-runner` only |
| Claim/renew/admit an MQTT, Pub/Sub, Service Bus or WebSocket stream | `worker:execute` + `connector:stream` + binding | — | — | — | — | `integration-runner` only |

Ordinary `worker`, `runner` and `database-runner` cannot claim integration tasks through any
wildcard binding. Foreign resources are opaque `404`. When 029 is enabled, resource visibility and
use are decided by its evaluator over the resource's folder placement (FR-021-031): `viewer` confers
visibility, `runner` confers `resource:use` (and `resource:effect` under O1121); `resource:manage`
stays sensitive. Denials use the existing restricted denial
path; a failed denial audit never authorizes.

## Finite ceilings

Proposed values; each is confirmed or lowered by measurement (architecture principles: a ceiling
is accepted only when a measured run supports it).

| Resource | Default | Hard ceiling |
|---|---:|---:|
| Resources per namespace (active + paused) | 100 | 1,000 |
| Retained resource identities per namespace / revisions per resource | 1,024 / 1,000 | 1,024 / 1,000 |
| Settings JSON bytes per revision | 16 KiB | 64 KiB |
| Secret slots per resource | 4 | 8 |
| Concurrent operations per resource / namespace / tenant | 8 / 16 / 64 | 32 / 64 / 256 |
| Operation deadline / connect deadline | 30 s / 5 s | 120 s / 10 s |
| HTTP authored headers / bytes; query parameters; path bytes | 16 / 8 KiB; 32; 1 KiB | 64 / 16 KiB; 128; 2 KiB |
| Inline request or response body | 256 KiB | 256 KiB (payload limit) |
| Blob body (HTTP blob mode, S3 get/put) | 16 MiB | 64 MiB (ADR-0023) |
| OAuth2 token response bytes | 16 KiB | 16 KiB |
| GraphQL document / variables bytes | 16 KiB / 64 KiB | 64 KiB / 256 KiB |
| S3 list page keys | 100 | 1,000 |
| SMTP recipients / attachments / message bytes | 10 / 5 / 1 MiB | 50 / 10 / 10 MiB |
| Slack text characters / blocks bytes | 4,000 / 16 KiB | 40,000 / 64 KiB |
| Variables per namespace / revisions per variable | 200 / 1,000 | 2,000 / 1,000 |
| Variable value bytes / references per definition | 4 KiB / 32 | 64 KiB / 128 |
| Routes per namespace (part of `event_endpoints` quota) | existing | existing |
| Route path parameters / captured headers / captured query parameters | 8 / 16 / 16 | 16 / 32 / 32 |
| Route and e-mail body | 240 KiB | 240 KiB (inbox) |
| CDC connectors per namespace | 4 | 16 |
| CDC batch events / bytes per `AdmitConnectorEvents` | 100 / 1 MiB | 1,000 / 4 MiB |
| CDC lag threshold (operator ceiling) | 1 GiB | 64 GiB |
| CDC tables with a correlation column | 16 | 64 |
| MQTT topic filters / in-flight QoS 1 | 8 / 100 | 32 / 1,000 |
| Pub/Sub messages per pull / ack extension | 100 / 60 s | 1,000 / 600 s |
| Service Bus concurrent peek-locks per connector | 4 | 16 |
| WebSocket frame bytes / subscribe message bytes | 240 KiB / 16 KiB | 240 KiB / 16 KiB |
| Connector filter | evaluator limits (ADR-0032 §7) | same |
| Preview sample bytes / retained events per preview / previews per minute per principal | 64 KiB / 20 / 30 | 240 KiB / 100 / 120 |

Effective value = min(request, namespace, deployment). Zero refuses. Count limits are checked in
the writing transaction; operation limits at publication and again at admission and claim.
Existing run, retained-content, `events_per_second`, `event_connectors`, `event_endpoints` and
`secrets` quotas also apply.

## Failure and recovery matrix

| Failure | Outcome |
|---|---|
| Runner crash after resolve, before send | Lease expires; next attempt resolves again (new pin); no effect happened. |
| Runner crash after send, before report | Safe op: retried. Effect op: retried only with declared idempotency header (same key, same body); S3 put: reconcile by checksum; otherwise `MANUAL_REVIEW`. |
| Report lost after commit | Receipt replay returns the committed result; no second transition. |
| Stale attempt reports late | Fenced by generation/lease; audited no-op. |
| Step deadline vs late completion (timeout vs signal race) | First commit wins; the loser is an audited no-op; a cancellation is not proof the request was not sent. |
| OpenBao unavailable/sealed | Resolve fails `CAPABILITY_UNAVAILABLE` (retryable) before any send; no fallback. |
| Egress denied / redirect | `RESOURCE_DESTINATION_DENIED` (operations) or connector health `CONNECTOR_DESTINATION_DENIED` / `PROVIDER_REJECTED`; denial audited; never retried automatically. |
| Lost NATS hint | Reconciler finds ready integration tasks (invariant 5). |
| Compensation fails repeatedly | Saga `onCompensationFailure` policy: review or `COMPENSATION_FAILED`. |
| Runner or engine crash between inbox commit and source ack | Redelivery (MQTT QoS 1, Pub/Sub, Service Bus, Kafka) → `DUPLICATE`. |
| Runner or engine crash before inbox commit | Redelivery → admitted once; WebSocket/QoS 0 frame lost (labelled). |
| MQTT admission blocked (backlog, rate) | No PUBACK; disconnect after a bounded wait; broker redelivers in order. |
| CDC runner crash between commit and flush report | Replication restarts from the stored checkpoint; the transaction is resent and its ids deduplicate, also after the dedupe window (purge guard). |
| Connector paused or secret ACL revoked during a stream | Next renewal or batch refused; lease ends; runner closes the source connection. |
| Captured HMAC route request replayed | Same idempotency key → `DUPLICATE`; changed captured header → `401`; `generate` impossible on HMAC/OIDC routes. |
| CDC stale holder after lease takeover | `AdmitConnectorEvents` refused by generation; holder stops. |
| CDC consumer stopped long | Slot lag grows; `LAGGING`, alert, runbook; slot untouched. |
| Inbox backlog full | Consumers stop reading; brokers keep messages; routes/e-mail answer `429`/provider retry. |
| Route caller loses `202` | Retry with same key → `DUPLICATE`. |
| Mailgun retry after commit | Same `Message-ID` → `DUPLICATE` with `200`. |
| Restart of everything | Leases expire; supervisors and runners resume from checkpoints and receipts. |

## Observability

**Spans** (attributes `sagat.tenant.id`, `sagat.namespace.id`, `sagat.run.id`, `sagat.node`,
`sagat.attempt`, `sagat.resource.type`, `sagat.operation`, `sagat.connector.kind`, `sagat.outcome`,
`sagat.failure.code`; never a URL, path, header, body, address or value): `resource.catalog.write`,
`resource.resolve`, `resource.operation`, `resource.probe`, `variable.write`, `connector.consume`,
`connector.cdc.batch`, `route.ingress`, `email.ingress`, `filter.preview`.

**Metrics** (closed label sets): `resource_catalog_changes_total{action}`,
`resource_resolutions_total{outcome}`, `resource_operations_total{type,operation,outcome}`,
`resource_operation_seconds{type,operation}`, `resource_operation_failures_total{type,code}`,
`resource_probes_total{type,outcome}`, `oauth2_token_requests_total{outcome}`,
`integration_runner_slots_in_use`, `integration_runner_task_panics_total`,
`connector_messages_total{kind,outcome}` (admitted, duplicate, set_aside, filtered, backlogged),
`connector_cdc_slot_lag_bytes_max`, `connector_cdc_flush_reports_total{outcome}`,
`route_requests_total{outcome}`, `email_ingress_total{provider,outcome}`,
`trigger_filter_previews_total{scope,source,outcome}`, `variable_pins_total`.

Label sets: `action` ∈ create, revise, pause, delete; `type` ∈ http_api, graphql, s3_bucket, smtp,
slack_bot; `operation` ∈ the nine operation names; `code` ∈ the failure vocabulary of
[error-reasons.md](contracts/error-reasons.md); `kind` ∈ the connector kinds; resolution `outcome`
∈ resolved, denied, unavailable, stale; operation `outcome` ∈ succeeded, failed, unknown, cancelled;
probe `outcome` ∈ succeeded, failed; token `outcome` ∈ issued, refused, unavailable; flush
`outcome` ∈ reported, refused; route `outcome` ∈ admitted, duplicate, conflict, unauthenticated,
paused, throttled, too_large, missing_key; e-mail `provider` ∈ mailgun, relay and `outcome` as
routes; preview `scope` ∈ trigger, connector, `source` ∈ sample, retained, `outcome` ∈ matched,
not_matched, error, denied.

**Audit** (new): `resource.changed`, `resource.denied`, `resource.probe_admitted`,
`variable.changed`, `connector.lagging`. Reused: `secret.resolved`, `event.endpoint_*`,
`connector.*`, `event.rejected`, `payload.pii_revealed`, `worker.denied`,
`access.denied.permission`. Alert kind `CONNECTOR_LAGGING` joins the namespace monitor.

## Migration and compatibility

- Additive migrations after `20261220000000` (timestamps allocated at merge, D1151): resource
  tables, variable tables, endpoint/connector/inbox extensions, `runs.origin` value
  `RESOURCE_PROBE`, quota resource values. Grants for `sagat_app` follow 017 (SELECT/INSERT/DELETE on
  immutable tables plus ALWAYS triggers refusing UPDATE). Every new variable-size column enters
  `crates/infra-postgres/storage-classification.json`. Down migrations refuse while pins, bindings or
  rows of new kinds exist.
- IR: `resourceOperation` and `vars` behind enabled-capability checks (ADR-0032 §3, §8); old
  definitions unchanged; FR-016-002 gains `resourceOperation` as a carrier (T021-064).
- Protocol: one additive minor (provisionally 1.7) adds `ResolveResource` and the stream RPCs;
  workers below it never receive integration tasks or streams.
- AsyncAPI: `EventDocument.event` gains optional `message` and `request`; new data messages
  `CdcChangeData` and `EmailMessageData`; baseline regenerated deliberately.
- OpenAPI: additive paths and schemas; new `details.reason` values only.
- Rollout one capability at a time; rollback disables publication/admission of that capability and
  keeps readers until affected runs drain.

## Complexity Tracking

| Choice | Why needed | Simpler alternative rejected because |
|---|---|---|
| New `apps/integration-runner`, machine role and image (constitution II: a process is split out only after measurement) | Principle V: tenant-authored requests (ADR-0055 rule 11a), customer credentials, response parsing and the four new stream kinds must not run in a process holding platform database credentials; precedent: 017's `apps/database-runner` was split out for the same isolation reason (ADR-0035 decision 3), not for performance. Its own NetworkPolicy keeps HTTP/SMTP/Slack/broker egress apart from customer-database routes | Engine execution contradicts ADR-0035 §3 and rule 11a; extending the database runner gives non-database egress the private database network; a sandbox runner holds no managed capability protocol and is per-script |
| Two catalogs with shared rules | 017 is released with live pins | Migration rewrites immutable revisions and FKs of a release |
| Managed CDC stream RPCs | Private customer DBs only through the operator-approved boundary | Engine CDC opens customer DB sockets in the engine |
| Managed stream leases for four connector kinds in the integration runner | Customer credentials, sockets and parsing stay out of the engine; reuses the CDC stream RPCs | Engine placement repeats ADR-0055 option 3's exposure and the rebinding risk (G1125) |

## Delivery sequence

1. Setup and foundations (T021-001..016): ADR acceptance, pinned dependencies, domain, migrations,
   stores, shared plumbing extraction with database-runner regression.
2. US01 resources and probe; US02 HTTP/GraphQL operations, runner, effects (MVP).
3. US03 routes.
4. US04 CDC; US06 MQTT, Pub/Sub, Service Bus, WebSocket, connector filters; US05 S3, SMTP, Slack;
   US07 variables.
5. US08 e-mail; US09 preview.
6. Polish and gates: SDKs, conformance, Studio E2E, contract merge, runbooks, benchmarks, review.

## Verification plan

- **Unit**: domain rules, path normalization, signing strings, failure mapping, LSN arithmetic.
- **Integration (real PostgreSQL + services)**: catalog, pins, two-phase resolution with OpenBao,
  each operation against its service, each connector against its broker, CDC against a real
  logical slot, ingress profiles, preview audit, runtime-role privileges (`SAGAT_TEST_AS_RUNTIME_ROLE=1`).
- **Contract**: OpenAPI, IR schema, AsyncAPI compatibility, protobuf minor negotiation, five-SDK
  conformance scenarios `integration-resources.json`, `variables.json`.
- **Security**: tenant battery over every new operation, ordinary-worker refusal, egress refusal
  (metadata, loopback, link-local, rebinding, redirect) for every operation and every new connector
  kind, slot destination binding, route replay and authentication-first answers, canaries, uniform
  `401`, header injection.
- **Fault (rule 9)**: crash at each commit point above; duplicate delivery and duplicate report;
  timeout vs late completion and wait timeout vs CDC/MQTT event; lost broker hint; failed
  compensation; restart of engine, runner and database runner; stream revocation; lock-order
  interleavings (T021-075).
- **Quota/ceiling**: every row of the table at and beyond its bound; zero.
- **E2E**: Studio happy/failure flows; quickstart demo.
- **Benchmark criteria (declared environment, no promise)**: admission p50/p95/p99 with pins;
  resolution and OAuth2 cost per attempt; HTTP operation overhead versus a direct call; connector
  throughput and CDC lag recovery rate; route ingress p95 (feature 004 target <300 ms is a criterion).

## Project Structure

```text
specs/021-integration-resources-triggers/
├── spec.md  research.md  plan.md  data-model.md  quickstart.md  tasks.md
├── checklists/requirements.md
└── contracts/
    ├── resources.openapi.yaml          # REST excerpt
    ├── resource-types.md               # type catalog v1, operation fields, results, error mapping
    ├── resource-operation.schema.json  # IR excerpt
    ├── integration-runner.md           # worker protocol excerpt and capability rules
    ├── connector-kinds.md              # new connector and endpoint kinds, guarantees, signing
    ├── events.asyncapi-fragment.yaml   # additive event document and data messages
    └── error-reasons.md                # details.reason values and failure vocabulary
```
