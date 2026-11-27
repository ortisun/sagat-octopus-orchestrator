# Implementation Plan: 021 Integration resources, managed operations, variables and triggers

**Branch**: `feature/021-integration-resources-triggers` | **Date**: 2026-10-26 | **Spec**: [spec.md](spec.md)

Status: Proposed — not started. Revised for build on 2026-11-24 against `develop` at `2c94064f`
([readiness review](research.md#readiness-review-2026-11-24), Decisions 021-001 to 021-021); the work is cut
into [build slices](#build-slices-2026-11-24) for two builders.

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
`crates/infra-connectors` and `crates/infra-database` may depend on them. (Revised 2026-11-24.) No
OAuth client library: the runner sends no token request (Decision 021-001). The output canary scanner
is one dependency-free crate, `crates/output-scan`, used by the runner and by the server's rescan so
that both scan identically.
**Storage**: PostgreSQL (authority); OpenBao KV v2 for slot values (ADR-0035); blob store for S3 bytes.
**Testing**: `sagat-tests` targets with real PostgreSQL, OpenBao, MinIO, Mosquitto, Kafka, the Pub/Sub
emulator, an SMTP test server, a WebSocket server and HTTP/OAuth2/Slack/Service Bus stubs;
`tests/contract/sdk-conformance`; Studio Vitest and Playwright.
**Target Platform**: Linux containers (Compose, Helm).
**Project Type**: modular monolith plus managed executors.
**Performance Goals**: none promised; benchmark criteria in "Verification plan" (G1139).
**Constraints**: finite ceilings below; no operation in API/scheduler/engine; additive contracts.
**Scale/Scope**: 36 FRs, 9 user stories, 80 tasks (2026-11-24: 38 requirements, 88 tasks).

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
| V. Isolation, tenant, secrets | Pass | Operations and the MQTT, Pub/Sub, Service Bus and WebSocket streams only in `apps/integration-runner`; CDC only in the database runner; the engine opens no new customer socket; slot values resolved per attempt under a fenced capability; binding a slot to a destination needs `secret:manage` (D1158). The pre-existing Kafka/NATS/SQS placement is unchanged (G1137). 2026-11-24: an OAuth slot is a design 047 credential; the runner receives only a current access token through `ResolveResource` and never a client secret or refresh token; the broker's token requests follow clause (e) of Architecture decision 0055 decision 11 (Decision 021-001). |
| IV, rollout (2026-11-24) | Pass | Five capabilities in the instance registry, switched on by audited operator activation (Decision 021-011); the protocol minor is the next free one at merge (Decision 021-005). |
| VII, run creation (2026-11-24) | Pass | Every run this feature creates passes the `RunInputGate` before its transaction (Decision 021-010). |
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

Revised placement (2026-11-24, verified at `2c94064f`):

| Place | Change |
|---|---|
| `crates/domain/src/workflow.rs`, `crates/domain/src/database.rs` | `ManagedKind::Integration` beside the database and lake kinds; every site listed by `crates/application/tests/managed_kind_sites.rs` decides it (Decision 021-008). |
| `crates/application/src/worker.rs`, `crates/infra-postgres/src/execution.rs` | Integration admission beside `database_claim` in the claim transaction, gated by 021's minor and role; design 038's budget moved in the same change. |
| `crates/application/src/run_input.rs`, `crates/application/src/runs/ad_hoc.rs` | `Creator::ResourceProbe`; the probe as a retained platform-built definition with origin `RESOURCE_PROBE` (Decisions 021-010, 021-015). |
| `crates/application/src/instances.rs` + migration | The five capabilities of Decision 021-011. |
| `crates/infra-ir/src/{schema,convert}.rs` | `resourceOperation` in `ROOT_NODE_TYPES` and the dispatch; the probe node private if Architecture decision 0080 is accepted (Decision 021-015). |
| `crates/infra-managed` (new) | Extracted from both profiles of `apps/database-runner` (SQL and lake): `runner`, `rpc`, `lease`, `slots` shared parts; the lake suites join the guard (Decision 021-007). |
| `crates/output-scan` (new, no dependencies) | Inline and cross-chunk canary scanner used by the runner and the server rescan. |
| `crates/infra-integrations` | No OAuth token client; the `oauth` scheme builds the `Authorization` header from the resolved access token. |

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
   commit leaves no run and no pin. (2026-11-24) The input was checked by the `RunInputGate` before
   this transaction opened, with no transaction open (Architecture decision 0090 decision 4); the
   probe's creator is `Creator::ResourceProbe`, every triggered run's is the dispatcher's.
5. **Claim** — existing lease transaction plus per-resource, namespace and tenant integration
   concurrency counts; requires current role `integration-runner`, binding and protocol minor.
   (Revised 2026-11-24, Decisions 021-008 and 021-009.) No new counter: namespace and tenant bounds
   are the activity quotas the claim already rechecks for the reserved activities; a per-resource
   bound held the step outside `READY` before it could be claimed (022's gated step). The
   integration admission is one call beside `database_claim`, inside design 038's moved budget.
6. **Resolve phase 1** — fences (lease, generation, session SHARE guard with post-lock DB clock,
   binding), pinned revision, current status, publisher/initiator rights, slot ACLs, provider
   metadata; commits nothing. Then OpenBao reads with no transaction open (and, for an `oauth`
   slot, the request to design 047's broker for the credential's current access token, also with no
   transaction open; Decision 021-001).
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
| Concurrent operations per resource / namespace / tenant (2026-11-24: per resource through a 022 concurrency key; namespace and tenant through the activity quotas of `sagat.integration.*`, Decision 021-009) | 8 / 16 / 64 | 32 / 64 / 256 |
| Operation deadline / connect deadline | 30 s / 5 s | 120 s / 10 s |
| HTTP authored headers / bytes; query parameters; path bytes | 16 / 8 KiB; 32; 1 KiB | 64 / 16 KiB; 128; 2 KiB |
| Inline request or response body | 256 KiB | 256 KiB (payload limit) |
| Blob body (HTTP blob mode, S3 get/put) | 16 MiB | 64 MiB (ADR-0023) |
| OAuth2 token response bytes (2026-11-24: withdrawn, the broker's bound is design 047's; the resolved access token counts against the slot value bound) | 16 KiB | 16 KiB |
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
| Token broker unavailable or credential revoked (design 047, 2026-11-24) | Resolve fails `CAPABILITY_UNAVAILABLE` (unavailable, retryable) or `RESOURCE_UNAVAILABLE` (revoked) before any send; the runner never falls back to a token of an earlier attempt. |
| Input refused by the `RunInputGate` (2026-11-24) | Triggered run: a delivery without a run, `run.input_refused` audited once; probe: refused before its transaction; nothing pinned. |
| Egress denied / redirect | `RESOURCE_DESTINATION_DENIED` (operations) or connector health `CONNECTOR_DESTINATION_DENIED` / `PROVIDER_REJECTED`; denial audited; never retried automatically. |
| Lost NATS hint (2026-11-24: lost transport hint, NATS JetStream or Redis Streams, Decision 021-013) | Reconciler finds ready integration tasks (invariant 5). |
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
`resource_probes_total{type,outcome}`, `oauth2_token_requests_total{outcome}` (2026-11-24: withdrawn, the
token requests are design 047's broker's; replaced by `resource_access_token_resolutions_total{outcome}` with
`outcome` ∈ issued, refused, unavailable),
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
- Revised 2026-11-24: migrations come after the newest on `develop` at merge (`20270119000000`
  today, Decision 021-006); `runs.origin` already holds `AD_HOC_STEP`, `AD_HOC_WORKFLOW` and
  `CONNECTION_PROBE` and gains `RESOURCE_PROBE`; the quota values `integration_concurrency` and
  `tenant_integration_concurrency` are dropped (Decision 021-009); the protocol minor is the next free
  one after 1.8, provisionally 1.9 (Decision 021-005); the five capabilities widen the three registry
  CHECKs with `NOT VALID` and a separate validate migration (Decision 021-011); "rollout one
  capability at a time" is the operator's activation of each of them.

## Complexity Tracking

| Choice | Why needed | Simpler alternative rejected because |
|---|---|---|
| New `apps/integration-runner`, machine role and image (constitution II: a process is split out only after measurement) | Principle V: tenant-authored requests (ADR-0055 rule 11a), customer credentials, response parsing and the four new stream kinds must not run in a process holding platform database credentials; precedent: 017's `apps/database-runner` was split out for the same isolation reason (ADR-0035 decision 3), not for performance. Its own NetworkPolicy keeps HTTP/SMTP/Slack/broker egress apart from customer-database routes | Engine execution contradicts ADR-0035 §3 and rule 11a; extending the database runner gives non-database egress the private database network; a sandbox runner holds no managed capability protocol and is per-script |
| Two catalogs with shared rules | 017 is released with live pins | Migration rewrites immutable revisions and FKs of a release |
| Managed CDC stream RPCs | Private customer DBs only through the operator-approved boundary | Engine CDC opens customer DB sockets in the engine |
| Managed stream leases for four connector kinds in the integration runner | Customer credentials, sockets and parsing stay out of the engine; reuses the CDC stream RPCs | Engine placement repeats ADR-0055 option 3's exposure and the rebinding risk (G1125) |
| A separate binary rather than a third profile of `apps/database-runner` (2026-11-24) | Another role and another egress; the integration clients stay out of the binary that reaches private customer databases | A profile, as the lake runner is (feature 035), shares the database runner's role and boundary, which is why it fits the lake and not this (Decision 021-007) |
| `crates/output-scan` (2026-11-24) | The runner's scan and the server's rescan must be the same code | Two copies drift; the server cannot depend on `crates/infra-integrations`, which holds the provider clients |

## Delivery sequence

1. Setup and foundations (T021-001..016): ADR acceptance, pinned dependencies, domain, migrations,
   stores, shared plumbing extraction with database-runner regression.
2. US01 resources and probe; US02 HTTP/GraphQL operations, runner, effects (MVP).
3. US03 routes.
4. US04 CDC; US06 MQTT, Pub/Sub, Service Bus, WebSocket, connector filters; US05 S3, SMTP, Slack;
   US07 variables.
5. US08 e-mail; US09 preview.
6. Polish and gates: SDKs, conformance, Studio E2E, contract merge, runbooks, benchmarks, review.

Superseded for building on 2026-11-24 by [Build slices](#build-slices-2026-11-24), which keeps this
order inside each lane.

## Build slices (2026-11-24)

Decision 021-020. The tasks are cut so that two builders work at the same time without editing the
same crate. One shared slice comes first; then lane A builds the control plane and lane B the
executors and sources; join slices close the end-to-end proofs. Each slice is one pull request (or a
short stack), lands its contract and documentation change before or with its code (constitution 4.7.0,
documentation first), and is closed only with the evidence its tasks name.

### Who owns which crate

| Owner | Crates, apps and files (edited only by this owner after slice 0) |
|---|---|
| Slice 0 (one builder, before the split) | root `Cargo.toml` and `Cargo.lock` (every new external dependency pinned here, and the new workspace members `crates/infra-integrations`, `crates/infra-managed`, `crates/output-scan`, `apps/integration-runner` created empty); `tests/Cargo.toml` (every 021 test target declared); `crates/api-grpc/build.rs` and the generated messages of `worker.proto` (new RPCs answering `UNIMPLEMENTED`); `crates/application/src/connectors.rs` port shape only (`RawMessage.key`); `scripts/check-architecture.py` and `scripts/tests/` (client allowlist) |
| Lane A — control plane | `crates/domain`, `crates/application`, `crates/infra-postgres` (all 021 migrations, stores, `storage-classification.json`), `crates/infra-ir`, `crates/infra-expr`, `crates/infra-schema`, `crates/api-http`, `crates/api-grpc` (handlers), `crates/scheduler`, `apps/control-plane`, `apps/execution-engine`, `sdks/*`, `tests/contract/sdk-conformance/`; the canonical contracts `openapi.yaml`, `workflow.schema.json`, `events.asyncapi.yaml` and the `worker.proto` text under `specs/001-platform-foundation/contracts/` |
| Lane B — executors and sources | `crates/infra-managed`, `crates/output-scan`, `crates/infra-integrations`, `crates/infra-blobstore`, `crates/infra-egress`, `crates/infra-connectors`, `crates/infra-database`, `apps/integration-runner`, `apps/database-runner`, `infra/docker/`, `infra/helm/`, `frontend/apps/studio/src/features/{resources,variables,integrations}` |
| Tests | the one `sagat-tests` crate is shared, but after slice 0 each lane adds files only under its own directories (lane A: `tests/{integration,contract,security}/integration-resources/{catalog,variables,routes,email,preview}`; lane B: `tests/{integration,fault}/integration-resources/{operations,streams,cdc}`); neither edits `tests/Cargo.toml` again |

Lane B uses the generated client of `crates/api-grpc` and the domain types but never edits those
crates; a change it needs there is asked of lane A. Lane A never edits a crate of lane B. A lane that
finds it needs the other's crate stops and records it instead of editing.

### Slices

| Slice | Owner | Tasks | Touches | Needs | Evidence that closes it |
|---|---|---|---|---|---|
| 0 Contracts, domain types and scaffolding | one builder | T021-001 (allocations only), T021-002, T021-003, T021-005, T021-006, T021-007, T021-008, T021-016; the `worker.proto` messages of [integration-runner.md](contracts/integration-runner.md) | as "Slice 0" above, plus `crates/domain` for the types of T021-005 to T021-008 (no `ManagedKind` variant yet, which would force every crate's matches) | nothing | builds, unit tests, architecture check failing on each planted edge |
| A1 Catalogs and variables | lane A | T021-004, T021-009, T021-010, T021-011, T021-012, T021-013, T021-014, T021-017, T021-052, T021-075, T021-081, T021-085, T021-088 | domain, application, infra-postgres, infra-schema, api-http | slice 0; feature 018's lock-order harness (built) | real PostgreSQL and OpenBao suites; 017 suites unchanged |
| B1 Shared runner plumbing and scanner | lane B | T021-015, the inline part of T021-024's scanner in `crates/output-scan` | infra-managed, output-scan, apps/database-runner | slice 0 | database-runner, database-connections and lake suites of 035 unchanged; scanner unit tests |
| A2 Operations, server side | lane A | T021-021, T021-022, T021-086, T021-023, T021-027, T021-018, T021-082, T021-084 (refusal side), T021-087, T021-053 | domain (`ManagedKind::Integration`), application, infra-postgres, infra-ir, infra-expr, api-grpc, scheduler | A1 | gRPC tests (ordinary and database runner refused, revocation between phases), pins across 50 interleavings, design 038 budget test moved and passing |
| B2 Integration clients | lane B | T021-024, T021-025, T021-041, T021-042, T021-043, T021-076 | infra-integrations, infra-blobstore, infra-egress, output-scan | B1 | recording-server, MinIO, SMTP and Slack-stub tests |
| B3 Integration runner | lane B | T021-026, T021-065 | apps/integration-runner, infra/docker, infra/helm | B1, B2; A2's RPCs for its process tests against the real server (until then against a test double of the generated service) | process-level tests; built image, rendered chart |
| J1 Operations end to end (MVP: User stories 021-01 and 021-02) | either builder, after A2 and B3 merge | T021-019, T021-028, T021-029, T021-044 | test files only | A2, B3 | the four suites pass on the merged tree, rerun by the merger (AGENTS.md rule 17) |
| A3 Inbound, server side | lane A | T021-031, T021-032, T021-033, T021-056, T021-055, T021-037, T021-039, the engine-side half of T021-045 (filter in `ConnectorRuntime`, `event.message` in the inbox and the dispatcher) | domain, application, infra-postgres, api-http, api-grpc, scheduler, apps/execution-engine | A1 | route and e-mail suites; stream-lease RPC tests incl. pause and revocation mid-stream |
| B4 Sources | lane B | T021-080, T021-077, T021-046, T021-047, T021-048, T021-049, T021-036, T021-038, the adapter half of T021-045 (Kafka key read, `filter.rs`, the supervisor skipping `MANAGED`) | infra-connectors, infra-database, apps/integration-runner, apps/database-runner | B1; A3's stream RPCs for real-server tests | Mosquitto, Pub/Sub emulator, Service Bus stub, WebSocket server, real Kafka and real logical replication tests |
| J2 Sources end to end | either builder, after A3 and B4 merge | T021-040, T021-050, T021-057 | test files only | A3, B4; feature 032 for the `sync_quorum` case of T021-040 | the suites pass on the merged tree |
| A4 Preview, reset, folders, SDKs | lane A | T021-058, T021-079, T021-078 (with 029), T021-083 (with 022), T021-062, T021-063 | application, api-http, infra-postgres, sdks | A2, A3; 022's gate for T021-083; 029 for T021-078 | preview security tests; 018 reset corpus cases; five-SDK conformance |
| B5 Studio | lane B | T021-020, T021-030, T021-034, T021-051, T021-054, T021-059 | frontend features only | the REST contract of A1 and A3 merged into `openapi.yaml` (T021-060 part) | Vitest |
| J3 Gates | both, in turn | T021-060 (each slice merges its own fragment; this task checks the whole), T021-061, T021-064, T021-066 to T021-074 | documents, test files, `tests/performance/` | every slice | Task 021-074's recorded gate |

Order: slice 0, then lane A (A1, A2, A3, A4) and lane B (B1, B2, B3, B4, B5) side by side; J1 after A2
and B3; J2 after A3 and B4; J3 last. The shortest path to a usable increment is slice 0, A1, B1, A2,
B2 (HTTP and GraphQL only), B3, J1: resources, probes and HTTP and GraphQL operations, with the other
operation kinds and every inbound source following.

### Dependencies on other work

| Work | What 021 needs from it | What it needs from 021 |
|---|---|---|
| Design 047 (OAuth resources, hub) | nothing to build 021; T021-084's `oauth` scheme is switched on by 047's broker capability | the catalog and its REST (T021-005, T021-012, T021-017), `ResolveResource` (T021-023), the provenance extension point (T021-085); Gap 1152 |
| Feature 026 | nothing; `response.mode: wait` stays refused | the route kind (T021-031, T021-032) for its synchronous routes |
| Feature 028 | nothing (its signed-index requirements serve design 047) | nothing |
| Feature 015 and OpenBao | nothing: tests start their own OpenBao container; the development overlay `compose.database.yaml` already pins one | nothing |
| Design 042 | nothing; trigger chains run after the inbox | the filter preview (T021-058) if it adds a chain scope |
| Feature 022 throughput slice | its gate for the per-resource concurrency limit (T021-083) | nothing |
| Feature 029 | its `item_placements` for T021-078 | the permissions of T021-008 |
| Feature 031 | its encoded-field registry for T021-053 when it exists | 021's positions as registry rows |
| Feature 032 | its `sync_quorum` mode for one case of T021-040 | nothing |
| Delivery acknowledgement (minor 1.8, branch `docs/delivery-acknowledgement`) | the minor order: 021 takes the next free minor after it (Gap 1149) | nothing |
| Design 043 | the creator row for the probe (Gap 1148) | nothing |
| Architecture decisions 0073 and 0080 (proposed) | their decisions, for Gaps 1150 and 1146 | nothing |

Rules for both builders: a private `CARGO_TARGET_DIR` outside the repository, at most two heavy builds
on the machine, `df -h` before building (AGENTS.md rule 19); each lane's pull request reruns its own
suites and the guard suites named in its slice; the merger reruns the join suites on the merged tree.

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
