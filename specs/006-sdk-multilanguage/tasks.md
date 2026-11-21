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





## Phase: Delivery acknowledgement (Architecture decision 0099, protocol 1.8; Owner decision 2026-11-20-12)

The contract is `stream-completion.md`, "Delivery acknowledgement (protocol 1.8)"; the decisions are Decision 006-040 to Decision
006-066 of `research.md`. Every test below is written first and seen failing for the right reason (AGENTS.md rule 18); races are
held with a lock or a barrier, never a sleep; every task that touches a lease has a root and a scoped row in its evidence. A task is
ticked only with the named test and its result, or the command and its output.

**Before the first task.**
- (a) Architecture decision 0098 is accepted and both of its builds are merged: the server (`bugfix/withdraw-undelivered-leases`,
  pull request #260, with its scoped withdrawal, its reservation of `TASK_NOT_STARTED`, its cap of four per attempt, its replay floor
  and its generic `capability-activate`) and the SDK hand-back (`bugfix/withdraw-leases-sdks`, with its rewrite of a handler-raised
  code). Against its merged head (2026-11-21) only request (3) of Decision 006-059 is built; (1) for the secret fence, (2), (5),
  (6), (7), (8) and (9) go into a follow-up of pull request #260. The tasks that write or decode the cause, use the secret fence,
  read a requirement's kinds or run the server end to end wait for that follow-up to merge: Task 006-045 to Task 006-047, Task
  006-050, Task 006-051, Task 006-053 to Task 006-056, Task 006-058, Task 006-065 and Task 006-066; the registry migration of Task
  006-046 sorts after the follow-up's and its closed lists carry `lease-withdrawal-v2` too, since both rewrite the same three
  checks, while the marks' migration of Task 006-044 touches no closed check. Task 006-043, Task 006-044, Task 006-048, Task
  006-049, Task 006-052 and the SDK host code with its unit tests, Task 006-059 to Task 006-064, proceed; Task 006-049 and Task 006-052 write the marks and the withdrawn rows they need by SQL fixtures until Task 006-047 and
  Task 006-050 land, and repeat those cases on real rows then, and Task 006-052 rebases onto the follow-up's capability paths,
  where request (7) puts its second check, before it merges. A
  released build of the follow-up, with (2) (the cause decoded), (7) (an engine silent past the liveness window serves no
  capability call) and (9) (`lease-withdrawal-v2` listed on both kinds, the registry's floor, which keeps out every binary without
  them whatever was released before it), is a hard precondition of Task 006-057, the task that leaves dormancy. There is no fallback (Decision 006-059). This design is rebased
  onto `develop` once that branch has merged.
- (b) Task 041-3 has merged (the 1.7 contract in `worker.proto` and its baseline) before Task 006-043, which edits the same file;
  Task 041-7 has merged (the server serves 1.7) before Task 006-057 and before the `AppendStepLogs` case of Task 006-052;
  Task 041-15 to Task 041-20 (each SDK speaks 1.7) have merged before the matching SDK task, Task 006-059 to Task 006-064.
- (c) The owner has accepted Architecture decision 0099, with the trust assumption of its decision 12, and answered Owner question
  006-002.

**Ownership** (as "Implementation ownership and integration order" above): the server owner, Task 006-043 to Task 006-057; the
shared conformance owner, Task 006-058; one owner per SDK, Task 006-059 to Task 006-064; the integrator, Task 006-065 to Task
006-067. Shared files have one owner each.

### Server

- [ ] T006-043 The canonical contract in `specs/001-platform-foundation/contracts/worker.proto` ("Added in protocol 1.8"):
  `TaskLease.acknowledge_before_start = 23`, `TaskLease.attempt_deadline_epoch_ms = 24`, `WorkerCapabilities.declines_acknowledgement = 10`, `AcknowledgeTasks` and its messages
  with `lease_time_left_ms`, the verdict enum with `CANCELLED` and `RETRY`, `TASK_NOT_ACKNOWLEDGED` in the error comment; the Rust
  descriptor baseline; every message with a field ending in `_token` in `skip_debug` (`crates/api-grpc/build.rs`) with a `Debug`
  that prints counts; every implementation of the generated trait answers the call (`WorkerGrpc`, the `Stub`s of
  `tests/contract/durable-execution/grpc/limits.rs` and `tests/contract/security-multitenancy/grpc/stub.rs`, the `Proxy`s of
  `tests/integration/database-runner/proxy.rs` and `sdks/rust/conformance/src/faults/grpc.rs`, `Fake` in
  `apps/database-runner/src/testing.rs`, `Service` in `sdks/rust/src/worker_tests.rs`; `UNIMPLEMENTED` until Task 006-049); the six
  SDK copies synchronised with each SDK's script and their checked-in stubs regenerated, with no host behaviour, as Task 041-3 does
  (Decision 006-063). Evidence: the descriptor test of `contract_grpc_durable_execution`, `cargo clippy --workspace --all-targets`,
  token canaries, and each SDK's byte-equality test of its copy (`sdks/rust/tests/protocol.rs`, the Java `SharedContractsTest` and
  the others) green.
- [ ] T006-044 The migration of the marks (the `/migration` checklist), which touches no closed capability check and may sort
  before the follow-up's: on `steps`, `lease_hand_back_generation`, `lease_offered_generation` and `lease_capability_generation`
  (`bigint`), `lease_claimed_at`, `lease_claim_incarnation` and `lease_acknowledged_at` (`timestamptz`),
  `provisional_withdrawn_attempt integer`, `withdrawn_lease_token_hash bytea` and `withdrawn_on_hand_back boolean`; on
  `worker_sessions`, `declines_acknowledgement boolean NOT NULL DEFAULT false`; no backfill, no `CHECK` on the marks; the
  table-level grants of `sagat_app` cover the columns, and the runtime role reads `pg_postmaster_start_time()` on the pinned image
  (verified, not assumed; whether a crash the postmaster recovers without restarting changes the start time is recorded); a `down`
  that refuses while a provisional mark counts and drops the columns; the storage classification; the privilege audit. The
  registry's part (the requirements read, the trigger, the closed checks) is Task 006-046's, which waits for the follow-up.
  Evidence: the forward test, the refused `down`, the start time read as the runtime role, the privilege audit of
  `integration_platform_foundation`, also as the runtime role.
- [ ] T006-045 The cause `NOT_ACKNOWLEDGED` of `step.lease_withdrawn` confirmed in the event, the replay, the reset copy, the codec of
  `crates/infra-postgres/src/event_codec.rs`, the events contract and the Studio's closed list of causes, as the follow-up of pull
  request #260 added it (request (2) of Decision 006-059). Evidence: codec round trip, a fixture holding the cause in
  `crates/domain/tests/legacy_replay.rs`, reset copying it.
- [ ] T006-046 The registry, dormant: the capability `delivery-acknowledgement-v1` known to the registry but listed by no engine yet,
  and no activation entry yet (Task 006-057 adds it); its migration (the `/migration` checklist), sorted after the follow-up's:
  `delivery_acknowledgement_required()` (`SECURITY DEFINER`, `search_path = pg_catalog, pg_temp`, `public.` names, both requirement
  rows `FOR SHARE` in removal order, returning `lease-withdrawal-v2` as recorded only for both kinds, `delivery-acknowledgement-v1`
  for engines and its `required_since`, revoked from `PUBLIC` and granted to `sagat_app`); the trigger on `platform_requirements`,
  enabled `ALWAYS`, which takes the epoch row `FOR UPDATE` itself (`BEFORE INSERT`: refusing `delivery-acknowledgement-v1` while
  `lease-withdrawal-v2` is not recorded for both kinds or while any row of `serving_instances` lacks it; `BEFORE DELETE`: refusing
  to remove `lease-withdrawal-v2` while `delivery-acknowledgement-v1` is recorded); the three closed capability checks extended
  `NOT VALID` and then validated with `delivery-acknowledgement-v1` and the follow-up's `lease-withdrawal-v2`, the variant entering
  `Capability::ALL` in the same task so the check of Decision 006-033 holds; a `down` that refuses while the history names the
  capability or a live process lists it, drops the trigger, its function and the requirements read by full signature, and
  restores the closed checks to the follow-up's lists, `lease-withdrawal-v2` kept; this build keeps request (7) of Decision 006-059 as the follow-up built it and
  request (9) (`lease-withdrawal-v2` listed on both kinds, its activation entry and its replay-floor guard) as it built them (an
  engine silent for 25 s, from the start of its last successful beat, serves no capability call until a beat that reports no change
  in progress and its gate's decision succeed; the registry connection's statement timeout at most 5 s; the beat
  `instance_heartbeat_reporting_change` taking the epoch row `FOR SHARE SKIP LOCKED` first and reading it plainly only when it is
  held, with whether it is held; a committed
  change closing the gate of a process that lacks it; the refusals counted in
  `worker_capability_calls_refused_total{method, reason="REGISTRY_SILENT"}` and the beats in
  `instance_heartbeats_change_in_progress_total{kind}`; the second check before the first byte of a call that hands data out and before the write of one that
  writes; heartbeats, reports and claims flow); `docs/runbooks/lease-withdrawal.md` and section 5 of
  `docs/runbooks/instance-registry.md` updated (the kinds of each requirement, the removal order, that the wrong order costs only
  marks, and never to leave the owner transaction open), section 3 checked as the floor build updated it; "Refined by" pointers for
  Architecture decision 0098 decisions 5 and 8 prepared. Evidence: an owner transaction holding `platform_requirement_lock()` for 60 s:
  no gate closes, no readiness failure, heartbeats and reports flow, serving engines keep serving `ResolveSecrets` and an engine that
  was silent for 31 s before the hold keeps refusing it with `REGISTRY_SILENT` until after the commit and its own decision; an engine
  whose registry connection fails while its pool works serves heartbeats and refuses `ResolveSecrets` after 25 s, and serves it again
  once its gate is decided; the boundary test with the beat's answer held; the registry connection's statement timeout read back as
  at most 5 s, with no configuration refused; the beat's `prosecdef`, `proconfig` and `PUBLIC`'s missing `EXECUTE` as the runtime role; a missing epoch
  row raising; the function's result for each recording (both kinds, engines only, none); the trigger refusing an insert of
  `delivery-acknowledgement-v1` and a removal of `lease-withdrawal-v2` by hand SQL in each case and accepting them otherwise; each
  refused `down`; a requirement activated while a process that lacks it is lapsed, then the epoch held by a second owner transaction:
  the process's first beat after it comes back closes its gate; `lease-withdrawal-v2` refused removal while a journal holds
  `step.lease_withdrawn` with the cause `NOT_ACKNOWLEDGED` and allowed while it holds only the causes of Architecture decision 0098;
  a barrier holding the beat between its locked read and its commit while an activation waits (the activation, in `READ
  COMMITTED` with its census after the lock, then counts the engine live and refuses with `INSTANCES_LACK_CAPABILITY` naming it),
  and an activation holding the row during the beat (the engine reports a change in progress and keeps refusing); the runbook's forward order and its rule that a silent row is
  removed only once its process is confirmed stopped; the capability unknown to the activation command.
- [ ] T006-047 The claim writes the marks, in each of the three lease statements (`upsert_step`, `upsert_scoped_instance`,
  `upsert_path_instance`): `lease_hand_back_generation`, `lease_claimed_at` and `lease_claim_incarnation` for a session at 1.8 or later; the
  provisional mark and the flag under the conditions of the contract (session at 1.8 or later not declining, both requirements known
  as recorded for their kinds, engine switch on, `provisional_withdrawn_attempt` not the attempt claimed, earlier deadline at least
  18 s after the claim), and the first skip reason that holds otherwise, returned by the statement; the frame and the poll answer of a flagged lease
  carrying `attempt_deadline_epoch_ms` when its attempt deadline comes before its expiry, the stored expiry unchanged; the claim's assignment
  conditioned in SQL on a new lease; the engine's other statements keep or clear the marks. Evidence: a matrix test in
  `integration_durable_execution` (minors 1.4, 1.7 and 1.8; declining or not; each requirement recorded for both kinds, for engines
  only, or not; switch on or off; the attempt flag at the claimed attempt N, at N − 1 and null; time-to-live 17 and 18 s; `timeoutMs` 17 and 18 s; each
  statement; root and scoped); a lease written afterwards with the old column list is not marked; a row whose incarnation is other
  or null, or claimed before the requirement's current recording, does not count for a withdrawal; the static check in
  `scripts/check-architecture.py` of every assignment to a mark, with negative tests for a bound-flag form and a read-back form, and
  for the attempt flag an assignment outside the three named functions, `THEN $m`, a branch that yields `NULL`, a form without the
  old-row fence and a second assignment of the column in one statement; the
  barrier test with a run transaction loaded before an acknowledgement and writing the row through each statement that can touch a
  `LEASED` row; the parity test of the macro's two forms against the pure function; the unchanged `LEASE` line of Task 006-055; a flagged lease with a 20 s `timeoutMs` and a 60 s time-to-live carrying its attempt
  deadline 20 s after the claim, and one with a 60 s `timeoutMs` carrying none.
- [ ] T006-048 Registration: `declines_acknowledgement` stored on the session from each registration (the upsert sets it from the
  registration, so one without it clears it) and read by the claim; the negotiation code that answers 8, written and tested with the
  server's constant still at 7. Evidence: the ceiling unit tests of `crates/application/src/worker.rs` with the constant raised in the
  test only, `tests/contract/sdk-multilanguage/grpc/negotiation.rs`, a registration that declines and a later one that does not.
- [ ] T006-049 `AcknowledgeTasks`: the application service in a module of its own (so the test selector can match it by path), with the
  lease identity type `Heartbeat` uses; the binding checks extracted into one function over a narrow trait, shared with the run
  transactions; the call's deadline from its arrival in the outermost layer of the gRPC server, before the per-connection
  concurrency limit, pool acquisition bounded by it, checked before each statement with the
  timeouts set once to 1 s, a rollback and the connection returned at the deadline, answering `UNAVAILABLE`
  (`ACKNOWLEDGEMENT_DEADLINE`), every server-made status carrying the envelope `x-sagat-error`; the shed of the class answering `RESOURCE_EXHAUSTED`
  (`ACKNOWLEDGEMENT_SHED`); the port `AcknowledgementTx` and its PostgreSQL adapter (the one settings
  statement, with `synchronous_commit` raised to `local` when it is `off`, `matched`, the binding and
  membership checks under their locks, `lock_matched` by primary key with `FOR NO KEY UPDATE OF steps SKIP LOCKED` in primary-key
  order, the step's kind, the run's stop reason, the server's start time and the clock as columns, the decision on the locked rows, the deadline check,
  `acknowledge`); the bounded audits through the gate; the gRPC handler in the class `Acknowledge`; the request bounds; the counters.
  Evidence: a test per verdict and reason (acknowledged, duplicate, recovered, too late before each deadline, withdrawn, replaced,
  unmarked, wrong attempt or generation, a forged token with no lock taken, a foreign tenant's run audited once per call and the gate
  full, a revoked binding answered `unbound` and audited once, two namespaces refused, a cancellation requested and a run stopping
  answered `CANCELLED` with nothing written, a row held by another transaction answered `RETRY` while the others are acknowledged, a
  row share-locked by a foreign-key check not skipped, a deadlock answered as a retryable failure and a repeat answered `duplicate`, a
  slow binding lock and a slow membership lock together past the call's deadline committing nothing, a pool held full for 5 s
  answered `UNAVAILABLE` (`ACKNOWLEDGEMENT_STORE_UNREACHABLE`) within 4.5 s with no connection discarded, also when the
  authentication's lookups find the pool full after the token verified, a read-only server (`25006`) answered the same, a deadline
  passed after the transaction began answered `ACKNOWLEDGEMENT_DEADLINE`, and a call that reached the handler with too little time
  left after the concurrency layer answered `ACKNOWLEDGEMENT_DEADLINE`, the answer reporting `attempt_time_left_ms` when the attempt has
  a deadline, a test that every `UNAVAILABLE` and `RESOURCE_EXHAUSTED` the handler or its admission returns carries the envelope, a per-connection counter in `CappedConnectInfo` for the conformance assertion of Task 006-058; a connection
  with more than 128 calls and one with 64 transfers in flight answered within the bound with the compiled minor raised in
  the test (tonic's per-connection limit replaced by the platform's own layer under the deadline's), and, with the minor at 7, a
  start with a release-0.2 configuration giving every class, `Acknowledge` included, 64 places per connection, a probe answered `NOT_HELD` with the reason `withdrawn`, a compensation of a cancelled run and a reconciliation under a stop answered
  `ACKNOWLEDGED`, a
  repeated acknowledgement and a lease whose mark does not count each answered `ACKNOWLEDGED` with the margin and `CANCELLED`
  (`too_late`, `stop_asked`) without it or under a stop, and, with `ALTER ROLE sagat_app SET synchronous_commit = off`, the
  acknowledgement's transaction committing with `local`, read from a test hook of the adapter); both the run transactions and the
  acknowledgement refuse the same narrowed binding; the admission properties for limits 1 to 64; the runtime role.
- [ ] T006-050 The withdrawals: the lost-lease pass withdraws a provisional lease at the first of its expiry and attempt deadline
  (`NOT_ACKNOWLEDGED`); every withdrawal of a marked lease (the pass's, the hand-back's, `NOT_SENT`) takes the report path's binding
  checks (the pass has none), reads both requirements under the locks their removals wait for (one statement, returned as the
  witness `RequirementsHeld`, which the decision function and the application's withdrawal services take instead of booleans), then re-reads
  the row `FOR UPDATE` and decides with the pure function, and the domain's `withdraw_lease` applies the cap from the journal count; the
  hand-back of a lease whose mark counts that is `LEASED` with no capability served, provisional or acknowledged, withdrawn
  (`NOT_STARTED`); that withdrawal, and any that brings the attempt to the cap, set `provisional_withdrawn_attempt` to the stored
  attempt in the one fenced form that the three projection statements carry for every caller, `$flag` bound per instance through the
  port method `mark_withdrawn_attempt(&instance, generation)` of the run and path transactions (the `ExecutionInstanceId`, the root
  instance for a root step) and false for every other call, a mark never consumed failing the commit; in the pass
  every outcome other than a withdrawal ends the attempt with `expire_attempt` in the same transaction; the switch not consulted for a
  marked lease; the hand-back of an unmarked 1.8 lease withdrawn only while `delivery-acknowledgement-v1` is recorded and for a lease
  claimed after its recording on the current incarnation; a `NOT_SENT` withdrawal of a marked lease needing only
  `lease-withdrawal-v1`, every other withdrawal of a marked lease `lease-withdrawal-v2`; the hand-back of an acknowledged `LEASED` lease withdrawn with a stop asked or past its expiry, through the
  report fence of a step with declared secrets too;
  `withdrawn_lease_token_hash` kept by every withdrawal of a marked lease, `NOT_STARTED` included; the hand-back decided from
  `lease_hand_back_generation` (replacing, from Task 006-057, the raised stored minor of the server build's tests), with the expiry
  condition lifted for a `LEASED` lease whose mark counts, provisional or acknowledged; a lease claimed on another incarnation, a
  `RUNNING` step, or a lease to which a capability was served, handed back: an ordinary failure; the attempt flag in its fenced form
  in the three projection statements, `$flag` bound by the application's withdrawal service alone. Evidence: the unit
  test of the pure function over every combination of status, marks, incarnation, recording and deadlines; a provisional `Activity` lease past its
  expiry, and one past its attempt deadline, withdrawn with no review and the attempt reused, root and scoped; an acknowledged one
  expired as an unknown outcome; a lease handed back once and the next lease of its attempt unflagged, so that with every
  acknowledgement failing at the server the step runs on its second lease and no attempt is spent, at an 18 s and a 30 s time-to-live
  and with a namespace lock held, also with a re-hint and a run transaction touching the `READY` step and a `NOT_SENT` withdrawal of
  the second lease before the next claim; the column equal to the attempt `step.lease_withdrawn` carries; four `NOT_SENT`
  withdrawals and the fifth lease unflagged; a committed acknowledgement whose answer is dropped, repeated under the margin, with
  `maxAttempts = 1`, withdrawn on its hand-back and run on the next lease; a cancelled run's flagged compensation acknowledged, run
  once, never withdrawn; after four withdrawals of the attempt, and with a requirement removed, a marked lease past its expiry
  is ended by one pass with `expire_attempt` (`capped`, `lapsed`) and not picked again by the next, and its hand-back spends the
  attempt; at the cap, the pass and the hand-back in both orders; a hand-back of an unmarked 1.8 lease before the activation, an
  ordinary failure; a removal and a re-activation, after which a lease claimed before the new recording is an ordinary failure on
  hand-back; the hand-back after `CANCELLED` (`stop_asked`, and `too_late` 1 ms past the expiry with the pass held) of an
  acknowledged `LEASED` lease with declared secrets, withdrawn and the step cancelled under the stop; a recovered lease answered
  `CANCELLED` and handed back, an ordinary failure; two leases of one run withdrawn in one transaction, the second's settle touching
  the first's row, the flag left at the first's attempt; two `LEASED` siblings with equal generations, one withdrawn, the other
  touched by the same append, the other's flag unchanged, and the same for two items of one collection node at equal generations
  through `upsert_scoped_instance` and `upsert_path_instance`; a withdrawal service that calls `mark_withdrawn_attempt` for an
  instance no call writes, refused at the commit; the domain property test that a stop is never planned for a compensation
  or reconciliation; a `CompleteStep` after the `NOT_STARTED` withdrawal of an acknowledged lease
  counted and audited as a call after a withdrawal; a `Heartbeat` after it counted `heartbeat_after_hand_back`, and one after a
  `NOT_ACKNOWLEDGED` withdrawal counted `heartbeat`; an engine with the switch off withdraws a
  marked lease; a hand-back after its session registered again at 1.7 under the
  same name, still withdrawn; a hand-back of a lease claimed at 1.7 while the session row says 1.8, an ordinary failure; a stopping
  collection drains a withdrawn instance; a cancelled step's hand-back with declared secrets withdraws and cancels.
- [ ] T006-051 The races, in `tests/fault/sdk-multilanguage/`: the acknowledgement holds the row while the pass's withdrawal waits, and
  the reverse; the same with the hand-back's withdrawal; a repeated acknowledgement while the first is held before its commit (the
  second answers `RETRY`, then `ACKNOWLEDGED`); an acknowledgement and a hand-back of the same lease in both orders; a holder that only locks the row; an acknowledgement after a deadline and before the
  pass; a hand-back 1 ms after the expiry with the pass held (withdrawn, attempt not spent), and the same after `CANCELLED`; an
  acknowledged `LEASED` lease 1 ms past its expiry with its hand-back and the pass in both orders (exactly one of `step.lease_withdrawn`
  and `expire_attempt`); a
  capability call against a hand-back, in both orders; the removal of each requirement held between the pass's read and its commit,
  in both orders, with an empty journal, and both removals in one transaction of hand SQL (a detected deadlock at worst, no event
  without its floor); a binding revocation queued between a withdrawal's binding checks and its re-read,
  and against an acknowledgement. Evidence: barrier tests watched in `pg_stat_activity`, each asserting the journal and that the
  handler would run exactly once, root and scoped.
- [ ] T006-052 The capability refusal and record: the fence variants (root and path) called only from the capability paths, after the
  token and the binding matched; `lease_capability_generation` written once per generation by a conditional update, under the run or
  path lock, committed with `synchronous_commit` at least `local` before anything is handed over or accepted, the call refused when the lease is no longer held, for each
  method in the transaction Decision 006-050 names (root `WriteBlob`'s check transaction now commits); the descriptor-driven contract
  test, walking nested, `oneof` and repeated fields, failing for a lease-token method with no case and for a message with a field
  ending in `_token` outside `skip_debug`. Evidence: that test in `contract_grpc_durable_execution` (refused before, served after the
  acknowledgement, recorded before the serve, root and scoped; `WriteBlob` root and scoped followed by a hand-back, an ordinary
  failure and a violation; the `AppendStepLogs` case once Task 041-7 has merged; with `ALTER ROLE sagat_app SET synchronous_commit =
  off`, the record committing with `local`); a report on a provisional lease is served; a wrong
  token gets the ordinary stale answer.
- [ ] T006-053 What the server sees of the trust assumption: the violations of Decision 006-053, and calls after a withdrawal, counted and
  audited once per step and generation by the identical-record rule, the refusal path committing its audit; hand-backs and
  acknowledgements exempt and counted apart. Evidence: a test for each, including a give-up hand-back before and after the withdrawal
  and an acknowledgement after it, none of which is a violation; a heartbeat after a `NOT_ACKNOWLEDGED` withdrawal raising `SagatCallsAfterWithdrawal` and one
  after a hand-back's withdrawal not raising it, the kind taken from `withdrawn_on_hand_back`; 100 refused capability calls on one provisional lease give one audit
  record, and 100 stale calls with one withdrawn token and 100 different request generations give one.
- [ ] T006-054 Observability and operations: the counters (with `worker_task_acknowledgement_failures_total{reason}`), the two
  histograms, the gauges (`FILTER` clauses of the operations statistics query, each defined by the macro, read on the primary), the eight alerts (`SagatLeaseAttemptCapped` and `SagatCapabilitiesRefusedRegistrySilent` owned here, the `stop` hand-backs excluded
  from `SagatAcknowledgementsFailing` through the new `mark` and `stop` labels of `worker_leases_undelivered_total`, the ratios over
  the new `worker_lease_marks_total`) with `promtool test rules` cases at the default and at an 18 s time-to-live (added to the CI step), the symptoms in
  `docs/runbooks/sdks.md` (naming that each 1.8 host holds two connections, so the acknowledgement connection refused at
  `max_connections_per_peer` or `max_connections` leads to the fallback, with `grpc_connections_refused_total{reason="per_peer"}`
  rising, and that a fleet behind shared addresses needs the per-peer cap doubled; naming `synchronous_commit` and `requirement_missing`, and that `SagatLeaseAttemptCapped` and
  `SagatCapabilitiesRefusedRegistrySilent` come with this build, so a fleet of the floor build alone has neither); in section 7 of
  `docs/runbooks/disaster-recovery.md`, the engines stopped before and PostgreSQL restarted after a restore that does not start the
  server, checked by `pg_postmaster_start_time()`, replacing the marker "(pending Task 006-054)", with the topologies the rule is proven on and the
  assumption of one writable server; in `infra/dr/drill.sh`, the start time read before and after each disaster, after a
  snapshot-style restart and after that restart, and a provisional mark held at the kill not withdrawn after the promotion; a row
  per new rule in the fitness-function table of `docs/engineering/architecture-principles.md` (the witness type and the attempt
  flag's allowed set among them); a check in `scripts/check-design-docs.py` that the index of `docs/adr/README.md` has one row per
  record whose status starts with the record's first clause, and line 13 of that README naming the script; a rule in `scripts/check-design-docs.py` that fails on a "(pending Task NNN-xxx" marker whose task
  is ticked.
  Evidence: a telemetry test with bounded labels and a token canary, the `promtool` output, and one drill run.
- [ ] T006-055 The budget (`tests/integration/claim-transition-efficiency/work_budget.rs`): a run of three chained activities at 1.8 with
  the requirement recorded; `LEASE` and both completion lines unchanged; new lines `ACKNOWLEDGEMENT` (one lease) and `FIRST_HEARTBEAT`
  pinned at their measured counts, asserting no run lock, no scan of the run's steps, no journal or outbox insert, and fewer statements
  than `FIRST_HEARTBEAT`; a call of 64 leases itemised, with the triggers it fires named. Evidence: two equal passes and the itemised
  statements in the task.
- [ ] T006-056 The gate, before any SDK work and before 1.8 is offered: the workers of `tests/performance/scalability-ha` acknowledge with
  the minor raised in the test; the transitions scenario at 1.8 against 1.7 on the same machine and day, with a network between the
  workers and the services, the median of five interleaved runs of each; a fan-out run acknowledged by several hosts; a burst that fills
  the admission reserve; the call duration for 64 leases, the heartbeat's 99th percentile in the burst, the delivery delay, the rise of
  ready to handler entry and the share of claims unmarked as `short_deadline`. Done when the
  transitions runs at 1.8 are no more than 15% below 1.7, the timed 64-lease calls are within 3 s at the 99th percentile over every call with
  fewer than 0.1% answered other than `OK`, and, in the burst run, no heartbeat is shed while acknowledgements are admitted and the
  heartbeat's 99th percentile is under 2 s; the pass's withdrawal rate in a burst of provisional expiries (a worker fleet cut off) is
  reported; a 5-minute injection of a statement timeout on the acknowledgement alone, a 15 s database failover (the time-to-live and
  the heartbeat cadence stated), a 10 s burst and a 5-minute run of sheds while heartbeats run, reporting the hand-backs, the second claims, the sheds of the class `Lease`, the heartbeat's 99th
  percentile, the throughput, the reviews (none) and the time to recover and the connections per host; otherwise the build stops here, the minor stays at 7, the feature stays dormant, and the owner decides (Decision
  006-057). Evidence: the section of `tests/performance/reports/scalability-ha.md` with its raw capture, and the result recorded in
  Architecture decision 0099.
- [ ] T006-057 The feature leaves dormancy, only once a released build of the follow-up of pull request #260 has requests (2), (7)
  and (9) of Decision 006-059: `PROTOCOL_MINOR` and `DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR` at 8; every engine of the
  build lists `delivery-acknowledgement-v1`, required of engines only; its activation entry, with `empty_fleet: false` and the field
  `requires` of `Activatable`, refusing while `lease-withdrawal-v2` is not recorded for both kinds and while any registered instance, live or lapsed, lacks it;
  the reserve of 4 places for the class `Acknowledge`, which the raised minor turns on; the tests of the hand-back use real 1.8
  sessions. Evidence: the ceiling tests,
  `apps/execution-engine/tests/registry.rs` lists the capability and the control plane's registry test does not; the command refused
  the capability as not activatable before this task (a test of Task 006-046), refuses it without `lease-withdrawal-v2` for both
  kinds and with a lapsed instance of a build of pull request #260 that does not list it, by the command and by hand SQL alike, and an activation passes on a fleet of
  the build; the per-connection reserve in force; the tests of Task 006-050 pass with a real 1.8 session.

### Conformance and SDKs

- [ ] T006-058 The shared harness (`tests/contract/sdk-conformance/`): the fault actions `hold` (a call kept at the transport hook until
  the scenario releases it) and `dropFrame` (one frame of a stream lost); the platform booted with `lease-withdrawal-v1`, `lease-withdrawal-v2` and
  `delivery-acknowledgement-v1` recorded, and a scenario attribute that boots without them; a host setting that shortens the decline
  period and one that sets the acknowledgement's first backoff step, so a lease can be held in its backoff across a promotion; an
  assertion that the server sees the acknowledgements on another connection than the stream's, by a per-connection counter that
  Task 006-049 adds to `CappedConnectInfo` (never an address) and a test hook of the conformance platform exposes, with the Rust
  runner's fault proxy opening one upstream connection per downstream connection; a platform of two engines, one of whose pools can
  be held unreachable whole (its authentication's lookups included), with a harness proxy routing the host's second connection to
  it; the override `SAGAT_SDK_PROTOCOL_OVERRIDE=1.8`, which raises host scenarios pinned at 1.4 or 1.5, runs with step logs off,
  compares only the negotiated `protocolVersion` against the pinned one and, for 1.4, the lease's instance identity, asserts that
  each raised scenario acknowledged a flagged lease (but `protocol-negotiation`, `worker-stream-faults` and
  `worker-transport-faults`, with their 3 s time-to-live) and that no unexpected `TASK_NOT_STARTED` was sent, and lists by name the
  scenarios that hand back by design (found by a run without the override, `worker-shutdown` expected among them), which run without
  it; the scenario `delivery-acknowledgement.json`: a lost `PollTasks` answer then `expireLease` (withdrawn, run once, attempt 1
  twice, no review), a dropped stream frame (the same), a lost acknowledgement answer (one handler, `faultsApplied` 1), an
  acknowledgement that never leaves twice (one handler), a held acknowledgement released after `expireLease` (`NOT_HELD`, the first
  lease never runs, the second runs once), a held acknowledgement released after `cancel` (`CANCELLED`, handed back, the run
  `CANCELLED`, no handler, no review), a held acknowledgement and `stopWorker` released after the stop (the task runs),
  `UNIMPLEMENTED` for the call (handed back, the worker registers again at 1.8 declining and runs the task once, unflagged, then
  registers again without declining after the shortened period), a flagged frame delivered after the declining registration (handed
  back, nothing run), a 30 s `UNAVAILABLE` outage of the server with three flagged frames in flight (no fallback), three calls whose
  every attempt is held unanswered while the stream is answered (the fallback at the third counted call, the probes held too), an
  outage of every call ending 3 s after the give-ups at an 18 s and a 30 s time-to-live (no fallback, the probes answered), two
  minutes of a 503 without the server's envelope for this call alone (the fallback, no attempt spent), the same with `max_inflight` equal to the frame
  size so that only the hand-backs are answered and at an 18 s time-to-live (the fallback, no attempt spent), frames received faster
  than one a second with the call swallowed (the fallback), a 15 s database failover with flagged leases waiting, one of them held in its backoff across the promotion
  and a frame arriving right after it (no early give-up, no review, no attempt spent, the leases run as `recovered`, with a
  synchronous standby), the acknowledgement's connection on the engine held unreachable after a promotion while the other answers
  heartbeats (`ACKNOWLEDGEMENT_STORE_UNREACHABLE`, no give-up, then `recovered`), a proxy that cuts the call at 1 s (each lease given up at the end of its first
  attempt after a heartbeat answered, once one receive bound has passed, no fallback, a running task heartbeating, the probe not
  delayed), the 15 s failover at a 30 s time-to-live with a running task
  heartbeating 20 ms before the failure (no early give-up) and at an 18 s time-to-live (the outcome of Gap 006-047, an ordinary failure or a review, reported), a host
  whose only work is flagged leases, with its slots full so that no frame arrives (no early give-up), the fallback at an 18 s
  time-to-live with the pass running every second, two minutes of answers with the envelope (no fallback, each lease given up at
  the end of its first attempt after a heartbeat answered, handed back once and run on its next lease, no attempt spent), a compensation of a cancelled run
  (acknowledged and run), an `ACKNOWLEDGED` with under 3 s before the expiry after the attempt (the
  first heartbeat sent before the handler) and one with under 2 s before the attempt deadline (handed back), a
  hand-back that lands after the expiry (withdrawn), a host clock set 60 s ahead and one 60 s behind, a configured heartbeat interval of 10 s, secrets and an input
  blob read after `ACKNOWLEDGED`, a failure retried at 1.8, a handler that raises `TASK_NOT_STARTED` (reported `WORKER_ERROR`), a
  hand-back held across a re-registration (still withdrawn), a lease with a 10 s `timeoutMs` (unflagged), and a collection item
  through the lost-answer case; `RETRY` is a server test (Task 006-049) and an SDK unit test, since the harness holds no row, and
  so is the repeat on `REGISTRY_SILENT`, since the harness cannot silence a registry.
  Evidence: the scenario reviewed; the harness's unit tests of the actions and of the override's comparisons and assertions.
- [ ] T006-059 TypeScript (`sdks/typescript`): the protocol constant 1.8 (the proto copy and stubs came with Task 006-043),
  `AcknowledgeTasks` in the call wrapper, the host procedure (the flag, never dispatched without `ACKNOWLEDGED` even while declining,
  the acknowledgement concurrent with the next receive, on a transport connection of its own (the SDK's means of §6.2), built by the same function
  as the main connection, with at most 4 calls in flight under the SDK's own permit taken per attempt, retries taking a free permit
  before new frames, waiting frames merged into a new call up to 64 leases, the window and the give-up kept per lease, the
  connection dialled again after an attempt with no answer (a transport failure, or the host's own deadline with no status) while
  the main connection answers, or after two `ACKNOWLEDGEMENT_STORE_UNREACHABLE` in a row while a call on the main connection was
  answered `OK` after them, the old connection drained and an attempt ended by the host's own close sent again and counted for
  nothing, the verdicts with `RETRY` asked again together on its own backoff, the repeats
  of §5 with the split after a failure carrying the server's envelope, the give-up on the monotonic clock with its clamp and never
  while an attempt is in flight, hand-backs, the bounded fallback with its triggers (a closed list of refusals; three counted calls
  in the run since the last `ACKNOWLEDGED`, given up with no answer or without the envelope and whose probe, sent after a hand-back
  answered with the envelope, failed the same way; a transient answer with the envelope never counting; the early give-up decided by an attempt
  sent after the evidence, another call reaching the database answered `OK` or a frame arriving after a failed attempt, and given
  up only when that attempt fails too with neither a shed nor `ACKNOWLEDGEMENT_STORE_UNREACHABLE`, no shed before, one receive
  bound passed and no try of another call failed with the statuses of §6.2 since the evidence, attempts keeping the backoff and any
  hint and sent early, after up to 200 ms, only once per lease per receive bound and only when a give-up could follow, a `RETRY`
  restarting the window; capability calls repeated on `REGISTRY_SILENT` within the handler's deadline), which waits a random delay, waits for
  its hand-backs, registers again declining at its own minor and again without declining after its period, the stop that waits for
  a verdict within its timeout, the 6 s bound of a poll and of an attempt, the first heartbeat at a third of `lease_time_left_ms`
  less the attempt's duration, sent before the handler under 3 s, and the lease handed back under 2 s before the attempt deadline; the
  give-up counted from the earlier of the expiry and `attempt_deadline_epoch_ms`; a late `ACKNOWLEDGED` arriving after the give-up is decided never both run and handed
  back), the `hold` and `dropFrame` hooks, the metrics hooks, unit tests
  including clocks ahead and behind, a slow answer, `RETRY`, an outage that does not fall back, a shed that does not fall back, an
  idle stream with full slots whose hand-backs are the only answers, a long configured interval, a lease in backoff when the first `OK` after a run of failures arrives (its next attempt answered `ACKNOWLEDGED`, run, not handed
  back, held with a barrier), a deciding attempt answered `ACKNOWLEDGEMENT_STORE_UNREACHABLE` deciding nothing, a burst of sheds
  with heartbeats answered every 100 ms keeping the backoff's count of attempts, attempts failing in 5 ms with heartbeats answered
  every 100 ms making no more attempts in the first 6 s than §5's count plus one, two flagged calls failing together while
  heartbeats are answered both given up early, twelve single-lease frames with every attempt failing at 6 s all given up early, none by the
  ordinary give-up, the last within three attempts' time plus 1 s of the first frame, a half-open acknowledgement connection
  dialled again with no fallback, a redial with three other attempts in flight (none given up, none counted), the two connections
  on different transports (their local socket addresses compared), the acknowledgement connection against a TLS server with a
  private certificate authority that requires a client certificate (it succeeds, presents the same certificate, and the user's
  interceptor sees the call), a 3 s call timeout on a stream host with the acknowledgement failing alone (given up early), a fifth acknowledgement waiting for the permit and not counted,
  and a token canary of the request. Evidence: `npm test` and the checks of `sdks/AGENTS.md`.
- [ ] T006-060 Go (`sdks/go`), as T006-059. Evidence: `go test ./...`, `gofmt -l`, `go vet`.
- [ ] T006-061 Python (`sdks/python`), as T006-059. Evidence: the unit tests, `mypy`, `ruff`.
- [ ] T006-062 C# (`sdks/dotnet`), as T006-059. Evidence: the unit tests, `dotnet format --verify-no-changes`.
- [ ] T006-063 Java (`sdks/java`), as T006-059. Evidence: `./mvnw -q verify`.
- [ ] T006-064 Rust (`sdks/rust`, its four lock files together), as T006-059. Evidence: its unit tests, `cargo clippy`.

### Integration

- [ ] T006-065 Six languages: `delivery-acknowledgement` passes in all six with a golden reviewed against what the scenario claims and
  blessed once, deliberately; under the override the host scenarios other than `protocol-negotiation` and those that hand back by design
  pass at 1.8 with their existing goldens without `SAGAT_SDK_BLESS`, each having acknowledged a flagged lease; without it every scenario
  passes as before; `protocol-negotiation` gets its new golden for the 1.8 offer; an informative re-run of `sdk-streaming` at 1.8 is
  recorded. Evidence: the `sdk_conformance` runs with their counts (scenarios times languages), skips, the diff of the goldens, and the
  report section.
- [ ] T006-066 Released v0.2 and the older hosts: the upgrade scenario `provisional-lease-cutover` in
  `scripts/test-release-upgrade-profile.py` (a v0.2 worker's lease in flight across the upgrade keeps its meaning; no flag before the
  activation; after it a current worker's leases are flagged; the current build replays a journal holding the cause) and its rollback
  case (the switch set off and confirmed, the provisional leases drained while a long acknowledged task runs and a 1.8 worker keeps
  working, the requirement removed, the binary rolled back to the floor, which replays the journal, the switch set back and the floor
  then withdrawing an unmarked lease; a session stored at 8
  whose hand-back on the floor is an ordinary failure, once after a capability was served and once without; a binary below the floor
  refused by the registry, an engine and a control plane of a build of pull request #260 without its follow-up among them, while a
  `NOT_ACKNOWLEDGED` withdrawal exists, from a third pinned phase at #260's merge commit, or its release tag if one exists, with its
  own source-manifest check: the binary lists `lease-withdrawal-v1`, is refused `CAPABILITY_MISSING` with `out_missing =
  {lease-withdrawal-v2}`, and, registered before the recording and held silent past the 30 s window at it, closes its gate at its
  next beat); a mixed fleet before the activation and after the removal, the stream on an engine of this build and
  `ResolveSecrets` on an engine of the floor build, the hand-back received by this build (an ordinary failure); an engine of the
  floor build held silent across an activation refusing worker calls after it reconnects until its beat; a v0.2 handler that raises `TASK_NOT_STARTED` (an
  ordinary failure); a host of the SDK build of
  Architecture decision 0098 at 1.5 that hands back (an ordinary failure, the attempt spent). Evidence: the `released-upgrade` job's
  receipts.
- [ ] T006-067 Documents and review: the 1.8 row of `docs/sdk-compatibility.md` table 1 with its test names, the README of the
  conformance suite, the workflow-document reference on the wall time of one attempt under losses, the clarifications of Requirement
  037-006 confirmed against the build (the wording of Decision 006-045: not yet `RUNNING`, no capability served), and the correctness, security and architecture reviewers run on the merged diff with every
  finding answered. Evidence: the reviews saved beside this design's and the `/verify` output.
