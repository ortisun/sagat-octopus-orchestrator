# Tasks: 021 Integration resources, managed operations, variables and triggers

**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md),
[data-model.md](data-model.md), [contracts/](contracts/), [quickstart.md](quickstart.md).

Status: Proposed — not started; revised 2026-10-27 after the architecture and security reviews
(T021-075..077 added in the phases they belong to; T021-078/079 after the coordinator's
cross-feature decisions; T021-080 after the final verification). No task is checked; a task is closed only with the evidence it
names, recorded in a future `verification.md`.

Format: `- [ ] T021-NNN [P?] [USnn?] Description — files — evidence — traces`. `[P]` = can run in
parallel with other `[P]` tasks of the same phase (different files, no dependency). Tests are part
of each task (constitution VIII); a test is written first and seen failing.

## Phase 1: Setup

- [ ] T021-001 Owner review of spec, [ADR-0055](../../docs/adr/ADR-0055-integration-resources-and-inbox-connector-kinds.md) and O1120–O1123 (O1123: the placement rule of ADR-0055 decision 11); record answers in `specs/021-integration-resources-triggers/research.md`; obtain from the coordinator the protocol minor, migration timestamps and permission order (D1151) — evidence: recorded decisions — all FR.
- [ ] T021-002 [P] Pin and review new dependencies (SMTP, MQTT, WebSocket, GraphQL parser, replication client candidates) in `Cargo.toml` and `crates/infra-integrations/Cargo.toml`; licence and advisory audit — evidence: dependency audit job passes — FR-021-036.
- [ ] T021-003 [P] Test targets and fixtures: `tests/Cargo.toml` (`integration_integration_resources`, `contract_rest_integration_resources`, `fault_integration_resources`, `e2e_integration_resources`), directories `tests/{contract,integration,fault,e2e,security}/integration-resources/`, Testcontainers for MinIO, Mosquitto, SMTP server, Pub/Sub emulator, HTTP/OAuth2 server, Service Bus REST stub, WebSocket server — evidence: empty targets build and run — FR-021-036.

## Phase 2: Foundational (blocks every story)

- [ ] T021-004 Extract shared revision/receipt/lifecycle rules into `crates/domain/src/managed_catalog.rs`, used by `crates/domain/src/database.rs` — evidence: domain tests plus unchanged `tests/integration/database-connections` and `tests/contract/database-connections` — FR-021-005, SC-021-003.
- [ ] T021-005 [P] Resource types v1, settings, slots, status, limits in `crates/domain/src/resources.rs` and `crates/domain/src/resources/catalog.rs` — evidence: unit tests per bound and `CREDENTIAL_IN_SETTINGS` — FR-021-001..003.
- [ ] T021-006 [P] Operations, safe/effect class, effect-key rule, failure vocabulary in `crates/domain/src/resources/{operations,failure}.rs` — evidence: unit tests — FR-021-010..015, FR-021-017.
- [ ] T021-007 [P] Variables in `crates/domain/src/variables.rs` (types, levels, bounds, static references) — evidence: unit tests — FR-021-018, FR-021-019.
- [ ] T021-008 Permissions (six), role `integration-runner`, audit actions, quota resources, alert `CONNECTOR_LAGGING` in `crates/domain/src/{identity,audit,quota}.rs` — evidence: catalog and role-matrix tests — FR-021-031..033.
- [ ] T021-009 Migration for resources, revisions, bindings, credential pins, `runs.origin`, quota values, grants, ALWAYS and retained-byte triggers in `crates/infra-postgres/migrations/` plus `crates/infra-postgres/storage-classification.json` — evidence: forward/down tests, down refuses with rows — FR-021-002, FR-021-004.
- [ ] T021-010 [P] Migration for variables, revisions and `run_variable_bindings` — evidence: forward/down tests — FR-021-018, FR-021-019.
- [ ] T021-011 Migration for endpoint kinds, route prefixes, connector kinds/placement/filter/health, `connector_leases.generation`, `inbox_events.envelope_extension`, dead-letter reason — evidence: forward/down tests; existing 004 suites unchanged — FR-021-021..029.
- [ ] T021-012 Resource catalog service and store in `crates/application/src/resources.rs` and `crates/infra-postgres/src/resources.rs` (receipts, conflicts, ADR-0050 decision 7 lock order, OpenBao-only slots, `secret:manage` required to bind a slot to a destination, refusal of every database driver type, opaque 404) — evidence: real PostgreSQL/OpenBao integration tests — FR-021-002, FR-021-003, FR-021-005, FR-021-031.
- [ ] T021-013 [P] Variable service and store in `crates/application/src/variables.rs` and `crates/infra-postgres/src/variables.rs` — evidence: real PostgreSQL tests — FR-021-018, FR-021-020.
- [ ] T021-014 [P] Settings validation against type-version schemas in `crates/infra-schema` — evidence: unit tests with bounded documents — FR-021-001, FR-021-002.
- [ ] T021-015 Extract session/poll/heartbeat/receipt/slot plumbing from `apps/database-runner/src/{runner,rpc,slots}.rs` into `crates/infra-managed` without behaviour change — evidence: `tests/integration/database-runner` and database-connections suites pass unchanged — FR-021-008 (D1153).
- [ ] T021-016 Update `scripts/check-architecture.py` and `scripts/tests/`: add a per-crate client allowlist (`object_store` only in `crates/infra-blobstore`; SMTP, GraphQL and HTTP-provider clients only in `crates/infra-integrations`; broker, MQTT, WebSocket and cloud clients only in `crates/infra-connectors`; replication client only in `crates/infra-database`) and accept `crates/infra-integrations` and `crates/infra-managed` — evidence: architecture job passes and fails on each planted forbidden edge — FR-021-036 (D1133).

- [ ] T021-075 021 cases in ADR-0050's lock-order interleaving harness (T018-012): admissions pinning connections, resources and variables (and 024 assets when present) against concurrent resource, variable, secret-ACL and connection writers, in `tests/fault/integration-resources/lock_order.rs` — evidence: no deadlock or lock timeout over seeded interleavings — FR-021-004, FR-021-019 (D1159).

**Checkpoint**: catalogs, variables and migrations exist; 017 behaviour unchanged.

## Phase 3: US01 — Register a resource and probe it (P1)

Independent test: SC-021-001 against real OpenBao and local HTTP/OAuth2 servers.

- [ ] T021-017 [US01] REST for resource types and resources in `crates/api-http/src/routes/resources.rs` and `crates/api-http/src/dto_resources.rs` — evidence: contract tests against `contracts/resources.openapi.yaml`, no-store, current-role checks — FR-021-001..003, FR-021-031.
- [ ] T021-018 [US01] Fixed probes per type as durable runs with origin `RESOURCE_PROBE` and latest-probe metadata — evidence: integration tests incl. paused refusal and revision change — FR-021-006.
- [ ] T021-019 [US01] Security tests in `tests/security/integration-resources/`: cross-tenant/namespace, stale revision, revoked ACL, local-secret refusal, `MANAGED_BY_CONNECTIONS` for all seven driver types, a `secret:use`-only holder refused when changing a host-bearing setting, refused header names (`api_key_header.headerName`, captured `Set-Cookie`/`WWW-Authenticate`), five probe failure categories — evidence: suite passes — FR-021-003, FR-021-005, FR-021-006, FR-021-017, FR-021-031; SC-021-001, SC-021-003, SC-021-016.
- [ ] T021-020 [US01] Studio Resources page in `frontend/apps/studio/src/features/resources/` (list, form with slot secret create/rotate/select reusing `features/secrets`, probe, revisions; connections listed with a link) — evidence: Vitest — FR-021-003, FR-021-035.

## Phase 4: US02 — HTTP and GraphQL operations (P1, MVP)

Independent test: SC-021-004/005/007/008 with the recording HTTP server.

- [ ] T021-021 [US02] `resourceOperation` in `crates/infra-ir` codec and publication validation (capability, operation/type match, root-only, reserved prefix, compensate/reconcile) with fixture `tests/contract/integration-resources/resource-operation.workflow.json` — evidence: codec and schema tests — FR-021-007, FR-021-014.
- [ ] T021-022 [US02] Pin resource revisions in every admission path (API, trigger, child, rerun per ADR-0039, receipt replay, probe) under ADR-0050 decision 7 (slots 4a, 4b, 5, 6) — evidence: real PostgreSQL tests incl. crash before commit and 50 edit/retry interleavings — FR-021-004; SC-021-002.
- [ ] T021-023 [US02] Protocol minor with `ResolveResource` (two phases), claim gating and `readable_blob_ids` in `crates/api-grpc/src/resources.rs` and `crates/application/src/worker/resources.rs` — evidence: gRPC tests incl. ordinary worker and database runner refusal, revocation between phases — FR-021-008, FR-021-031.
- [ ] T021-024 [US02] `crates/infra-integrations`: guarded HTTP over `crates/infra-egress`, auth schemes, OAuth2 client credentials per attempt, output canary scanner — evidence: unit tests and local OAuth2 server tests — FR-021-009, FR-021-010, FR-021-016.
- [ ] T021-025 [US02] `http.request` and `graphql.execute` executors with result envelopes and status mapping (blob mode stays refused until T021-076) — evidence: recording-server tests (refused headers, path encoding, redirect refusal, kind mismatch, `GRAPHQL_ERRORS`) — FR-021-010, FR-021-014, FR-021-016, FR-021-017.
- [ ] T021-026 [US02] `apps/integration-runner` (config, sessions via `crates/infra-managed`, attempts, heartbeat/cancel, reports, metrics endpoint, SIGTERM) — evidence: process-level tests — FR-021-008, FR-021-032.
- [ ] T021-027 [US02] Effects: idempotency header, unknown-outcome transitions, declared reconcile/compensate as derived steps in `crates/domain/src/execution` and `crates/scheduler` — evidence: unit and integration tests; irreversible → `PARTIAL` — FR-021-015.
- [ ] T021-028 [US02] Fault suite `tests/fault/integration-resources/operations.rs`: crash before/after send, stale lease, duplicate report, lost NATS hint, deadline vs late completion, runner and engine restart, failed compensation under both policies — evidence: suite passes with journal consistency checks — FR-021-008, FR-021-015; SC-021-005, SC-021-006.
- [ ] T021-029 [US02] Egress and canary tests: loopback, link-local, metadata, platform endpoints, DNS rebinding, redirects, token request, echoed token, log/span/audit canaries — evidence: suite passes — FR-021-009, FR-021-016, FR-021-032; SC-021-007, SC-021-008.
- [ ] T021-076 [US02] Bounded cross-chunk streaming scanner for blob outputs in `crates/infra-integrations/src/scan.rs` (all encodings, three base64 alignments, pattern overlap across chunks) and server rescan before `COMMITTED`; then enable blob mode — evidence: canaries split across every chunk boundary and encoding are caught; G1143 closed — FR-021-016.
- [ ] T021-030 [US02] Studio `resourceOperation` node in the shared editor and runtime graph (no URLs or values) — evidence: Vitest — FR-021-007, FR-021-035.

## Phase 5: US03 — Custom HTTP routes (P1)

Independent test: SC-021-012.

- [ ] T021-031 [US03] Route kind, path grammar, shape conflicts and namespace prefix rotation in `crates/domain/src/events.rs` and `crates/infra-postgres/src/events.rs` — evidence: unit and store tests — FR-021-027.
- [ ] T021-032 [US03] Ingress `/v1/routes/{prefix}/{path}` in `crates/api-http/src/routes/http_routes.rs`: route-independent pre-authentication bounds only, then ADR-0026 profile, `octopus-route-in-v1` string with captured headers, then pause/size/rate/idempotency-header answers; `generate` refused on HMAC/OIDC routes at creation; captures into `event.request`; `202` after commit — evidence: integration tests — FR-021-027, FR-021-031.
- [ ] T021-033 [US03] Route suites: contract, duplicate/conflict/generate on mTLS, `generate` refused on HMAC/OIDC, captured-header tampering on replay, identical `401` for unknown/paused/oversized/header-less unauthenticated calls, connections killed under load — evidence: suites pass — SC-021-012.
- [ ] T021-034 [US03] Studio route form and prefix display/rotation in `frontend/apps/studio/src/features/integrations/` — evidence: Vitest — FR-021-035.

## Phase 6: US04 — PostgreSQL CDC (P2)

Independent test: SC-021-010 with real logical replication.

- [ ] T021-035 [US04] CDC settings (`slot` `^[a-z0-9_]{1,63}$`, single-identifier `publication`), event mapping, LSN rules and data-derived correlation marking in `crates/domain/src/connectors.rs` — evidence: unit tests incl. injection attempts in identifiers — FR-021-022, FR-021-031.
- [ ] T021-036 [US04] Replication client in `crates/infra-database/src/replication.rs` (G1121): choice with evidence, frame/aggregate budget, pgoutput decoding, standby updates — evidence: decision recorded in research, real PostgreSQL tests — FR-021-022.
- [ ] T021-037 [US04] Stream lease RPCs for every managed kind in `crates/api-grpc/src/connector_streams.rs`, `crates/application/src/connector_streams.rs`, `crates/infra-postgres/src/connector_streams.rs`: kind-to-role check (`database-runner` CDC, `integration-runner` the four others), generation fence, batch receipt, checkpoint CAS, revocation recheck on renewal and admission, no credential on renewal — evidence: integration tests incl. pause/ACL revocation mid-stream — FR-021-021, FR-021-022.
- [ ] T021-038 [US04] CDC consumer `apps/database-runner/src/cdc.rs` (prerequisites, batching, flush after commit, lag, backpressure) — evidence: process tests — FR-021-022.
- [ ] T021-039 [US04] Lag health, `connector.lagging` audit and `CONNECTOR_LAGGING` alert in `crates/scheduler/src/monitor.rs`; CDC section in `docs/runbooks/events.md` (customer SQL, `max_slot_wal_keep_size`, slot removal is the customer's decision) — evidence: monitor tests, runbook review — FR-021-022, FR-021-032, FR-021-036.
- [ ] T021-040 [US04] CDC suite: all ops, TOAST unchanged, `RELATION` change, crash between commit and flush (also with the restart after the dedupe window, proving the purge guard), stale holder, pause and ACL revocation ending the lease, missing prerequisites, lag threshold, wait timeout vs CDC event race, tenant isolation; no flush report before quorum durability in a `sync_quorum` cell (with 032); confirm `max_slot_wal_keep_size` semantics; runbook `pg_hba` guidance (database-specific line, never the `replication` keyword) — evidence: suite passes — SC-021-010.

## Phase 7: US05 — S3, SMTP, Slack (P2)

Independent test: SC-021-005/006 for these kinds.

- [ ] T021-041 [US05] S3 operations in `crates/infra-integrations/src/s3.rs` over the `CustomerBucket` factory added to `crates/infra-blobstore` (`object_store` with a guarded `HttpConnector`; fallback SigV4, G1122), blob streaming through the T021-076 scanner, checksum reconciliation — evidence: MinIO tests incl. unknown put — FR-021-011, FR-021-015.
- [ ] T021-042 [US05] `smtp.send` in `crates/infra-integrations/src/smtp.rs` (TLS, allowed senders, header injection, `Message-ID`, blob attachments) — evidence: SMTP server tests incl. drop after end-of-data — FR-021-012, FR-021-015.
- [ ] T021-043 [US05] `slack.postMessage` in `crates/infra-integrations/src/slack.rs` — evidence: stub tests (`429`, `ok:false`, unknown after write) — FR-021-013, FR-021-015.
- [ ] T021-044 [US05] Compensation faults: failing `s3.deleteObject` compensation of `s3.putObject`; irreversible SMTP in a saga ends `PARTIAL` — evidence: fault suite — SC-021-006.

## Phase 8: US06 — MQTT, Pub/Sub, Service Bus, WebSocket, connector filters (P2)

Independent test: SC-021-011, SC-021-014.

- [ ] T021-077 [US06] Managed stream host in `apps/integration-runner/src/streams.rs`: claims MQTT, Pub/Sub, Service Bus and WebSocket stream leases with `connector:stream`, renews, admits batches, stops on refusal; the engine supervisor skips `MANAGED` connectors — evidence: process tests incl. lease takeover between two runners — FR-021-021 (D1156).
- [ ] T021-080 [US06] One cargo feature per connector kind in `crates/infra-connectors/Cargo.toml` (`mqtt`, `pubsub`, `servicebus`, `websocket`; existing kinds unchanged), enabled only by `apps/integration-runner`, and a CI step (with `scripts/check-architecture.py` tests) asserting that `cargo tree -e features -p sagat-execution-engine` enables none of them — evidence: the check fails on a planted feature edge in the engine and passes otherwise — FR-021-021 (ADR-0055 Enforced by).
- [ ] T021-045 [US06] Connector filters for every kind and Kafka key exposure in `crates/infra-connectors/src/{filter,kafka}.rs` and `crates/application/src/connectors.rs`; `event.message` extension — evidence: real Kafka tests (key filter, position moves, `filter_error`) — FR-021-029; SC-021-014.
- [ ] T021-046 [P] [US06] MQTT consumer `crates/infra-connectors/src/mqtt.rs`, hosted by the integration runner (QoS 1 manual ack in PUBLISH order, persistent session, single stream holder, disconnect on blocked admission, QoS 0 opt-in, pre-connected TLS socket) — evidence: Mosquitto tests incl. ack order and redelivery after disconnect — FR-021-023.
- [ ] T021-047 [P] [US06] Pub/Sub consumer `crates/infra-connectors/src/pubsub.rs`, hosted by the integration runner (REST pull/acknowledge/modifyAckDeadline through the guarded client; token endpoint pinned) — evidence: emulator tests and confirmed API semantics in research — FR-021-024.
- [ ] T021-048 [P] [US06] Service Bus consumer `crates/infra-connectors/src/servicebus.rs`, hosted by the integration runner (REST peek-lock/complete/renew; SAS and Entra; host and tenant validation) — evidence: stub tests; semantics confirmed against Microsoft documentation in research — FR-021-025.
- [ ] T021-049 [P] [US06] WebSocket consumer `crates/infra-connectors/src/websocket.rs`, hosted by the integration runner (pre-checked socket, single stream holder, subscribe message, best-effort label) — evidence: server tests — FR-021-026.
- [ ] T021-050 [US06] Connector fault and security suite for every new kind: crash before/after commit, duplicates, lease takeover, backlog full, pause/ACL revocation at renewal, egress denial, DNS rebinding to metadata and private space (no handshake or credential leaves), credential rotation, tenant isolation — evidence: suite passes — FR-021-021; SC-021-007, SC-021-011.
- [ ] T021-051 [US06] Studio forms for the new kinds with guarantee label and filter field in `frontend/apps/studio/src/features/integrations/` — evidence: Vitest — FR-021-035.

## Phase 9: US07 — Variables (P2)

Independent test: SC-021-009.

- [ ] T021-052 [US07] REST variables in `crates/api-http/src/routes/variables.rs` with disclosure by level — evidence: contract tests — FR-021-018, FR-021-020.
- [ ] T021-053 [US07] `vars` references: extraction at publication in `crates/infra-ir`, evaluator context in `crates/infra-expr`, admission pins, 031's `ENCODING_READ_BY_EXPRESSION` check for every 021 expression position — evidence: tests for pins across retries, deletion, chain successor, filters refusing `vars` — FR-021-019; SC-021-009.
- [ ] T021-054 [US07] Studio Variables page in `frontend/apps/studio/src/features/variables/` and `vars` autocomplete — evidence: Vitest — FR-021-035.

## Phase 10: US08 — Inbound e-mail (P3)

Independent test: SC-021-013.

- [ ] T021-055 [US08] `MAILGUN` profile in `crates/api-http/src/routes/events.rs` and `crates/application/src/signing.rs` (JSON forward, HMAC over timestamp and body, authentication before per-endpoint answers, `200`/`406`, form refusal); confirm the scheme with provider fixtures or a sandbox account and settle G1140 before enabling — evidence: integration tests and recorded fixture provenance — FR-021-028.
- [ ] T021-056 [US08] `EMAIL` endpoint kind with `GENERIC_RELAY` (ADR-0026 profiles, authentication first, document validation, event id = digest of the normalized document, no subject copy) — evidence: integration tests — FR-021-028.
- [ ] T021-057 [US08] E-mail suite with the fixture classes of SC-021-013, including two messages sharing a `Message-ID` — evidence: suite passes — SC-021-013.

## Phase 11: US09 — Filter preview (P3)

Independent test: SC-021-015.

- [ ] T021-058 [US09] Preview in `crates/application/src/filter_preview.rs` and `crates/api-http/src/routes/filter_previews.rs` (sample/retained, reveal audit per event, per-principal rate, no echo) — evidence: integration and security tests — FR-021-030.
- [ ] T021-059 [US09] Studio preview panel for trigger and connector filters — evidence: Vitest proving no value beyond the user's sample is rendered — FR-021-030, FR-021-035; SC-021-015.

## Phase 12: Polish and gates

- [ ] T021-078 029 folder placement for resources: `item_kind = resource` in `item_placements` (migration with whichever of 029/021 lands second), evaluator checks at publication, admission and capability use, resource operations in 029's opacity battery and `x-access` declarations — evidence: 029 battery passes for resources; unlisted publisher refused like a revoked permission — FR-021-031; SC-021-016.
- [ ] T021-079 018 reset rules: `REMATERIALIZE` of `run_resource_bindings` and `run_variable_bindings` in 018's reset transaction (current-authority recheck, refusal reasons), no copy of credential pins; registry entries in 018's `contracts/journal-and-events.md` — evidence: reset cases in 018's replay-equals-projection corpus (T018-055) with resource and variable pins — FR-021-004, FR-021-019; SC-021-002.
- [ ] T021-060 Merge contract fragments into `specs/001-platform-foundation/contracts/{openapi.yaml,workflow.schema.json,worker.proto,events.asyncapi.yaml}` and their baselines; regenerate the Studio client — evidence: contract jobs and compatibility tests pass — FR-021-036; SC-021-014.
- [ ] T021-061 Spans, metrics with the closed label sets of the plan, audit actions; telemetry tests and OTLP canary checks — evidence: telemetry suite — FR-021-032; SC-021-008.
- [ ] T021-062 [P] SDK builders TypeScript and Go (`sdks/typescript/src/resources.ts`, `sdks/go/workflow/resources.go`) and scenarios `tests/contract/sdk-conformance/scenarios/{integration-resources,variables}.json` — evidence: conformance passes for both — FR-021-034; SC-021-004.
- [ ] T021-063 [P] SDK builders Python, C#, Java (`sdks/python/sagat_octopus_orchestrator/resources.py`, `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workflows/FlowResourceOperation.cs`, `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/workflow/ResourceOperationOptions.java`); all five pass the shared scenarios — evidence: `sdk_conformance` — FR-021-034; SC-021-004.
- [ ] T021-064 Dependent documents in the same delivery: the FR-016-002 carrier amendment stated in spec "Amendment to feature 016" (one consolidated amendment with the other parity drafts, coordinator C6) in `specs/016-payload-classification/spec.md`, notes in 004 and 017, `docs/architecture/{worker-runtime-protocol,storage-and-events,security-and-sandbox,overview}.md`, `docs/event-schema-evolution.md` — evidence: review — FR-021-016, FR-021-036.
- [ ] T021-065 Deployment: `infra/docker/Dockerfile` target, `infra/docker/compose.integrations.yaml` (development only), `infra/helm` role, service account, NetworkPolicy — evidence: built image, rendered chart checks — FR-021-036.
- [ ] T021-066 Runbooks `docs/runbooks/integration-resources.md`, `docs/runbooks/integration-runner.md`, new sources in `docs/runbooks/events.md`; run the quickstart and record outputs — evidence: recorded demo — FR-021-036; SC-021-018.
- [ ] T021-067 Every ceiling of the plan at and beyond its bound, zero refusal, operator ceilings — evidence: quota suite — FR-021-033; SC-021-017.
- [ ] T021-068 Extend `tests/contract/security-multitenancy/rest/battery.rs` to every new operation; `event.message`/`event.request` disclosure — evidence: battery passes — FR-021-031; SC-021-016.
- [ ] T021-069 Runtime-role sweep (`SAGAT_TEST_AS_RUNTIME_ROLE=1`) and storage-classification check — evidence: suites pass — NFR-021-001.
- [ ] T021-070 Studio E2E `frontend/e2e/tests/integration-resources.spec.ts` (happy/failure, accessibility) — evidence: Playwright run — FR-021-035; SC-021-001.
- [ ] T021-071 Benchmarks in `tests/performance/integration-resources/` and report `tests/performance/reports/integration-resources.md` on a declared environment — evidence: report with hardware, load, failures — NFR-021-005; SC-021-018.
- [ ] T021-072 Restart and coordinated PostgreSQL/OpenBao restore drill covering pins, checkpoints and stream leases — evidence: drill record — NFR-021-002.
- [ ] T021-073 Independent security, correctness and architecture reviews of the diff — evidence: findings closed or recorded as gaps — all FR.
- [ ] T021-074 Final gate (format, clippy, architecture, agent config, suites, contracts, five SDK lints) with actual results, skips and remaining gaps in `specs/021-integration-resources-triggers/verification.md` — evidence: recorded results — all FR.

## Dependencies and order

- Phase 2 blocks every story. T021-004 precedes T021-009/012; T021-015 precedes T021-026, T021-038
  and T021-077; T021-075 precedes T021-022; T021-076 precedes enabling blob mode (T021-025, T021-041).
- US01 → US02 (operations need resources); US03 needs only Phase 2 and T021-011; US04 needs T021-015
  and T021-011; US05 needs US02's runner (T021-023..027); US06 needs T021-011; US07 needs T021-013;
  US08 needs US03's endpoint work (T021-031); US09 needs T021-045 for connector filters.
- T021-060 runs after the shapes stabilise but before release; T021-062/063 after T021-021 and
  T021-053; T021-073/074 last.

## Parallel opportunities

T021-002/003; T021-005/006/007; T021-010/013/014; the four consumers T021-046..049; the SDK pairs
T021-062/063; Studio tasks of different features once their APIs exist.

## Traceability

| FR | Tasks |
|---|---|
| 001–003 | 005, 012, 014, 017, 019, 020 |
| 004–006 | 004, 009, 012, 018, 019, 022, 075 |
| 007–010, 014 | 021, 024, 025, 030 |
| 008 | 015, 023, 026, 028 |
| 009, 016 | 024, 029, 064, 076 |
| 011–013, 015 | 006, 027, 041–044 |
| 017 | 006, 019, 025 |
| 018–020 | 007, 010, 013, 052–054, 079 |
| 021–022 | 011, 035–040, 050, 077, 080 |
| 023–026 | 046–050 |
| 027 | 031–034 |
| 028 | 055–057 |
| 029–030 | 045, 058, 059 |
| 031–033 | 008, 012, 023, 061, 067, 068, 078 |
| 034 | 062, 063 |
| 035 | 020, 030, 034, 051, 054, 059, 070 |
| 036 | 002, 003, 016, 060, 064–066 |
