# Tasks: TypeScript, Go, Python, C# and Java SDKs

**Input:** `specs/006-sdk-multilanguage/spec.md`, `plan.md`, `research.md`, `data-model.md`, contracts from 001.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [x] T006-001 Check FR-006-001..FR-006-008, SCs and ADRs; record gaps in `specs/006-sdk-multilanguage/research.md`. — gaps G121–G141 and decisions D119–D137 in `research.md`; the decision record is ADR-0015.
- [x] T006-002 [P] Define fixtures and contract validation in `tests/contract/sdk-multilanguage/fixtures/`. — there is no `fixtures/` directory: the fixtures are the 15 scenarios and their reviewed goldens in `tests/contract/sdk-conformance/{scenarios,expected}/` (the contract is its `README.md`), and the negotiation contract of the server is `tests/contract/sdk-multilanguage/grpc/negotiation.rs` (7 tests).
- [x] T006-003 [P] Document types, transitions and invariants in `specs/006-sdk-multilanguage/data-model.md`. — `data-model.md` (what an SDK holds in memory, the worker host and task states, the invariants; no persistent state, no migration).

## Phase 2: Foundation
- [x] T006-004 Create the necessary domain contracts/interfaces in `proto/sagat_octopus_orchestrator/v1/worker.proto` and invariant tests. — `worker.proto` is not changed by this feature (1.0–1.2 are 001's); the file each SDK copies is `specs/001-platform-foundation/contracts/worker.proto`, not `proto/…` (G121), with a drift test in each SDK; the canonical errors are `sdks/error-catalog.json` (19 codes, one test per SDK); the invariants are `negotiation.rs` (7) and `sdks/SPEC.md`.
- [x] T006-005 Implement ports/use cases in the appropriate module based on `specs/006-sdk-multilanguage/plan.md`. — server side: `crates/application/src/worker.rs` (negotiation, the heartbeat of 1.0 takes the step's own identity) and `crates/api-grpc/src/{request,service}.rs`; the SDK side is `sdks/SPEC.md` and the five libraries.
- [x] T006-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`. — no migration: the feature adds no table or column (`data-model.md`, "Migrations").
- [x] T006-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/sdk-multilanguage/`. — there is no `tests/integration/sdk-multilanguage/`: the SDKs are played against a real PostgreSQL and the real engine by the conformance suite, and the tenant boundary is `tests/fault/sdk-multilanguage/tenant.rs` (2 tests: another tenant's run is `NOT_FOUND`, a bad token is refused, an identity with no binding cannot register).

## Phase 3: US01 — Create an application run in C# (P1)
**Independent test:** Given an ASP.NET Core service; When it registers a client via DI and calls StartAsync; Then Run is created with idempotencyKey and trace context propagated.
- [x] T006-008 [P] [US01] Write a Given/When/Then test for creating an application run in C# in `tests/e2e/sdk-multilanguage/us01.rs`. — `tests/e2e/sdk-multilanguage/us01.rs` (2 tests: every SDK starts the run with its key and the trace it was given; the ASP.NET Core service through DI).
- [x] T006-009 [US01] Implement the domain path and error handling for creating an application run in C# in `proto/sagat_octopus_orchestrator/v1/worker.proto`. — `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/` (`DependencyInjection/`, `SagatClient.cs`, `Workers/`) and the clients of the other four (`sdks/*/`), not `proto/…/worker.proto`.
- [x] T006-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`. — no persistence is added; the adapters are the existing REST and gRPC ones (`crates/api-http`, `crates/api-grpc`, `crates/application/src/worker.rs`), with the heartbeat fix of 1.0.
- [x] T006-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`. — `worker_registrations_total{protocol}` on the engine (`crates/api-grpc/src/service.rs`); the C# `ActivitySource`/`Meter`, hooks in the others; a token, a secret or a payload is in no SDK output (the harness fails a scenario when one is).
- [x] T006-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/sdk-multilanguage/`. — `tests/fault/sdk-multilanguage/network.rs` (2: a lost answer or a request that never left is one run; a transport that is down ends in the canonical error), `tenant.rs` (2) and the `lost-responses`, `errors` and `worker-transport-faults` scenarios.

## Phase 4: US02 — Register workers in any SDK (P1)
**Independent test:** Given a developer with a TS/Go/Python/C#/Java activity; When they register a worker and receive a task; Then the server validates capabilities and completes the step via the common protocol.
- [x] T006-013 [P] [US02] Write a Given/When/Then test for registering workers in any SDK in `tests/e2e/sdk-multilanguage/us02.rs`. — `tests/e2e/sdk-multilanguage/us02.rs` (1 test: a worker of each SDK registers, receives the task and completes the step).
- [x] T006-014 [US02] Implement the domain path and error handling for registering workers in any SDK in `sdks/typescript/src/index.ts`. — `sdks/typescript/src/worker/host.ts` and the hosts of the other four (`sdks/python/…/worker/`, `sdks/go/worker/`, `sdks/dotnet/…/Workers/`, `sdks/java/…/worker/`).
- [x] T006-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/worker.rs` and `crates/api-grpc/src/request.rs` (the heartbeat of protocol 1.0 was refused by the server until the `protocol-negotiation` scenario ran it); no persistence change.
- [x] T006-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`. — `worker_registrations_total{protocol}`; the hosts' metrics hooks (tasks taken, completed, failed, stale, heartbeats, polls, reconnects); the `secrets` scenario reads a secret through `ctx.secrets()` in the five SDKs against the real server (the value is in no output; a protocol-1 worker gets `UNSUPPORTED_PROTOCOL`) and each SDK's unit tests cover its lifetime (wiped when the handler returns, a later `secrets()` raises).
- [x] T006-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/sdk-multilanguage/`. — `tests/fault/sdk-multilanguage/workers.rs` (5: silent worker loses its lease, a network that refuses each call once or twice, a cancelled run and a wait that times out, a worker stopped while it works, a handler that ignores its cancellation) and the scenarios `lease-lost`, `worker-shutdown`, `cancellation`.

## Phase 5: US03 — Write a typed Saga (P1)
**Independent test:** Given an activity with compensate/reconcile defined; When a workflow fails; Then all SDKs receive the same semantics and canonical errors.
- [x] T006-018 [P] [US03] Write a Given/When/Then test for writing a typed saga in `tests/e2e/sdk-multilanguage/us03.rs`. — `tests/e2e/sdk-multilanguage/us03.rs` (2: a failing saga compensated in reverse order and an unknown outcome reconciled; the failures of an activity reach the run as the same canonical codes).
- [x] T006-019 [US03] Implement the domain path and error handling for writing a typed saga in `sdks/go/client/client.go`. — the builders and compensation/reconciliation handlers of the five SDKs (`sdks/go/workflow/`, `sdks/go/worker/`, `sdks/typescript/src/workflow.ts`, …), not `sdks/go/client/client.go`; the `builders` and `saga-compensation` scenarios.
- [x] T006-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`. — no persistence is added; compensation and reconciliation are 003's and 002's on the engine, driven by the `task_kind` of protocol 1.2.
- [x] T006-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`. — as T006-016; the `saga-compensation` scenario compares the journal read over REST.
- [x] T006-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/sdk-multilanguage/`. — `workers.rs` and `network.rs` (as T006-012 and T006-017) plus the `saga-compensation` scenario (three verdicts, an unknown compensated step).

## Phase 6: US04 — Migrate the SDK version (P1)
**Independent test:** Given a compatible previous minor version; When the worker connects to the updated server; Then protocol negotiation supports progressive upgrade.
- [x] T006-023 [P] [US04] Write a Given/When/Then test for migrating the SDK version in `tests/e2e/sdk-multilanguage/us04.rs`. — `tests/e2e/sdk-multilanguage/us04.rs` (1 test: workers of an older and of a newer minor version are both served and another major is refused).
- [x] T006-024 [US04] Implement the domain path and error handling for migrating the SDK version in `sdks/python/sagat_octopus_orchestrator/client.py`. — `sdks/python/sagat_octopus_orchestrator/` (client, worker host, negotiation by minor version); the `protocol-negotiation` scenario with the five SDKs.
- [x] T006-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/worker.rs` (`negotiate_protocol`) and `crates/api-grpc/`; no persistence change.
- [x] T006-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`. — `worker_registrations_total{protocol}` tells an operator who still runs an old minor (a test of `negotiation.rs`).
- [x] T006-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/sdk-multilanguage/`. — `negotiation.rs` (7 tests) and `fault_sdk_multilanguage` (as above).

## Phase: Engineering design details (story-linked hardening)

- [x] T006-028 [US01] Establish `ProtocolConformance` fixtures and version matrix in `tests/contract/sdk-conformance/`. — `tests/contract/sdk-conformance/` (15 scenarios; the version matrix is the `protocol-negotiation` scenario with the five SDKs and `negotiation.rs`).
- [x] T006-029 [US01] Implement DI, worker hosting, ActivitySource and CancellationToken in `sdks/dotnet/src/`. — `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/` (`AddSagatOctopusOrchestrator`, a hosted worker, `ActivitySource`, `CancellationToken`); 410 tests including `DependencyInjectionTests`, `AspNetCoreTests`, `TelemetryTests`.
- [x] T006-030 [US01] Implement typed IR builders without side effects in the five languages in `sdks/*/`. — the `builders` scenario: the five SDKs write the same document and the same canonical (RFC 8785) bytes, with `invalidWorkflows` refused alike.
- [x] T006-031 [US01] Test idempotency, decimal serialization, UTC, signal and stale token in all SDKs. — the scenarios `lost-responses` (idempotency), `serialization` (numbers, instants in UTC), `signals-and-events`, `lease-lost` (stale token), each in the five SDKs.

## Final Phase: Hardening / rollout
- [x] T006-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/sdk-multilanguage/`. — `contract_grpc_sdk_multilanguage` (7 tests, negotiation and the 1.0 heartbeat) and every contract suite of the workspace run (87 test binaries, 1 341 passed, 0 failed, 23 ignored); OpenAPI lint valid with the same 9 warnings as `develop`, AsyncAPI valid, `worker.proto` compiles and the protobuf compatibility baseline passes (006 changed no proto).
- [x] T006-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`. — `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` (clean), `python3 scripts/check-architecture.py` (19 crates, layers respected, no cycles; the SDKs are not crates), `cargo audit --deny warnings` (exit 0). The SDKs have their own linters: `mypy --strict` and `ruff`, `go vet` and `gofmt`, `dotnet format`, `tsc`.
- [x] T006-034 Run the benchmark defined in `specs/006-sdk-multilanguage/plan.md` and publish the report in `tests/performance/reports/sdk-multilanguage.md`. — `tests/performance/sdk-multilanguage/` (release, `--ignored`); the report is committed; three complete runs, all hypotheses held in the last after H1, H3 and H4 were restated openly; G141 found by it.
- [x] T006-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/006-sdk-multilanguage/quickstart.md`. — four independent reviews and their re-verifications (security and correctness, `research.md` G134–G141 for what stayed open), the READMEs of the five SDKs, `sdks/README.md`, `docs/runbooks/sdks.md`; the evidence is in `quickstart.md`.
- [x] T006-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`. — `docs/traceability.md`, section "Evidence: 006 sdk-multilanguage" (FR/NFR/SC with states; partial: FR-006-008 publishing, NFR-006-OBS exporters; pending decisions G122, G123, G125, G126).

## Dependencies & independent delivery

- This feature requires: 001, 002, 003, 004.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.

## Completion of remaining requirements

- [x] T006-037 Apply owner-selected MIT licensing, build reproducible distributable artifacts for five SDKs, install each into a clean consumer and publish CI artifacts (FR-006-008). Registry release requires namespace/credentials. Evidence: `scripts/package-sdks.sh` produced 11 distributables plus SHA256SUMS, including Maven source/Javadoc JARs and release notes; five clean consumers passed in both runs; paired same-input artifact hashes matched. All three Maven JARs contain the MIT license, classifier installation byte-matches the candidates, and Java verification passes 205 tests.
- [x] T006-038 Verify opt-in integrations with each ecosystem's real OpenTelemetry API, in-memory export, safe attributes, propagation and exporter failure isolation (NFR-006-OBS; G126).
- [x] T006-039 Add protocol 1.3 blob upload/download and explicit descriptor completion to all five SDKs, with common conformance tests and older-server refusal (FR-002-008, FR-006-001).

T006-039 completion evidence: five-language real blob and run-input conformance, EOF/integrity/older-server refusal checks, TypeScript 99 tests, Python 231 plus Ruff/mypy, .NET 410, all Go tests/vet and Maven verify. Five distributable SDK artifacts pass clean-consumer smoke checks; public registry publication remains a separate credential/namespace gate.

## Task streaming completion — ADR-0029

- [x] T006-040 Implement canonical protocol 1.4 finite-credit StreamTasks and exact transactional
  session admission shared with PollTasks; prove compatibility, token/tenant/binding boundaries,
  concurrent claim limits, byte budgets, resource cleanup and durable disconnect recovery.
- [x] T006-041 Implement TypeScript/Python/Go/C#/Java public WorkerHost streaming with common
  auto/poll/stream behavior, immediate frame dispatch, bounded capacity, partial-delivery survival,
  shutdown and older-server fallback; regenerate each canonical protocol copy.
- [x] T006-042 Extend and execute shared five-language conformance and faults for streaming,
  measure poll/stream idle RPCs, claim attempts, latency, throughput and backpressure; update
  contracts, runbooks, version matrix and packages. Close G125 only with integrated evidence.


T006-040 evidence (2026-10-12): real durable gRPC contract 61/61, durable concurrency 14/14,
binding revoke/narrow 16/16, drain overlap 1/1, security gRPC/TLS 10/10, API gRPC 15 unit plus
2 compatibility tests, and streamed-lease engine-runtime restart/reclaim 2/2 pass. Includes actual
zero-window HTTP/2 reader, body-held caps and watchdog disarming, exact mixed poll/stream session
limits, stale-snapshot session replacement, blocked-claim cancellation, cumulative byte bounds,
fresh authority and bearer expiry. Targeted Clippy passes with warnings denied. Independent
server review is closed. The additional named `stream_telemetry` contract passes (1/1), proving
metric cleanup, bounded labels, trace preservation and secret-canary absence. A second named
contract holds both subject stream permits, verifies further admissions are refused, and proves
heartbeat/completion plus another tenant’s streamed work progress before the held bodies drop (1/1). Integrated five-SDK
evidence is below; benchmark/package gates remain T006-042.

T006-041 integrated evidence (2026-10-12): 19 common scenarios × five SDKs (95 executions), six end-to-end tests and nine injected-fault tests all pass against the real platform. Protocol probes explicitly cover 1.0–1.4 and a newer offered minor; the new stream scenario preserves partial deliveries through injected receive failure. All five protocol copies are byte-identical to the canonical schema. Final performance/package evidence remains T006-042.

T006-042 measurement evidence (2026-10-12): all 30 release comparison rows passed in 292.59s;
see `tests/performance/reports/sdk-streaming.md` and its raw capture. Stream claim counts
are reconstructed snapshot deltas with the documented shutdown-attribution caveat; the harness
now records explicit before/after snapshots. Signed ready→handler samples retain unknown cross-clock
offset; RSS covers the sampled runner process tree. No universal speedup or production throughput
is claimed. Final packages from SDK source commit `517c657` passed all five clean-consumer checks
(`target/sdk-packages-517c657`, 11 distributables plus checksums). G125 is closed within the
report’s stated measurement limitations; public registry publication remains separate.


## Approved dependent delivery: Rust SDK (2026-10-27)

The owner requested an official Rust SDK during the active 0.3 delivery. Feature
[034](../034-rust-sdk/spec.md) and [ADR-0070](../../docs/adr/ADR-0070-rust-sdk-shares-the-canonical-sdk-contract.md)
extend the same SDK semantic contract, canonical protocol, errors, conformance and package gates to
Rust. Existing five-language evidence stays intact. This authorizes the dependency; the new SDK's
actual acceptance is tracked by T034 tasks and is not supplied by this note.
