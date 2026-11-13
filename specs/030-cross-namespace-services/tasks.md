# Tasks — 030 cross-namespace service operations

Status: Proposed — no task started. Every task stays unchecked until the evidence it names exists
(a named test and its real result, a command and its output); a partly done task gets a dated note.

**Input**: [spec.md](spec.md), [plan.md](plan.md), [data-model.md](data-model.md),
[research.md](research.md), [contracts/](contracts/README.md).

Format: `- [ ] T030-NNN [P?] [USnn?] Description. Requirements. Files. Evidence.` `[P]` marks a task that
touches different files from the other open tasks of its phase and has no unfinished dependency.
New test targets (T030-003): `contract_service_operations`, `integration_service_operations`,
`fault_service_operations`, `e2e_service_operations`, `security_service_operations`,
`perf_service_operations`, all under `tests/<kind>/service-operations/`.

## Phase 1 — Setup and governance

- [ ] T030-001 Obtain the owner's acceptance of ADR-0065 and answers (or confirmed defaults) to
  O1480–O1482; the coordinator records them in `docs/owner-decisions.md` and the ADR index. All FR.
  Files: `docs/adr/ADR-0065-*.md`, `specs/030-cross-namespace-services/research.md`. Evidence: dated
  owner decision; ADR status changed from Proposed.
- [ ] T030-002 Record the approved dependencies in the dependent specs: 002 (`runs.service_call_depth`,
  child inheritance), 014 (capability gate, quiescence), 016 (carrier extension, G1493), 009 (catalog
  and quotas), 011 (pins on promotion). FR-030-007, 021, 024, 026, 027. Files: the `spec.md` of those
  features (dependency notes only). Evidence: reviewed notes; 016 owner acceptance of the carrier.
- [ ] T030-003 [P] Create the six test targets and empty suites with one failing placeholder each.
  Files: `tests/Cargo.toml`, `tests/{contract,integration,fault,e2e,security,performance}/service-operations/main.rs`.
  Evidence: `cargo test -p sagat-tests --test contract_service_operations --locked` compiles and runs.
- [ ] T030-004 Add the module skeletons (`crates/domain/src/services.rs`,
  `crates/application/src/services.rs`, `crates/application/src/service_operations.rs`) with no
  adapter imports. FR-030-001..036. Evidence: `python3 scripts/check-architecture.py` passes.
- [ ] T030-005 [P] Merge `contracts/service.schema.json` as
  `specs/001-platform-foundation/contracts/service.schema.json`; compile it and validate its examples
  in a contract test. FR-030-001, 002. Evidence: `contract_service_operations` schema tests.
- [ ] T030-006 [P] Merge `$defs.serviceCall` and its `node` branch into `workflow.schema.json`
  (not into `bodyNode`). FR-030-007, 035. Evidence: the existing IR fixture corpus still validates;
  new shape and refusal tests.
- [ ] T030-007 [P] Merge the OpenAPI paths/schemas (`x-proposed: '030'`) and the AsyncAPI channels;
  regenerate `events.baseline.json` deliberately. FR-030-028, 032, 035. Evidence: OpenAPI lint,
  `contract_asyncapi_events_webhooks_messaging`, reviewed baseline diff.
- [ ] T030-008 Merge `contracts/errors.md` into the `Error.details` description and `sdks/SPEC.md` §2;
  prove `sdks/error-catalog.json` unchanged. FR-030-015, 035. Evidence: contract test comparing the
  catalog; reviewer sign-off on all four merges.

## Phase 2 — Foundational: domain, IR, schemas

- [ ] T030-009 Domain names, modes, policies, bindings, allow-list, limits, reserved codes and outcome
  mapping. FR-030-001..006, 015, 019, 026. Files: `crates/domain/src/services.rs`,
  `crates/domain/tests/services.rs`. Evidence: unit and property tests.
- [ ] T030-010 [P] `OperationId::derive` and handler run id derivation, domain-separated and
  length-delimited. FR-030-010. Evidence: stability vectors shared with the five SDK test suites;
  collision property test.
- [ ] T030-011 `NodeKind::ServiceCall` and definition validation (root only, required fields, sync
  and saga rules, 64-node ceiling). FR-030-007, 019, 036. Files: `crates/domain/src/workflow.rs`.
  Evidence: domain validation tests.
- [ ] T030-012 Journal events `service_call.*`, `service_operation.*`, wait kind `SERVICE_CALL`,
  reducer transitions and replay. FR-030-009, 013..017. Files:
  `crates/domain/src/execution/{event,state,command}.rs`. Evidence: `crates/domain/tests/execution.rs`
  replay equals projection; event-code round trips in `crates/infra-postgres/src/event_codec.rs`.
- [ ] T030-013 [P] IR codec for `serviceCall` and the `Service` document with canonical digest.
  FR-030-001, 003, 007. Files: `crates/infra-ir/src/`. Evidence: round-trip and digest tests.
- [ ] T030-014 [P] `ContractSchemas` port and `infra-schema` implementation: closed/bounded output,
  no `secret`, classification walk (reuse feature 016), canonical equality, flat-handler check.
  FR-030-002, 005, 021. Files: `crates/application/src/services.rs`, `crates/infra-schema/src/`.
  Evidence: one test per defect class of `contracts/errors.md`.

## Phase 3 — US01 Publish a contract and register an endpoint (P1)

- [ ] T030-015 [US01] Migration for `services`, `service_revisions`, `service_endpoints`,
  `service_endpoint_revisions`, `service_endpoint_callers`: composite FKs, immutability triggers,
  grants, storage-classification entries, refusing down. FR-030-003, 004, 006, 025, 029. Files:
  `crates/infra-postgres/migrations/`, `crates/infra-postgres/storage-classification.json`.
  Evidence: migration integration tests incl. manifest-vs-catalog check and runtime-role privileges.
- [ ] T030-016 [US01] Contract publication (T1): create-on-first-use, idempotent by digest, receipts,
  quotas, audit. FR-030-001..003, 026, 031. Files: `crates/application/src/services.rs`,
  `crates/infra-postgres/src/services.rs`. Evidence: `integration_service_operations::contracts`.
- [ ] T030-017 [US01] Endpoint create/update/pause/resume/delete and allow-list (T2): CAS on revision,
  binding validation and health, tombstones, audit. FR-030-004..006, 031. Evidence:
  `integration_service_operations::endpoints` (every binding defect, name conflict, foreign namespace;
  with T030-014 and T030-016, SC-030-001).
- [ ] T030-018 [US01] HTTP routes and DTOs with `Idempotency-Key`, pagination and typed errors; CLI
  wiring is T030-054. FR-030-032. Files: `crates/api-http/src/routes/services.rs`,
  `crates/api-http/src/dto_services.rs`. Evidence: `contract_service_operations::rest` against the
  merged OpenAPI.
- [ ] T030-019 [P] [US01] Four permissions, role grants, audit actions and resource types.
  FR-030-024, 031. Files: `crates/domain/src/{identity,audit}.rs`. Evidence: full role-matrix test;
  catalog count 52.

## Phase 4 — US02 Call an async operation (P1)

- [ ] T030-020 [US02] Caller publication (T3): resolve, check, pin; `workflow_version_service_pins`
  migration; promotion re-resolution. FR-030-008, 011 (pins). Evidence:
  `integration_service_operations::publication` (pin immutability, deleted endpoint, promotion).
- [ ] T030-021 [US02] Migration for `service_operations`, `service_operation_executions`,
  `service_operation_messages`, `runs.service_call_depth`, wait kind, partial indexes, grants.
  FR-030-009..014, 027, 029. Evidence: migration tests, runtime-role privileges, refusing down.
- [ ] T030-070 [US02] Registrations: messages and call records in 020's content-store registry
  (`crates/infra-postgres/run-content-references.json`, erasure hook), every new time column in 028's
  `crates/infra-postgres/time-classification.json`, the four permissions in 029's list of additive
  grants. FR-030-016, 024, 029. Evidence: the registries' migration tests pass with the new columns.
- [ ] T030-022 [US02] Scheduling CP1 in the caller transaction. FR-030-009, 010, 016. Files:
  `crates/application/src/service_operations.rs`, `crates/infra-postgres/src/service_operations.rs`.
  Evidence: `integration_service_operations::schedule` (atomic rows, refusals write nothing).
- [ ] T030-023 [US02] Admission pass CP2/CP3/deferral under the handler-run shard fence. FR-030-011,
  012. Files: `crates/scheduler/src/passes.rs`. Evidence: `integration_service_operations::admission`.
- [ ] T030-024 [US02] Handler terminal hook CP5: result validation, declared-code mapping, disclosure
  recheck, `COMPLETED` message. FR-030-013, 015, 020. Evidence:
  `integration_service_operations::completion` (valid, invalid, undeclared code, revoked).
- [ ] T030-025 [US02] Notification pass CP4/CP6, discard path, routing and output charge. FR-030-014,
  015. Evidence: `e2e_service_operations::us02` (acceptance scenarios 1–5).

## Phase 5 — US04 Crashes, duplicates, lost hints, races (P1)

- [ ] T030-026 [US04] Outbox hints and polling recovery with bounded batches. FR-030-028. Evidence:
  lost-hint suite in `fault_service_operations::hints`.
- [ ] T030-027 [US04] Dead messages (T11), redrive route, audit. FR-030-028, 031, 032. Evidence:
  `integration_service_operations::dead_letters`.
- [ ] T030-028 [P] [US04] Retention purge of settled messages and record retention with runs.
  FR-030-029. Evidence: `integration_service_operations::retention`.
- [ ] T030-029 [US04] Crash matrix: before and after each of CP1–CP8, plus `COMPLETED` applied before
  `STARTED`, a `STARTED` that goes `DEAD` and is later redriven, and a start deadline racing a dead
  `STARTED`. FR-030-009..014. Evidence: `fault_service_operations::crashes`, `::ordering` (SC-030-003).
- [ ] T030-030 [US04] Duplicate delivery of every message kind in every order, concurrent claims by two
  engines, and a `CANCEL` for a purged handler run. FR-030-010, 014, 017. Evidence:
  `fault_service_operations::duplicates` (SC-030-004).
- [ ] T030-068 [US04] Declare the reset rules of every new event type in 018's reset-rule registry and
  test a reset over a finished call (`COPY`, no rows re-created) and over an open call (`REFUSE`), and a
  reset of a handler run (backs no operation). FR-030-038. Files:
  `specs/018-schedules-bulk-operations/contracts/journal-and-events.md` (registry entry, with 018's
  owner), `crates/domain/src/execution/event.rs`. Evidence: `integration_service_operations::reset`
  (SC-030-017).
- [ ] T030-031 [US04] Races (completion vs deadline, cancellation vs admission, 100 rounds each);
  restart and shard takeover with 1,000 calls in flight. FR-030-014, 016, 017. Evidence:
  `fault_service_operations::races`, `::restart` (SC-030-005, SC-030-015).

## Phase 6 — US03 Authorization, tenant boundary, disclosure (P1)

- [ ] T030-032 [US03] Publication and scheduling authority (`service:call`, allow-list advisory,
  publisher downgrade). FR-030-008, 009, 024. Evidence: `integration_service_operations::authority`.
- [ ] T030-033 [US03] Admission allow-list and completion revocation (`OPERATION_ACCESS_REVOKED`, no
  value retained). FR-030-011, 013. Evidence: `integration_service_operations::revocation`.
- [ ] T030-034 [P] [US03] Cross-tenant battery over every new route, name and id, driven by the
  merged OpenAPI, with audit of probes. FR-030-025. Evidence: `security_service_operations::tenants`
  (SC-030-006).
- [ ] T030-035 [P] [US03] Disclosure canaries (secret, unmarked, undeclared field, undeclared code) in
  journals, graph, stream, telemetry, logs, metrics, audit and listings of both namespaces.
  FR-030-020..023, 030. Evidence: `security_service_operations::canaries` (SC-030-007).
- [ ] T030-036 [P] [US03] Runtime-role privilege sweep: no `UPDATE` on immutable tables, no direct
  quota writes. FR-030-025. Evidence: `security_service_operations::privileges`.
- [ ] T030-037 [US03] Classified disclosure of call results through feature 016 (after its
  acceptance), `structure-only` before. FR-030-021. Evidence:
  `integration_service_operations::classification` and the 016 surface script.
- [ ] T030-066 [US03] `inputDisclosure` and the input classification check for direct source paths
  (`INPUT_CLASSIFICATION_LOOSENED`; sources outside a carrier count as `pii`). FR-030-007, 008, 021.
  Files: `crates/infra-ir/src/`, `crates/infra-schema/src/`. Evidence: publication tests per source kind;
  a computed expression is accepted and recorded as G1496.
- [ ] T030-067 [US03] Feature 031 interplay (refused, not carried; D1661): `ENCODING_NOT_APPLICABLE` for
  `x-sagat-encoding` in a `Service` document and for binding a handler version that declares encoding;
  `EncodingCheck` at caller publication refusing `serviceCall.input`/`compensateWith.input` from an
  encoded source; 031's runtime envelope check at admission (refuse the start) and at result return
  (withhold the value, effect `applied`). FR-030-011, 013, 039. Files: `crates/infra-ir/src/encoding_check.rs`
  (031), `crates/application/src/service_operations.rs`. Evidence: `integration_service_operations::encoding`,
  conformance `service-call-encoding` (SC-030-018).
- [ ] T030-069 [US03] Open or join the consolidated FR-016-002 amendment (the `serviceCall` carrier and
  the `Service` document schemas) and the classification rules version 2 entry in
  `SPECIFICATION-INDEX.md`, with 016's owner. FR-030-021. Evidence: accepted amendment text; 016
  publication tests over the new carrier.

## Phase 7 — US05 Cancellation and deadlines (P2); US06 Quotas and depth (P2)

- [ ] T030-038 [US05] Effective deadlines, start timeout, CP7 deadline path, void start deadline after
  admission. FR-030-016. Evidence: `integration_service_operations::deadlines`.
- [ ] T030-039 [US05] Cancellation policies, `START` withdrawal, CP8, handler-side cancel mapping,
  paused caller. FR-030-017, 018. Evidence: `e2e_service_operations::us05`.
- [ ] T030-040 [US05] Saga/scope drain and `continueAsNew` quiescence with a call in flight.
  FR-030-017. Evidence: `integration_service_operations::quiescence`.
- [ ] T030-065 [US05] Effect classes: record the class on every settlement, the `LOOKING_UP` state and
  outcome lookup window, the late outcome on a discarded `COMPLETED`, routing of effect-possible failures
  to saga compensation or `step.needs_review`, and `RETRY_STEP` as a new call attempt. FR-030-010,
  015..017, 037. Files: `crates/domain/src/services.rs`, `crates/domain/src/execution/{state,command}.rs`.
  Evidence: `integration_service_operations::effects` and `fault_service_operations::lookup`
  (SC-030-016).
- [ ] T030-041 [US06] Quota resources, effective limits, rate bucket source kind, migrations of the
  quota checks. FR-030-026. Files: `crates/domain/src/quota.rs`, `crates/infra-postgres/src/quotas.rs`.
  Evidence: quota unit and migration tests.
- [ ] T030-042 [US06] Caller-side refusals and handler-side deferrals with closed reasons, no attempt
  consumed. FR-030-012, 026. Evidence: `integration_service_operations::quotas` (SC-030-008).
- [ ] T030-043 [US06] Call depth across namespaces and child runs. FR-030-027. Evidence:
  `integration_service_operations::depth`.

## Phase 8 — US07 Sync operations (P2); US08 Observability and Studio (P2)

- [ ] T030-044 [US07] Sync binding and caller rules (pinned, flat, `request_cancel`, ≤ 60 s).
  FR-030-005, 019. Evidence: `integration_service_operations::sync_rules`.
- [ ] T030-045 [US07] Sync runtime and deadline expiry stopping the handler run. FR-030-019.
  Evidence: `e2e_service_operations::us07` (SC-030-010).
- [ ] T030-046 [US08] Journal-derived spans, computed ids, reciprocal links, live spans. FR-030-030.
  Files: `crates/application/src/telemetry.rs`, `crates/telemetry/src/span.rs`. Evidence: collector
  test with one trace across both namespaces.
- [ ] T030-047 [P] [US08] Metrics with closed labels and the two namespace alerts with runbook links.
  FR-030-028, 030. Files: `crates/telemetry/src/metrics.rs`, `crates/scheduler/src/monitor.rs`.
  Evidence: label-set test; alert fire/resolve test.
- [ ] T030-048 [US08] Operation listing and reading routes; runtime graph fields for the node.
  FR-030-022, 032. Evidence: `contract_service_operations::operations`.
- [ ] T030-049 [P] [US08] Studio registry, contract, endpoint and operations screens (tokens, WCAG AA,
  table alternatives, no sample values). FR-030-033. Files:
  `frontend/apps/studio/src/features/services/`. Evidence: Vitest and accessibility checks.
- [ ] T030-050 [US08] Studio `serviceCall` inspector, runtime node, cross-namespace links, unsupported
  fallback. FR-030-033. Files: `frontend/apps/studio/src/features/{workflows,runs}/`. Evidence:
  real-browser journey in `tests/e2e/workflow-studio/` (SC-030-011).

## Phase 9 — SDKs, CLI, conformance; US09 Compensation (P3)

- [ ] T030-051 [P] TypeScript and Go builders (`callService`, `service(...)` contract builder) and
  clients. FR-030-034. Files: `sdks/typescript/src/workflow.ts`, `sdks/typescript/src/services.ts`,
  `sdks/go/workflow/service.go`, `sdks/go/client/`. Evidence: SDK unit tests; shared id vectors.
- [ ] T030-052 [P] Python, C# and Java builders and clients. FR-030-034. Files:
  `sdks/python/sagat_octopus_orchestrator/{workflow,services}.py`,
  `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workflows/FlowServiceCall.cs`,
  `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/workflow/ServiceCallOptions.java`.
  Evidence: SDK unit tests.
- [ ] T030-053 Conformance scenarios of `contracts/conformance.md` with reviewed goldens in all five
  SDKs. FR-030-034. Evidence: `cargo test -p sagat-tests --test sdk_conformance -- --ignored` output
  for all five languages (SC-030-002, SC-030-009).
- [ ] T030-054 [P] CLI `sagat service`, `sagat service-endpoint`, `sagat service-operation`; `sdks/SPEC.md`
  §7.2. FR-030-034. Files: `apps/cli/src/services.rs`, `apps/cli/src/args.rs`. Evidence: CLI tests.
- [ ] T030-055 [US09] `compensateWith`/`irreversible` rules and the derived compensation instance
  (ADR-0010). FR-030-036. Evidence: domain tests.
- [ ] T030-056 [US09] Runtime compensation call through the same machinery. FR-030-036. Evidence:
  `integration_service_operations::compensation`.
- [ ] T030-057 [US09] Failing compensation reaches manual review or `COMPENSATION_FAILED`.
  FR-030-036. Evidence: `fault_service_operations::compensation` (SC-030-013).

## Phase 10 — Documentation, benchmark and release gates

- [ ] T030-058 [P] Architecture and guide: `docs/architecture/execution-semantics.md` (service calls),
  `docs/architecture/security-and-sandbox.md` (crossing rules), `docs/guides/service-operations.md`.
  FR-030-001..036. Evidence: reviewed docs.
- [ ] T030-059 [P] Runbook `docs/runbooks/service-operations.md`: dead messages, deferrals,
  revocation, redrive, pinned versus active bindings; alert entries in `docs/runbooks/alerts.md`.
  FR-030-028. Evidence: runbook drill recorded.
- [ ] T030-060 Turn [quickstart.md](quickstart.md) into a checked script and record its real output
  (happy path and the four failure scenarios). Evidence: script output in the verification record.
- [ ] T030-061 Benchmark harness and report `tests/performance/reports/service-operations.md` stating
  environment, configuration, p50/p95/p99, failures. NFR-030-005. Evidence: the report (SC-030-014).
- [ ] T030-062 Compatibility suite: existing workflows, runs, SDK clients and an older Studio; capability
  disabled. FR-030-035. Evidence: `contract_service_operations::compatibility` (SC-030-012).
- [ ] T030-063 Release gates: fmt, clippy `-D warnings`, workspace tests, architecture, contracts,
  compose smoke, advisory audit; independent security, correctness and architecture reviews of the
  diff. All FR. Evidence: recorded commands and results, including failures and skips.
- [ ] T030-064 Enable the capability by deployment setting after T030-063; the coordinator updates
  `docs/traceability.md`, `docs/roadmap.md` and `SPECIFICATION-INDEX.md`. FR-030-035. Evidence:
  owner approval of enablement.

## Dependencies and parallelism

- Phase 1 blocks everything; T030-001 blocks T030-004 and every implementation task.
- Phase 2 blocks Phases 3–9. US01 blocks US02; US02 blocks US03–US09.
- US04 fault suites (T030-029..031) run after T030-022..025 and again before T030-063.
- T030-037 waits for feature 016's acceptance (T030-002, T030-069). T030-053 needs T030-051/052.
- T030-065..070 were added after the 2026-10-27 review; they sit in the phase of the story they serve.
  T030-067 and T030-068 depend on 031's and 018's registries existing; T030-070 on 020's and 028's.
- `[P]` tasks inside a phase touch disjoint files; SDK tasks T030-051/052 and Studio T030-049 can run in
  parallel with Phase 7.

## Requirement coverage

| Requirements | Tasks |
|---|---|
| FR-030-001..003 | T030-005, 009, 013, 014, 016 |
| FR-030-004..006 | T030-015, 017, 019 |
| FR-030-007, 008 | T030-006, 011, 013, 020, 032 |
| FR-030-009..015 | T030-010, 012, 021..025, 029, 030, 065, 067 |
| FR-030-016..018 | T030-038..040, 065, 070 |
| FR-030-019 | T030-044, 045 |
| FR-030-020..023 | T030-024, 033, 035, 037, 066, 069 |
| FR-030-024, 025 | T030-019, 032..036, 070 |
| FR-030-026, 027 | T030-041..043 |
| FR-030-028, 029 | T030-026..028, 070 |
| FR-030-030, 031 | T030-046, 047, 019 |
| FR-030-032..034 | T030-018, 048..054 |
| FR-030-035 | T030-006..008, 062, 064 |
| FR-030-036 | T030-055..057 |
| FR-030-037 | T030-065 |
| FR-030-038 | T030-068 |
| FR-030-039 | T030-067 |
| NFR-030-001..006 | T030-029..031, 034..036, 061, 062, 041 |
