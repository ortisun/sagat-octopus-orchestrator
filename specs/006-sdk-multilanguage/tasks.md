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



## Phase: Delivery acknowledgement (ADR-0099, protocol 1.8; owner decision OD-2026-11-20-12)

The contract is `stream-completion.md`, "Delivery acknowledgement (protocol 1.8)"; the decisions are D006-040 to D006-060 of
`research.md`. Every test below is written first and seen failing for the right reason (AGENTS.md rule 18); races are held with
the run lock or a barrier, never a sleep; every task that touches a lease has a root and a scoped row in its evidence. A task
is ticked only with the named test and its result, or the command and its output.

**Before the first task.**
- (a) ADR-0098 is accepted and both of its builds are merged: the server (`bugfix/withdraw-undelivered-leases`, draft pull
  request #260, with its scoped withdrawal) and the SDK hand-back (`bugfix/withdraw-leases-sdks`), with the two requests of
  decision D006-059 answered.
- (b) Task T041-7 has merged (the server serves 1.7) before task T006-048 and before the `AppendStepLogs` case of task
  T006-053; tasks T041-15 to T041-20 (each SDK speaks 1.7) have merged before the matching SDK task T006-058 to T006-063.
- (c) The owner has accepted ADR-0099, with the trust assumption of its decision 13.

**Ownership** (as "Implementation ownership and integration order" above): the server owner, tasks T006-043 to T006-056; the
shared conformance owner, task T006-057; one owner per SDK, tasks T006-058 to T006-063; the integrator, tasks T006-064 to
T006-067. Shared files have one owner each.

### Server

- [ ] T006-043 The canonical contract: `AcknowledgeTasks`, its messages and the enum with `CANCELLED` in
  `specs/001-platform-foundation/contracts/worker.proto` ("Added in protocol 1.8"), and `TASK_NOT_ACKNOWLEDGED` in its error
  comment; the Rust descriptor baseline; the request, the response and `TaskAcknowledgement` in `skip_debug`
  (`crates/api-grpc/build.rs`) with a `Debug` that prints counts; every implementation of the generated trait answers it
  (`WorkerGrpc`, the `Stub`s of `tests/contract/durable-execution/grpc/limits.rs` and
  `tests/contract/security-multitenancy/grpc/stub.rs`, the `Proxy`s of `tests/integration/database-runner/proxy.rs` and
  `sdks/rust/conformance/src/faults/grpc.rs`, `Fake` in `apps/database-runner/src/testing.rs`, `Service` in
  `sdks/rust/src/worker_tests.rs`; `UNIMPLEMENTED` until task T006-049). Evidence: the descriptor test of
  `contract_grpc_durable_execution`, `cargo clippy --workspace --all-targets`, a token canary on the request and on the element.
- [ ] T006-044 The migration (the `/migration` checklist): on `steps`, `lease_offered_generation bigint`, `lease_offered_at` and
  `lease_acknowledged_at timestamptz`, `lease_unacknowledged_withdrawals smallint NOT NULL DEFAULT 0`, `withdrawn_lease_token_hash
  bytea`; on `worker_sessions`, `acknowledgement_kept boolean NOT NULL DEFAULT true`; no backfill, no `CHECK`; the table-level
  grants of `sagat_app` cover them (verified, not assumed); a `down` that refuses while a mark counts; the storage
  classification; the privilege audit. Evidence: the forward test, the refused `down`, the privilege audit of
  `integration_platform_foundation`, also as the runtime role.
- [ ] T006-045 The domain: the cause `NOT_ACKNOWLEDGED` of `step.lease_withdrawn` (event, replay, reset copy, the codec of
  `crates/infra-postgres/src/event_codec.rs`, the events contract, the Studio's closed list of causes) and the failure code
  `ATTEMPT_TIMEOUT_BEFORE_START`, retryable with a known outcome (never consultation or review). Evidence: codec round trip, a
  fixture holding the cause in `crates/domain/tests/legacy_replay.rs`, reset copying it, a domain test that the code consumes
  the attempt and opens no review for an `Activity`.
- [ ] T006-046 The registry: the capability `delivery-acknowledgement-v1`, listed by every control plane and by an engine only
  when its lease time-to-live is at least 12 s; the guard of the dependency on `lease-withdrawal-v1` in the registry's
  `ENABLE ALWAYS` trigger function, both ways; `control-plane delivery-acknowledgement-activate` and `-remove` (`--operator`,
  `--expect-live N`, `--dry-run`); the removal's second transaction, after the learning bound, that expires the sessions accepted
  at 1.8; the runbook's activation and removal (`docs/runbooks/instance-registry.md`). Evidence: the trigger refuses both orders,
  also with the hand SQL of the runbook, with the race between activation and removal held open; an engine at 11 s does not list
  the capability and is refused after an activation; the removal expires the 1.8 sessions and their hosts register again at 1.7.
- [ ] T006-047 The claim marks a lease, in the root statement and in the scoped path statement, only for a session accepted at
  1.8 with `acknowledgement_kept`, while the engine knows the requirement, with a time-to-live of at least 12 s and fewer than
  three withdrawals in a row; the condition is one SQL macro and the store returns the marks typed. Evidence: a matrix test in
  `integration_durable_execution` (minors 1.4, 1.7 and 1.8, requirement recorded or not, each condition alone, root and scoped);
  a lease written afterwards with the old column list is not provisional; the unchanged `LEASE` line of task T006-055.
- [ ] T006-048 Negotiation: `PROTOCOL_MINOR` and `DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR` at 8, `protocol_ceiling` answering 8
  only when it would answer 7, the store proves the columns and the requirement is known, logging once what capped it; the
  database runner keeps 6; `GET /version` reports the gated ceiling. Evidence: the ceiling unit tests of
  `crates/application/src/worker.rs` (each condition alone caps at 7), `tests/contract/sdk-multilanguage/grpc/negotiation.rs`,
  a test of `GET /version` with and without the requirement.
- [ ] T006-049 `AcknowledgeTasks`: the application service in a module of its own (so the test selector can match it by path),
  the store methods (the unlocked indexed pre-read, the lock-only open that takes the chain lock and then the run row's lock in
  the order of `begin` and `begin_path`, the conditional write with `RETURNING`), the binding check on the returned activity with
  the patterns read once per call, the bounded audits, the gRPC handler in class `Lease`, the request bounds, the counters.
  Evidence: a test per verdict and per path (held, duplicate, renewed, expired before the pass, withdrawn, replaced, wrong attempt
  or generation, forged token with no lock taken, a foreign tenant's run audited once per call, a revoked binding rolled back
  and audited once, a run that cannot be opened answered `NOT_HELD` while the others are acknowledged, a store failure after the
  first run then a repeat answered `duplicate`), the class test
  `only_the_calls_that_keep_leased_work_alive_are_reserved_capacity` listing it, and the runtime role.
- [ ] T006-050 Cancellation before the start: a provisional lease whose cancellation is requested is withdrawn (`NOT_STARTED`)
  with the settle in the acknowledgement's transaction and answered `CANCELLED`, root and scoped, with and without declared
  secrets. Evidence: the run ends `CANCELLED`, no handler would run, no `step.needs_review`; a compensation of a stopping run is
  never answered `CANCELLED`.
- [ ] T006-051 The lost-lease pass: the re-read under the lock (`lease_deadlines_instance` extended), the pure decision function
  with a unit test over every combination of the marks, the status and the two deadlines, the fenced withdrawal with
  `NOT_ACKNOWLEDGED` (root in the whole-run transaction, scoped in the path transaction), the counter of withdrawals in a row,
  `withdrawn_lease_token_hash`, `ATTEMPT_TIMEOUT_BEFORE_START`; a `TASK_NOT_STARTED` for an acknowledged lease is an ordinary
  retryable failure; ADR-0098's `NOT_SENT` withdrawal requires an unacknowledged lease. Evidence: a provisional `Activity` lease
  that expires is withdrawn with no review and the attempt reused by the next lease; an acknowledged one that expires is an
  unknown outcome; a provisional one past its attempt deadline ends `ATTEMPT_TIMEOUT_BEFORE_START` and is retried; the fourth
  loss of a step lapses; a stopping collection drains a withdrawn instance.
- [ ] T006-052 The races, in `tests/fault/sdk-multilanguage/`: the acknowledgement holds the run lock while the withdrawal waits,
  and the reverse; the pass's candidate read taken before an acknowledgement that renews the lease, then the pass applied after
  the renewed expiry has passed; an acknowledgement after the expiry and before the pass; the engine stopped after the
  acknowledgement's commit and before its answer. Evidence: barrier tests watched in `pg_stat_activity`, each asserting the
  journal and that the handler would run exactly once.
- [ ] T006-053 The capability refusal: the fence variants (root and path) called only from the capability paths, after the token
  and the binding matched; the descriptor-driven contract test over every lease-token method but `Heartbeat`, `CompleteStep`,
  `FailStep` and `AcknowledgeTasks`. Evidence: that test in `contract_grpc_durable_execution` (refused before, served after the
  acknowledgement, root and scoped; the `AppendStepLogs` case once task T041-7 has merged); a report on a provisional lease is
  served; a wrong token on a provisional lease gets the ordinary stale answer; no statement added to the budget of task
  T006-055.
- [ ] T006-054 What the server sees of the trust assumption: a `Heartbeat`, `CompleteStep` or `FailStep` on a provisional lease
  clears `acknowledgement_kept` and is counted; a stale report or heartbeat carrying a withdrawn token is counted and audited.
  Evidence: a test for each, and that the session's next lease is not provisional.
- [ ] T006-055 The budget (`tests/integration/claim-transition-efficiency/work_budget.rs`): a run of three chained activities
  served at 1.8 with the requirement recorded; `LEASE`, `COMPLETION_WITH_A_NEXT_STEP` and `COMPLETION_THAT_ENDS_THE_RUN`
  unchanged; new lines `ACKNOWLEDGEMENT` (one task, one run) and `FIRST_HEARTBEAT` pinned at their measured counts, asserting
  that `ACKNOWLEDGEMENT` has no step scan of the run and no journal or outbox insert, and fewer statements than
  `FIRST_HEARTBEAT`; a call of 64 tasks over 64 runs measured and itemised. Evidence: two equal passes and the itemised
  statements of the new lines in the task.
- [ ] T006-056 Observability and operations: the counters, the two histograms, the gauges of the operations statistics query,
  the four alerts with `promtool test rules` cases at the default and at a 12 s time-to-live (added to the CI step), the symptoms
  in `docs/runbooks/sdks.md`, and a row per new rule in the fitness-function table of
  `docs/engineering/architecture-principles.md`. Evidence: a telemetry test with bounded labels and a token canary, and the
  `promtool` output.

### Conformance and SDKs

- [ ] T006-057 The shared harness (`tests/contract/sdk-conformance/`): the fault actions `hold` (a call kept at the transport hook
  until the scenario releases it) and `dropFrame` (one frame of a stream lost); the platform booted with `lease-withdrawal-v1`
  and `delivery-acknowledgement-v1` recorded, and a scenario attribute that boots without them; the override
  `SAGAT_SDK_PROTOCOL_OVERRIDE=1.8`, which raises only scenarios pinned at 1.5; the scenario `delivery-acknowledgement.json`: a
  lost `PollTasks` answer then `expireLease` (withdrawn, run once, attempt 1 twice, no review), a dropped stream frame (the same),
  a lost acknowledgement answer (one handler, `faultsApplied` 1), an acknowledgement that never leaves twice (one handler), a held
  acknowledgement released after `expireLease` (`NOT_HELD`, the first lease never runs, the second runs once), a held
  acknowledgement released after `cancel` (`CANCELLED`, the run `CANCELLED`, no handler, no review), a held acknowledgement and
  `stopWorker` (handed back), `UNIMPLEMENTED` for the call (handed back, the worker registers again at 1.7 and runs the task
  once), a host clock set 60 s ahead (acknowledges and runs as any other), a frame of two runs with one held busy (the other's
  handler waits for the call, both run once), and a collection item through the lost-answer case; `protocol-negotiation` gains a
  probe with its own server block at 30 s (accepted at 1.8 with the requirement, 1.7 without) and one at 6 s (1.7). Evidence:
  the scenario files reviewed; the harness's unit tests of the actions and of the override.
- [ ] T006-058 TypeScript (`sdks/typescript`): the proto copy and stubs, the protocol constant 1.8, `AcknowledgeTasks` in the call
  wrapper, the host procedure (acknowledge, verdicts, repeats bounded on the monotonic clock with the clamp, hand-backs, the
  fallback to 1.7, the 6 s bound of a poll and of an attempt, the heartbeat from the verdict), the `hold` and `dropFrame` hooks,
  the metrics hooks, unit tests including a skewed clock. Evidence: `npm test` and the checks of `sdks/AGENTS.md`.
- [ ] T006-059 Go (`sdks/go`), as T006-058. Evidence: `go test ./...`, `gofmt -l`, `go vet`.
- [ ] T006-060 Python (`sdks/python`), as T006-058. Evidence: the unit tests, `mypy`, `ruff`.
- [ ] T006-061 C# (`sdks/dotnet`), as T006-058. Evidence: the unit tests, `dotnet format --verify-no-changes`.
- [ ] T006-062 Java (`sdks/java`), as T006-058. Evidence: `./mvnw -q verify`.
- [ ] T006-063 Rust (`sdks/rust`, its four lock files together), as T006-058. Evidence: its unit tests, `cargo clippy`.

### Integration

- [ ] T006-064 Throughput and the bounds: the workers of `tests/performance/scalability-ha` acknowledge at 1.8; the transitions
  scenario at 1.8 against 1.7 on the same machine and day, with a network between the workers and the services; a fan-out run
  acknowledged by several hosts; `worker_task_acknowledgement_call_seconds` for calls of 64 runs and the delivery delay recorded.
  Done when 1.8 is no more than 15% below 1.7 and a full call is answered within 3 s at the 99th percentile; otherwise the build
  stops here and reports to the owner. Evidence: the section of `tests/performance/reports/scalability-ha.md` with its raw
  capture.
- [ ] T006-065 Six languages: `delivery-acknowledgement` passes in all six with a golden reviewed against what the scenario
  claims and blessed once, deliberately; the 26 scenarios that pin no minor, and the 29 pinned at 1.5 under the override, pass at
  1.8 with their existing goldens without `SAGAT_SDK_BLESS`; the scenarios pinned at 1.4 and at 1 pass as before; only
  `protocol-negotiation` and the new scenario have new goldens. Evidence: the `sdk_conformance` runs with their counts (scenarios
  times languages), skips, and the diff of the goldens.
- [ ] T006-066 Released v0.2: the upgrade scenario `provisional-lease-cutover` in `scripts/test-release-upgrade-profile.py` (a
  v0.2 worker's lease in flight across the upgrade keeps its meaning; no mark and no `NOT_ACKNOWLEDGED` before the activation;
  after it a current worker's leases are provisional; the current build replays a journal holding the cause) and its rollback case
  (the removal, the sessions expired, the marks drained, the binary rolled back to the floor, which replays the journal); the
  archived v0.2 SDKs (`SAGAT_SDK_SOURCE_ROOT`) running `run-lifecycle` against the new server with the requirement recorded.
  Evidence: the `released-upgrade` job's receipts.
- [ ] T006-067 Documents and review: the 1.8 row of `docs/sdk-compatibility.md` table 1 with its test names, the README of the
  conformance suite, the clarification of FR-037-006 confirmed against the build, and the correctness, security and architecture
  reviewers run on the merged diff with every finding answered. Evidence: the reviews saved beside this design's and the
  `/verify` output.
